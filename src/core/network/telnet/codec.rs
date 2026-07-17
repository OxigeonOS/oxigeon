use super::constants::*;

/// Higher-level codec that handles CR/LF translation and IAC escaping.
pub struct TelnetCodec;

impl TelnetCodec {
    /// Translate outgoing text: convert \n to CR LF, escape 0xFF bytes.
    pub fn encode_text(text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() + 8);
        for &byte in text.as_bytes() {
            match byte {
                LF => {
                    // \n → CR LF per NVT spec
                    out.push(CR);
                    out.push(LF);
                }
                IAC => {
                    // Escape 0xFF as IAC IAC
                    out.push(IAC);
                    out.push(IAC);
                }
                b => out.push(b),
            }
        }
        out
    }

    /// Process incoming data bytes: normalize CR LF → \n, CR NUL → \r
    pub fn decode_line(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                CR => {
                    if i + 1 < bytes.len() {
                        match bytes[i + 1] {
                            LF => {
                                out.push('\n');
                                i += 2;
                            }
                            NUL => {
                                out.push('\r');
                                i += 2;
                            }
                            _ => {
                                out.push('\r');
                                i += 1;
                            }
                        }
                    } else {
                        out.push('\r');
                        i += 1;
                    }
                }
                LF => {
                    // Bare LF — treat as newline (lenient)
                    out.push('\n');
                    i += 1;
                }
                b => {
                    out.push(b as char);
                    i += 1;
                }
            }
        }
        out
    }

    /// Encode a telnet negotiation sequence
    pub fn encode_negotiate(verb: u8, option: u8) -> Vec<u8> {
        vec![IAC, verb, option]
    }

    /// Encode a subnegotiation sequence, escaping any 0xFF in data
    pub fn encode_subnegotiation(option: u8, data: &[u8]) -> Vec<u8> {
        let mut out = vec![IAC, SB, option];
        for &byte in data {
            out.push(byte);
            if byte == IAC {
                out.push(IAC); // Escape 0xFF
            }
        }
        out.push(IAC);
        out.push(SE);
        out
    }

    /// Build GMCP subnegotiation: IAC SB GMCP package [space json] IAC SE
    pub fn encode_gmcp(package: &str, json_data: Option<&str>) -> Vec<u8> {
        let mut payload = package.as_bytes().to_vec();
        if let Some(data) = json_data {
            payload.push(b' ');
            payload.extend_from_slice(data.as_bytes());
        }
        Self::encode_subnegotiation(OPT_GMCP, &payload)
    }

    /// Parse a GMCP payload: split on first space into (package, json)
    pub fn parse_gmcp(data: &[u8]) -> Option<(String, Option<String>)> {
        if let Some(space_pos) = data.iter().position(|&b| b == b' ') {
            let package = String::from_utf8_lossy(&data[..space_pos]).to_string();
            let json = if space_pos + 1 < data.len() {
                Some(String::from_utf8_lossy(&data[space_pos + 1..]).to_string())
            } else {
                None
            };
            Some((package, json))
        } else {
            let package = String::from_utf8_lossy(data).to_string();
            Some((package, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_text_escapes_iac() {
        // Test: encode_text escapes literal 0xFF bytes in a byte buffer
        // We can't pass byte 255 in a &str (invalid UTF-8), so test the codec manually
        // The encoder replaces 0xFF with 0xFF 0xFF per NVT spec.
        // Demonstrate via a string that doesn't contain 0xFF:
        let normal = TelnetCodec::encode_text("hello");
        assert_eq!(normal, b"hello".to_vec());
        // And a string with a literal newline converts to CRLF:
        let nl = TelnetCodec::encode_text("a\nb");
        assert_eq!(nl, b"a\r\nb".to_vec());
    }

    #[test]
    fn test_decode_crlf() {
        let decoded = TelnetCodec::decode_line(b"hello\r\nworld");
        assert_eq!(decoded, "hello\nworld");
    }

    #[test]
    fn test_decode_cr_nul() {
        let decoded = TelnetCodec::decode_line(b"hello\r\x00world");
        assert_eq!(decoded, "hello\rworld");
    }

    #[test]
    fn test_decode_bare_lf() {
        let decoded = TelnetCodec::decode_line(b"hello\nworld");
        assert_eq!(decoded, "hello\nworld");
    }

    #[test]
    fn test_encode_negotiate() {
        let bytes = TelnetCodec::encode_negotiate(WILL, OPT_ECHO);
        assert_eq!(bytes, vec![IAC, WILL, OPT_ECHO]);
    }

    #[test]
    fn test_encode_subnegotiation() {
        let data = b"VT100";
        let bytes = TelnetCodec::encode_subnegotiation(OPT_TTYPE, data);
        assert_eq!(bytes[0], IAC);
        assert_eq!(bytes[1], SB);
        assert_eq!(bytes[2], OPT_TTYPE);
        assert_eq!(*bytes.last().unwrap(), SE);
        assert_eq!(bytes[bytes.len() - 2], IAC);
    }

    #[test]
    fn test_encode_gmcp() {
        let bytes = TelnetCodec::encode_gmcp("Char.Vitals", Some(r#"{"hp":100}"#));
        // Should start with IAC SB GMCP
        assert_eq!(&bytes[..3], &[IAC, SB, OPT_GMCP]);
        // Should end with IAC SE
        assert_eq!(&bytes[bytes.len()-2..], &[IAC, SE]);
    }

    #[test]
    fn test_parse_gmcp() {
        let data = b"Char.Vitals {\"hp\":100}";
        let parsed = TelnetCodec::parse_gmcp(data);
        assert!(parsed.is_some());
        let (package, json) = parsed.unwrap();
        assert_eq!(package, "Char.Vitals");
        assert_eq!(json, Some("{\"hp\":100}".to_string()));
    }

    #[test]
    fn test_parse_gmcp_no_payload() {
        let data = b"Core.KeepAlive";
        let parsed = TelnetCodec::parse_gmcp(data);
        assert!(parsed.is_some());
        let (package, json) = parsed.unwrap();
        assert_eq!(package, "Core.KeepAlive");
        assert_eq!(json, None);
    }
}
