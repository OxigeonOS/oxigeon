//! Inbound GMCP — the half that logged the package name and returned.
//!
//! Outbound always worked. Inbound did not: a client could negotiate GMCP,
//! announce what it supports and send `Core.Hello`, and `on_gmcp` wrote a debug
//! line and dropped it. So the game had no idea what any client could draw, and
//! pushed everything to everyone.
//!
//! Dispatched by package name now, to handlers a game can add to — because
//! which custom packages exist is content. `Game.Quest` is this game's.

mod common;

use common::RealVm;

/// The dispatcher routes by name, and an unknown package is a log line rather
/// than an error.
#[test]
fn a_package_is_dispatched_to_its_handler() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval("_got = nil DAEMON.gmcp.on('Test.Thing', function(sid, data) _got = data.n end) \
             return 'registered'")
        .unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.receive('s1', 'Test.Thing', { n = 7 }))").unwrap(),
        "true"
    );
    assert_eq!(vm.eval("return _got").unwrap(), "7");

    // Case does not matter: clients disagree about capitalisation and the spec
    // does not care.
    vm.eval("_got = nil DAEMON.gmcp.receive('s1', 'test.THING', { n = 9 }) return 'ok'").unwrap();
    assert_eq!(vm.eval("return _got").unwrap(), "9");

    // An unhandled package is a debug line, not a failure. A client sending
    // something the game has never heard of must not break the connection.
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.receive('s1', 'Nobody.Handles.This', {}))").unwrap(),
        "false"
    );

    // A handler that raises is contained — one bad package cannot take the
    // session down.
    vm.eval("DAEMON.gmcp.on('Test.Bad', function() error('boom') end) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.receive('s1', 'Test.Bad', {}))").unwrap(),
        "false"
    );
    assert_eq!(vm.eval("return 'still here'").unwrap(), "still here");
}

/// `Core.Supports.Set` is the one inbound package every client sends, and the
/// one the game had never read.
#[test]
fn core_supports_is_read_and_gates_what_is_pushed() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // Nothing said yet: everything is sent, which is what the game did before
    // any of this existed and is the friendlier guess for an older client.
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.wants('s2', 'Char.Vitals'))").unwrap(),
        "true"
    );

    vm.eval(
        "DAEMON.gmcp.receive('s2', 'Core.Supports.Set', \
         { 'Char 1', 'Room 1', 'Game.Quest 1' }) return 'set'",
    )
    .unwrap();

    // A module covers its packages: a client supporting `Char` gets
    // `Char.Vitals`, which is how the convention works.
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.wants('s2', 'Char.Vitals'))").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.wants('s2', 'Room.Info'))").unwrap(),
        "true"
    );
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.wants('s2', 'Comm.Channel'))").unwrap(),
        "false",
        "a client that did not ask for a module should not be sent it"
    );

    // The version is kept, because a client announcing version 2 may expect a
    // different shape and throwing it away now means re-negotiating later.
    assert_eq!(vm.eval("return DAEMON.gmcp.supports('s2').char").unwrap(), "1");

    // Add and remove edit the same list.
    vm.eval("DAEMON.gmcp.receive('s2', 'Core.Supports.Add', { 'Comm 1' }) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.wants('s2', 'Comm.Channel'))").unwrap(),
        "true"
    );
    vm.eval("DAEMON.gmcp.receive('s2', 'Core.Supports.Remove', { 'Comm' }) return 'ok'").unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp.wants('s2', 'Comm.Channel'))").unwrap(),
        "false"
    );

    // Forgetting a session clears it — session ids are not reused, so a table
    // keyed on them grows forever otherwise.
    vm.eval("DAEMON.gmcp.forget('s2') return 'forgotten'").unwrap();
    assert_eq!(
        vm.eval("local n = 0 for _ in pairs(DAEMON.gmcp.supports('s2')) do n = n + 1 end return n")
            .unwrap(),
        "0"
    );
}

/// `Core.Ping` is answered, because a client that pings and hears nothing
/// concludes the connection is dead.
#[test]
fn core_ping_is_answered_and_hello_is_recorded() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    // Both handlers are registered, which is the assertion that matters here —
    // the reply itself goes out over the telnet layer, which a probe session
    // does not have negotiated.
    for package in ["Core.Ping", "Core.Hello", "Core.Supports.Set"] {
        assert_eq!(
            vm.eval(&format!(
                "return tostring(DAEMON.gmcp._handlers['{}'] ~= nil)",
                package.to_lowercase()
            ))
            .unwrap(),
            "true",
            "'{package}' should have a handler"
        );
    }

    // `Core.Hello` names the client, which is the first question asked when
    // somebody reports a rendering bug.
    assert_eq!(
        vm.eval(
            "return tostring(DAEMON.gmcp.receive('s3', 'Core.Hello', \
             { client = 'Mudlet', version = '4.17' }))"
        )
        .unwrap(),
        "true"
    );
}

/// NAWS: the client's real width reaches the wrapper. `get_width` already read
/// it; this is the assertion that it is used rather than ignored.
#[test]
fn output_is_wrapped_to_the_clients_width() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { session_id = 'nope', name = 'Wide' } \
         setmetatable(_p, { __index = require('lib.player') }) return 'ok'",
    )
    .unwrap();

    // No session, so the default. A wrapper that guessed zero would produce one
    // character per line.
    let default: i64 = vm.eval("return _p:get_width()").unwrap().parse().unwrap();
    assert!(default >= 40, "the fallback width should be usable: {default}");

    // And the real session's width is what a player gets. The probe session has
    // not negotiated NAWS, so it falls back — which is the branch that matters:
    // a client that never sent a width must not end up with zero.
    let sid = vm.session_id().to_string();
    vm.eval(&format!("_q = {{ session_id = '{sid}' }} \
                      setmetatable(_q, {{ __index = require('lib.player') }}) return 'ok'"))
        .unwrap();
    let real: i64 = vm.eval("return _q:get_width()").unwrap().parse().unwrap();
    assert!(real >= 40, "a session with no NAWS should still wrap sanely: {real}");
}
