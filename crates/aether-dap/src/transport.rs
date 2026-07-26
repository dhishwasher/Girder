//! DAP wire framing: `Content-Length: N\r\n\r\n<json>`.
//!
//! The protocol is identical to LSP framing. Every message is preceded by
//! HTTP-style headers terminated by a blank line; the only header Bit Code
//! reads or writes is `Content-Length`.

use crate::DapError;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

/// Encode a JSON value as a complete DAP frame (headers + body).
pub fn encode(value: &serde_json::Value) -> Result<Vec<u8>, DapError> {
    let json = serde_json::to_string(value)?;
    let body = json.as_bytes();
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut frame = header.into_bytes();
    frame.extend_from_slice(body);
    Ok(frame)
}

/// Read one DAP frame from `reader`, returning the raw JSON bytes.
///
/// Handles the `Content-Length` header and reads exactly that many bytes for
/// the body. Unknown headers are silently skipped.
pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>, DapError>
where
    R: AsyncBufReadExt + AsyncReadExt + Unpin,
{
    let mut content_length: Option<usize> = None;

    // Read header lines until the blank separator line.
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(DapError::AdapterExited);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line → end of headers
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length: ") {
            content_length =
                Some(rest.trim().parse().map_err(|_| {
                    DapError::Protocol(format!("invalid Content-Length: '{rest}'"))
                })?);
        }
    }

    let len = content_length
        .ok_or_else(|| DapError::Protocol("message missing Content-Length header".into()))?;

    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(body)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_encode_decode() {
        let msg = serde_json::json!({
            "seq": 1,
            "type": "request",
            "command": "initialize",
            "arguments": { "clientID": "bitcode" }
        });

        let frame = encode(&msg).unwrap();

        // Frame must start with the header.
        let frame_str = std::str::from_utf8(&frame).unwrap();
        assert!(frame_str.starts_with("Content-Length: "));

        // Decode via the async reader.
        let mut reader = tokio::io::BufReader::new(frame.as_slice());
        let body = read_frame(&mut reader).await.unwrap();
        let decoded: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(decoded, msg);
    }

    #[tokio::test]
    async fn multiple_frames_in_sequence() {
        let m1 = serde_json::json!({"seq": 1, "type": "request", "command": "initialize"});
        let m2 = serde_json::json!({"seq": 2, "type": "event", "event": "initialized"});

        let mut buf = encode(&m1).unwrap();
        buf.extend(encode(&m2).unwrap());

        let mut reader = tokio::io::BufReader::new(buf.as_slice());
        let b1 = read_frame(&mut reader).await.unwrap();
        let b2 = read_frame(&mut reader).await.unwrap();

        let d1: serde_json::Value = serde_json::from_slice(&b1).unwrap();
        let d2: serde_json::Value = serde_json::from_slice(&b2).unwrap();
        assert_eq!(d1["seq"], 1);
        assert_eq!(d2["event"], "initialized");
    }

    #[tokio::test]
    async fn missing_content_length_returns_error() {
        let bad_frame = b"X-Custom: whatever\r\n\r\n{}";
        let mut reader = tokio::io::BufReader::new(bad_frame.as_slice());
        let err = read_frame(&mut reader).await.unwrap_err();
        assert!(
            matches!(err, DapError::Protocol(_)),
            "expected Protocol error, got {err:?}"
        );
    }
}
