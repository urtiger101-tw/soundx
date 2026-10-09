//! A local, newline-delimited JSON-RPC MCP transport. Child commands use the
//! same executable and argument vectors, never a shell.
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_MESSAGE: u64 = 1024 * 1024;
const MAX_OUTPUT: usize = 256 * 1024;
const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    arguments: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}
fn default_timeout() -> u64 {
    120
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoudnessArgs {
    inputs: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArgs {
    arguments: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JobArgs {
    job_id: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EndpointArgs {
    device_id: Option<String>,
    #[serde(default = "output_flow")]
    flow: String,
    volume: Option<f32>,
    mute: Option<bool>,
}
fn output_flow() -> String {
    "output".into()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionsArgs {
    device_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionArgs {
    device_id: Option<String>,
    session_id: String,
    volume: Option<f32>,
    mute: Option<bool>,
}

struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}
fn capture(mut input: impl Read + Send + 'static) -> JoinHandle<std::io::Result<Capture>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut truncated = false;
        let mut chunk = [0; 8192];
        loop {
            let count = input.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            let keep = count.min(MAX_OUTPUT - bytes.len());
            bytes.extend_from_slice(&chunk[..keep]);
            truncated |= keep < count;
        }
        Ok(Capture { bytes, truncated })
    })
}

/// Closing the owning server also terminates its children after a forced exit.
#[cfg(windows)]
struct WindowsJob(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl WindowsJob {
    fn new() -> Result<Self> {
        use windows::Win32::System::JobObjects::*;
        let job = Self(unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )?;
        }
        Ok(job)
    }
    fn assign(&self, child: &Child) -> Result<()> {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            windows::Win32::System::JobObjects::AssignProcessToJobObject(
                self.0,
                windows::Win32::Foundation::HANDLE(child.as_raw_handle()),
            )?;
        }
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

struct Job {
    child: Child,
    #[cfg(windows)]
    _process_job: WindowsJob,
    arguments: Vec<String>,
    stop_file: PathBuf,
    stdout: Option<JoinHandle<std::io::Result<Capture>>>,
    stderr: Option<JoinHandle<std::io::Result<Capture>>>,
    result: Option<Value>,
}
impl Job {
    fn spawn(arguments: Vec<String>, stop_file: PathBuf) -> Result<Self> {
        validate_arguments(&arguments)?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(&arguments)
            .env("SOUNDX_STOP_FILE", &stop_file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        #[cfg(windows)]
        let process_job = WindowsJob::new().context("could not create child process job")?;
        let mut child = command.spawn().context("could not start soundx command")?;
        #[cfg(windows)]
        if let Err(error) = process_job.assign(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.context("could not own child process lifetime"));
        }
        let stdout = child.stdout.take().map(capture);
        let stderr = child.stderr.take().map(capture);
        Ok(Self {
            child,
            #[cfg(windows)]
            _process_job: process_job,
            arguments,
            stop_file,
            stdout,
            stderr,
            result: None,
        })
    }
    fn poll(&mut self) -> Result<Value> {
        if let Some(result) = &self.result {
            return Ok(result.clone());
        }
        let Some(status) = self.child.try_wait()? else {
            return Ok(
                json!({"state":"running","process_id":self.child.id(),"arguments":self.arguments}),
            );
        };
        let mut read = |stdout: bool| -> Result<Capture> {
            let reader = if stdout {
                self.stdout.take()
            } else {
                self.stderr.take()
            };
            reader
                .ok_or_else(|| anyhow!("missing child output reader"))?
                .join()
                .map_err(|_| anyhow!("child output reader panicked"))?
                .context("failed to read child output")
        };
        let out = read(true)?;
        let err = read(false)?;
        let stdout = String::from_utf8_lossy(&out.bytes);
        let result = json!({
            "state": if status.success() { "completed" } else { "failed" },
            "exit_code":status.code(), "process_id":self.child.id(), "arguments":self.arguments,
            "stdout":stdout, "stderr":String::from_utf8_lossy(&err.bytes),
            "output_truncated":out.truncated || err.truncated,
            "output_json":serde_json::from_str::<Value>(&stdout).ok()
        });
        self.result = Some(result.clone());
        Ok(result)
    }
    fn stop(&mut self) -> Result<Value> {
        if self.child.try_wait()?.is_none() {
            std::fs::write(&self.stop_file, b"stop\n")
                .context("could not request graceful stop")?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.child.try_wait()?.is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            if self.child.try_wait()?.is_none() {
                self.child.kill()?;
                self.child.wait()?;
                let mut result = self.poll()?;
                result["forced_stop"] = json!(true);
                result["warning"] =
                    json!("Forced termination; recording finalization is not guaranteed");
                self.result = Some(result.clone());
                return Ok(result);
            }
        }
        self.poll()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        if self.result.is_none()
            && let Err(error) = self.stop()
        {
            eprintln!("soundx MCP job cleanup: {error:#}");
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_file(&self.stop_file);
    }
}

fn validate_arguments(arguments: &[String]) -> Result<()> {
    if arguments.is_empty() || arguments.len() > 256 {
        bail!("arguments must contain 1 to 256 entries");
    }
    if arguments.iter().any(|value| value.contains('\0'))
        || arguments.iter().map(String::len).sum::<usize>() > 65_536
    {
        bail!("arguments contain NUL or exceed 64 KiB");
    }
    Ok(())
}

struct Server {
    initialized: bool,
    ready: bool,
    directory: PathBuf,
    jobs: BTreeMap<u64, Job>,
    next_job: u64,
}
impl Server {
    fn new() -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let directory =
            std::env::temp_dir().join(format!("soundx-mcp-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&directory)?;
        Ok(Self {
            initialized: false,
            ready: false,
            directory,
            jobs: BTreeMap::new(),
            next_job: 1,
        })
    }
    /// Run a finite command to completion (or kill it at the timeout).
    fn run_finite(&mut self, arguments: Vec<String>, timeout_seconds: u64) -> Result<Value> {
        if !(1..=600).contains(&timeout_seconds) {
            bail!("timeout_seconds must be between 1 and 600");
        }
        let mut job = Job::spawn(arguments, self.directory.join("run.stop"))?;
        let started = Instant::now();
        loop {
            let result = job.poll()?;
            if result["state"] != "running" {
                return Ok(result);
            }
            if started.elapsed() >= Duration::from_secs(timeout_seconds) {
                job.child.kill()?;
                job.child.wait()?;
                let mut result = job.poll()?;
                result["state"] = json!("timed_out");
                return Ok(result);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn call(&mut self, name: &str, arguments: Value) -> Result<Value> {
        match name {
            "soundx_run" => {
                let args: RunArgs = serde_json::from_value(arguments)?;
                validate_arguments(&args.arguments)?;
                if !matches!(
                    args.arguments[0].as_str(),
                    "info"
                        | "loudness"
                        | "formats"
                        | "devices"
                        | "convert"
                        | "concat"
                        | "mix"
                        | "synth"
                        | "stream"
                        | "batch"
                        | "run-plan"
                        | "help"
                        | "--help"
                        | "--version"
                ) {
                    bail!(
                        "command is not supported by soundx_run; use soundx_start for play/record and Windows tools for volume control"
                    );
                }
                self.run_finite(args.arguments, args.timeout_seconds)
            }
            "soundx_loudness" => {
                let args: LoudnessArgs = serde_json::from_value(arguments)?;
                if args.inputs.is_empty() || args.inputs.len() > 64 {
                    bail!("inputs must contain 1 to 64 paths");
                }
                let mut arguments = vec!["loudness".to_string()];
                // `--` keeps file names that start with a dash from being flags.
                arguments.push("--json".into());
                arguments.push("--".into());
                arguments.extend(args.inputs);
                let mut result = self.run_finite(arguments, args.timeout_seconds)?;
                result["loudness"] = result["output_json"].clone();
                Ok(result)
            }
            "soundx_start" => {
                let args: StartArgs = serde_json::from_value(arguments)?;
                validate_arguments(&args.arguments)?;
                if !matches!(args.arguments[0].as_str(), "play" | "record") {
                    bail!("soundx_start accepts play or record only");
                }
                if self.jobs.len() >= 32 {
                    let mut finished = Vec::new();
                    for (&id, job) in &mut self.jobs {
                        if job.poll()?["state"] != "running" {
                            finished.push(id);
                        }
                    }
                    for id in finished {
                        self.jobs.remove(&id);
                    }
                    if self.jobs.len() >= 32 {
                        bail!("stop an existing job before starting another (limit: 32)");
                    }
                }
                let id = self.next_job;
                self.next_job = self
                    .next_job
                    .checked_add(1)
                    .context("job identifier exhausted")?;
                let job = Job::spawn(args.arguments, self.directory.join(format!("{id}.stop")))?;
                let process_id = job.child.id();
                self.jobs.insert(id, job);
                Ok(json!({"job_id":id,"state":"started","process_id":process_id}))
            }
            "soundx_jobs" => {
                require_empty(&arguments)?;
                let mut jobs = Vec::new();
                for (&id, job) in &mut self.jobs {
                    let mut result = job.poll()?;
                    result["job_id"] = json!(id);
                    jobs.push(result);
                }
                Ok(json!({"jobs":jobs}))
            }
            "soundx_stop" => {
                let args: JobArgs = serde_json::from_value(arguments)?;
                let job = self
                    .jobs
                    .get_mut(&args.job_id)
                    .context("unknown job_id in this MCP session")?;
                let mut result = job.stop()?;
                result["job_id"] = json!(args.job_id);
                Ok(result)
            }
            "soundx_windows_devices" => {
                require_empty(&arguments)?;
                crate::windows_audio::endpoints()
            }
            "soundx_windows_endpoint" => {
                let args: EndpointArgs = serde_json::from_value(arguments)?;
                crate::windows_audio::endpoint(
                    args.device_id.as_deref(),
                    &args.flow,
                    args.volume,
                    args.mute,
                )
            }
            "soundx_windows_sessions" => {
                let args: SessionsArgs = serde_json::from_value(arguments)?;
                crate::windows_audio::sessions(args.device_id.as_deref())
            }
            "soundx_windows_session" => {
                let args: SessionArgs = serde_json::from_value(arguments)?;
                crate::windows_audio::session(
                    args.device_id.as_deref(),
                    &args.session_id,
                    args.volume,
                    args.mute,
                )
            }
            _ => bail!("unknown tool: {name}"),
        }
    }
    fn dispatch(&mut self, request: Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str);
        if !request.is_object()
            || request["jsonrpc"] != "2.0"
            || method.is_none()
            || id
                .as_ref()
                .is_some_and(|id| !(id.is_string() || id.is_i64() || id.is_u64()))
        {
            return Some(rpc_error(Value::Null, -32600, "Invalid JSON-RPC request"));
        }
        let method = method.unwrap();
        let Some(id) = id else {
            if method == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return None;
        };
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return Some(rpc_error(id, -32602, "params must be an object"));
        }
        let result = match method {
            "initialize" => {
                if self.initialized {
                    return Some(rpc_error(id, -32600, "Already initialized"));
                }
                let Some(version) = params["protocolVersion"].as_str() else {
                    return Some(rpc_error(id, -32602, "protocolVersion is required"));
                };
                if !params["capabilities"].is_object()
                    || !params["clientInfo"]["name"].is_string()
                    || !params["clientInfo"]["version"].is_string()
                {
                    return Some(rpc_error(
                        id,
                        -32602,
                        "capabilities and clientInfo are required",
                    ));
                }
                self.initialized = true;
                json!({"protocolVersion":if VERSIONS.contains(&version) {version} else {VERSIONS[0]},
                    "capabilities":{"tools":{"listChanged":false},"resources":{}},
                    "serverInfo":{"name":"soundx","version":env!("CARGO_PKG_VERSION")},
                    "instructions":"Local audio processing. Use absolute file paths. Start play/record with soundx_start, inspect soundx_jobs, and finalize with soundx_stop. Windows volume changes affect the selected endpoint or session."})
            }
            "ping" => json!({}),
            _ if !self.ready => {
                return Some(rpc_error(id, -32002, "Complete MCP initialization first"));
            }
            "tools/list" => json!({"tools":tools()}),
            "tools/call" => {
                let Some(name) = params["name"].as_str() else {
                    return Some(rpc_error(id, -32602, "tool name is required"));
                };
                if !tools().iter().any(|tool| tool["name"] == name) {
                    return Some(rpc_error(id, -32602, "Unknown tool"));
                }
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match self.call(name, args) {
                    Ok(value) => {
                        let error = matches!(value["state"].as_str(), Some("failed" | "timed_out"))
                            || value["forced_stop"] == true;
                        tool_result(value, error)
                    }
                    Err(error) => tool_result(json!({"error":format!("{error:#}")}), true),
                }
            }
            "resources/list" => json!({"resources":[
                {"uri":"soundx://guide","name":"soundx agent guide","mimeType":"text/markdown"},
                {"uri":"soundx://compatibility","name":"Format and effect limits","mimeType":"text/markdown"}]}),
            "resources/read" => {
                let uri = params["uri"].as_str().unwrap_or("");
                let text = match uri {
                    "soundx://guide" => include_str!("../skills/soundx/SKILL.md"),
                    "soundx://compatibility" => include_str!("../docs/SOX_COMPATIBILITY.md"),
                    _ => return Some(rpc_error(id, -32002, "Resource not found")),
                };
                json!({"contents":[{"uri":uri,"mimeType":"text/markdown","text":text}]})
            }
            _ => return Some(rpc_error(id, -32601, "Method not found")),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        // Request all stops before waiting, so recordings end together.
        for job in self.jobs.values() {
            let _ = std::fs::write(&job.stop_file, b"stop\n");
        }
        self.jobs.clear();
        let _ = std::fs::remove_dir(&self.directory);
    }
}
fn require_empty(value: &Value) -> Result<()> {
    if !value.as_object().is_some_and(|object| object.is_empty()) {
        bail!("this tool accepts an empty arguments object");
    }
    Ok(())
}
fn tool_result(value: Value, error: bool) -> Value {
    let value = if value.is_object() {
        value
    } else {
        json!({"data":value})
    };
    json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":error})
}
fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn tools() -> Vec<Value> {
    let arguments = json!({"type":"array","items":{"type":"string"},"minItems":1,"maxItems":256});
    let device =
        json!({"type":"string","description":"Exact endpoint id; omit for default device"});
    let volume = json!({"type":"number","minimum":0,"maximum":100});
    let mute = json!({"type":"boolean"});
    [
        ("soundx_run", "Run finite soundx CLI processing with separate arguments, no shell. Commands: info, loudness, formats, devices, convert, concat, mix, synth, stream, batch, run-plan, help, --help, --version. Use help for flags. May create/overwrite output files. Returns exit status and captured output.", schema(json!({"arguments":arguments,"timeout_seconds":{"type":"integer","minimum":1,"maximum":600,"default":120}}), &["arguments"]), false),
        ("soundx_loudness", "Measure ITU-R BS.1770-4 / EBU R128 loudness of audio files: integrated LUFS, loudness range (LRA), true peak (dBTP), sample peak, momentary and short-term maxima. Read-only. Returns `loudness`, an array with one object per input (integrated_lufs is null for silence).", schema(json!({"inputs":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64,"description":"Absolute audio file paths"},"timeout_seconds":{"type":"integer","minimum":1,"maximum":600,"default":120}}), &["inputs"]), true),
        ("soundx_start", "Start play or record as a managed job. Includes looping playback and continuous WAV capture. Returns a job id, not completion. Jobs end when this MCP connection closes. Use soundx_stop to finalize recording.", schema(json!({"arguments":arguments}), &["arguments"]), false),
        ("soundx_jobs", "Read states and captured results of this connection's jobs. At most 32 jobs are retained.", schema(json!({}), &[]), true),
        ("soundx_stop", "Gracefully stop a managed play/record job and return its final status. Never accepts arbitrary process ids.", schema(json!({"job_id":{"type":"integer","minimum":1}}), &["job_id"]), false),
        ("soundx_windows_devices", "Windows Core Audio: enumerate active input/output endpoints, defaults and volume/mute state.", schema(json!({}), &[]), true),
        ("soundx_windows_endpoint", "Windows Core Audio: read endpoint volume/mute; supply volume (0-100) or mute to change it and read back the result. Omit both for read-only.", schema(json!({"device_id":device,"flow":{"type":"string","enum":["input","output"],"default":"output"},"volume":volume,"mute":mute}), &[]), false),
        ("soundx_windows_sessions", "Windows Core Audio: list output audio sessions and their stable ids, process ids and volume/mute state.", schema(json!({"device_id":device}), &[]), true),
        ("soundx_windows_session", "Windows Core Audio: read or change a single output session selected by its exact session id. Volume uses 0-100 percent.", schema(json!({"device_id":device,"session_id":{"type":"string"},"volume":volume,"mute":mute}), &["session_id"]), false),
    ].into_iter().map(|(name, description, input, read_only)| json!({"name":name,"description":description,"inputSchema":input,
        "annotations":{"readOnlyHint":read_only,"destructiveHint":!read_only,"openWorldHint":false}})).collect()
}

pub fn serve() -> Result<()> {
    let mut server = Server::new()?;
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    loop {
        let mut line = Vec::new();
        let count = (&mut input)
            .take(MAX_MESSAGE + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        let response = if count as u64 > MAX_MESSAGE {
            if !line.ends_with(b"\n") {
                input.skip_until(b'\n')?;
            }
            Some(rpc_error(Value::Null, -32600, "Message exceeds 1 MiB"))
        } else {
            match serde_json::from_slice(&line) {
                Ok(request) => server.dispatch(request),
                Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
            }
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handshake_and_protocol_errors() {
        let mut server = Server::new().unwrap();
        let request = |method| json!({"jsonrpc":"2.0","id":1,"method":method});
        assert_eq!(
            server.dispatch(request("tools/list")).unwrap()["error"]["code"],
            -32002
        );
        let init = json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}});
        assert_eq!(
            server.dispatch(init.clone()).unwrap()["result"]["protocolVersion"],
            "2025-06-18"
        );
        assert_eq!(server.dispatch(init).unwrap()["error"]["code"], -32600);
        assert!(
            server
                .dispatch(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                .is_none()
        );
        assert_eq!(
            server.dispatch(request("tools/list")).unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            9
        );
        assert_eq!(server.dispatch(json!([])).unwrap()["error"]["code"], -32600);
        assert_eq!(
            server.dispatch(request("absent")).unwrap()["error"]["code"],
            -32601
        );
        assert!(
            server
                .call("soundx_run", json!({"arguments":["mcp"]}))
                .is_err()
        );
        assert!(
            server
                .call("soundx_start", json!({"arguments":["integrate"]}))
                .is_err()
        );
        assert!(
            server
                .call(
                    "soundx_run",
                    json!({"arguments":["formats"],"timeout_seconds":0})
                )
                .is_err()
        );
    }
}
