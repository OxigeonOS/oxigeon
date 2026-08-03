//! Output used to vanish under backpressure with no log, no counter and no
//! marker. Ten send sites were all `let _ = try_send(..)` against a 64-slot
//! channel, so a player on a slow link — or any burst over 64 messages — just
//! lost text. It presented as "the MUD ate my output" and was close to
//! impossible to reproduce on demand.
//!
//! Dropping is still the only option: every one of those callers is on the Lua
//! thread, which is the whole game. What changed is that the loss is now
//! counted and the player is told.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use oxigeon::config::MultisessionMode;
use oxigeon::core::session::{Session, SessionHandler, SessionOutput};
use tokio::sync::mpsc;

/// A session whose reader never drains, with a channel small enough to fill.
fn stalled_session(capacity: usize) -> (Session, mpsc::Receiver<SessionOutput>) {
    let (tx, rx) = mpsc::channel(capacity);
    let addr: SocketAddr = "127.0.0.1:4000".parse().unwrap();
    (Session::new("telnet".to_string(), addr, tx), rx)
}

#[test]
fn a_full_channel_is_counted_rather_than_swallowed() {
    let (session, _rx) = stalled_session(4);

    for i in 0..4 {
        assert!(
            session.try_send(SessionOutput::Text(format!("line {i}"))),
            "the first {} sends should fit",
            4
        );
    }
    assert_eq!(session.dropped_output(), 0);

    for _ in 0..10 {
        assert!(!session.try_send(SessionOutput::Text("overflow".into())));
    }
    assert_eq!(session.dropped_output(), 10, "every drop must be counted");
}

/// The player finds out. Once the reader catches up, the next send is preceded
/// by a marker saying output was lost — which is the difference between a bug
/// report that can be acted on and "the MUD ate my text".
#[test]
fn the_player_is_told_that_output_was_truncated() {
    let (session, mut rx) = stalled_session(2);

    session.try_send(SessionOutput::Text("first".into()));
    session.try_send(SessionOutput::Text("second".into()));
    assert!(!session.try_send(SessionOutput::Text("lost".into())));

    // Drain, so there is room to say something.
    let mut seen = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let SessionOutput::Text(t) = msg {
            seen.push(t);
        }
    }
    assert_eq!(seen, vec!["first", "second"]);

    assert!(session.try_send(SessionOutput::Text("third".into())));
    let mut after = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let SessionOutput::Text(t) = msg {
            after.push(t);
        }
    }
    assert_eq!(after.len(), 2, "expected a marker and then the message");
    assert!(
        after[0].contains("output truncated"),
        "the marker should come first, got {after:?}"
    );
    assert_eq!(after[1], "third");
}

/// One notice per burst, not one per lost message — otherwise recovering from
/// a 500-message overflow would itself flood the player.
#[test]
fn the_truncation_notice_is_not_repeated_for_every_lost_message() {
    let (session, mut rx) = stalled_session(2);

    session.try_send(SessionOutput::Text("a".into()));
    session.try_send(SessionOutput::Text("b".into()));
    for _ in 0..50 {
        session.try_send(SessionOutput::Text("lost".into()));
    }
    while rx.try_recv().is_ok() {}

    for i in 0..2 {
        session.try_send(SessionOutput::Text(format!("recovered {i}")));
    }
    let mut texts = Vec::new();
    while let Ok(SessionOutput::Text(t)) = rx.try_recv() {
        texts.push(t);
    }
    let markers = texts.iter().filter(|t| t.contains("output truncated")).count();
    assert_eq!(markers, 1, "one notice per burst; got {texts:?}");
}

/// A closed channel is not a drop. The player has already gone; counting it
/// would make every ordinary disconnect look like lost output.
#[test]
fn a_closed_channel_is_not_recorded_as_a_drop() {
    let (session, rx) = stalled_session(4);
    drop(rx);

    assert!(!session.try_send(SessionOutput::Text("nobody home".into())));
    assert_eq!(session.dropped_output(), 0);
}

/// `broadcast` reports how many sessions could not take it, so an operator can
/// see that a server-wide message did not land everywhere.
#[test]
fn broadcast_reports_how_many_sessions_it_could_not_reach() {
    let mut handler = SessionHandler::new(MultisessionMode::Single, 8);

    let (healthy, _healthy_rx) = stalled_session(16);
    let (stalled, stalled_rx) = stalled_session(1);
    handler.connect(healthy).unwrap();
    handler.connect(stalled).unwrap();

    // Fill the stalled session's single slot.
    assert_eq!(handler.broadcast("first"), 0);
    assert_eq!(handler.broadcast("second"), 1, "one session is now full");
    assert_eq!(handler.dropped_output_total(), 1);

    drop(stalled_rx);
}

/// The counter is reachable from Lua, which is where an admin command would
/// read it.
#[test]
fn the_handler_totals_drops_across_sessions() {
    let mut handler = SessionHandler::new(MultisessionMode::Single, 8);
    let (a, _a_rx) = stalled_session(1);
    let (b, _b_rx) = stalled_session(1);
    handler.connect(a).unwrap();
    handler.connect(b).unwrap();

    assert_eq!(handler.dropped_output_total(), 0);
    handler.broadcast("fills both");
    handler.broadcast("drops on both");
    handler.broadcast("drops on both again");
    assert_eq!(handler.dropped_output_total(), 4);
}

/// Sessions with a shared `Arc<RwLock<..>>` are reached through a read lock, so
/// the counter has to work through `&Session` — this is the shape every efun
/// uses.
#[test]
fn drops_are_recorded_through_a_shared_read_lock() {
    let handler = Arc::new(RwLock::new(SessionHandler::new(MultisessionMode::Single, 4)));
    let (session, _rx) = stalled_session(1);
    let id = handler.write().unwrap().connect(session).unwrap();

    {
        let guard = handler.read().unwrap();
        let s = guard.get(&id).unwrap();
        assert!(s.try_send(SessionOutput::Text("fits".into())));
        assert!(!s.try_send(SessionOutput::Text("does not".into())));
    }

    assert_eq!(handler.read().unwrap().get(&id).unwrap().dropped_output(), 1);
}
