//! Async DAP client over subprocess stdio.
//!
//! [`DapClient`] launches an adapter process, writes requests to its stdin,
//! reads responses and events from its stdout, and dispatches them:
//!
//! - **Responses** are matched by `request_seq` and delivered to the
//!   per-request [`tokio::sync::oneshot`] waiting in [`DapClient::send`].
//! - **Events** are broadcast on a [`tokio::sync::broadcast`] channel;
//!   callers subscribe with [`DapClient::subscribe_events`].
//!
//! The client is `Clone`; all clones share the same underlying connection.

use crate::transport::{encode, read_frame};
use crate::types::{DapEvent, DapResponse};
use crate::DapError;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{broadcast, oneshot};

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<DapResponse>>>>;

struct DapClientInner {
    seq: AtomicU64,
    stdin: tokio::sync::Mutex<BufWriter<ChildStdin>>,
    pending: PendingMap,
    events_tx: broadcast::Sender<DapEvent>,
    // Keep child alive so stdin/stdout stay open.
    _child: tokio::sync::Mutex<Child>,
}

/// A live connection to a DAP adapter subprocess.
///
/// Clone freely — each clone shares the same subprocess and broadcast channel.
#[derive(Clone)]
pub struct DapClient {
    inner: Arc<DapClientInner>,
}

impl DapClient {
    /// Spawn `program args` as a DAP adapter and return a connected client.
    ///
    /// The adapter must speak DAP on its stdin/stdout. Stderr is inherited so
    /// adapter diagnostics appear in the terminal.
    pub async fn spawn(program: &str, args: &[&str]) -> Result<Self, DapError> {
        let mut child = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin: ChildStdin = child.stdin.take().expect("stdin is piped");
        let stdout: ChildStdout = child.stdout.take().expect("stdout is piped");

        let (events_tx, _) = broadcast::channel::<DapEvent>(128);
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        // Background task: reads frames from the adapter and dispatches them.
        let pending_bg = pending.clone();
        let events_bg = events_tx.clone();
        tokio::spawn(async move {
            read_loop(stdout, pending_bg, events_bg).await;
        });

        Ok(DapClient {
            inner: Arc::new(DapClientInner {
                seq: AtomicU64::new(0),
                stdin: tokio::sync::Mutex::new(BufWriter::new(stdin)),
                pending,
                events_tx,
                _child: tokio::sync::Mutex::new(child),
            }),
        })
    }

    /// Send a request and wait up to 30 seconds for the matching response.
    pub async fn send(
        &self,
        command: &str,
        arguments: serde_json::Value,
    ) -> Result<DapResponse, DapError> {
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1;

        let msg = serde_json::json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        let frame = encode(&msg)?;

        // Register the pending response channel BEFORE writing so we never miss it.
        let (tx, rx) = oneshot::channel::<DapResponse>();
        self.inner.pending.lock().unwrap().insert(seq, tx);

        {
            let mut stdin = self.inner.stdin.lock().await;
            if let Err(err) = stdin.write_all(&frame).await {
                remove_pending(&self.inner.pending, seq);
                return Err(err.into());
            }
            if let Err(err) = stdin.flush().await {
                remove_pending(&self.inner.pending, seq);
                return Err(err.into());
            }
        }

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(DapError::AdapterExited),
            Err(_) => {
                remove_pending(&self.inner.pending, seq);
                Err(DapError::Timeout)
            }
        }
    }

    /// Subscribe to all events emitted by the adapter.
    ///
    /// The broadcast channel holds up to 128 events; slow receivers may lag.
    pub fn subscribe_events(&self) -> broadcast::Receiver<DapEvent> {
        self.inner.events_tx.subscribe()
    }
}

fn remove_pending(pending: &PendingMap, seq: u64) {
    pending.lock().unwrap().remove(&seq);
}

// ── Background read loop ───────────────────────────────────────────────────

async fn read_loop(
    stdout: ChildStdout,
    pending: PendingMap,
    events_tx: broadcast::Sender<DapEvent>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(f) => f,
            Err(_) => break, // adapter exited or IO error
        };

        let value: serde_json::Value = match serde_json::from_slice(&frame) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed frames
        };

        match value.get("type").and_then(|t| t.as_str()) {
            Some("response") => {
                let response = match serde_json::from_value::<DapResponse>(value) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                if let Some(tx) = pending.lock().unwrap().remove(&response.request_seq) {
                    let _ = tx.send(response);
                }
            }
            Some("event") => {
                let event = match serde_json::from_value::<DapEvent>(value) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let _ = events_tx.send(event);
            }
            _ => {} // ignore unknown message types
        }
    }

    // Drain all pending senders so callers don't hang.
    let mut map = pending.lock().unwrap();
    map.clear();
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::encode;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn encode_decode_matches_round_trip() {
        // Verify that encode produces frames the transport can decode.
        use crate::transport::read_frame;

        let request = serde_json::json!({
            "seq": 1u64,
            "type": "request",
            "command": "initialize",
            "arguments": {"clientID": "girder"}
        });
        let frame = encode(&request).unwrap();
        let mut reader = tokio::io::BufReader::new(frame.as_slice());
        let body = read_frame(&mut reader).await.unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(decoded["command"], "initialize");
    }

    #[test]
    fn remove_pending_drops_timed_out_request_sender() {
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (tx, _rx) = oneshot::channel();
        pending.lock().unwrap().insert(7, tx);

        remove_pending(&pending, 7);

        assert!(pending.lock().unwrap().is_empty());
    }
}
