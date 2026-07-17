use super::constants::*;

/// Q Method state per RFC 1143 — prevents infinite negotiation loops.
/// Each option has independent state for "us" (local) and "him" (remote).
#[derive(Debug, Clone, PartialEq)]
pub enum QState {
    /// Option disabled, no negotiation pending
    No,
    /// Option enabled, no negotiation pending
    Yes,
    /// We sent WONT/DONT, awaiting response. queue=want to re-enable
    WantNo { queue: bool },
    /// We sent WILL/DO, awaiting response. queue=want to disable
    WantYes { queue: bool },
}

impl Default for QState {
    fn default() -> Self {
        QState::No
    }
}

/// Commands produced by the option negotiator
#[derive(Debug, Clone, PartialEq)]
pub enum NegotiationCommand {
    SendWill(u8),
    SendWont(u8),
    SendDo(u8),
    SendDont(u8),
}

impl NegotiationCommand {
    /// Encode the command as bytes to send over the wire
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            NegotiationCommand::SendWill(opt) => vec![IAC, WILL, *opt],
            NegotiationCommand::SendWont(opt) => vec![IAC, WONT, *opt],
            NegotiationCommand::SendDo(opt) => vec![IAC, DO, *opt],
            NegotiationCommand::SendDont(opt) => vec![IAC, DONT, *opt],
        }
    }
}

/// Events returned when option state changes
#[derive(Debug, Clone, PartialEq)]
pub enum NegotiationResult {
    /// Send this command to the peer
    Send(NegotiationCommand),
    /// Option is now enabled (local or remote)
    Enabled,
    /// Option is now disabled
    Disabled,
    /// No action needed
    None,
}

/// Per-option Q Method state. Tracks both local (us) and remote (him) sides.
#[derive(Debug, Clone, Default)]
pub struct OptionState {
    /// What WE do with this option
    pub us: QState,
    /// What THEY do with this option  
    pub him: QState,
}

/// RFC 1143 Q Method option negotiation state machine.
/// Tracks state for all 256 possible Telnet options.
pub struct OptionNegotiator {
    options: std::collections::HashMap<u8, OptionState>,
}

impl OptionNegotiator {
    pub fn new() -> Self {
        OptionNegotiator {
            options: std::collections::HashMap::new(),
        }
    }

    fn get_state(&mut self, option: u8) -> &mut OptionState {
        self.options.entry(option).or_default()
    }

    /// Check if an option is enabled locally (we are performing it)
    pub fn is_local_enabled(&self, option: u8) -> bool {
        self.options.get(&option).map(|s| s.us == QState::Yes).unwrap_or(false)
    }

    /// Check if an option is enabled remotely (they are performing it)
    pub fn is_remote_enabled(&self, option: u8) -> bool {
        self.options.get(&option).map(|s| s.him == QState::Yes).unwrap_or(false)
    }

    /// We want to enable an option locally (send WILL).
    /// Returns the command to send, if any.
    pub fn request_local_enable(&mut self, option: u8) -> Option<NegotiationCommand> {
        let state = self.get_state(option);
        match state.us {
            QState::No => {
                state.us = QState::WantYes { queue: false };
                Some(NegotiationCommand::SendWill(option))
            }
            QState::Yes => None, // Already enabled
            QState::WantNo { .. } => {
                state.us = QState::WantNo { queue: true };
                None
            }
            QState::WantYes { .. } => {
                state.us = QState::WantYes { queue: false };
                None
            }
        }
    }

    /// We want to disable an option locally (send WONT).
    pub fn request_local_disable(&mut self, option: u8) -> Option<NegotiationCommand> {
        let state = self.get_state(option);
        match state.us {
            QState::No => None, // Already disabled
            QState::Yes => {
                state.us = QState::WantNo { queue: false };
                Some(NegotiationCommand::SendWont(option))
            }
            QState::WantNo { .. } => {
                state.us = QState::WantNo { queue: false };
                None
            }
            QState::WantYes { .. } => {
                state.us = QState::WantYes { queue: true };
                None
            }
        }
    }

    /// We want them to enable an option remotely (send DO).
    pub fn request_remote_enable(&mut self, option: u8) -> Option<NegotiationCommand> {
        let state = self.get_state(option);
        match state.him {
            QState::No => {
                state.him = QState::WantYes { queue: false };
                Some(NegotiationCommand::SendDo(option))
            }
            QState::Yes => None,
            QState::WantNo { .. } => {
                state.him = QState::WantNo { queue: true };
                None
            }
            QState::WantYes { .. } => {
                state.him = QState::WantYes { queue: false };
                None
            }
        }
    }

    /// We want them to disable a remote option (send DONT).
    pub fn request_remote_disable(&mut self, option: u8) -> Option<NegotiationCommand> {
        let state = self.get_state(option);
        match state.him {
            QState::No => None,
            QState::Yes => {
                state.him = QState::WantNo { queue: false };
                Some(NegotiationCommand::SendDont(option))
            }
            QState::WantNo { .. } => {
                state.him = QState::WantNo { queue: false };
                None
            }
            QState::WantYes { .. } => {
                state.him = QState::WantYes { queue: true };
                None
            }
        }
    }

    /// Handle received WILL from peer (they want to enable a remote option).
    /// Returns commands to send and whether option is now enabled.
    pub fn receive_will(&mut self, option: u8) -> (Option<NegotiationCommand>, bool) {
        let state = self.get_state(option);
        match state.him {
            QState::No => {
                // Unsolicited WILL — we accept (default policy: accept all)
                state.him = QState::Yes;
                (Some(NegotiationCommand::SendDo(option)), true)
            }
            QState::Yes => (None, true), // Already on, ignore
            QState::WantNo { queue } => {
                if queue {
                    state.him = QState::WantNo { queue: false };
                    (Some(NegotiationCommand::SendDont(option)), false)
                } else {
                    state.him = QState::No;
                    (None, false)
                }
            }
            QState::WantYes { queue } => {
                if queue {
                    state.him = QState::WantNo { queue: false };
                    (Some(NegotiationCommand::SendDont(option)), false)
                } else {
                    state.him = QState::Yes;
                    (None, true)
                }
            }
        }
    }

    /// Handle received WONT from peer.
    pub fn receive_wont(&mut self, option: u8) -> (Option<NegotiationCommand>, bool) {
        let state = self.get_state(option);
        match state.him {
            QState::No => (None, false), // Already off
            QState::Yes => {
                state.him = QState::No;
                (Some(NegotiationCommand::SendDont(option)), false)
            }
            QState::WantNo { queue } => {
                if queue {
                    state.him = QState::WantYes { queue: false };
                    (Some(NegotiationCommand::SendDo(option)), false)
                } else {
                    state.him = QState::No;
                    (None, false)
                }
            }
            QState::WantYes { .. } => {
                state.him = QState::No;
                (None, false)
            }
        }
    }

    /// Handle received DO from peer (they want us to enable a local option).
    pub fn receive_do(&mut self, option: u8) -> (Option<NegotiationCommand>, bool) {
        let state = self.get_state(option);
        match state.us {
            QState::No => {
                // Unsolicited DO — we accept by default
                state.us = QState::Yes;
                (Some(NegotiationCommand::SendWill(option)), true)
            }
            QState::Yes => (None, true),
            QState::WantNo { queue } => {
                if queue {
                    state.us = QState::WantNo { queue: false };
                    (Some(NegotiationCommand::SendWont(option)), false)
                } else {
                    state.us = QState::No;
                    (None, false)
                }
            }
            QState::WantYes { queue } => {
                if queue {
                    state.us = QState::WantNo { queue: false };
                    (Some(NegotiationCommand::SendWont(option)), false)
                } else {
                    state.us = QState::Yes;
                    (None, true)
                }
            }
        }
    }

    /// Handle received DONT from peer.
    pub fn receive_dont(&mut self, option: u8) -> (Option<NegotiationCommand>, bool) {
        let state = self.get_state(option);
        match state.us {
            QState::No => (None, false),
            QState::Yes => {
                state.us = QState::No;
                (Some(NegotiationCommand::SendWont(option)), false)
            }
            QState::WantNo { queue } => {
                if queue {
                    state.us = QState::WantYes { queue: false };
                    (Some(NegotiationCommand::SendWill(option)), false)
                } else {
                    state.us = QState::No;
                    (None, false)
                }
            }
            QState::WantYes { .. } => {
                state.us = QState::No;
                (None, false)
            }
        }
    }
}

impl Default for OptionNegotiator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_local_enable_sends_will() {
        let mut neg = OptionNegotiator::new();
        let cmd = neg.request_local_enable(OPT_ECHO);
        assert_eq!(cmd, Some(NegotiationCommand::SendWill(OPT_ECHO)));
        assert!(matches!(neg.get_state(OPT_ECHO).us, QState::WantYes { .. }));
    }

    #[test]
    fn test_receive_do_after_will_enables_option() {
        let mut neg = OptionNegotiator::new();
        neg.request_local_enable(OPT_ECHO);
        let (cmd, enabled) = neg.receive_do(OPT_ECHO);
        assert!(enabled);
        assert_eq!(cmd, None); // No unsolicited response needed
        assert!(neg.is_local_enabled(OPT_ECHO));
    }

    #[test]
    fn test_receive_dont_disables_option() {
        let mut neg = OptionNegotiator::new();
        neg.request_local_enable(OPT_ECHO);
        neg.receive_do(OPT_ECHO);
        let (_cmd, enabled) = neg.receive_dont(OPT_ECHO);
        assert!(!enabled);
        assert!(!neg.is_local_enabled(OPT_ECHO));
    }

    #[test]
    fn test_no_infinite_loop_duplicate_request() {
        let mut neg = OptionNegotiator::new();
        // Send WILL
        neg.request_local_enable(OPT_ECHO);
        // Send WILL again while awaiting response — should not send another
        let cmd = neg.request_local_enable(OPT_ECHO);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_receive_will_unsolicited() {
        let mut neg = OptionNegotiator::new();
        let (cmd, enabled) = neg.receive_will(OPT_SGA);
        // Default policy: accept with DO
        assert_eq!(cmd, Some(NegotiationCommand::SendDo(OPT_SGA)));
        assert!(enabled);
        assert!(neg.is_remote_enabled(OPT_SGA));
    }

    #[test]
    fn test_receive_wont_clears_state() {
        let mut neg = OptionNegotiator::new();
        neg.request_remote_enable(OPT_GMCP);
        let (_cmd, enabled) = neg.receive_wont(OPT_GMCP);
        assert!(!enabled);
        assert!(!neg.is_remote_enabled(OPT_GMCP));
    }

    #[test]
    fn test_already_enabled_no_duplicate() {
        let mut neg = OptionNegotiator::new();
        neg.request_local_enable(OPT_SGA);
        neg.receive_do(OPT_SGA);
        // Try to enable again — should be no-op
        let cmd = neg.request_local_enable(OPT_SGA);
        assert_eq!(cmd, None);
    }

    #[test]
    fn test_negotiation_command_to_bytes() {
        assert_eq!(NegotiationCommand::SendWill(OPT_ECHO).to_bytes(), vec![IAC, WILL, OPT_ECHO]);
        assert_eq!(NegotiationCommand::SendWont(OPT_ECHO).to_bytes(), vec![IAC, WONT, OPT_ECHO]);
        assert_eq!(NegotiationCommand::SendDo(OPT_ECHO).to_bytes(), vec![IAC, DO, OPT_ECHO]);
        assert_eq!(NegotiationCommand::SendDont(OPT_ECHO).to_bytes(), vec![IAC, DONT, OPT_ECHO]);
    }
}
