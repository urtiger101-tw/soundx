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
        9
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
