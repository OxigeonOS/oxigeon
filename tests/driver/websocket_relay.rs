//! End-to-end WebSocket: a real client against a real listener in front of a
//! real Lua VM.
//!
//! Modelled on `dap_attach.rs` — port 0, every wait on a timeout — for the same
//! reason: these bind real sockets, so one hang would block the whole suite.
//!
//! The shape is a plain `#[test]` driving an explicit runtime rather than
//! `#[tokio::test]`. `RealVm` is blocking and thread-based, and several of its
//! methods `blocking_recv`, which panics inside an async context. Keeping the
//! VM calls outside `block_on` and the socket work inside it means neither has
//! to know about the other. The runtime is multi-threaded, so the listener and
//! connection tasks keep being polled between `block_on` calls.

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures_util::{FutureExt, SinkExt, StreamExt};
use oxigeon::config::WebSocketServerConfig;
use oxigeon::core::network::websocket::{self, WsDeps};
use oxigeon::core::{SessionHandler, SessionId};
use oxigeon::testkit::RealVm;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

const STEP: Duration = Duration::from_secs(10);

// ─── harness ────────────────────────────────────────────────────────────────

struct Harness {
    vm: RealVm,
    rt: tokio::runtime::Runtime,
    addr: SocketAddr,
    handler: Arc<RwLock<SessionHandler>>,
    /// Held only so the generated certificate outlives the listener.
    _certs_dir: Option<tempfile::TempDir>,
}

impl Harness {
    /// The probe boot: `on_input` compiles what it is handed and sends the
    /// result back, and `vm.eval` reaches the efuns directly. Everything here
    /// that is about the *transport* uses this, because it can drive the
    /// relay's inputs and outputs without depending on a word of game prose.
    fn boot() -> Self {
        Self::boot_with(64 * 1024, 4096)
    }

    fn boot_with(max_frame_bytes: usize, input_buffer_bytes: usize) -> Self {
        Self::around(RealVm::boot_real_mudlib_with_probe(), max_frame_bytes, input_buffer_bytes)
    }

    /// A boot whose `on_input` is the mudlib's own, for the one test that goes
    /// through the real login state machine. The probe replaces `on_input`
    /// globally — it is what makes `eval` possible — so the two cannot be had
    /// at once.
    fn boot_playing() -> Self {
        Self::around(RealVm::boot_real_mudlib(0), 64 * 1024, 4096)
    }

    /// The same, over `wss://`. Returns the harness and the PEM certificate the
    /// test client has to trust, since it is generated fresh per run.
    fn boot_tls() -> (Self, String) {
        let (h, pem, _paths) = Self::boot_tls_paths();
        (h, pem)
    }

    /// As above, also returning the on-disk paths so a test can replace the
    /// certificate under a running listener.
    fn boot_tls_paths() -> (Self, String, (std::path::PathBuf, std::path::PathBuf)) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("a self-signed certificate");
        let cert_pem = cert.cert.pem();
        let key_pem = cert.key_pair.serialize_pem();

        // Written to a temp directory because the acceptor loads from paths —
        // which is the interface an operator uses, so it is the one worth
        // testing.
        let dir = tempfile::tempdir().expect("a temp dir");
        let cert_path = dir.path().join("server.crt");
        let key_path = dir.path().join("server.key");
        std::fs::write(&cert_path, &cert_pem).unwrap();
        std::fs::write(&key_path, &key_pem).unwrap();

        let h = Self::around_with(
            RealVm::boot_real_mudlib_with_probe(),
            64 * 1024,
            4096,
            Some((
                cert_path.to_string_lossy().into_owned(),
                key_path.to_string_lossy().into_owned(),
            )),
            Some(dir),
        );
        (h, cert_pem, (cert_path, key_path))
    }

    /// A listener that only accepts the given browser origins.
    fn boot_origins(allowed: &[&str]) -> Self {
        Self::around_full(
            RealVm::boot_real_mudlib_with_probe(),
            64 * 1024,
            4096,
            None,
            None,
            allowed.iter().map(|s| s.to_string()).collect(),
        )
    }

    fn around(vm: RealVm, max_frame_bytes: usize, input_buffer_bytes: usize) -> Self {
        Self::around_with(vm, max_frame_bytes, input_buffer_bytes, None, None)
    }

    fn around_with(
        vm: RealVm,
        max_frame_bytes: usize,
        input_buffer_bytes: usize,
        tls: Option<(String, String)>,
        certs_dir: Option<tempfile::TempDir>,
    ) -> Self {
        Self::around_full(vm, max_frame_bytes, input_buffer_bytes, tls, certs_dir, Vec::new())
    }

    fn around_full(
        vm: RealVm,
        max_frame_bytes: usize,
        input_buffer_bytes: usize,
        tls: Option<(String, String)>,
        certs_dir: Option<tempfile::TempDir>,
        allowed_origins: Vec<String>,
    ) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime");

        let handler = vm.session_handler();
        let deps = WsDeps {
            session_handler: handler.clone(),
            cmd_tx: vm.engine().cmd_tx.clone(),
            // The harness has no failed-login tally to forget.
            auth_worker: None,
            input_buffer_bytes,
        };
        let cfg = WebSocketServerConfig {
            enabled: true,
            bind: "127.0.0.1".to_string(),
            port: 0,
            max_frame_bytes,
            // Off: nothing here is testing keepalive, and a stray ping would
            // only add a frame every reader has to skip.
            ping_interval_secs: 0,
            missed_pongs: 3,
            cert_path: tls.as_ref().map(|(c, _)| c.clone()),
            key_path: tls.as_ref().map(|(_, k)| k.clone()),
            allowed_origins,
            // Fast enough that a test can watch a renewal land without a sleep
            // measured in minutes.
            cert_reload_seconds: 1,
        };

        let addr = rt
            .block_on(websocket::serve(&cfg, deps))
            .expect("the listener binds");

        Harness { vm, rt, addr, handler, _certs_dir: certs_dir }
    }

    fn connect(&self) -> WsClient {
        self.rt.block_on(WsClient::connect(self.addr))
    }

    fn connect_q(&self, query: &str) -> WsClient {
        self.rt.block_on(WsClient::connect_q(self.addr, query))
    }

    fn connect_origin(
        &self,
        origin: &str,
    ) -> Result<WsClient, tokio_tungstenite::tungstenite::Error> {
        self.rt.block_on(WsClient::connect_origin(self.addr, origin))
    }

    fn connect_tls(&self, cert_pem: &str) -> WsClient {
        self.rt.block_on(WsClient::connect_tls(self.addr, cert_pem))
    }

    /// The one session that arrived over the WebSocket.
    ///
    /// Found by `protocol`, which is the field the driver sets and nothing has
    /// ever read — the probe VM has a session of its own in the same handler.
    fn ws_session_id(&self) -> SessionId {
        let handler = self.handler.read().unwrap();
        let mut found: Vec<SessionId> = handler
            .all_ids()
            .into_iter()
            .filter(|id| handler.get(id).is_some_and(|s| s.protocol == "websocket"))
            .collect();
        assert_eq!(found.len(), 1, "expected exactly one websocket session");
        found.pop().unwrap()
    }

    fn session_count(&self) -> usize {
        self.handler.read().unwrap().count()
    }

    /// Read a capability off the live Session, as `get_session` would.
    fn caps<T>(&self, f: impl FnOnce(&oxigeon::core::ClientCapabilities) -> T) -> T {
        let id = self.ws_session_id();
        let handler = self.handler.read().unwrap();
        f(&handler.get(&id).expect("the session").capabilities)
    }
}

// ─── client ─────────────────────────────────────────────────────────────────

/// Either kind of client socket.
///
/// The tests drive `ws://` and `wss://` through the same helpers, which is the
/// point: a client should not be able to tell, and neither should a test.
enum ClientSock {
    Plain(WebSocketStream<MaybeTlsStream<TcpStream>>),
    Tls(Box<WebSocketStream<tokio_rustls::client::TlsStream<TcpStream>>>),
}

struct WsClient {
    ws: ClientSock,
}

impl ClientSock {
    async fn next(&mut self) -> Option<Result<Message, tokio_tungstenite::tungstenite::Error>> {
        match self {
            ClientSock::Plain(s) => s.next().await,
            ClientSock::Tls(s) => s.next().await,
        }
    }
    async fn send(&mut self, m: Message) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        match self {
            ClientSock::Plain(s) => s.send(m).await,
            ClientSock::Tls(s) => s.send(m).await,
        }
    }
}

impl WsClient {
    async fn connect(addr: SocketAddr) -> Self {
        Self::connect_q(addr, "").await
    }

    /// Connect with an `Origin` header, as a browser page would.
    async fn connect_origin(
        addr: SocketAddr,
        origin: &str,
    ) -> Result<Self, tokio_tungstenite::tungstenite::Error> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut req = format!("ws://{addr}/").into_client_request().unwrap();
        req.headers_mut()
            .insert("origin", origin.parse().expect("a header value"));
        let (ws, _) = tokio::time::timeout(STEP, connect_async(req))
            .await
            .expect("connect timed out")?;
        Ok(WsClient { ws: ClientSock::Plain(ws) })
    }

    async fn connect_q(addr: SocketAddr, query: &str) -> Self {
        let (ws, _) = tokio::time::timeout(STEP, connect_async(format!("ws://{addr}/{query}")))
            .await
            .expect("connect timed out")
            .expect("connect failed");
        WsClient { ws: ClientSock::Plain(ws) }
    }

    /// A `wss://` client that trusts exactly the certificate the server was
    /// given — not a disabled verifier. A test that turns verification off
    /// would still pass if the server presented the wrong chain, which is most
    /// of what there is to get wrong here.
    async fn connect_tls(addr: SocketAddr, cert_pem: &str) -> Self {
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut cert_pem.as_bytes()) {
            roots.add(cert.expect("a certificate")).expect("a usable root");
        }
        let config = tokio_rustls::rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            tokio_rustls::rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();

        let tcp = tokio::time::timeout(STEP, TcpStream::connect(addr))
            .await
            .expect("connect timed out")
            .expect("connect failed");
        let name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let tls = tokio::time::timeout(
            STEP,
            tokio_rustls::TlsConnector::from(std::sync::Arc::new(config)).connect(name, tcp),
        )
        .await
        .expect("TLS handshake timed out")
        .expect("TLS handshake failed");

        let (ws, _) = tokio::time::timeout(
            STEP,
            tokio_tungstenite::client_async(format!("ws://localhost:{}/", addr.port()), tls),
        )
        .await
        .expect("upgrade timed out")
        .expect("upgrade failed");
        WsClient { ws: ClientSock::Tls(Box::new(ws)) }
    }

    async fn send_json(&mut self, v: Value) {
        self.ws
            .send(Message::text(v.to_string()))
            .await
            .expect("send failed");
    }

    /// Send something that is not a JSON envelope at all.
    async fn send_raw(&mut self, m: Message) {
        self.ws.send(m).await.expect("send failed");
    }

    /// The next text frame whose spans mention `needle`, discarding what came
    /// before — the login banner is still arriving when these tests start.
    async fn wait_spans_containing(&mut self, needle: &str) -> Vec<Value> {
        for _ in 0..200 {
            let f = self.next_frame().await;
            if f["type"] != "text" {
                continue;
            }
            if let Some(spans) = f["spans"].as_array() {
                if spans
                    .iter()
                    .any(|sp| sp["text"].as_str().unwrap_or_default().contains(needle))
                {
                    assert!(f.get("text").is_none(), "spans mode must not also send text: {f}");
                    return spans.clone();
                }
            }
        }
        panic!("no spans frame containing {needle:?} within 200 frames");
    }

    /// Announce `ansi: spans` and wait until the server has acted on it.
    async fn hello_spans(&mut self) {
        self.send_json(json!({"type": "hello", "ansi": "spans"})).await;
        self.send_json(json!({"type": "ping"})).await;
        self.wait_for("pong").await;
    }

    /// The next JSON frame, skipping protocol-level ping/pong traffic.
    async fn next_frame(&mut self) -> Value {
        loop {
            let msg = tokio::time::timeout(STEP, self.ws.next())
                .await
                .expect("timed out waiting for a frame")
                .expect("the server closed the connection unexpectedly")
                .expect("socket error");
            match msg {
                Message::Text(t) => {
                    return serde_json::from_str(t.as_str())
                        .unwrap_or_else(|e| panic!("not JSON: {e}: {t}"))
                }
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => panic!("the server closed while a frame was expected"),
                other => panic!("unexpected message: {other:?}"),
            }
        }
    }

    /// The next frame of a given `type`, discarding others.
    async fn wait_for(&mut self, ty: &str) -> Value {
        for _ in 0..200 {
            let f = self.next_frame().await;
            if f["type"] == ty {
                return f;
            }
        }
        panic!("no {ty} frame within 200 frames");
    }

    async fn wait_gmcp(&mut self, package: &str) -> Value {
        for _ in 0..200 {
            let f = self.next_frame().await;
            if f["type"] == "gmcp" && f["package"] == package {
                return f;
            }
        }
        panic!("no {package} GMCP frame within 200 frames");
    }

    /// The next text frame containing `needle`, discarding everything before it.
    async fn wait_text_containing(&mut self, needle: &str) -> String {
        for _ in 0..200 {
            let f = self.next_frame().await;
            if f["type"] == "text" {
                let text = f["text"].as_str().unwrap_or_default().to_string();
                if text.contains(needle) {
                    return text;
                }
            }
        }
        panic!("no text frame containing {needle:?} within 200 frames");
    }

    /// Whether the connection is closed, from the client's side.
    async fn is_closed(&mut self) -> bool {
        loop {
            match tokio::time::timeout(STEP, self.ws.next()).await {
                Err(_) => return false,
                Ok(None) => return true,
                Ok(Some(Ok(Message::Close(_)))) => return true,
                Ok(Some(Err(_))) => return true,
                Ok(Some(Ok(_))) => continue,
            }
        }
    }
}

// ─── tests ──────────────────────────────────────────────────────────────────

#[test]
fn a_websocket_client_reaches_the_login_banner() {
    let h = Harness::boot();
    let mut c = h.connect();

    let text = h.rt.block_on(c.wait_text_containing("Welcome"));

    // The line endings the mudlib appends are the transport's problem, and this
    // transport's answer is to remove them. A `\r` reaching a browser is at
    // best invisible and at worst a glyph.
    assert!(!text.contains('\r'), "no carriage returns should survive: {text:?}");
    assert!(!text.ends_with('\n'), "the trailing terminator should be stripped: {text:?}");
}

#[test]
fn input_reaches_lua_and_the_reply_comes_back() {
    let h = Harness::boot();
    let mut c = h.connect();

    // The probe boot's `on_input` compiles whatever it is handed and sends the
    // result back, which makes the round trip assertable without going near the
    // login flow's wording. What is under test is that a client frame becomes an
    // `on_input` carrying the *right session id* — a reply that comes back to
    // this socket proves both halves.
    h.rt.block_on(async {
        c.send_json(json!({"type": "input", "text": "return 'reached'"}))
            .await;
        c.wait_text_containing("reached").await;
    });
}

#[test]
fn a_multi_line_input_frame_becomes_several_commands() {
    let h = Harness::boot();
    let mut c = h.connect();

    // A paste arrives as one frame. Delivering it as one `on_input` would hand
    // the mudlib a "command" containing newlines, which no dispatcher expects —
    // so the transport splits, exactly as the telnet loop does.
    h.rt.block_on(async {
        c.send_json(json!({"type": "input", "text": "return 'one'\r\nreturn 'two'"}))
            .await;
        c.wait_text_containing("one").await;
        c.wait_text_containing("two").await;
    });
}

#[test]
fn a_prompt_arrives_as_a_prompt_frame_not_text() {
    let mut h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    let sid = h.ws_session_id();
    h.vm
        .eval(&format!("send_prompt('{sid}', 'HP:40/40 > ') return 'sent'"))
        .unwrap();

    let f = h.rt.block_on(c.wait_for("prompt"));
    // Exactly, including the trailing space and with nothing appended.
    // `send_prompt` exists precisely to leave the cursor on the line, and this
    // is what pins the `SessionOutput::Raw` → prompt mapping if a second
    // producer of `Raw` is ever added.
    assert_eq!(f, json!({"type": "prompt", "text": "HP:40/40 > "}));
}

#[test]
fn echo_control_masks_and_keeps_its_place_in_the_stream() {
    let mut h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    let sid = h.ws_session_id();
    // The shape `login.lua` produces around a password: mask, prompt, unmask.
    h.vm
        .eval(&format!(
            "start_echo('{sid}') send('{sid}', 'PWPROMPT') stop_echo('{sid}') return 'sent'"
        ))
        .unwrap();

    h.rt.block_on(async {
        // Order is the assertion, not presence. Masking that arrives after the
        // prompt it protects has already let the player type in the clear, and
        // unmasking that arrives early does the same. Both are control messages
        // and neither may be reordered around the text between them.
        assert_eq!(
            c.wait_for("echo").await,
            json!({"type": "echo", "masked": true}),
            "start_echo means the server echoes, so the client must not — masked"
        );
        assert_eq!(c.wait_for("text").await["text"], "PWPROMPT");
        assert_eq!(
            c.wait_for("echo").await,
            json!({"type": "echo", "masked": false}),
            "stop_echo restores the player's own echo — unmasked"
        );
    });
}

#[test]
fn the_real_login_flow_masks_the_password_over_a_websocket() {
    // The one test here that goes through the mudlib's actual login state
    // machine, which needs a boot whose `on_input` is the real one rather than
    // the probe's. It earns its place — and its dependence on the shipped
    // wording — by being the only thing that proves the whole path works
    // together: greeting, input, masking, and the order of all three.
    let h = Harness::boot_playing();
    let mut c = h.connect();

    h.rt.block_on(async {
        c.wait_text_containing("Username").await;
        c.send_json(json!({"type": "input", "text": "new"})).await;
        c.wait_text_containing("username").await;
        c.send_json(json!({"type": "input", "text": "wsplayer"})).await;

        let mut saw_mask = false;
        for _ in 0..200 {
            let f = c.next_frame().await;
            if f["type"] == "echo" {
                assert_eq!(f["masked"], true, "the first echo frame must mask");
                saw_mask = true;
                continue;
            }
            if f["type"] == "text"
                && f["text"].as_str().unwrap_or_default().contains("assword")
            {
                assert!(saw_mask, "masking must arrive before the password prompt");
                break;
            }
        }
        assert!(saw_mask, "no masking instruction was sent before the password prompt");

        c.send_json(json!({"type": "input", "text": "a good long test password"}))
            .await;
        let f = c.wait_for("echo").await;
        assert_eq!(f["masked"], false, "echo must be restored after the password");
    });
}

#[test]
fn capabilities_are_sane_before_any_hello() {
    let h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    // The direct regression guard for the bug `publish_capabilities` documents.
    // With `gmcp_supported` false every `gmcp_d` sender returns at its first
    // guard, no GMCP reaches the client, and nothing else in the system
    // notices — the link keeps looking healthy.
    assert!(h.caps(|c| c.gmcp_supported), "GMCP must default on for a WebSocket client");
    assert_eq!(
        h.caps(|c| c.window_width),
        Some(80),
        "a client that never says hello still needs a width, or everything wraps to a guess"
    );
    assert_eq!(h.caps(|c| c.window_height), Some(24));
}

#[test]
fn a_hello_frame_lands_on_the_session_and_can_resize() {
    let h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    h.rt.block_on(async {
        c.send_json(json!({
            "type": "hello", "width": 132, "height": 50, "gmcp": true, "terminal": "web"
        }))
        .await;
        // Round-trip a frame so the hello is known to have been processed
        // before the assertions read the session.
        c.send_json(json!({"type": "ping"})).await;
        c.wait_for("pong").await;
    });

    assert_eq!(h.caps(|c| c.window_width), Some(132));
    assert_eq!(h.caps(|c| c.window_height), Some(50));
    assert_eq!(h.caps(|c| c.terminal_type.clone()), Some("web".to_string()));
    assert!(h.caps(|c| c.gmcp_supported));

    let handler = h.handler.read().unwrap();
    assert_eq!(
        handler.get(&h.ws_session_id()).unwrap().protocol,
        "websocket",
        "the protocol field is how anything downstream tells the transports apart"
    );
    drop(handler);

    // A second hello is a resize. This is the transport's NAWS, and NAWS
    // arrives again every time the window changes.
    h.rt.block_on(async {
        c.send_json(json!({"type": "hello", "width": 80})).await;
        c.send_json(json!({"type": "ping"})).await;
        c.wait_for("pong").await;
    });
    assert_eq!(h.caps(|c| c.window_width), Some(80));
    assert_eq!(
        h.caps(|c| c.window_height),
        Some(50),
        "a hello that omits a field must not clear it"
    );
}

#[test]
fn gmcp_flows_in_both_directions() {
    let mut h = Harness::boot();
    let mut c = h.connect();

    // The greeting the transport sends itself, before Lua is involved.
    let hello = h.rt.block_on(c.wait_gmcp("Core.Hello"));
    assert_eq!(hello["data"]["client"], "Oxigeon");

    let sid = h.ws_session_id();
    h.vm
        .eval(&format!(
            "send_gmcp('{sid}', 'Room.Info', {{ name = 'A clearing', num = 7 }}) return 'sent'"
        ))
        .unwrap();

    let f = h.rt.block_on(c.wait_gmcp("Room.Info"));
    assert!(
        f["data"].is_object(),
        "GMCP data must nest, not arrive as a JSON-encoded string: {f}"
    );
    assert_eq!(f["data"]["name"], "A clearing");
    assert_eq!(f["data"]["num"], 7);

    // Inbound. `Core.Ping` is handled by the mudlib's own gmcp_d, so a reply
    // proves the frame reached Lua rather than being dropped in the relay.
    h.rt.block_on(async {
        c.send_json(json!({"type": "gmcp", "package": "Core.Ping", "data": {}}))
            .await;
        c.wait_gmcp("Core.Ping").await;
    });
}

#[test]
fn a_disconnect_from_lua_closes_the_socket_cleanly() {
    let mut h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    let before = h.session_count();
    let sid = h.ws_session_id();
    h.vm.eval(&format!("disconnect('{sid}') return 'sent'")).unwrap();

    h.rt.block_on(async {
        // The goodbye comes before the close frame, so a client can tell an
        // intentional end from a dropped socket.
        let f = c.wait_for("bye").await;
        assert_eq!(f["type"], "bye");
        assert!(c.is_closed().await, "the socket should close after bye");
    });

    wait_until(|| h.session_count() < before, "the session to be deregistered");
}

#[test]
fn an_oversized_frame_is_refused_without_killing_the_server() {
    // A 4 KiB message cap, and an input limit under it, so both refusals are
    // reachable from one harness.
    let h = Harness::boot_with(4096, 512);
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    // Over the input limit but under the frame cap: an advisory error, and the
    // session lives.
    h.rt.block_on(async {
        c.send_json(json!({"type": "input", "text": "x".repeat(1000)})).await;
        let f = c.wait_for("error").await;
        assert!(
            f["message"].as_str().unwrap().contains("exceeds"),
            "the error should say what the limit was: {f}"
        );
        c.send_json(json!({"type": "ping"})).await;
        c.wait_for("pong").await;
    });

    // Over the frame cap: the protocol layer refuses it and that connection
    // ends. One abusive client is not an outage, so a second must still work.
    let mut big = h.connect();
    h.rt.block_on(async {
        big.wait_text_containing("Welcome").await;
        big.send_json(json!({"type": "input", "text": "x".repeat(64 * 1024)}))
            .await;
        assert!(big.is_closed().await, "an oversized frame should end that connection");
    });

    let mut fresh = h.connect();
    h.rt.block_on(fresh.wait_text_containing("Welcome"));
}

#[test]
fn a_malformed_frame_gets_an_error_and_the_session_survives() {
    let h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    h.rt.block_on(async {
        // An unknown type. A running server outlives several client versions,
        // so this must not be fatal.
        c.send_json(json!({"type": "nope"})).await;
        c.wait_for("error").await;

        // Not JSON at all.
        c.send_raw(Message::text("not json")).await;
        c.wait_for("error").await;

        // And the session is still usable — a full round trip through Lua.
        c.send_json(json!({"type": "input", "text": "return 'still here'"}))
            .await;
        c.wait_text_containing("still here").await;
    });
}

#[test]
fn a_client_that_vanishes_is_cleaned_up() {
    let h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    let before = h.session_count();
    // Dropped with no close handshake — the ordinary way a browser tab dies.
    drop(c);

    wait_until(
        || h.session_count() < before,
        "the session to be deregistered after the socket vanished",
    );
}

/// Poll a condition to a deadline. The relay's cleanup is on another thread, so
/// there is nothing to await on from here.
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


// ─── TLS ────────────────────────────────────────────────────────────────────

#[test]
fn a_wss_client_plays_exactly_as_a_ws_client_does() {
    let (mut h, cert) = Harness::boot_tls();
    let mut c = h.connect_tls(&cert);

    // Everything below this line is a copy of what the plaintext tests assert,
    // deliberately: the promise of the TLS listener is that nothing above the
    // socket can tell the difference.
    h.rt.block_on(c.wait_text_containing("Welcome"));

    assert!(h.caps(|c| c.gmcp_supported));
    let handler = h.handler.read().unwrap();
    assert_eq!(
        handler.get(&h.ws_session_id()).unwrap().protocol,
        "websocket",
        "TLS is a property of the socket, not a different protocol"
    );
    drop(handler);

    h.rt.block_on(async {
        c.send_json(json!({"type": "input", "text": "return 'over tls'"})).await;
        c.wait_text_containing("over tls").await;
    });

    let sid = h.ws_session_id();
    h.vm.eval(&format!("send_gmcp('{sid}', 'Room.Info', {{ name = 'X' }}) return 'sent'"))
        .unwrap();
    let f = h.rt.block_on(c.wait_gmcp("Room.Info"));
    assert_eq!(f["data"]["name"], "X");
}

#[test]
fn a_plaintext_client_cannot_talk_to_the_tls_listener() {
    let (h, _cert) = Harness::boot_tls();

    // The HTTP upgrade goes out as cleartext, the server tries to read it as a
    // ClientHello, and the connection dies. What matters is that it dies rather
    // than falling back — a listener that served plaintext when the handshake
    // failed would make the secure port a lie.
    let result = h.rt.block_on(async {
        tokio::time::timeout(STEP, connect_async(format!("ws://{}/", h.addr))).await
    });
    match result {
        Ok(Ok(_)) => panic!("a plaintext client must not complete a handshake on the TLS port"),
        Ok(Err(_)) | Err(_) => {}
    }
}

#[test]
fn a_certificate_that_does_not_load_is_a_startup_error() {
    // Not a warning and not a plaintext fallback: an operator who asked for TLS
    // and got a port anyway has no way to notice.
    let vm = RealVm::boot_real_mudlib_with_probe();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let deps = WsDeps {
        session_handler: vm.session_handler(),
        cmd_tx: vm.engine().cmd_tx.clone(),
        auth_worker: None,
        input_buffer_bytes: 4096,
    };
    let cfg = WebSocketServerConfig {
        enabled: true,
        bind: "127.0.0.1".to_string(),
        port: 0,
        cert_path: Some("does/not/exist.crt".into()),
        key_path: Some("does/not/exist.key".into()),
        ..Default::default()
    };
    let err = rt.block_on(websocket::serve(&cfg, deps)).unwrap_err();
    assert!(
        err.to_string().contains("cannot read certificate"),
        "the error should name what could not be read: {err}"
    );
}

#[test]
fn half_a_tls_config_is_refused() {
    // A cert with no key is a typo, and serving plaintext on a port called
    // `_tls` is the worst possible reading of it.
    let vm = RealVm::boot_real_mudlib_with_probe();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let deps = WsDeps {
        session_handler: vm.session_handler(),
        cmd_tx: vm.engine().cmd_tx.clone(),
        auth_worker: None,
        input_buffer_bytes: 4096,
    };
    let cfg = WebSocketServerConfig {
        enabled: true,
        bind: "127.0.0.1".to_string(),
        port: 0,
        cert_path: Some("server.crt".into()),
        key_path: None,
        ..Default::default()
    };
    let err = rt.block_on(websocket::serve(&cfg, deps)).unwrap_err();
    assert!(err.to_string().contains("key_path"), "got: {err}");
}

// ─── colour spans ───────────────────────────────────────────────────────────

#[test]
fn a_client_that_asks_for_spans_gets_structured_colour() {
    let mut h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));
    h.rt.block_on(c.hello_spans());

    // Through the mudlib's own colour layer, not a hand-written escape code:
    // what is under test is that what `lib/color.lua` emits is what the parser
    // reads.
    let sid = h.ws_session_id();
    h.vm
        .eval(&format!(
            "local C = require('lib.color') \
             send('{sid}', C.colorize('{{red}}DANGER{{/}} plain')) return 'sent'"
        ))
        .unwrap();

    let spans = h.rt.block_on(c.wait_spans_containing("DANGER"));
    assert_eq!(spans[0]["text"], "DANGER");
    assert_eq!(spans[0]["fg"], 1, "{{red}} is palette index 1");
    assert_eq!(spans[1]["text"], " plain");
    assert!(spans[1].get("fg").is_none(), "the reset must clear the colour");
}

#[test]
fn raw_is_still_the_default_and_none_strips() {
    let mut h = Harness::boot();
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));

    let sid = h.ws_session_id();
    let paint = format!(
        "local C = require('lib.color') \
         send('{sid}', C.colorize('{{red}}MARKER{{/}}')) return 'sent'"
    );

    // No hello at all: escape codes, as before spans existed.
    h.vm.eval(&paint).unwrap();
    let text = h.rt.block_on(c.wait_text_containing("MARKER"));
    assert!(
        text.contains('\u{1b}'),
        "raw is the default and keeps the escape codes: {text:?}"
    );

    h.rt.block_on(async {
        c.send_json(json!({"type": "hello", "ansi": "none"})).await;
        c.send_json(json!({"type": "ping"})).await;
        c.wait_for("pong").await;
    });
    h.vm.eval(&paint).unwrap();
    let text = h.rt.block_on(c.wait_text_containing("MARKER"));
    assert_eq!(text, "MARKER", "none strips to bare text");
}


#[test]
fn the_upgrade_url_can_declare_capabilities_before_the_banner() {
    // The race this closes: `on_connect` writes the banner immediately, so a
    // `hello` frame always arrives too late for the first several lines. Left
    // to `hello` alone the boundary between raw and spans moves with the
    // handshake latency, and a wss:// client disagrees with a ws:// one against
    // the same server.
    let h = Harness::boot();
    let mut c = h.connect_q("?ansi=spans&width=132&height=44&terminal=web");

    let f = h.rt.block_on(c.wait_for("text"));
    assert!(
        f.get("spans").is_some(),
        "the very first text frame must already be in spans mode: {f}"
    );
    assert!(f.get("text").is_none());

    assert_eq!(h.caps(|c| c.window_width), Some(132));
    assert_eq!(h.caps(|c| c.window_height), Some(44));
    assert_eq!(h.caps(|c| c.terminal_type.clone()), Some("web".to_string()));
}

#[test]
fn a_nonsense_query_string_is_ignored_rather_than_refused() {
    // A query string is part of a URL a human may have typed. Losing the
    // session over a typo in an optional hint is the worse outcome.
    let h = Harness::boot();
    let mut c = h.connect_q("?ansi=purple&width=banana&nonsense&other=1");
    h.rt.block_on(c.wait_text_containing("Welcome"));
    assert_eq!(h.caps(|c| c.window_width), Some(80), "a bad width falls back to the default");
}


// ─── Origin ─────────────────────────────────────────────────────────────────

#[test]
fn an_allowed_origin_connects_and_others_are_refused() {
    let h = Harness::boot_origins(&["https://play.example.com", "http://localhost:5173"]);

    // On the list: an ordinary session.
    let mut ok = match h.connect_origin("https://play.example.com") {
        Ok(c) => c,
        Err(e) => panic!("an allowed origin should connect: {e:?}"),
    };
    h.rt.block_on(ok.wait_text_containing("Welcome"));

    // Not on the list: refused at the upgrade, before a session exists.
    let before = h.session_count();
    match h.connect_origin("https://evil.example.com") {
        Ok(_) => panic!("a foreign origin must be refused"),
        Err(e) => assert!(
            matches!(e, tokio_tungstenite::tungstenite::Error::Http(_)),
            "the refusal should be an HTTP response, not a dropped socket: {e:?}"
        ),
    }
    assert_eq!(
        h.session_count(),
        before,
        "a refused upgrade must not leave a session behind"
    );
}

#[test]
fn origins_are_matched_exactly() {
    let h = Harness::boot_origins(&["https://play.example.com"]);
    // A prefix, a suffix and a different scheme are all different origins, and
    // wildcard matching is exactly where this kind of check springs a leak.
    for bad in [
        "https://play.example.com.evil.net",
        "http://play.example.com",
        "https://play.example.com:8443",
        "https://sub.play.example.com",
    ] {
        assert!(h.connect_origin(bad).map(|_| ()).is_err(), "{bad} should not match");
    }
}

#[test]
fn a_client_that_sends_no_origin_is_allowed() {
    // Browsers always send `Origin`; anything else does not, and could forge it
    // if it wanted to. Refusing the absent case would break every non-browser
    // client — including this suite — while stopping nothing.
    let h = Harness::boot_origins(&["https://play.example.com"]);
    let mut c = h.connect();
    h.rt.block_on(c.wait_text_containing("Welcome"));
}

#[test]
fn no_configured_origins_accepts_anyone() {
    // The default. A server that has not thought about this is no worse off
    // than it was before the check existed.
    let h = Harness::boot();
    let mut c = match h.connect_origin("https://anywhere.example.com") {
        Ok(c) => c,
        Err(e) => panic!("any origin should be allowed by default: {e:?}"),
    };
    h.rt.block_on(c.wait_text_containing("Welcome"));
}


// ─── certificate reload ─────────────────────────────────────────────────────

/// Generate a fresh self-signed pair and write it over the given paths.
fn write_cert(cert_path: &std::path::Path, key_path: &std::path::Path) -> String {
    let c = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let pem = c.cert.pem();
    std::fs::write(cert_path, &pem).unwrap();
    std::fs::write(key_path, c.key_pair.serialize_pem()).unwrap();
    pem
}

/// Retry a TLS connection until it either succeeds or the deadline passes.
///
/// The reload is a poll on another task, so there is no event to await from
/// here — and the point of the test is that it happens without one.
fn connects_with(h: &Harness, pem: &str) -> bool {
    let deadline = std::time::Instant::now() + STEP;
    loop {
        let ok = h.rt.block_on(async {
            std::panic::AssertUnwindSafe(WsClient::connect_tls(h.addr, pem))
                .catch_unwind()
                .await
                .is_ok()
        });
        if ok || std::time::Instant::now() > deadline {
            return ok;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn a_renewed_certificate_is_picked_up_without_a_restart() {
    let (h, first, (cert_path, key_path)) = Harness::boot_tls_paths();

    // The listener is serving the certificate it started with.
    let mut c = h.connect_tls(&first);
    h.rt.block_on(c.wait_text_containing("Welcome"));

    // Renewal: the files change under a running server.
    let second = write_cert(&cert_path, &key_path);
    assert_ne!(first, second, "the two certificates must actually differ");

    // A client that trusts only the new one now connects — which it could not
    // have done a moment ago, and which is the whole feature.
    assert!(
        connects_with(&h, &second),
        "the renewed certificate should be served without a restart"
    );

    // And the old one is no longer accepted, so this is a real swap rather
    // than the listener happening to accept anything.
    assert!(
        !connects_with(&h, &first),
        "the superseded certificate should no longer be served"
    );
}

#[test]
fn a_broken_certificate_leaves_the_previous_one_serving() {
    let (h, first, (cert_path, key_path)) = Harness::boot_tls_paths();
    let mut c = h.connect_tls(&first);
    h.rt.block_on(c.wait_text_containing("Welcome"));

    // Renewal is not atomic: there is a moment where the certificate on disk is
    // the new one and the key is still the old. A poll landing there must not
    // take the listener down.
    std::fs::write(&cert_path, "-----BEGIN CERTIFICATE-----\nnot a certificate\n").unwrap();
    std::thread::sleep(Duration::from_millis(2500));

    assert!(
        connects_with(&h, &first),
        "a certificate that will not load must leave the previous one serving"
    );

    // And once the pair is whole again, it is picked up.
    let second = write_cert(&cert_path, &key_path);
    assert!(
        connects_with(&h, &second),
        "the reload should recover on the next tick once the files are consistent"
    );
}
