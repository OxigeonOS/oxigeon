//! One WebSocket client, for its entire lifetime.
//!
//! The twin of `driver::handle_connection`, with the telnet machinery deleted
//! rather than reimplemented. A WebSocket is already message-framed, so there
//! is no IAC parser, no option negotiator, no escaping, and — this is the part
//! worth saying out loud — **no line accumulator**. The telnet path keeps an
//! uncapped `String` that a client sending no newline can grow without bound;
//! here a message arrives whole and `max_message_size` caps it at the protocol
//! layer besides. Anyone making this file "more like the telnet one" should
//! read that sentence first.
//!
//! ANSI escape codes are passed through untouched. Stripping them here would
//! duplicate a policy the mudlib already owns — `Player:_process_output` calls
//! `color.strip` when the player has colour off — and it is irreversible, while
//! a browser renders ANSI in about two kilobytes of JavaScript and can always
//! strip it itself. Structured colour spans are the better long-term answer and
//! the tagged envelope leaves room for them: a future `hello` field selecting
//! `"raw" | "spans" | "none"` needs no change to any existing client.

use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_tungstenite::{accept_hdr_async_with_config, WebSocketStream};

use super::protocol::{AnsiMode, ClientFrame, ServerFrame};
use super::{WsDeps, WsRuntime};
use crate::core::lock::RwLockExt;
use crate::core::network::MaybeTls;
use crate::core::session::{publish_capabilities, ClientCapabilities};
use crate::core::{LuaCommand, Session, SessionOutput};

/// How long a peer gets to complete the HTTP upgrade.
///
/// A TCP connection that opens and then says nothing would otherwise pin a task
/// forever. This is the *second* deadline a `wss://` connection passes: the TLS
/// handshake has its own, in `network::tls::wrap`.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn run(stream: MaybeTls, addr: SocketAddr, deps: WsDeps, cfg: WsRuntime) {
    // `WebSocketConfig` is `#[non_exhaustive]`, so it is built rather than
    // struct-literalled. Both caps are set: `max_message_size` bounds a
    // reassembled message and `max_frame_size` bounds one frame of it.
    let ws_cfg = WebSocketConfig::default()
        .max_message_size(Some(cfg.max_frame_bytes))
        .max_frame_size(Some(cfg.max_frame_bytes));

    // Capabilities can be declared in the upgrade URL's query string as well as
    // in a `hello` frame, and they have to be, for one reason: the mudlib's
    // `on_connect` writes the login banner immediately, and a `hello` sent by
    // the client cannot arrive before it. Left to `hello` alone the first
    // several frames render in whatever mode was the default and the rest in
    // the chosen one — and *which* frames land on which side of that boundary
    // depends on the handshake latency, so a TLS connection and a plaintext one
    // to the same server disagree. Declaring it in the URL settles the question
    // before `OnConnect` is ever sent.
    //
    //     ws://host:4001/?ansi=spans&width=120
    let mut query = String::new();
    let hdr_cb = |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                  res: tokio_tungstenite::tungstenite::handshake::server::Response| {
        if let Some(q) = req.uri().query() {
            query = q.to_string();
        }
        check_origin(req, &cfg.allowed_origins)?;
        Ok(res)
    };

    let mut ws = match timeout(
        HANDSHAKE_TIMEOUT,
        accept_hdr_async_with_config(stream, hdr_cb, Some(ws_cfg)),
    )
    .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            tracing::debug!("WebSocket handshake from {} failed: {}", addr, e);
            return;
        }
        Err(_) => {
            tracing::debug!("WebSocket handshake from {} timed out", addr);
            return;
        }
    };

    // ── Session setup ─────────────────────────────────────────
    // The same 64 slots as telnet, deliberately: `Session::try_send`'s
    // backpressure reasoning — drop, count, log, tell the player — is written
    // against that number, and a transport with a different one would make it
    // quietly false.
    let (output_tx, mut output_rx) = mpsc::channel::<SessionOutput>(64);

    let session = Session::new("websocket".to_string(), addr, output_tx);
    let session_id = session.id;
    let session_id_str = session_id.to_string();

    // Registered only now, after a successful handshake, so a port scanner
    // cannot burn a `max_connections` slot by opening a socket and leaving.
    let connect_result = {
        let mut handler = deps.session_handler.write_recover();
        handler.connect(session)
    };
    if let Err(e) = connect_result {
        tracing::warn!("Cannot register WebSocket session from {}: {}", addr, e);
        let _ = send_frame(
            &mut ws,
            &session_id_str,
            ServerFrame::plain("Server is full. Try again later."),
        )
        .await;
        let _ = ws.close(None).await;
        return;
    }

    tracing::info!("WebSocket accepted: {} ({})", session_id_str, addr);

    // Capabilities before `on_connect`, not after: a game's connect hook may
    // read `get_session(sid).window_width`, and there must be no window in
    // which that is nil. See `publish_capabilities` for what happened the last
    // time a transport left them at their defaults.
    let mut caps = ClientCapabilities::for_websocket();
    publish_capabilities(&deps.session_handler, session_id, &caps);

    // How this client wants colour. `raw` unless the URL or a later `hello`
    // says otherwise, so a client written before spans existed sees no change.
    let mut ansi = AnsiMode::default();
    apply_query(&query, &mut caps, &mut ansi);
    if !query.is_empty() {
        publish_capabilities(&deps.session_handler, session_id, &caps);
    }

    // The counterpart of the telnet `Core.Hello`, which the driver sends
    // directly and which never touches Lua.
    let _ = send_frame(
        &mut ws,
        &session_id_str,
        ServerFrame::Gmcp {
            package: "Core.Hello".into(),
            data: serde_json::json!({
                "client": "Oxigeon",
                "version": env!("CARGO_PKG_VERSION"),
            }),
        },
    )
    .await;

    let _ = deps.cmd_tx.send(LuaCommand::OnConnect {
        session_id: session_id_str.clone(),
    });

    // ── Relay ─────────────────────────────────────────────────
    // One `&mut ws` across all three arms rather than `StreamExt::split()`:
    // `futures-util` is declared without default features and `split` needs
    // `alloc`. `next()` and `send()` both work without it, which is what the
    // DAP session already does inside a `select!`.
    let mut missed_pongs: u32 = 0;
    let mut ping = tokio::time::interval(Duration::from_secs(if cfg.ping_interval_secs == 0 {
        // The arm is disabled by its guard; the interval still needs a
        // non-zero period because `Duration::ZERO` panics `interval`.
        3600
    } else {
        cfg.ping_interval_secs
    }));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await; // the first tick is immediate; skip it

    loop {
        tokio::select! {
            // ── From the client ────────────────────
            incoming = ws.next() => {
                match incoming {
                    None | Some(Ok(Message::Close(_))) => break,
                    Some(Err(e)) => {
                        tracing::debug!("WebSocket error for {}: {}", session_id_str, e);
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        if !handle_client_text(
                            text.as_str(), &mut ws, &mut caps, &mut ansi, session_id,
                            &session_id_str, &deps, &cfg,
                        ).await {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        if send_frame(&mut ws, &session_id_str, ServerFrame::Error {
                            message: "binary frames are not accepted; send JSON text frames".into(),
                        }).await.is_err() {
                            break;
                        }
                    }
                    // A pong is the only thing that clears the miss counter.
                    Some(Ok(Message::Pong(_))) => missed_pongs = 0,
                    // tungstenite queues the reply to a Ping itself.
                    Some(Ok(Message::Ping(_))) => {}
                    Some(Ok(Message::Frame(_))) => {}
                }
            }

            // ── From Lua ───────────────────────────
            msg = output_rx.recv() => {
                match msg {
                    // The Session was dropped from the handler; nothing further
                    // can arrive.
                    None => break,
                    Some(out) => {
                        let goodbye = matches!(out, SessionOutput::Disconnect);
                        if send_frame(&mut ws, &session_id_str, ServerFrame::from_output(out, ansi)).await.is_err() {
                            break;
                        }
                        if goodbye {
                            let _ = ws.close(None).await;
                            break;
                        }
                    }
                }
            }

            // ── Keepalive ──────────────────────────
            _ = ping.tick(), if cfg.ping_interval_secs > 0 => {
                if missed_pongs >= cfg.missed_pongs {
                    tracing::debug!(
                        "WebSocket {} stopped answering pings ({} missed)",
                        session_id_str, missed_pongs
                    );
                    break;
                }
                if ws.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
                missed_pongs += 1;
            }
        }
    }

    // ── Cleanup ───────────────────────────────────────────────
    // The order is the telnet path's, and it is not arbitrary.
    //
    // `disconnect` first, so an `on_disconnect` hook cannot find and write to a
    // session it is being told is gone. `OnDisconnect` second. `forget` third,
    // because the address's failed-login tally must be dropped exactly once per
    // connection whatever the mudlib does — and note it deliberately does not
    // clear an active lockout, so reconnecting is not a free reset. Closing the
    // socket last, so any write the mudlib triggers on its way out is a benign
    // drop rather than a hard error.
    {
        let mut handler = deps.session_handler.write_recover();
        handler.disconnect(&session_id);
    }

    let _ = deps.cmd_tx.send(LuaCommand::OnDisconnect {
        session_id: session_id_str.clone(),
    });

    if let Some(auth) = &deps.auth_worker {
        auth.forget(Some(addr.ip()));
    }

    let _ = ws.close(None).await;
    tracing::info!("WebSocket closed: {} ({})", session_id_str, addr);
}

/// Refuse a browser page that was not invited.
///
/// A WebSocket is not subject to the same-origin policy: any page a visitor
/// loads can open a socket to this server from their browser, with their IP and
/// their network position. That matters less here than for a cookie-backed API
/// — there is no ambient credential and login is in band, so nobody's account
/// is at risk — but it is how an unrelated site turns its visitors into
/// connections to your MUD, and `max_connections` is a shared resource.
///
/// **An absent `Origin` is allowed**, and that is not an oversight. Browsers
/// always send one; anything else — a bot, `wscat`, a native client, this
/// repository's own tests — sends none and could put any value there if it
/// wanted to. Rejecting the absent case would break every non-browser client
/// while stopping nothing, because the header is only trustworthy in exactly
/// the case where the browser controls it.
///
/// So this is a defence against *other people's pages*, not against attackers,
/// and the empty default reflects that: a MUD that has not thought about it is
/// no worse off than before.
fn check_origin(
    req: &tokio_tungstenite::tungstenite::handshake::server::Request,
    allowed: &[String],
) -> std::result::Result<(), tokio_tungstenite::tungstenite::handshake::server::ErrorResponse> {
    if allowed.is_empty() {
        return Ok(());
    }
    let Some(origin) = req.headers().get("origin") else {
        return Ok(());
    };
    let origin = origin.to_str().unwrap_or("");

    // Compared exactly. An `Origin` is scheme + host + port and nothing else,
    // so there is no path to normalise and no wildcard to get subtly wrong —
    // `*.example.com` matching is where this kind of check usually springs a
    // leak, and a list of exact origins is both shorter to write and harder to
    // be wrong about.
    if allowed.iter().any(|a| a == origin) {
        return Ok(());
    }

    tracing::warn!("WebSocket: refused an upgrade from origin {:?}", origin);

    // The status has to be set explicitly: `ErrorResponse::new` builds a 200,
    // and a 200 that is not a `101 Switching Protocols` is a malformed
    // handshake rather than a refusal — the client reports it as a truncated
    // connection and the operator has nothing to go on.
    let mut res = tokio_tungstenite::tungstenite::handshake::server::ErrorResponse::new(Some(
        "this origin is not permitted to connect".to_string(),
    ));
    *res.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
    Err(res)
}

/// Read capabilities out of the upgrade URL's query string.
///
/// The same four things a `hello` frame carries, spelled as `ansi=spans`,
/// `width=120`, `height=40`, `gmcp=false`. Anything unrecognised or unparseable
/// is ignored rather than refused: a query string is part of a URL a human may
/// have typed, and losing a session over a typo in an optional hint is a worse
/// outcome than defaulting.
fn apply_query(query: &str, caps: &mut ClientCapabilities, ansi: &mut AnsiMode) {
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else { continue };
        match k {
            "ansi" => match v {
                "spans" => *ansi = AnsiMode::Spans,
                "none" => *ansi = AnsiMode::None,
                "raw" => *ansi = AnsiMode::Raw,
                _ => {}
            },
            "width" => {
                if let Ok(w) = v.parse() {
                    caps.window_width = Some(w);
                }
            }
            "height" => {
                if let Ok(h) = v.parse() {
                    caps.window_height = Some(h);
                }
            }
            "gmcp" => caps.gmcp_supported = v != "false" && v != "0",
            "terminal" => caps.terminal_type = Some(v.to_string()),
            _ => {}
        }
    }
}

/// Decode and act on one text frame. Returns whether the connection survives.
async fn handle_client_text(
    raw: &str,
    ws: &mut WebSocketStream<MaybeTls>,
    caps: &mut ClientCapabilities,
    ansi: &mut AnsiMode,
    session_id: crate::core::SessionId,
    session_id_str: &str,
    deps: &WsDeps,
    cfg: &WsRuntime,
) -> bool {
    let frame: ClientFrame = match serde_json::from_str(raw) {
        Ok(f) => f,
        Err(e) => {
            // Advisory, and the session lives. A running server outlives
            // several versions of a client; closing on an unrecognised frame
            // would make every client deploy a hostile act.
            return send_frame(
                ws,
                session_id_str,
                ServerFrame::Error { message: format!("could not read that frame: {e}") },
            )
            .await
            .is_ok();
        }
    };

    match frame {
        ClientFrame::Input { text } => {
            if text.len() > cfg.input_buffer_bytes {
                return send_frame(
                    ws,
                    session_id_str,
                    ServerFrame::Error {
                        message: format!(
                            "input of {} bytes exceeds the {}-byte limit",
                            text.len(),
                            cfg.input_buffer_bytes
                        ),
                    },
                )
                .await
                .is_ok();
            }

            // One `OnInput` per line, mirroring the telnet path's split. A
            // paste that arrives as one frame is several commands, not one
            // command containing newlines that no mudlib parser expects. Empty
            // lines are kept: the login flow branches on `text == ""` and the
            // pager advances on a bare Enter.
            for line in text.split('\n') {
                let line = line.strip_suffix('\r').unwrap_or(line);
                let _ = deps.cmd_tx.send(LuaCommand::OnInput {
                    session_id: session_id_str.to_string(),
                    text: line.to_string(),
                });
            }
        }

        ClientFrame::Hello { width, height, gmcp, terminal, ansi: mode } => {
            *ansi = mode;
            // Only what was sent is updated, and it may be sent any number of
            // times — that is how a browser reports a resize, the same reason
            // the telnet path republishes after every negotiation rather than
            // once at the end.
            if width.is_some() {
                caps.window_width = width;
            }
            if height.is_some() {
                caps.window_height = height;
            }
            if terminal.is_some() {
                caps.terminal_type = terminal;
            }
            caps.gmcp_supported = gmcp;
            publish_capabilities(&deps.session_handler, session_id, caps);
        }

        ClientFrame::Gmcp { package, data } => {
            // A client that is sending GMCP has necessarily negotiated it.
            if !caps.gmcp_supported {
                caps.gmcp_supported = true;
                publish_capabilities(&deps.session_handler, session_id, caps);
            }
            let _ = deps.cmd_tx.send(LuaCommand::OnGmcp {
                session_id: session_id_str.to_string(),
                package,
                data,
            });
        }

        ClientFrame::Ping => {
            return send_frame(ws, session_id_str, ServerFrame::Pong).await.is_ok();
        }
    }

    true
}

/// Serialise and write one frame.
///
/// Always awaited, never a try-send: `StartEcho`/`StopEcho` are control
/// messages, and a dropped one leaves a password visible. `Session::try_send`
/// already has one bounded drop path with the counters and the warning to go
/// with it; this must not add a second, silent one.
async fn send_frame(
    ws: &mut WebSocketStream<MaybeTls>,
    session_id_str: &str,
    frame: ServerFrame,
) -> Result<(), ()> {
    let json = match serde_json::to_string(&frame) {
        Ok(json) => json,
        Err(e) => {
            // Cannot happen today — `lua_to_json` refuses NaN and infinity at
            // the efun boundary, so no unserialisable value reaches here.
            // Logged rather than unwrapped because a panic in a connection task
            // presents to the player as an unexplained disconnect.
            tracing::error!("WebSocket {}: undeliverable frame: {}", session_id_str, e);
            return Ok(());
        }
    };
    ws.send(Message::text(json)).await.map_err(|e| {
        tracing::debug!("WebSocket {}: write failed: {}", session_id_str, e);
    })
}
