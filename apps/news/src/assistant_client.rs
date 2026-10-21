//! Unix socket client for Nixe Assistant. This app does not load a model.
//!
//! Connect failure is “Assistant isn’t running”. A read that passes the deadline
//! is a timeout. The article pane decides what to keep.

use nixe_news_core::{AiFailure, Assistant, Step, interpret};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const COMPLETE_WAIT: Duration = Duration::from_secs(60);
const SPEAK_WAIT: Duration = Duration::from_secs(180);

pub struct Client;

impl Assistant for Client {
    fn complete(&mut self, prompt: &str) -> Result<String, AiFailure> {
        exchange("complete", prompt, COMPLETE_WAIT)
    }

    fn speak(&mut self, script: &str) -> Result<(), AiFailure> {
        exchange("speak", script, SPEAK_WAIT).map(|_| ())
    }
}

/// Stop the turn this app started. A missing daemon is ignored.
pub fn cancel() {
    std::thread::spawn(|| {
        let Ok(mut stream) = UnixStream::connect(socket_path()) else { return };
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let _ = writeln!(stream, r#"{{"type":"cancel"}}"#);
    });
}

fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(runtime) => PathBuf::from(runtime).join("nixe-assistant/assistant.sock"),
        None => dirs_cache().join("nixe-assistant/runtime/assistant.sock"),
    }
}

fn dirs_cache() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
}

fn exchange(kind: &str, text: &str, wait: Duration) -> Result<String, AiFailure> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|_| AiFailure::Down)?;
    stream.set_read_timeout(Some(wait)).map_err(|_| AiFailure::Failed("Assistant socket failed.".into()))?;
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let line = serde_json::json!({"type": kind, "text": text});
    writeln!(stream, "{line}").map_err(|_| AiFailure::Down)?;
    let mut reader = BufReader::new(stream);
    let started = Instant::now();
    let mut written = None;
    let mut finished = false;
    loop {
        if started.elapsed() > wait {
            return Err(AiFailure::TimedOut);
        }
        let mut buf = String::new();
        match reader.read_line(&mut buf) {
            Ok(0) => break,
            Ok(_) => match interpret(buf.trim()) {
                Step::Continue => {}
                Step::Done(text) => written = Some(text),
                Step::Failed(err) => return Err(err),
            },
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock || err.kind() == std::io::ErrorKind::TimedOut => {
                return Err(AiFailure::TimedOut);
            }
            Err(_) => return Err(AiFailure::Failed("Assistant stopped answering.".into())),
        }
        if timing_line(buf.trim()) {
            finished = true;
            break;
        }
    }
    match written {
        Some(text) => Ok(text),
        None if finished && kind == "speak" => Ok(String::new()),
        None => Err(AiFailure::Failed("Assistant returned nothing.".into())),
    }
}

fn timing_line(line: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { return false };
    if value.get("type").and_then(|v| v.as_str()) == Some("timing") {
        return true;
    }
    value.get("event").and_then(|event| event.get("type")).and_then(|v| v.as_str()) == Some("timing")
}
