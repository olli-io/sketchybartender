//! Direct client for the AeroSpace.app socket protocol.
//!
//! Instead of spawning the `aerospace` CLI for every query, we talk to the
//! running AeroSpace server over its Unix-domain socket. The CLI does exactly
//! this internally, so the `stdout` we get back is byte-identical to what the
//! CLI would have printed — existing parsing of the result stays unchanged.
//!
//! Protocol: https://nikitabobko.github.io/AeroSpace/guide#socket-protocol

use std::env;
use std::fmt;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use serde::{Deserialize, Serialize};

/// The only valid socket protocol version.
const SOCKET_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug)]
pub enum AerospaceError {
    /// Could not connect / read / write the socket.
    Io(std::io::Error),
    /// Server reported a protocol version we don't support.
    VersionMismatch(u32),
    /// Request/response (de)serialization failed.
    Json(serde_json::Error),
    /// The command ran but exited non-zero; carries stderr.
    Command { exit_code: i32, stderr: String },
}

impl fmt::Display for AerospaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AerospaceError::Io(e) => write!(f, "socket io error: {}", e),
            AerospaceError::VersionMismatch(v) => {
                write!(f, "unsupported server protocol version {}", v)
            }
            AerospaceError::Json(e) => write!(f, "json error: {}", e),
            AerospaceError::Command { exit_code, stderr } => {
                write!(f, "aerospace exited with {}: {}", exit_code, stderr.trim())
            }
        }
    }
}

impl std::error::Error for AerospaceError {}

impl From<std::io::Error> for AerospaceError {
    fn from(e: std::io::Error) -> Self {
        AerospaceError::Io(e)
    }
}

impl From<serde_json::Error> for AerospaceError {
    fn from(e: serde_json::Error) -> Self {
        AerospaceError::Json(e)
    }
}

#[derive(Serialize)]
struct ClientRequest {
    args: Vec<String>,
    stdin: String,
    #[serde(rename = "windowId")]
    window_id: Option<u32>,
    workspace: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerAnswer {
    exit_code: i32,
    stdout: String,
    stderr: String,
    // serverVersionAndHash is intentionally ignored.
}

/// Path to the release-build AeroSpace socket for the current user.
fn socket_path() -> String {
    let user = env::var("USER").unwrap_or_default();
    format!("/tmp/bobko.aerospace-{}.sock", user)
}

/// One-shot version handshake: send our version, read + validate the server's.
fn handshake(stream: &mut UnixStream) -> Result<(), AerospaceError> {
    stream.write_all(&SOCKET_PROTOCOL_VERSION.to_le_bytes())?;

    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf)?;
    let server_version = u32::from_le_bytes(buf);
    if server_version != SOCKET_PROTOCOL_VERSION {
        return Err(AerospaceError::VersionMismatch(server_version));
    }
    Ok(())
}

/// Write a length-prefixed (u32 LE) frame.
fn write_frame(stream: &mut UnixStream, payload: &[u8]) -> Result<(), AerospaceError> {
    stream.write_all(&(payload.len() as u32).to_le_bytes())?;
    stream.write_all(payload)?;
    Ok(())
}

/// Read a length-prefixed (u32 LE) frame.
fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>, AerospaceError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    Ok(payload)
}

/// Run a one-shot aerospace command over the server socket.
///
/// `args` is exactly what you'd pass to the `aerospace` CLI, excluding the
/// program name, e.g. `["list-windows", "--all", "--format", "%{...}", "--json"]`.
///
/// Returns the command's stdout on success (`exitCode == 0`). Returns `Err` for
/// a missing/unreachable socket, a protocol version mismatch, a non-zero exit,
/// or an IO/JSON failure.
pub fn run(args: &[&str]) -> Result<String, AerospaceError> {
    let mut stream = UnixStream::connect(socket_path())?;

    handshake(&mut stream)?;

    let request = ClientRequest {
        args: args.iter().map(|s| s.to_string()).collect(),
        stdin: String::new(),
        window_id: None,
        workspace: None,
    };
    let payload = serde_json::to_vec(&request)?;
    write_frame(&mut stream, &payload)?;

    let answer_bytes = read_frame(&mut stream)?;
    let answer: ServerAnswer = serde_json::from_slice(&answer_bytes)?;

    if answer.exit_code != 0 {
        return Err(AerospaceError::Command {
            exit_code: answer.exit_code,
            stderr: answer.stderr,
        });
    }

    Ok(answer.stdout)
}

/// A live subscription to AeroSpace server events.
///
/// After connecting and handshaking, the client sends one `subscribe` request
/// and the server streams length-prefixed JSON `ServerEvent` frames until the
/// connection closes. The client must not send anything else on the connection.
pub struct Subscription {
    stream: UnixStream,
}

impl Subscription {
    /// Open a subscription. `events` is the list of event names to subscribe to
    /// (e.g. `["focused-workspace-changed"]`); pass an empty slice together with
    /// `all = true` to subscribe to every event.
    ///
    /// With `send_initial = true` the server immediately emits the current state
    /// as events, which is convenient for painting the initial UI.
    pub fn open(
        events: &[&str],
        all: bool,
        send_initial: bool,
    ) -> Result<Subscription, AerospaceError> {
        let mut stream = UnixStream::connect(socket_path())?;
        handshake(&mut stream)?;

        let mut args: Vec<String> = vec!["subscribe".to_string()];
        if all {
            args.push("--all".to_string());
        }
        if !send_initial {
            args.push("--no-send-initial".to_string());
        }
        args.extend(events.iter().map(|s| s.to_string()));

        let request = ClientRequest {
            args,
            stdin: String::new(),
            window_id: None,
            workspace: None,
        };
        let payload = serde_json::to_vec(&request)?;
        write_frame(&mut stream, &payload)?;

        Ok(Subscription { stream })
    }

    /// Block until the next event frame arrives and return it parsed as generic
    /// JSON. Returns `Err` when the connection closes or a read/parse fails.
    pub fn next_event(&mut self) -> Result<serde_json::Value, AerospaceError> {
        let frame = read_frame(&mut self.stream)?;
        let value = serde_json::from_slice(&frame)?;
        Ok(value)
    }
}
