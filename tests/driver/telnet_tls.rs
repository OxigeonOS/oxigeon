//! `telnets://` — the telnet protocol inside TLS.
//!
//! The claim under test is narrow and worth pinning: TLS is a property of the
//! socket, so a `telnets` session must be indistinguishable from a `telnet` one
//! everywhere above it. Same IAC negotiation, same `protocol` string, same
//! session registry. If this file ever needs a special case, the layering has
//! gone wrong.
//!
//! Port 0 and every wait on a timeout, like `dap_attach.rs`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use oxigeon::config::TelnetServerConfig;
use oxigeon::core::network::telnet::{self, TelnetDeps};
use oxigeon::testkit::RealVm;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

const STEP: Duration = Duration::from_secs(10);

/// IAC constants, spelled out rather than imported: a test that borrows the
/// server's own idea of the bytes cannot notice it changing them.
const IAC: u8 = 255;
const DO: u8 = 253;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const OPT_GMCP: u8 = 201;
const OPT_NAWS: u8 = 31;

struct Harness {
    vm: RealVm,
    rt: tokio::runtime::Runtime,
    addr: SocketAddr,
    cert_pem: String,
    handler: Arc<std::sync::RwLock<oxigeon::core::SessionHandler>>,
    _certs: tempfile::TempDir,
}

fn boot_tls() -> Harness {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_pem = cert.cert.pem();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s.crt"), &cert_pem).unwrap();
    std::fs::write(dir.path().join("s.key"), cert.key_pair.serialize_pem()).unwrap();

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
        // As production defaults it. This suite's job is to prove that
        // `telnets://` is the same protocol inside TLS, so it should carry the
        // same option burst a plaintext listener does.
        mxp: true,
    };
    let cfg = TelnetServerConfig {
        enabled: true,
        bind: "127.0.0.1".to_string(),
        port: 0,
        cert_path: Some(dir.path().join("s.crt").to_string_lossy().into_owned()),
        key_path: Some(dir.path().join("s.key").to_string_lossy().into_owned()),
        cert_reload_seconds: 1,
        mxp: true,
    };
    let addr = rt
        .block_on(telnet::serve(&cfg, "telnet_tls", deps))
        .expect("the telnets listener binds");

    Harness { vm, rt, addr, cert_pem, handler, _certs: dir }
}

/// A TLS client that trusts exactly the certificate the server was given — not
/// a disabled verifier, which would still pass against the wrong chain.
async fn connect(addr: SocketAddr, cert_pem: &str) -> TlsStream<TcpStream> {
    let mut roots = tokio_rustls::rustls::RootCertStore::empty();
    for c in rustls_pemfile::certs(&mut cert_pem.as_bytes()) {
        roots.add(c.unwrap()).unwrap();
    }
    let config = tokio_rustls::rustls::ClientConfig::builder_with_provider(Arc::new(
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
    tokio::time::timeout(
        STEP,
        tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp),
    )
    .await
    .expect("TLS handshake timed out")
    .expect("TLS handshake failed")
}

/// Read until `needle` appears, or time out. Returns everything read.
async fn read_until(s: &mut TlsStream<TcpStream>, needle: &str) -> Vec<u8> {
    let mut all = Vec::new();
    let deadline = tokio::time::Instant::now() + STEP;
    loop {
        let mut buf = [0u8; 4096];
        let n = tokio::time::timeout_at(deadline, s.read(&mut buf))
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for {needle:?}; got {:?}",
                    String::from_utf8_lossy(&all)
                )
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

#[test]
fn a_telnets_client_negotiates_and_reaches_the_banner() {
    let h = boot_tls();
    let mut s = h.rt.block_on(connect(h.addr, &h.cert_pem));

    // The server opens with its usual negotiation offers, inside TLS. If the
    // handshake and the IAC stream were fighting, this is where it would show.
    let bytes = h.rt.block_on(read_until(&mut s, "Username"));
    assert!(bytes.contains(&IAC), "the initial telnet negotiations should still be sent");
    assert!(
        String::from_utf8_lossy(&bytes).contains("Welcome"),
        "the mudlib's greeting should arrive unchanged"
    );

    // And the session looks like any other telnet session.
    let handler = h.handler.read().unwrap();
    let id = {
        let found: Vec<_> = handler
            .all_ids()
            .into_iter()
            .filter(|id| handler.get(id).is_some_and(|s| s.protocol == "telnet"))
            .collect();
        assert_eq!(
            found.len(),
            1,
            "a TLS-wrapped telnet session is still `telnet` — encryption is the socket's business"
        );
        found[0]
    };
    assert_eq!(handler.get(&id).unwrap().protocol, "telnet");
}

#[test]
fn input_and_output_survive_the_tunnel() {
    let mut h = boot_tls();
    let mut s = h.rt.block_on(connect(h.addr, &h.cert_pem));
    h.rt.block_on(read_until(&mut s, "Username"));

    // The probe boot compiles whatever it is sent, so a reply proves the line
    // arrived intact and was split on the newline as usual.
    h.rt.block_on(async {
        s.write_all(b"return 'through the tunnel'\r\n").await.unwrap();
    });
    h.rt.block_on(read_until(&mut s, "through the tunnel"));

    // And output initiated from Lua reaches the encrypted socket.
    let sid = telnet_session(&h);
    h.vm.eval(&format!("send('{sid}', 'PUSHED') return 'sent'")).unwrap();
    h.rt.block_on(read_until(&mut s, "PUSHED"));
}

#[test]
fn gmcp_and_naws_still_work_inside_tls() {
    let h = boot_tls();
    let mut s = h.rt.block_on(connect(h.addr, &h.cert_pem));
    h.rt.block_on(read_until(&mut s, "Username"));

    h.rt.block_on(async {
        // Accept GMCP, which makes the server push Core.Hello.
        s.write_all(&[IAC, DO, OPT_GMCP]).await.unwrap();
        // And report a window size.
        s.write_all(&[IAC, WILL, OPT_NAWS]).await.unwrap();
        s.write_all(&[IAC, SB, OPT_NAWS, 0, 120, 0, 40, IAC, SE]).await.unwrap();
    });

    h.rt.block_on(read_until(&mut s, "Core.Hello"));

    // NAWS reached `Session.capabilities` — the join that has silently failed
    // before, and would fail the same way on any new transport.
    wait_until(
        || {
            let handler = h.handler.read().unwrap();
            handler.all_ids().into_iter().any(|id| {
                handler
                    .get(&id)
                    .is_some_and(|s| s.capabilities.window_width == Some(120))
            })
        },
        "NAWS to reach the session",
    );
}

#[test]
fn a_plaintext_client_gets_no_banner_from_the_telnets_listener() {
    let h = boot_tls();
    // A plain telnet client waits to be spoken to, and the server's first move
    // is a ClientHello it will never send. What must not happen is the server
    // falling back and greeting it in the clear.
    let got = h.rt.block_on(async {
        let mut tcp = TcpStream::connect(h.addr).await.expect("tcp connects");
        let mut buf = [0u8; 512];
        match tokio::time::timeout(Duration::from_secs(2), tcp.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => None,
            Ok(Ok(n)) => Some(String::from_utf8_lossy(&buf[..n]).into_owned()),
        }
    });
    if let Some(text) = got {
        assert!(
            !text.contains("Welcome"),
            "the TLS port must never serve the banner in cleartext, got: {text:?}"
        );
    }
}
