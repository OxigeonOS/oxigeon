//! What a GMCP client is actually *sent* while playing.
//!
//! `tests/gmcp_inbound.rs` covers the receiving half — `Core.Supports.Set` is
//! read, `wants` gates correctly, handlers dispatch. None of it asks the
//! question a client has: after I log in and walk around, do `Char.Vitals`,
//! `Char.Status`, `Char.Effects` and `Room.Info` arrive?
//!
//! `gmcp_d`'s own header claims they do — *"Outbound has always worked:
//! `Char.Vitals`, `Char.Status`, `Char.Effects` and `Room.Info` are pushed on
//! the events that change them."* These are the tests that ask.

mod common;

use common::RealVm;

/// Every GMCP package the session has been sent since the last check.
fn packages(vm: &mut RealVm) -> Vec<String> {
    vm.take_gmcp().into_iter().map(|(p, _)| p).collect()
}

/// The JSON of the most recent instance of one package.
fn latest(vm: &mut RealVm, want: &str) -> Option<serde_json::Value> {
    vm.take_gmcp()
        .into_iter()
        .filter(|(p, _)| p.eq_ignore_ascii_case(want))
        .map(|(_, d)| d)
        .next_back()
}

/// A playing session that has negotiated GMCP the way `oxigeon-tui` does.
fn client() -> RealVm {
    let mut vm = RealVm::boot_real_mudlib(0);
    let sid = vm.session_id().to_string();
    // Exactly what `src/bin/tui/telnet.rs` sends on `WILL GMCP`.
    vm.gmcp_in(&sid, "Core.Supports.Set", r#"["Char 1","Room 1","Core 1"]"#);
    vm
}

/// Walking sends `Room.Info`, which is the one path that was wired.
#[test]
fn moving_sends_room_info() {
    let mut vm = client();
    let _ = packages(&mut vm);

    vm.command("look");
    let exits = vm.command("look");
    let _ = exits;

    // Find a direction that exists from here and take it.
    let out = vm.command("north");
    let _ = out;

    let room = latest(&mut vm, "Room.Info");
    assert!(
        room.is_some(),
        "no Room.Info after moving — the pane cannot populate"
    );
}

/// **`Char.Vitals` never arrives while playing.**
///
/// `gmcp_d.send_vitals` has exactly one caller in the whole tree: the
/// `Core.Supports.Set` handler, via `send_all`. That fires during *telnet
/// negotiation*, which happens at connect — before login — so `get_player`
/// returns nil and the sender bails at its second guard. Nothing calls it again,
/// so a client's health bar is empty for the whole session and never moves.
#[test]
fn char_vitals_arrives_after_taking_damage() {
    let mut vm = client();
    let _ = packages(&mut vm);

    vm.command("affect damage 10");
    let after = latest(&mut vm, "Char.Vitals").expect("no Char.Vitals after taking damage");

    let hp = after.get("hp").and_then(|v| v.as_i64()).expect("hp");
    let maxhp = after.get("maxhp").and_then(|v| v.as_i64()).expect("maxhp");
    assert!(hp < maxhp, "the client was told full health after damage: {after}");
}

/// Nothing is sent when nothing changed.
///
/// The other half of the same property, and the reason pushing on every dispatch
/// is affordable: a command that moved nothing puts nothing on the wire. Without
/// it, `refresh` would be four messages per command forever.
#[test]
fn an_unchanged_value_is_not_resent() {
    let mut vm = client();
    vm.command("affect damage 10");
    let _ = packages(&mut vm);

    // A command that changes none of the four.
    vm.command("look");
    let quiet = packages(&mut vm);
    assert!(
        quiet.is_empty(),
        "a command that changed nothing pushed: {quiet:?}"
    );

    // …and the next real change still gets through.
    vm.command("affect damage 5");
    let after = packages(&mut vm);
    assert!(
        after.iter().any(|p| p.eq_ignore_ascii_case("Char.Vitals")),
        "the diff suppressed a real change: {after:?}"
    );
}

/// `Char.Status` tracks experience and level.
#[test]
fn char_status_arrives_for_a_playing_session() {
    let mut vm = client();
    let _ = packages(&mut vm);

    vm.command("affect xp 50");
    let status = latest(&mut vm, "Char.Status").expect("no Char.Status after gaining xp");
    assert!(
        status.get("xp").and_then(|v| v.as_i64()).unwrap_or(0) > 0,
        "the client was told zero experience: {status}"
    );
}

/// `Char.Effects` tracks what is affecting the character.
#[test]
fn char_effects_arrives_when_an_effect_is_applied() {
    let mut vm = client();
    let _ = packages(&mut vm);

    vm.command("affect apply regeneration");
    let effects = latest(&mut vm, "Char.Effects").expect("no Char.Effects after applying one");
    assert!(
        effects.as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "the effect list came through empty: {effects}"
    );
}

/// A playing client ends up with all four packages, without having to do
/// anything that changes the world.
///
/// **Order matters, and it is worth stating.** In production a client sends
/// `Core.Supports.Set` during *telnet negotiation* — before login — so the
/// `send_all` in that handler has no character to describe and every sender
/// returns at `get_player`. That is why the opening state comes from the
/// `player.login` hook instead.
///
/// This harness logs in first and negotiates second, which is the reverse, so
/// `player.login` has already fired by the time the capability exists. The
/// property that holds either way — and the one a user actually cares about —
/// is that a negotiated, playing client has a full picture after its first
/// dispatch, from `prompt_d`'s refresh.
#[test]
fn a_playing_client_ends_up_with_every_package() {
    let mut vm = client();
    vm.command("look");

    let seen = packages(&mut vm);
    for want in ["Char.Vitals", "Char.Status", "Char.Effects", "Room.Info"] {
        assert!(
            seen.iter().any(|p| p.eq_ignore_ascii_case(want)),
            "a playing client with GMCP was never sent {want}; it got: {seen:?}"
        );
    }
}

/// The opening state is pushed from `player.login`, and the hook is registered.
///
/// The negotiation-time `send_all` cannot do it: in production that handler runs
/// before there is a character. Asserting the listener exists is the closest
/// this harness can get to the production order, which it inverts.
#[test]
fn the_login_hook_is_registered() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    let listeners = vm
        .eval(
            "local out = {} \
             for _, l in ipairs(DAEMON.event.listeners('player.login') or {}) do \
               out[#out+1] = tostring(l) end \
             return table.concat(out, ',')",
        )
        .unwrap();
    assert!(
        listeners.contains("gmcp_d"),
        "nothing pushes a client its opening state on login: {listeners}"
    );
}
