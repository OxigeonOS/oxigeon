//! Debug Adapter Protocol framing.
//!
//! ```text
//! Content-Length: <byte-count>\r\n
//! [other headers, ignored]\r\n
//! \r\n
//! <byte-count bytes of UTF-8 JSON>
//! ```
//!
//! `Content-Length` counts **bytes, not characters** — a body with any non-ASCII
//! in it (a room description, a player name) will desynchronize the stream if
//! this is measured in `chars`.

use bytes::{Buf, BufMut, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

/// Refuse absurd headers rather than pre-allocating from them.
const MAX_BODY: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub struct DapCodec {
    /// Body length parsed from the header block, awaiting a complete body.
    need: Option<usize>,
}

fn io_err(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.into())
}

impl Decoder for DapCodec {
    type Item = serde_json::Value;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            match self.need {
                None => {
                    let Some(end) = find_header_end(src) else {
                        return Ok(None); // header block still incomplete
                    };
                    let header = std::str::from_utf8(&src[..end])
                        .map_err(|_| io_err("DAP header is not valid UTF-8"))?
                        .to_owned();
                    src.advance(end + 4);

                    let len = header
                        .lines()
                        .filter_map(|l| l.split_once(':'))
                        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
                        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                        .ok_or_else(|| io_err("DAP header has no usable Content-Length"))?;

                    if len > MAX_BODY {
                        return Err(io_err(format!("DAP body of {len} bytes exceeds the limit")));
                    }
                    self.need = Some(len);
                }
                Some(len) => {
                    if src.len() < len {
                        src.reserve(len - src.len());
                        return Ok(None);
                    }
                    let body = src.split_to(len);
                    self.need = None;
                    let value = serde_json::from_slice(&body)
                        .map_err(|e| io_err(format!("malformed DAP message: {e}")))?;
                    return Ok(Some(value));
                }
            }
        }
    }
}

impl Encoder<serde_json::Value> for DapCodec {
    type Error = std::io::Error;

    fn encode(&mut self, item: serde_json::Value, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let body = serde_json::to_vec(&item)
            .map_err(|e| io_err(format!("cannot serialize DAP message: {e}")))?;
        dst.reserve(body.len() + 32);
        dst.put_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        dst.put_slice(&body);
        Ok(())
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn encoded(v: serde_json::Value) -> BytesMut {
        let mut out = BytesMut::new();
        DapCodec::default().encode(v, &mut out).unwrap();
        out
    }

    #[test]
    fn round_trips() {
        let msg = json!({"seq": 1, "type": "request", "command": "initialize"});
        let mut buf = encoded(msg.clone());
        assert_eq!(DapCodec::default().decode(&mut buf).unwrap(), Some(msg));
    }

    #[test]
    fn content_length_is_bytes_not_chars() {
        // A body whose char count differs from its byte count. If the encoder
        // measured chars, the decoder would stop short and desync the stream.
        let msg = json!({"text": "café ☕ — Zaphod"});
        let mut buf = encoded(msg.clone());
        let mut codec = DapCodec::default();
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(msg));
        assert!(buf.is_empty(), "decoder should consume exactly the body");
    }

    #[test]
    fn decodes_when_fed_one_byte_at_a_time() {
        let msg = json!({"seq": 7, "command": "stackTrace"});
        let whole = encoded(msg.clone());

        let mut codec = DapCodec::default();
        let mut buf = BytesMut::new();
        for (i, b) in whole.iter().enumerate() {
            buf.extend_from_slice(&[*b]);
            let got = codec.decode(&mut buf).unwrap();
            if i + 1 < whole.len() {
                assert!(got.is_none(), "decoded early at byte {i}");
            } else {
                assert_eq!(got, Some(msg.clone()));
            }
        }
    }

    #[test]
    fn decodes_two_messages_from_one_buffer() {
        let a = json!({"seq": 1});
        let b = json!({"seq": 2});
        let mut buf = encoded(a.clone());
        buf.extend_from_slice(&encoded(b.clone()));

        let mut codec = DapCodec::default();
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(a));
        assert_eq!(codec.decode(&mut buf).unwrap(), Some(b));
        assert_eq!(codec.decode(&mut buf).unwrap(), None);
    }

    #[test]
    fn extra_headers_are_ignored() {
        let mut buf = BytesMut::new();
        let body = br#"{"seq":3}"#;
        buf.extend_from_slice(b"Content-Type: application/vscode-jsonrpc\r\n");
        buf.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        buf.extend_from_slice(body);
        assert_eq!(
            DapCodec::default().decode(&mut buf).unwrap(),
            Some(json!({"seq": 3}))
        );
    }

    #[test]
    fn oversize_and_malformed_headers_error_rather_than_allocate() {
        let mut huge = BytesMut::from(&b"Content-Length: 999999999\r\n\r\n"[..]);
        assert!(DapCodec::default().decode(&mut huge).is_err());

        let mut junk = BytesMut::from(&b"Content-Length: banana\r\n\r\n"[..]);
        assert!(DapCodec::default().decode(&mut junk).is_err());
    }

    #[test]
    fn malformed_json_body_is_an_error() {
        let mut buf = BytesMut::from(&b"Content-Length: 3\r\n\r\n{{{"[..]);
        assert!(DapCodec::default().decode(&mut buf).is_err());
    }
}
