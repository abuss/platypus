use std::io::{BufRead, Write};

/// Write a JSON message with LSP framing: `Content-Length: N\r\n\r\n` + JSON body
pub fn write_message(w: &mut impl Write, msg: &serde_json::Value) -> std::io::Result<()> {
    let body = serde_json::to_string(msg)?;
    let content_length = body.len();
    w.write_all(format!("Content-Length: {}\r\n\r\n", content_length).as_bytes())?;
    w.write_all(body.as_bytes())?;
    w.flush()?;
    Ok(())
}

/// Frame parser with poisoned-stream semantics.
/// Parses LSP Content-Length framed messages from BufRead.
/// On malformed input (non-numeric Content-Length, >64KB, EOF mid-message), returns Ok(None)
/// once and marks the stream poisoned — all subsequent reads return Ok(None).
pub struct Frmr {
    buf: Vec<u8>,
    poisoned: bool,
}

impl Frmr {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            poisoned: false,
        }
    }

    pub fn read_message<R: BufRead>(
        &mut self,
        r: &mut R,
    ) -> std::io::Result<Option<serde_json::Value>> {
        // If poisoned, never read again
        if self.poisoned {
            return Ok(None);
        }

        loop {
            // Try to parse a complete message from the buffer
            if let Some(msg) = self.try_parse_message()? {
                return Ok(Some(msg));
            }

            // Buffer doesn't have a complete message; read more bytes
            let mut temp = vec![0; 4096];
            let n = r.read(&mut temp)?;

            if n == 0 {
                // EOF reached
                if self.buf.is_empty() {
                    // Clean EOF at message boundary
                    return Ok(None);
                } else {
                    // EOF mid-message: poison the stream
                    self.poisoned = true;
                    return Ok(None);
                }
            }

            self.buf.extend_from_slice(&temp[..n]);
        }
    }

    fn try_parse_message(&mut self) -> std::io::Result<Option<serde_json::Value>> {
        // Look for \r\n\r\n header terminator
        let header_end = self
            .buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n");

        let header_end = match header_end {
            Some(pos) => pos,
            None => {
                // Check if we already have a huge buffer (>64KB header)
                if self.buf.len() > 65536 {
                    self.poisoned = true;
                    return Ok(None);
                }
                return Ok(None); // Need more data
            }
        };

        // Parse headers before the \r\n\r\n
        let headers_bytes = &self.buf[..header_end];
        let headers_str = match std::str::from_utf8(headers_bytes) {
            Ok(s) => s,
            Err(_) => {
                self.poisoned = true;
                return Ok(None);
            }
        };

        // Extract Content-Length
        let mut content_length = None;
        for line in headers_str.lines() {
            if let Some(value) = line.strip_prefix("Content-Length:") {
                match value.trim().parse::<usize>() {
                    Ok(len) => {
                        if len > 67108864 {
                            // >64MB content — poison
                            self.poisoned = true;
                            return Ok(None);
                        }
                        content_length = Some(len);
                        break;
                    }
                    Err(_) => {
                        self.poisoned = true;
                        return Ok(None);
                    }
                }
            }
        }

        let content_length = match content_length {
            Some(len) => len,
            None => {
                self.poisoned = true;
                return Ok(None);
            }
        };

        let body_start = header_end + 4; // Skip \r\n\r\n
        let body_end = body_start + content_length;

        // Check if we have the full body
        if self.buf.len() < body_end {
            return Ok(None); // Need more data
        }

        // Extract and parse the body
        let body_bytes = &self.buf[body_start..body_end];
        let msg = match serde_json::from_slice::<serde_json::Value>(body_bytes) {
            Ok(m) => m,
            Err(_) => {
                self.poisoned = true;
                return Ok(None);
            }
        };

        // Remove the parsed message from the buffer
        self.buf.drain(..body_end);

        Ok(Some(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_round_trip_single_message() {
        let msg = serde_json::json!({"jsonrpc": "2.0", "method": "test"});
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();

        let mut framer = Frmr::new();
        let mut cursor = Cursor::new(&buf);
        let read_msg = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read_msg, Some(msg));
    }

    #[test]
    fn test_round_trip_three_messages_mixed_unicode() {
        // Test with ASCII, Latin-1, and emoji (multibyte UTF-8)
        let msg1 = serde_json::json!({"text": "hello"});
        let msg2 = serde_json::json!({"text": "café"});
        let msg3 = serde_json::json!({"text": "🦀💯"});

        let mut buf = Vec::new();
        write_message(&mut buf, &msg1).unwrap();
        write_message(&mut buf, &msg2).unwrap();
        write_message(&mut buf, &msg3).unwrap();

        let mut framer = Frmr::new();
        let mut cursor = Cursor::new(&buf);

        let read_msg1 = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read_msg1, Some(msg1));

        let read_msg2 = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read_msg2, Some(msg2));

        let read_msg3 = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read_msg3, Some(msg3));

        // EOF returns clean None
        let read_eof = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read_eof, None);
    }

    #[test]
    fn test_byte_at_a_time_reader() {
        let msg = serde_json::json!({"id": 42, "method": "test", "params": {"key": "value"}});
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();

        // Feed bytes gradually to stress the framing logic
        let mut framer = Frmr::new();
        let mut idx = 0;

        loop {
            if idx >= buf.len() {
                break;
            }
            // Create a cursor with remaining bytes starting from idx
            let remaining = &buf[idx..];
            let mut cursor = Cursor::new(remaining);
            match framer.read_message(&mut cursor).unwrap() {
                Some(m) => {
                    assert_eq!(m, msg);
                    break;
                }
                None => {
                    // Read failed; consume one byte and retry
                    idx += 1;
                }
            }
        }
    }

    #[test]
    fn test_garbage_header_non_numeric_content_length() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Content-Length: abc\r\n\r\n");
        buf.extend_from_slice(b"{}");

        let mut framer = Frmr::new();
        let mut cursor = Cursor::new(&buf);

        let result = framer.read_message(&mut cursor).unwrap();
        assert_eq!(result, None);
        // Stream should be poisoned
        let result2 = framer.read_message(&mut cursor).unwrap();
        assert_eq!(result2, None);
    }

    #[test]
    fn test_oversized_header_exceeds_64kb() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Content-Length: 99999999999\r\n\r\n");
        buf.extend_from_slice(b"{}");

        let mut framer = Frmr::new();
        let mut cursor = Cursor::new(&buf);

        let result = framer.read_message(&mut cursor).unwrap();
        assert_eq!(result, None);
        // Stream should be poisoned
        let result2 = framer.read_message(&mut cursor).unwrap();
        assert_eq!(result2, None);
    }

    #[test]
    fn test_eof_mid_message_poisons_stream() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Content-Length: 100\r\n\r\n");
        buf.extend_from_slice(b"{}"); // Only 2 bytes, not 100

        let mut framer = Frmr::new();
        let mut cursor = Cursor::new(&buf);

        let result = framer.read_message(&mut cursor).unwrap();
        assert_eq!(result, None);
        // Stream should be poisoned
        let result2 = framer.read_message(&mut cursor).unwrap();
        assert_eq!(result2, None);
    }

    #[test]
    fn test_multiple_messages_in_one_read() {
        let msg1 = serde_json::json!({"id": 1});
        let msg2 = serde_json::json!({"id": 2});

        let mut buf = Vec::new();
        write_message(&mut buf, &msg1).unwrap();
        write_message(&mut buf, &msg2).unwrap();

        let mut framer = Frmr::new();
        let mut cursor = Cursor::new(&buf);

        let read1 = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read1, Some(msg1));

        let read2 = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read2, Some(msg2));
    }

    #[test]
    fn test_clean_eof_at_boundary() {
        let msg = serde_json::json!({"test": true});
        let mut buf = Vec::new();
        write_message(&mut buf, &msg).unwrap();

        let mut framer = Frmr::new();
        let mut cursor = Cursor::new(&buf);

        let read_msg = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read_msg, Some(msg));

        // Clean EOF (buffer exhausted at message boundary)
        let read_eof = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read_eof, None);

        // Can call again after clean EOF (stream not poisoned)
        let read_again = framer.read_message(&mut cursor).unwrap();
        assert_eq!(read_again, None);
    }
}
