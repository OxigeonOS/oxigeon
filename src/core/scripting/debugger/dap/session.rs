//! One attached debug client: the request/response state machine.

use std::sync::atomic::Ordering;
use std::time::Duration;
use crate::core::lock::MutexExt;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

use super::codec::DapCodec;
use crate::core::scripting::debugger::paths;
use crate::core::scripting::debugger::state::{
    BreakpointSpec, DebugEventMsg, ResumeKind, SharedDebugState, StopId, VmRequest, WORLD_STOP,
};

/// How long to wait for the Lua thread to answer a VM-touching request. It is
/// stopped inside the hook and should answer immediately; this only guards
/// against it having resumed in the meantime.
const VM_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// Which stop a request is about.
///
/// The protocol carries `threadId` on everything that inspects or resumes, and
/// with several dispatches suspended at once that is the only thing telling
/// them apart. A request without one — or naming a stop that has since
/// resumed — falls back to the most recent, which is what a client that has not
/// caught up would have meant.
fn thread_of(args: &Value, st: &SharedDebugState) -> StopId {
    let asked = args.get("threadId").and_then(Value::as_i64);
    let parked = st.parked_list();
    match asked {
        Some(id) if parked.iter().any(|(p, _, _)| *p == id) => id,
        // The freezing path reports one thread and parks nothing, so anything
        // it asks about is the world stop.
        _ if parked.is_empty() => WORLD_STOP,
        _ => parked.last().map(|(id, _, _)| *id).unwrap_or(WORLD_STOP),
    }
}

/// Whether anything is stopped: the whole VM, or any single dispatch.
///
/// `DebugState::stopped` alone is not the question any more — it means "the
/// world is frozen", and on a server debugging without freezing it is false
/// while several dispatches sit at breakpoints. Gating the inspect requests on
/// it would refuse every one of them.
fn anything_stopped(st: &SharedDebugState) -> bool {
    st.stopped.load(Ordering::Acquire) || st.parked_count.load(Ordering::Acquire) > 0
}

pub fn detach(st: &SharedDebugState) {
    st.clients.store(0, Ordering::Relaxed);
    st.pause_req.store(false, Ordering::Relaxed);
    st.clear_breakpoints();
    *st.evt_tx.lock_recover() = None;
    // If the VM is parked in the pause loop it must not stay there.
    let _ = st.send_vm(VmRequest::Detach);
}

pub async fn run(stream: TcpStream, st: SharedDebugState) -> std::io::Result<()> {
    let _ = stream.set_nodelay(true);
    let mut framed = Framed::new(stream, DapCodec::default());

    let (evt_tx, mut evt_rx) = tokio::sync::mpsc::unbounded_channel::<DebugEventMsg>();
    *st.evt_tx.lock_recover() = Some(evt_tx);

    let mut seq: i64 = 0;
    let mut next_seq = move || {
        seq += 1;
        seq
    };

    loop {
        tokio::select! {
            incoming = framed.next() => {
                let Some(msg) = incoming else { break };  // client closed
                let msg = msg?;
                let outgoing = handle(&st, &msg, &mut next_seq).await;
                for m in outgoing {
                    framed.send(m).await?;
                }
                if msg.get("command").and_then(Value::as_str) == Some("disconnect") {
                    break;
                }
            }
            Some(evt) = evt_rx.recv() => {
                let m = match evt {
                    DebugEventMsg::Stopped { stop, reason, world } => {
                        event(next_seq(), "stopped", json!({
                            "reason": reason.as_str(),
                            "threadId": stop,
                            // Was hard-coded true, which stopped being true the
                            // moment a stop could hold one dispatch and let the
                            // rest of the game run.
                            "allThreadsStopped": world,
                        }))
                    }
                    DebugEventMsg::Continued { stop, world } => {
                        event(next_seq(), "continued", json!({
                            "threadId": stop,
                            "allThreadsContinued": world,
                        }))
                    }
                    DebugEventMsg::Output { text, important } => {
                        event(next_seq(), "output", json!({
                            // The protocol's own distinction: `important` is for
                            // what the user should look at, `console` for
                            // ordinary reporting. A logpoint is the latter, and
                            // rendering it as a warning made a working one look
                            // broken.
                            "category": if important { "important" } else { "console" },
                            "output": text,
                        }))
                    }
                };
                framed.send(m).await?;
            }
        }
    }
    Ok(())
}

/// Read the optional gates off one `SourceBreakpoint`.
///
/// `hitCondition` is a free-form string in the protocol (`">5"`, `"%3"`, …).
/// Only a plain count is supported — anything else is ignored rather than
/// guessed at, since a misread gate would silently change where you stop.
///
/// `logMessage` turns the breakpoint into a *logpoint*: it reports and keeps
/// running. VS Code sends it for what it calls a Logpoint, so this needs no
/// bespoke client support on either side.
fn parse_bp_spec(b: &Value) -> BreakpointSpec {
    let non_empty = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    BreakpointSpec {
        condition: non_empty(b.get("condition")),
        hit_condition: non_empty(b.get("hitCondition"))
            .and_then(|s| s.trim_start_matches(['>', '=', ' ']).parse::<u32>().ok())
            .filter(|n| *n > 1),
        log_message: non_empty(b.get("logMessage")),
    }
}

fn variables_json(vars: &[crate::core::scripting::debugger::state::DapVariable]) -> Vec<Value> {
    vars.iter()
        .map(|v| json!({
            "name": v.name,
            "value": v.value,
            "type": v.ty,
            "variablesReference": v.var_ref,
        }))
        .collect()
}

fn event(seq: i64, name: &str, body: Value) -> Value {
    json!({ "seq": seq, "type": "event", "event": name, "body": body })
}

fn ok_response(seq: i64, req: &Value, body: Value) -> Value {
    json!({
        "seq": seq,
        "type": "response",
        "request_seq": req.get("seq").and_then(Value::as_i64).unwrap_or(0),
        "success": true,
        "command": req.get("command").and_then(Value::as_str).unwrap_or(""),
        "body": body,
    })
}

fn err_response(seq: i64, req: &Value, message: &str) -> Value {
    json!({
        "seq": seq,
        "type": "response",
        "request_seq": req.get("seq").and_then(Value::as_i64).unwrap_or(0),
        "success": false,
        "command": req.get("command").and_then(Value::as_str).unwrap_or(""),
        "message": message,
    })
}

async fn handle(
    st: &SharedDebugState,
    req: &Value,
    next_seq: &mut impl FnMut() -> i64,
) -> Vec<Value> {
    let command = req.get("command").and_then(Value::as_str).unwrap_or("");
    let args = req.get("arguments").cloned().unwrap_or(Value::Null);

    match command {
        "initialize" => {
            // The response must precede the `initialized` event. Reversing these
            // is the classic reason a DAP session hangs at "starting".
            vec![
                ok_response(next_seq(), req, json!({
                    "supportsConfigurationDoneRequest": true,
                    "supportsTerminateRequest": true,
                    "supportsEvaluateForHovers": true,
                    // Writes are refused by the helper: an assignment applied
                    // from the wrong stack depth would corrupt live game state.
                    "supportsSetVariable": false,
                    "supportsConditionalBreakpoints": true,
                    "supportsHitConditionalBreakpoints": true,
                    "supportsFunctionBreakpoints": false,
                    "exceptionBreakpointFilters": [],
                })),
                event(next_seq(), "initialized", json!({})),
            ]
        }

        // The server is already running, so launch and attach are the same thing.
        "attach" | "launch" => {
            st.clients.store(1, Ordering::Relaxed);
            st.republish();
            vec![ok_response(next_seq(), req, Value::Null)]
        }

        "setBreakpoints" => {
            let path = args
                .get("source")
                .and_then(|s| s.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let specs: Vec<(u32, BreakpointSpec)> = args
                .get("breakpoints")
                .and_then(Value::as_array)
                .map(|bps| {
                    bps.iter()
                        .filter_map(|b| {
                            let line = b.get("line").and_then(Value::as_u64)? as u32;
                            Some((line, parse_bp_spec(b)))
                        })
                        .collect()
                })
                .unwrap_or_default();

            let key = paths::normalize(path);
            let ids = st.set_breakpoints(key, &specs);

            // We cannot cheaply tell whether a line is executable, so every
            // breakpoint in a known file is reported verified. One on a blank or
            // comment line simply never fires.
            let verified: Vec<Value> = ids
                .iter()
                .zip(&specs)
                .map(|(id, (line, _))| json!({ "id": id, "verified": true, "line": line }))
                .collect();
            vec![ok_response(next_seq(), req, json!({ "breakpoints": verified }))]
        }

        // VS Code sends this during configuration even with no filters, and
        // hangs waiting for the response.
        "setExceptionBreakpoints" => vec![ok_response(next_seq(), req, json!({}))],

        "configurationDone" => {
            st.republish();
            vec![ok_response(next_seq(), req, Value::Null)]
        }

        // One entry per suspended dispatch, named for what it is, so a client
        // can tell "sheridan: hit" from "timer:combat.round" and pick between
        // them. Falls back to the single thread when nothing is parked — which
        // is always the case on the freezing path, where the VM itself is the
        // one thing that is stopped.
        "threads" => {
            let parked = st.parked_list();
            let threads: Vec<Value> = if parked.is_empty() {
                vec![json!({ "id": WORLD_STOP, "name": "mudlib" })]
            } else {
                parked
                    .into_iter()
                    .map(|(id, session, what)| {
                        let name = match (session.is_empty(), what.is_empty()) {
                            (true, true) => "mudlib".to_string(),
                            (true, false) => what,
                            (false, true) => session,
                            (false, false) => format!("{session}: {what}"),
                        };
                        json!({ "id": id, "name": name })
                    })
                    .collect()
            };
            vec![ok_response(next_seq(), req, json!({ "threads": threads }))]
        }

        "stackTrace" => {
            if !anything_stopped(st) {
                return vec![err_response(next_seq(), req, "not stopped")];
            }
            let levels = args
                .get("levels")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
                .unwrap_or(64) as usize;

            let (tx, rx) = tokio::sync::oneshot::channel();
            if !st.send_vm(VmRequest::StackTrace { stop: thread_of(&args, st), levels, reply: tx }) {
                return vec![err_response(next_seq(), req, "debug channel closed")];
            }
            match tokio::time::timeout(VM_REPLY_TIMEOUT, rx).await {
                Ok(Ok(frames)) => {
                    let out: Vec<Value> = frames
                        .iter()
                        .map(|f| {
                            let mut v = json!({
                                "id": f.id,
                                "name": f.name,
                                "line": f.line,
                                "column": 1,
                            });
                            if let Some(p) = &f.path {
                                v["source"] = json!({ "path": p, "name": paths::short(p) });
                            }
                            v
                        })
                        .collect();
                    vec![ok_response(next_seq(), req, json!({
                        "stackFrames": out,
                        "totalFrames": frames.len(),
                    }))]
                }
                _ => vec![err_response(next_seq(), req, "VM did not answer in time")],
            }
        }

        "scopes" => {
            if !anything_stopped(st) {
                return vec![err_response(next_seq(), req, "not stopped")];
            }
            let frame = args.get("frameId").and_then(Value::as_i64).unwrap_or(0);
            let (tx, rx) = tokio::sync::oneshot::channel();
            if !st.send_vm(VmRequest::Scopes { stop: thread_of(&args, st), frame, reply: tx }) {
                return vec![err_response(next_seq(), req, "debug channel closed")];
            }
            match tokio::time::timeout(VM_REPLY_TIMEOUT, rx).await {
                Ok(Ok(scopes)) => {
                    let out: Vec<Value> = scopes
                        .iter()
                        .map(|s| json!({
                            "name": s.name,
                            "variablesReference": s.var_ref,
                            "expensive": s.expensive,
                        }))
                        .collect();
                    vec![ok_response(next_seq(), req, json!({ "scopes": out }))]
                }
                _ => vec![err_response(next_seq(), req, "VM did not answer in time")],
            }
        }

        "variables" => {
            if !anything_stopped(st) {
                return vec![err_response(next_seq(), req, "not stopped")];
            }
            let var_ref = args.get("variablesReference").and_then(Value::as_i64).unwrap_or(0);
            let (tx, rx) = tokio::sync::oneshot::channel();
            if !st.send_vm(VmRequest::Variables { var_ref, reply: tx }) {
                return vec![err_response(next_seq(), req, "debug channel closed")];
            }
            match tokio::time::timeout(VM_REPLY_TIMEOUT, rx).await {
                Ok(Ok(vars)) => {
                    vec![ok_response(next_seq(), req, json!({ "variables": variables_json(&vars) }))]
                }
                _ => vec![err_response(next_seq(), req, "VM did not answer in time")],
            }
        }

        "evaluate" => {
            if !anything_stopped(st) {
                return vec![err_response(next_seq(), req, "not stopped")];
            }
            let frame = args.get("frameId").and_then(Value::as_i64).unwrap_or(0);
            let expr = args.get("expression").and_then(Value::as_str).unwrap_or("").to_string();
            let (tx, rx) = tokio::sync::oneshot::channel();
            if !st.send_vm(VmRequest::Evaluate { stop: thread_of(&args, st), frame, expr, reply: tx }) {
                return vec![err_response(next_seq(), req, "debug channel closed")];
            }
            match tokio::time::timeout(VM_REPLY_TIMEOUT, rx).await {
                Ok(Ok(Ok(v))) => vec![ok_response(next_seq(), req, json!({
                    "result": v.value,
                    "type": v.ty,
                    "variablesReference": v.var_ref,
                }))],
                // A failed expression is a normal outcome in a REPL, so report
                // the Lua error as the response message rather than logging it.
                Ok(Ok(Err(e))) => vec![err_response(next_seq(), req, &e)],
                _ => vec![err_response(next_seq(), req, "VM did not answer in time")],
            }
        }

        "continue" | "next" | "stepIn" | "stepOut" => {
            if !anything_stopped(st) {
                return vec![err_response(next_seq(), req, "not stopped")];
            }
            let kind = match command {
                "next" => ResumeKind::Next,
                "stepIn" => ResumeKind::StepIn,
                "stepOut" => ResumeKind::StepOut,
                _ => ResumeKind::Continue,
            };
            let stop = thread_of(&args, st);
            if !st.send_vm(VmRequest::Resume { stop, kind }) {
                return vec![err_response(next_seq(), req, "debug channel closed")];
            }
            // Only the named dispatch continues; the others stay where they are.
            vec![ok_response(next_seq(), req, json!({
                "allThreadsContinued": st.freezes(),
            }))]
        }

        "pause" => {
            // Consumed by the next line event. If the VM is idle nothing is
            // executing, so the stop lands on the next player command.
            st.pause_req.store(true, Ordering::Release);
            vec![ok_response(next_seq(), req, Value::Null)]
        }

        "disconnect" | "terminate" => {
            detach(st);
            vec![ok_response(next_seq(), req, Value::Null)]
        }

        other => {
            tracing::debug!("debugger: ignoring unsupported request '{other}'");
            vec![err_response(next_seq(), req, &format!("unsupported request '{other}'"))]
        }
    }
}
