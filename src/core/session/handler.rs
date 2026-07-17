use std::collections::HashMap;
use crate::config::server_config::MultisessionMode;
use crate::error::{OxigeonError, Result};
use super::session::{Session, SessionId, SessionOutput, SessionState};

/// Central registry of all active sessions.
/// Non-persistent — cleared on shutdown.
pub struct SessionHandler {
    sessions: HashMap<SessionId, Session>,
    multisession_mode: MultisessionMode,
    max_connections: usize,
}

impl SessionHandler {
    pub fn new(multisession_mode: MultisessionMode, max_connections: usize) -> Self {
        SessionHandler {
            sessions: HashMap::new(),
            multisession_mode,
            max_connections,
        }
    }

    /// Register a new session. Returns its ID.
    pub fn connect(&mut self, session: Session) -> Result<SessionId> {
        if self.sessions.len() >= self.max_connections {
            return Err(OxigeonError::MaxConnectionsReached);
        }
        let id = session.id;
        self.sessions.insert(id, session);
        tracing::debug!("Session connected: {} (total: {})", id, self.sessions.len());
        Ok(id)
    }

    /// Remove and return a session (for cleanup on disconnect).
    pub fn disconnect(&mut self, id: &SessionId) -> Option<Session> {
        let session = self.sessions.remove(id);
        if session.is_some() {
            tracing::debug!("Session disconnected: {} (total: {})", id, self.sessions.len());
        }
        session
    }

    /// Transition a session to Authenticated.
    /// Enforces multisession policy.
    /// Returns the SessionId of any kicked session (in Single mode).
    pub fn authenticate(&mut self, id: &SessionId, account_id: i64) -> Result<Option<SessionId>> {
        if !self.sessions.contains_key(id) {
            return Err(OxigeonError::SessionNotFound(id.to_string()));
        }

        let mut kicked_id: Option<SessionId> = None;

        match self.multisession_mode {
            MultisessionMode::Single => {
                // Find any existing session for this account and kick it
                let existing: Vec<SessionId> = self.sessions
                    .iter()
                    .filter(|(sid, s)| **sid != *id && s.state.account_id() == Some(account_id))
                    .map(|(sid, _)| *sid)
                    .collect();

                for old_id in existing {
                    if let Some(old_session) = self.sessions.get(&old_id) {
                        let _ = old_session.output_tx.try_send(SessionOutput::Disconnect);
                    }
                    self.sessions.remove(&old_id);
                    kicked_id = Some(old_id);
                    tracing::info!("Kicked old session {} for account {} (single mode)", old_id, account_id);
                }
            }
            MultisessionMode::SharedCharacter |
            MultisessionMode::MultiCharacter |
            MultisessionMode::FullMulti => {
                // Allow multiple sessions — no kicking
            }
        }

        if let Some(session) = self.sessions.get_mut(id) {
            session.state = SessionState::Authenticated { account_id };
        }

        Ok(kicked_id)
    }

    /// Transition a session to Playing and populate its permission cache.
    pub fn enter_game(
        &mut self,
        id: &SessionId,
        account_id: i64,
        character_id: i64,
        permissions: Vec<String>,
        is_admin: bool,
    ) -> Result<()> {
        let session = self.sessions.get_mut(id)
            .ok_or_else(|| OxigeonError::SessionNotFound(id.to_string()))?;
        session.state = SessionState::Playing { account_id, character_id };
        session.permissions.clear();
        if is_admin {
            session.permissions.insert("**superuser**".to_string());
        }
        for perm in permissions {
            session.permissions.insert(perm);
        }
        Ok(())
    }

    /// Check if a session has a specific permission.
    /// Superusers (indicated by "**superuser**" sentinel) always return true.
    pub fn has_permission(&self, id: &SessionId, perm: &str) -> bool {
        match self.sessions.get(id) {
            None => false,
            Some(s) => s.permissions.contains("**superuser**")
                    || s.permissions.contains(perm),
        }
    }

    /// Load permissions into a session's cache (for refresh_permissions efun).
    pub fn set_permissions(
        &mut self,
        id: &SessionId,
        permissions: Vec<String>,
        is_admin: bool,
    ) -> Result<()> {
        let session = self.sessions.get_mut(id)
            .ok_or_else(|| OxigeonError::SessionNotFound(id.to_string()))?;
        session.permissions.clear();
        if is_admin {
            session.permissions.insert("**superuser**".to_string());
        }
        for perm in permissions {
            session.permissions.insert(perm);
        }
        Ok(())
    }

    /// Update a session's state by name (used from Lua efuns).
    pub fn set_state_by_name(&mut self, id: &SessionId, state_name: &str) -> Result<()> {
        let session = self.sessions.get_mut(id)
            .ok_or_else(|| OxigeonError::SessionNotFound(id.to_string()))?;
        session.state = match state_name {
            "connected" => SessionState::Connected,
            "authenticating" => SessionState::Authenticating,
            _ => return Err(OxigeonError::Internal(
                format!("Cannot set state '{}' via name (use authenticate/enter_game)", state_name)
            )),
        };
        Ok(())
    }

    pub fn get(&self, id: &SessionId) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn get_mut(&mut self, id: &SessionId) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// All sessions for a given account
    pub fn sessions_for_account(&self, account_id: i64) -> Vec<&Session> {
        self.sessions.values()
            .filter(|s| s.state.account_id() == Some(account_id))
            .collect()
    }

    /// Send text to all connected sessions
    pub fn broadcast(&self, text: &str) {
        for session in self.sessions.values() {
            let _ = session.output_tx.try_send(SessionOutput::Text(text.to_string()));
        }
    }

    /// Send text to a specific session
    pub fn send_to(&self, id: &SessionId, text: &str) -> Result<()> {
        let session = self.sessions.get(id)
            .ok_or_else(|| OxigeonError::SessionNotFound(id.to_string()))?;
        session.output_tx.try_send(SessionOutput::Text(text.to_string()))
            .map_err(|e| OxigeonError::Internal(format!("Channel send error: {}", e)))?;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    /// All active session IDs
    pub fn all_ids(&self) -> Vec<SessionId> {
        self.sessions.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use tokio::sync::mpsc;

    fn make_session() -> (Session, mpsc::Receiver<SessionOutput>) {
        let (tx, rx) = mpsc::channel(16);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345);
        let session = Session::new("telnet".to_string(), addr, tx);
        (session, rx)
    }

    use std::net::SocketAddr;

    #[test]
    fn test_connect_and_disconnect() {
        let mut handler = SessionHandler::new(MultisessionMode::Single, 256);
        let (session, _rx) = make_session();
        let id = handler.connect(session).unwrap();
        assert_eq!(handler.count(), 1);
        let removed = handler.disconnect(&id);
        assert!(removed.is_some());
        assert_eq!(handler.count(), 0);
    }

    #[test]
    fn test_max_connections() {
        let mut handler = SessionHandler::new(MultisessionMode::Single, 2);
        let (s1, _r1) = make_session();
        let (s2, _r2) = make_session();
        let (s3, _r3) = make_session();
        handler.connect(s1).unwrap();
        handler.connect(s2).unwrap();
        let result = handler.connect(s3);
        assert!(matches!(result, Err(OxigeonError::MaxConnectionsReached)));
    }

    #[test]
    fn test_authenticate_single_mode_kicks_old() {
        let mut handler = SessionHandler::new(MultisessionMode::Single, 256);
        let (s1, _r1) = make_session();
        let (s2, _r2) = make_session();

        let id1 = handler.connect(s1).unwrap();
        let id2 = handler.connect(s2).unwrap();

        // First session logs in as account 1
        handler.authenticate(&id1, 1).unwrap();
        assert_eq!(handler.count(), 2);

        // Second session also logs in as account 1 — should kick first
        let kicked = handler.authenticate(&id2, 1).unwrap();
        assert_eq!(kicked, Some(id1));
        assert_eq!(handler.count(), 1);
    }

    #[test]
    fn test_authenticate_multi_mode_no_kick() {
        let mut handler = SessionHandler::new(MultisessionMode::SharedCharacter, 256);
        let (s1, _r1) = make_session();
        let (s2, _r2) = make_session();

        let id1 = handler.connect(s1).unwrap();
        let id2 = handler.connect(s2).unwrap();

        handler.authenticate(&id1, 1).unwrap();
        let kicked = handler.authenticate(&id2, 1).unwrap();
        assert_eq!(kicked, None);
        assert_eq!(handler.count(), 2);
    }

    #[test]
    fn test_sessions_for_account() {
        let mut handler = SessionHandler::new(MultisessionMode::FullMulti, 256);
        let (s1, _r1) = make_session();
        let (s2, _r2) = make_session();
        let (s3, _r3) = make_session();

        let id1 = handler.connect(s1).unwrap();
        let id2 = handler.connect(s2).unwrap();
        let id3 = handler.connect(s3).unwrap();

        handler.authenticate(&id1, 1).unwrap();
        handler.authenticate(&id2, 1).unwrap();
        handler.authenticate(&id3, 2).unwrap();

        let account1_sessions = handler.sessions_for_account(1);
        assert_eq!(account1_sessions.len(), 2);

        let account2_sessions = handler.sessions_for_account(2);
        assert_eq!(account2_sessions.len(), 1);
    }

    #[test]
    fn test_set_state_to_authenticating() {
        let mut handler = SessionHandler::new(MultisessionMode::Single, 256);
        let (session, _rx) = make_session();
        let id = handler.connect(session).unwrap();
        handler.set_state_by_name(&id, "authenticating").unwrap();
        let s = handler.get(&id).unwrap();
        assert_eq!(s.state, SessionState::Authenticating);
    }
}
