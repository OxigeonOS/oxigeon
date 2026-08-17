//! MXP — telnet option 91, over a real socket.
//!
//! Two claims, and they pull in opposite directions, which is why they are
//! tested together:
//!
//! 1. **Nothing changes for anyone who did not ask.** A client that refuses
//!    MXP, or never answers, gets the bytes it got before this existed. Even a
//!    client that accepts gets its ordinary game text verbatim, because the
//!    driver locks the stream to LOCKED mode rather than escaping every line.
//! 2. **A player cannot get a secure tag onto another player's screen.** That
//!    is the whole security argument of MXP and it rests on one strip, which
//!    [`a_line_mode_sequence_in_game_text_is_stripped`] is here to hold in
//!    place.
//!
//! Plaintext rather than TLS: `telnet_tls.rs` already establishes that the two
//! are the same protocol, and none of this is about the socket. The helpers are
//! copied from that file rather than shared, for the reason its own header
//! gives — a test that borrows the server's idea of the bytes cannot notice it
//! changing them.
//!
//! Port 0 and every wait on a timeout.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use oxigeon::config::TelnetServerConfig;
use oxigeon::core::network::telnet::{self, TelnetDeps};
use oxigeon::testkit::RealVm;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const STEP: Duration = Duration::from_secs(10);

/// Spelled out rather than imported, per the convention in `telnet_tls.rs`.
const IAC: u8 = 255;
const SE: u8 = 240;
const SB: u8 = 250;
const WILL: u8 = 251;
const DO: u8 = 253;
const DONT: u8 = 254;
const OPT_MXP: u8 = 91;

/// `ESC[7z` — lock the default line mode to LOCKED.
const LOCK_LOCKED: &str = "\x1b[7z";
/// `ESC[1z` — this line is secure.
const SECURE: &str = "\x1b[1z";

struct Harness {
    vm: RealVm,
    rt: tokio::runtime::Runtime,
    addr: SocketAddr,
    handler: Arc<std::sync::RwLock<oxigeon::core::SessionHandler>>,
}

fn boot() -> Harness {
    boot_with(true)
}

fn boot_with(mxp: bool) -> Harness {
    let vm = RealVm::boot_real_mudlib_with_probe();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let handler = vm.session_handler();
    let deps = TelnetDeps {
        session_handler: handler.clone(),
        cmd_tx: vm.engine().cmd_tx.clone(),
        auth_worker: None,
        input_buffer_bytes: 4096,
        mxp,
    };
    let cfg = TelnetServerConfig {
        enabled: true,
        bind: "127.0.0.1".to_string(),
        port: 0,
        cert_path: None,
        key_path: None,
        cert_reload_seconds: 0,
        mxp,
    };
    let addr = rt
        .block_on(telnet::serve(&cfg, "telnet", deps))
        .expect("the telnet listener binds");

    Harness { vm, rt, addr, handler }
}

async fn connect(addr: SocketAddr) -> TcpStream {
    tokio::time::timeout(STEP, TcpStream::connect(addr))
        .await
        .expect("connect timed out")
        .expect("connect failed")
}

/// Read until `needle` appears, or time out. Returns everything read.
async fn read_until(s: &mut TcpStream, needle: &str) -> Vec<u8> {
    let mut all = Vec::new();
    let deadline = tokio::time::Instant::now() + STEP;
    loop {
        let mut buf = [0u8; 4096];
        let n = tokio::time::timeout_at(deadline, s.read(&mut buf))
            .await
            .unwrap_or_else(|_| {
                panic!("timed out waiting for {needle:?}; got {:?}", String::from_utf8_lossy(&all))
            })
            .expect("read failed");
        assert!(n > 0, "server closed while waiting for {needle:?}");
        all.extend_from_slice(&buf[..n]);
        if String::from_utf8_lossy(&all).contains(needle) {
            return all;
        }
    }
}

/// The one telnet session in the handler.
fn telnet_session(h: &Harness) -> oxigeon::core::SessionId {
    let handler = h.handler.read().unwrap();
    let found: Vec<_> = handler
        .all_ids()
        .into_iter()
        .filter(|id| handler.get(id).is_some_and(|s| s.protocol == "telnet"))
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one telnet session");
    found[0]
}

fn wait_until(mut cond: impl FnMut() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + STEP;
    while std::time::Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

/// Connect, wait for the banner, accept MXP, and wait for the handshake.
fn accept_mxp(h: &Harness) -> TcpStream {
    let mut s = h.rt.block_on(connect(h.addr));
    h.rt.block_on(read_until(&mut s, "Username"));
    h.rt.block_on(async { s.write_all(&[IAC, DO, OPT_MXP]).await.unwrap() });
    h.rt.block_on(read_until(&mut s, "<SUPPORT>"));
    s
}

// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_server_offers_mxp_and_locks_the_stream_when_the_client_accepts() {
    let h = boot();
    let mut s = h.rt.block_on(connect(h.addr));
    let opening = h.rt.block_on(read_until(&mut s, "Username"));
    assert!(
        opening.windows(3).any(|w| w == [IAC, WILL, OPT_MXP]),
        "the opening burst should offer MXP"
    );

    h.rt.block_on(async { s.write_all(&[IAC, DO, OPT_MXP]).await.unwrap() });
    let started = h.rt.block_on(read_until(&mut s, "<SUPPORT>"));

    assert!(
        started.windows(5).any(|w| w == [IAC, SB, OPT_MXP, IAC, SE]),
        "the empty subnegotiation is what starts some clients' parsers"
    );

    // Order matters and is the safety property: the lock has to be in place
    // before anything else goes out, or there is a window in which the default
    // mode is still OPEN and a line of game text could be parsed as markup.
    let text = String::from_utf8_lossy(&started);
    let lock = text.find(LOCK_LOCKED).expect("ESC[7z should be sent on acceptance");
    let version = text.find("<VERSION>").expect("the version query should be sent");
    let support = text.find("<SUPPORT>").unwrap();
    assert!(lock < version && version < support, "expected lock, then VERSION, then SUPPORT");
    assert!(text[lock..].starts_with(&format!("{LOCK_LOCKED}{SECURE}<VERSION>")));

    wait_until(
        || {
            let handler = h.handler.read().unwrap();
            handler
                .get(&telnet_session_id(&handler))
                .is_some_and(|s| s.capabilities.mxp_supported)
        },
        "mxp_supported to reach the session",
    );
}

/// Helper for the one assertion above that already holds the read lock.
fn telnet_session_id(
    handler: &oxigeon::core::SessionHandler,
) -> oxigeon::core::SessionId {
    handler
        .all_ids()
        .into_iter()
        .find(|id| handler.get(id).is_some_and(|s| s.protocol == "telnet"))
        .expect("a telnet session")
}

/// The compatibility claim, written down.
///
/// A client with MXP negotiated must still receive ordinary game text exactly
/// as it did before MXP existed. If someone later decides the driver should
/// escape `< > &` on the way out, this fails loudly rather than the change
/// being noticed six months on by a player asking why signs read `&lt;`.
#[test]
fn game_text_is_byte_identical_once_mxp_is_on() {
    let mut h = boot();
    let mut s = accept_mxp(&h);
    let sid = telnet_session(&h);

    h.vm.eval(&format!(
        r#"send('{sid}', 'MARK You see a <sign> here. Tom & Sons say "hello". END')"#
    ))
    .unwrap();

    let got = h.rt.block_on(read_until(&mut s, "END"));
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains(r#"MARK You see a <sign> here. Tom & Sons say "hello". END"#),
        "game text was transformed: {text:?}"
    );
}

/// The injection this feature exists to prevent.
///
/// A player types the mode sequence into `say`; the mudlib round-trips it into
/// everybody else's output. The `<send>` behind it is inert while the default
/// mode is LOCKED — but a line-mode tag is honoured in *every* mode, including
/// locked, because that is how a client gets back out of locked mode. So the
/// tag is the lever, and stripping it is what closes the hole.
#[test]
fn a_line_mode_sequence_in_game_text_is_stripped() {
    let mut h = boot();
    let mut s = accept_mxp(&h);
    let sid = telnet_session(&h);

    h.vm.eval(&format!(
        "send('{sid}', 'MARK ' .. string.char(27) .. \
         '[1z<send href=\"quit\">free gold here</send> END')"
    ))
    .unwrap();

    let got = h.rt.block_on(read_until(&mut s, "END"));
    let line = String::from_utf8_lossy(&got);
    let after = &line[line.find("MARK").unwrap()..];
    assert!(
        !after.contains('\x1b'),
        "a mode sequence reached the client: {after:?}"
    );
    // The words are still there. Gagging them would be a game's decision.
    assert!(after.contains("free gold here"));
}

/// The assertion that matters most, and the one this codebase has been burned
/// by the absence of: a protocol that negotiates, looks healthy, and never
/// reaches Lua.
///
/// The client answers `<VERSION>` on the ordinary input stream. Without the
/// interception in `relay.rs` it arrives as a command, and every MXP client's
/// login ends with the mudlib complaining about something the *driver* asked
/// the client to send.
#[test]
fn the_version_reply_never_reaches_the_command_dispatcher() {
    let h = boot();
    let mut s = accept_mxp(&h);

    h.rt.block_on(async {
        s.write_all(b"\x1b[1z<VERSION MXP=0.4 CLIENT=mushclient VERSION=5.06>\r\n")
            .await
            .unwrap();
        s.write_all(b"\x1b[1z<SUPPORTS +b +send.href -image>\r\n").await.unwrap();
        // A real line behind them, so there is something to read either way.
        s.write_all(b"return 'MARKER'\r\n").await.unwrap();
    });

    let got = h.rt.block_on(read_until(&mut s, "MARKER"));
    let text = String::from_utf8_lossy(&got);
    // The probe dispatcher answers `COMPILE<tab>...` for anything that is not
    // valid Lua. Two replies reaching it would produce two of those.
    assert!(
        !text.contains("COMPILE"),
        "a handshake reply was dispatched as a command: {text:?}"
    );

    wait_until(
        || {
            let handler = h.handler.read().unwrap();
            handler.get(&telnet_session_id(&handler)).is_some_and(|s| {
                s.capabilities.mxp_client.as_deref() == Some("mushclient 5.06")
                    && s.capabilities.mxp_version.as_deref() == Some("0.4")
                    && s.capabilities.mxp_supports.iter().any(|t| t == "-image")
            })
        },
        "the handshake reply to reach Session.capabilities",
    );
}

#[test]
fn mxp_stops_when_the_client_says_dont_mid_session() {
    let mut h = boot();
    let mut s = accept_mxp(&h);

    h.rt.block_on(async {
        s.write_all(b"\x1b[1z<VERSION MXP=0.4 CLIENT=zmud VERSION=6.07>\r\n").await.unwrap();
    });
    wait_until(
        || {
            let handler = h.handler.read().unwrap();
            handler
                .get(&telnet_session_id(&handler))
                .is_some_and(|s| s.capabilities.mxp_client.is_some())
        },
        "the version reply to land",
    );

    h.rt.block_on(async { s.write_all(&[IAC, DONT, OPT_MXP]).await.unwrap() });

    wait_until(
        || {
            let handler = h.handler.read().unwrap();
            handler.get(&telnet_session_id(&handler)).is_some_and(|s| {
                !s.capabilities.mxp_supported
                    && s.capabilities.mxp_client.is_none()
                    && s.capabilities.mxp_supports.is_empty()
            })
        },
        "the capability and everything derived from it to be cleared",
    );

    // And a rich line degrades to plain text rather than emitting markup at a
    // client that stopped parsing it.
    let sid = telnet_session(&h);
    h.vm.eval(&format!(
        "send_rich('{sid}', {{'MARK ', {{send='look', text='here'}}, ' END'}})"
    ))
    .unwrap();
    let got = h.rt.block_on(read_until(&mut s, "END"));
    let text = String::from_utf8_lossy(&got);
    assert!(text.contains("MARK here END"), "{text:?}");
    assert!(!text.contains("SEND"), "markup went to a client that refused MXP: {text:?}");
}

/// `send_rich` end to end: the efun marshals, the renderer escapes, the
/// transport chooses MXP because this client has it.
///
/// The Lua lives here rather than in `tests/fixture/`, so there is no fixture
/// edit that could make this pass.
#[test]
fn send_rich_emits_a_secure_line_with_an_escaped_hostile_name() {
    let mut h = boot();
    let mut s = accept_mxp(&h);
    let sid = telnet_session(&h);

    // The name is what a player would pick if they had read the MXP spec. One
    // line: the probe reaches the VM through the ordinary input path, which is
    // line-oriented.
    h.vm.eval(&format!(
        r#"send_rich('{sid}', {{'MARK The baker offers ', {{ send = 'buy bread', hint = 'A fresh loaf', text = '"><send href="quit">gold</send>' }}, ' END'}})"#
    ))
    .unwrap();

    let got = h.rt.block_on(read_until(&mut s, "END"));
    let text = String::from_utf8_lossy(&got);
    let line = &text[text.find("\x1b[1zMARK").expect("a secure line tag before the content")..];

    assert!(line.contains(r#"<SEND href="buy bread" hint="A fresh loaf">"#), "{line:?}");
    // Exactly one SEND element: the driver's own. The name contributed none.
    assert_eq!(line.matches("<SEND").count(), 1, "{line:?}");
    assert!(line.contains("&lt;send href=&quot;quit&quot;&gt;"), "{line:?}");
    assert!(!line.contains(r#"href="quit""#), "the hostile href survived: {line:?}");
}

#[test]
fn a_client_that_refuses_mxp_is_never_offered_markup() {
    let mut h = boot();
    let mut s = h.rt.block_on(connect(h.addr));
    h.rt.block_on(read_until(&mut s, "Username"));
    h.rt.block_on(async { s.write_all(&[IAC, DONT, OPT_MXP]).await.unwrap() });

    let sid = telnet_session(&h);
    h.vm.eval(&format!(
        "send_rich('{sid}', {{'MARK ', {{send='look', text='here'}}, ' END'}})"
    ))
    .unwrap();

    let got = h.rt.block_on(read_until(&mut s, "END"));
    let text = String::from_utf8_lossy(&got);
    assert!(text.contains("MARK here END"), "{text:?}");
    assert!(!text.contains('\x1b'), "no escape sequence should reach a non-MXP client: {text:?}");
}

/// The config switch is the operator's kill switch, so it has to actually
/// suppress the offer rather than only the acceptance.
#[test]
fn a_listener_with_mxp_off_never_offers_it() {
    let h = boot_with(false);
    let mut s = h.rt.block_on(connect(h.addr));
    let opening = h.rt.block_on(read_until(&mut s, "Username"));
    assert!(
        !opening.windows(3).any(|w| w == [IAC, WILL, OPT_MXP]),
        "MXP was offered on a listener that has it disabled"
    );
}

/// A `<VAR>` update carries no terminator, so it does not put a blank line in
/// the scrollback of a client that renders the value.
#[test]
fn mxp_var_sets_a_client_variable_without_a_line_of_its_own() {
    let mut h = boot();
    let mut s = accept_mxp(&h);
    let sid = telnet_session(&h);

    h.vm.eval(&format!("mxp_var('{sid}', 'hp', 40) send('{sid}', 'END')")).unwrap();

    let got = h.rt.block_on(read_until(&mut s, "END"));
    let text = String::from_utf8_lossy(&got);
    assert!(text.contains("<VAR hp>40</VAR>"), "{text:?}");
    assert!(
        text.contains("<VAR hp>40</VAR>END") || text.contains("<VAR hp>40</VAR>\r\nEND"),
        "a var update should not add a line of its own: {text:?}"
    );
}

/// Author errors raise, and the message names the field — the same convention
/// `lua_to_json` and the `db_*` efuns follow.
#[test]
fn a_command_containing_a_menu_separator_is_refused_by_name() {
    let mut h = boot();
    let _s = accept_mxp(&h);
    let sid = telnet_session(&h);

    let err = h
        .vm
        .eval(&format!("send_rich('{sid}', {{{{ send = 'look|quit', text = 'x' }}}})"))
        .err();
    assert!(err.contains("send"), "the error should name the field: {err:?}");
    assert!(err.contains('|'), "the error should say what is wrong: {err:?}");

    let err = h
        .vm
        .eval(&format!(
            "send_rich('{sid}', {{{{ href = 'javascript:alert(1)', text = 'x' }}}})"
        ))
        .err();
    assert!(err.contains("href"), "{err:?}");
}
