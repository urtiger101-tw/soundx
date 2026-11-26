use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    responses: Receiver<Value>,
    id: u64,
}
impl Client {
    fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_soundx"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (send, responses) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = serde_json::from_str(&line.unwrap())
                    .expect("stdout must contain only MCP JSON");
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            responses,
            id: 0,
        }
    }
    fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &value).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
    }
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}));
        let reply = self
            .responses
            .recv_timeout(Duration::from_secs(15))
            .expect("MCP response timed out");
        assert_eq!(reply["id"], self.id);
        reply
    }
    fn initialize(&mut self) {
        let reply = self.request("initialize", json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"soundx-test","version":"1"}}));
        assert_eq!(reply["result"]["serverInfo"]["name"], "soundx");
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }
    /// Send a request without waiting; returns its id.
    fn send_request(&mut self, method: &str, params: Value) -> u64 {
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}));
        self.id
    }
    fn recv(&mut self, seconds: u64) -> Value {
        self.responses
            .recv_timeout(Duration::from_secs(seconds))
            .expect("MCP response timed out")
    }
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":arguments}))["result"].clone()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.input.take();
        let start = Instant::now();
        while self.child.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
        }
        self.child.wait().unwrap();
    }
}

#[test]
fn mcp_stdio_processes_real_audio_and_reports_failures() {
    let root = std::env::temp_dir().join(format!(
        "soundx-mcp-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let input = root.join("語音 sample.wav");
    let output = root.join("converted.flac");
    let mut client = Client::new();
    client.initialize();
    assert_eq!(
        client.request("tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        if cfg!(windows) { 9 } else { 5 }
    );
    let result = client.tool("soundx_run", json!({"arguments":["synth",input,"--duration","0.05","--waveform","silence","--stat-json"]}));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["exit_code"], 0);
    let result = client.tool("soundx_run", json!({"arguments":["convert",input,output]}));
    assert_eq!(result["isError"], false, "{result}");
    assert!(output.is_file());
    let result = client.tool("soundx_run", json!({"arguments":["info",output,"--json"]}));
    assert_eq!(result["isError"], false, "{result}");
    assert!(result["structuredContent"]["output_json"].is_array());
    let result = client.tool(
        "soundx_run",
        json!({"arguments":["info",root.join("missing.wav"),"--json"]}),
    );
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["exit_code"], 1);
    assert_eq!(
        client.tool(
            "soundx_run",
            json!({"arguments":["integrate","install","--agent","codex"]})
        )["isError"],
        true
    );
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));
    let resource = client.request("resources/read", json!({"uri":"soundx://compatibility"}));
    assert!(
        resource["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("WavPack")
    );
    drop(client);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_recording_job_has_observable_failure_without_device_access() {
    let mut client = Client::new();
    client.initialize();
    let result = client.tool(
        "soundx_start",
        json!({"arguments":["record","unsupported.flac","--continuous"]}),
    );
    let id = result["structuredContent"]["job_id"].as_u64().unwrap();
    let start = Instant::now();
    loop {
        let jobs = client.tool("soundx_jobs", json!({}));
        let job = &jobs["structuredContent"]["jobs"][0];
        if job["state"] != "running" {
            assert_eq!(job["state"], "failed", "{job}");
            assert!(job["stderr"].as_str().unwrap().contains("16-bit PCM WAV"));
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        client.tool("soundx_stop", json!({"job_id":id}))["isError"],
        true
    );
    assert_eq!(
        client.tool("soundx_stop", json!({"job_id":9999}))["isError"],
        true
    );
}

#[test]
fn malformed_and_oversized_requests_do_not_break_the_transport() {
    let mut client = Client::new();
    client
        .input
        .as_mut()
        .unwrap()
        .write_all(b"not json\n")
        .unwrap();
    assert_eq!(
        client
            .responses
            .recv_timeout(Duration::from_secs(5))
            .unwrap()["error"]["code"],
        -32700
    );
    let mut oversized = vec![b'x'; 1024 * 1024 + 1];
    oversized.push(b'\n');
    client
        .input
        .as_mut()
        .unwrap()
        .write_all(&oversized)
        .unwrap();
    assert_eq!(
        client
            .responses
            .recv_timeout(Duration::from_secs(5))
            .unwrap()["error"]["code"],
        -32600
    );
    client.initialize();
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));
}

fn scratch(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "soundx-mcp-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    root
}

/// Remove a directory once the killed children have released their files.
fn remove_when_children_exit(root: &std::path::Path) {
    let started = Instant::now();
    while std::fs::remove_dir_all(root).is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "child still alive"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn long_run_does_not_block_ping_or_jobs() {
    let root = scratch("slow");
    let mut client = Client::new();
    client.initialize();
    let run = client.send_request(
        "tools/call",
        json!({"name":"soundx_run","arguments":{"arguments":["synth",root.join("long.wav"),"--duration","60","--normalize"]}}),
    );
    let ping = client.send_request("ping", json!({}));
    let stop = client.send_request(
        "tools/call",
        json!({"name":"soundx_stop","arguments":{"job_id":1}}),
    );
    let mut order = Vec::new();
    for _ in 0..3 {
        order.push(client.recv(60));
    }
    let position = |id: u64| order.iter().position(|reply| reply["id"] == id).unwrap();
    assert!(position(ping) < position(run), "{order:?}");
    assert!(position(stop) < position(run), "{order:?}");
    let reply = &order[position(run)];
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert_eq!(reply["result"]["structuredContent"]["state"], "completed");
    drop(client);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelled_run_gets_no_response_and_is_killed() {
    let root = scratch("cancel");
    let mut client = Client::new();
    client.initialize();
    let run = client.send_request(
        "tools/call",
        json!({"name":"soundx_run","arguments":{"arguments":["synth",root.join("huge.wav"),"--duration","3000"]}}),
    );
    std::thread::sleep(Duration::from_millis(300));
    client.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":run}}),
    );
    // The next reply must be the ping's: nothing is ever sent for the cancelled id.
    let ping = client.send_request("ping", json!({}));
    assert_eq!(client.recv(5)["id"], ping);
    assert!(
        client
            .responses
            .recv_timeout(Duration::from_millis(700))
            .is_err()
    );
    // Cancelling an unknown request is ignored.
    client.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":9999}}),
    );
    assert_eq!(client.request("ping", json!({}))["result"], json!({}));
    // The killed child must have released its output file while the server lives.
    remove_when_children_exit(&root);
}

#[test]
fn run_timeout_is_reported_while_server_stays_responsive() {
    let root = scratch("timeout");
    let mut client = Client::new();
    client.initialize();
    let run = client.send_request(
        "tools/call",
        json!({"name":"soundx_run","arguments":{"arguments":["synth",root.join("huge.wav"),"--duration","3000"],"timeout_seconds":1}}),
    );
    let ping = client.send_request("ping", json!({}));
    assert_eq!(client.recv(5)["id"], ping);
    let reply = client.recv(30);
    assert_eq!(reply["id"], run);
    assert_eq!(reply["result"]["isError"], true, "{reply}");
    assert_eq!(reply["result"]["structuredContent"]["state"], "timed_out");
    remove_when_children_exit(&root);
}

#[test]
fn concurrent_finite_runs_are_bounded() {
    let root = scratch("bound");
    let mut client = Client::new();
    client.initialize();
    for index in 0..8 {
        client.send_request(
            "tools/call",
            json!({"name":"soundx_run","arguments":{"arguments":["synth",root.join(format!("{index}.wav")),"--duration","3000"]}}),
        );
    }
    let extra = client.send_request(
        "tools/call",
        json!({"name":"soundx_run","arguments":{"arguments":["synth",root.join("extra.wav"),"--duration","1"]}}),
    );
    let reply = client.recv(5);
    assert_eq!(reply["id"], extra);
    assert_eq!(reply["result"]["isError"], true);
    assert!(
        reply["result"]["structuredContent"]["error"]
            .as_str()
            .unwrap()
            .contains("limit")
    );
    drop(client);
    remove_when_children_exit(&root);
}

#[test]
fn closed_stdout_ends_the_server_and_cleans_up_in_flight_runs() {
    let root = scratch("epipe");
    let mut child = Command::new(env!("CARGO_BIN_EXE_soundx"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut input = child.stdin.take().unwrap();
    drop(child.stdout.take());
    let lines = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"soundx_run","arguments":{"arguments":["synth",root.join("huge.wav"),"--duration","3000"]}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"ping"}),
    ];
    for line in lines {
        let _ = serde_json::to_writer(&mut input, &line);
        let _ = input.write_all(b"\n");
        let _ = input.flush();
    }
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "server did not exit"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(input);
    let prefix = format!("soundx-mcp-{pid}-");
    let leftovers: Vec<_> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    remove_when_children_exit(&root);
}

#[cfg(not(windows))]
#[test]
fn windows_tools_are_not_listed_or_callable_elsewhere() {
    let mut client = Client::new();
    client.initialize();
    let tools = client.request("tools/list", json!({}))["result"]["tools"].clone();
    assert!(tools.as_array().unwrap().iter().all(|tool| {
        !tool["name"]
            .as_str()
            .unwrap()
            .starts_with("soundx_windows_")
    }));
    let reply = client.request(
        "tools/call",
        json!({"name":"soundx_windows_devices","arguments":{}}),
    );
    assert_eq!(reply["error"]["code"], -32602);
}
