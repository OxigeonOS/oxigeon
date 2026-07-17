use super::constants::*;

/// Events emitted by the Telnet parser
#[derive(Debug, Clone, PartialEq)]
pub enum TelnetEvent {
    /// Raw data bytes (game text / player input)
    Data(Vec<u8>),
    /// IAC command (NOP, AYT, BRK, IP, etc.)
    Command(u8),
    /// Option negotiation: WILL/WONT/DO/DONT + option code
    Negotiate { verb: u8, option: u8 },
    /// Subnegotiation data: option code + payload bytes
    Subnegotiation { option: u8, data: Vec<u8> },
}

/// Internal parser state machine states
#[derive(Debug, Clone, PartialEq)]
enum ParserState {
    Normal,
    InIac,
    InNegotiation(u8), // verb received, awaiting option byte
    InSubnegotiation(u8), // option code, accumulating data
    InSubnegotiationIac(u8), // saw IAC inside subnegotiation
}

/// Byte-level Telnet protocol parser implementing RFC 854.
/// Processes bytes one at a time and emits TelnetEvents.
pub struct TelnetParser {
    state: ParserState,
    data_buffer: Vec<u8>,
    sub_buffer: Vec<u8>,
}

impl TelnetParser {
    pub fn new() -> Self {
        TelnetParser {
            state: ParserState::Normal,
            data_buffer: Vec::new(),
            sub_buffer: Vec::new(),
        }
    }

    /// Feed a single byte into the parser.
    /// May return an event if one is complete.
    /// Note: Data events are returned when a command is encountered,
    /// or when flush() is called.
    pub fn feed(&mut self, byte: u8) -> Option<TelnetEvent> {
        match self.state.clone() {
            ParserState::Normal => {
                if byte == IAC {
                    // Flush any accumulated data before processing command
                    let data_event = self.flush_data();
                    self.state = ParserState::InIac;
                    return data_event;
                } else {
                    self.data_buffer.push(byte);
                    None
                }
            }
            ParserState::InIac => {
                match byte {
                    IAC => {
                        // Escaped IAC — emit as data byte 255
                        self.state = ParserState::Normal;
                        self.data_buffer.push(255);
                        None
                    }
                    WILL | WONT | DO | DONT => {
                        self.state = ParserState::InNegotiation(byte);
                        None
                    }
                    SB => {
                        self.state = ParserState::InSubnegotiation(0); // option TBD
                        self.sub_buffer.clear();
                        None
                    }
                    SE => {
                        // SE without SB — ignore
                        self.state = ParserState::Normal;
                        None
                    }
                    cmd => {
                        // Two-byte command (NOP, DM, BRK, IP, AO, AYT, EC, EL, GA)
                        self.state = ParserState::Normal;
                        Some(TelnetEvent::Command(cmd))
                    }
                }
            }
            ParserState::InNegotiation(verb) => {
                let option = byte;
                self.state = ParserState::Normal;
                Some(TelnetEvent::Negotiate { verb, option })
            }
            ParserState::InSubnegotiation(option) => {
                if option == 0 && self.sub_buffer.is_empty() {
                    // First byte after SB is the option code
                    self.state = ParserState::InSubnegotiation(byte);
                    None
                } else if byte == IAC {
                    self.state = ParserState::InSubnegotiationIac(option);
                    None
                } else {
                    self.sub_buffer.push(byte);
                    None
                }
            }
            ParserState::InSubnegotiationIac(option) => {
                match byte {
                    SE => {
                        // End of subnegotiation
                        let data = self.sub_buffer.clone();
                        self.sub_buffer.clear();
                        self.state = ParserState::Normal;
                        Some(TelnetEvent::Subnegotiation { option, data })
                    }
                    IAC => {
                        // Escaped IAC within subnegotiation
                        self.sub_buffer.push(255);
                        self.state = ParserState::InSubnegotiation(option);
                        None
                    }
                    _ => {
                        // Protocol error — treat as data and continue
                        self.state = ParserState::InSubnegotiation(option);
                        None
                    }
                }
            }
        }
    }

    /// Feed multiple bytes and collect all events produced.
    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Vec<TelnetEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            if let Some(event) = self.feed(byte) {
                events.push(event);
            }
        }
        // Flush any remaining data
        if let Some(event) = self.flush_data() {
            events.push(event);
        }
        events
    }

    /// Flush any buffered data as a Data event.
    pub fn flush(&mut self) -> Option<TelnetEvent> {
        self.flush_data()
    }

    fn flush_data(&mut self) -> Option<TelnetEvent> {
        if self.data_buffer.is_empty() {
            None
        } else {
            let data = self.data_buffer.clone();
            self.data_buffer.clear();
            Some(TelnetEvent::Data(data))
        }
    }
}

impl Default for TelnetParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_data() {
        let mut p = TelnetParser::new();
        let events = p.feed_bytes(b"hello");
        assert_eq!(events, vec![TelnetEvent::Data(b"hello".to_vec())]);
    }

    #[test]
    fn test_iac_escape() {
        // IAC IAC should become data byte 255
        let mut p = TelnetParser::new();
        let events = p.feed_bytes(&[IAC, IAC]);
        assert_eq!(events, vec![TelnetEvent::Data(vec![255])]);
    }

    #[test]
    fn test_negotiate_will() {
        let mut p = TelnetParser::new();
        let events = p.feed_bytes(&[IAC, WILL, OPT_ECHO]);
        assert_eq!(events, vec![TelnetEvent::Negotiate { verb: WILL, option: OPT_ECHO }]);
    }

    #[test]
    fn test_negotiate_do() {
        let mut p = TelnetParser::new();
        let events = p.feed_bytes(&[IAC, DO, OPT_SGA]);
        assert_eq!(events, vec![TelnetEvent::Negotiate { verb: DO, option: OPT_SGA }]);
    }

    #[test]
    fn test_subnegotiation() {
        let mut p = TelnetParser::new();
        // IAC SB TTYPE IS "VT100" IAC SE
        let mut bytes = vec![IAC, SB, OPT_TTYPE, 0];
        bytes.extend_from_slice(b"VT100");
        bytes.extend_from_slice(&[IAC, SE]);
        let events = p.feed_bytes(&bytes);
        assert!(events.iter().any(|e| matches!(e, TelnetEvent::Subnegotiation { option: 24, .. })));
    }

    #[test]
    fn test_subnegotiation_with_escaped_iac() {
        let mut p = TelnetParser::new();
        // IAC SB OPT data IAC IAC more_data IAC SE
        let bytes = vec![IAC, SB, OPT_GMCP, b'A', IAC, IAC, b'B', IAC, SE];
        let events = p.feed_bytes(&bytes);
        let sub = events.iter().find(|e| matches!(e, TelnetEvent::Subnegotiation { .. }));
        assert!(sub.is_some());
        if let Some(TelnetEvent::Subnegotiation { data, .. }) = sub {
            assert_eq!(data, &[b'A', 255, b'B']);
        }
    }

    #[test]
    fn test_command_ayt() {
        let mut p = TelnetParser::new();
        let events = p.feed_bytes(&[IAC, AYT]);
        assert_eq!(events, vec![TelnetEvent::Command(AYT)]);
    }

    #[test]
    fn test_data_then_iac() {
        let mut p = TelnetParser::new();
        let mut bytes = b"hello".to_vec();
        bytes.extend_from_slice(&[IAC, WILL, OPT_ECHO]);
        let events = p.feed_bytes(&bytes);
        assert_eq!(events[0], TelnetEvent::Data(b"hello".to_vec()));
        assert_eq!(events[1], TelnetEvent::Negotiate { verb: WILL, option: OPT_ECHO });
    }

    #[test]
    fn test_gmcp_subnegotiation() {
        let mut p = TelnetParser::new();
        // IAC SB GMCP "Char.Vitals" space json IAC SE
        let payload = b"Char.Vitals {\"hp\":100}";
        let mut bytes = vec![IAC, SB, OPT_GMCP];
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(&[IAC, SE]);
        let events = p.feed_bytes(&bytes);
        assert!(events.iter().any(|e| {
            if let TelnetEvent::Subnegotiation { option, data } = e {
                *option == OPT_GMCP && data.starts_with(b"Char.Vitals")
            } else {
                false
            }
        }));
    }
}
