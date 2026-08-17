//! `send_rich`, `mxp_var` and `mxp_expire` — the driver's markup surface.
//!
//! A sibling of `efuns_io.rs` and `efuns_document.rs` rather than three hundred
//! more lines in `efuns.rs`.
//!
//! # Why the authoring form is a table and not a string
//!
//! It would be shorter to write
//!
//! ```lua
//! mxp_send(sid, ('<send href="buy bread">%s</send>'):format(item.short))
//! ```
//!
//! and that is the API this driver deliberately does not have. `item.short` is
//! attacker-supplied in any game where a player can name something, and the
//! MXP specification is blunt about the consequence: a player who gets a secure
//! tag onto another player's screen can do real damage. An escaping helper
//! would exist only to be forgotten at the one call site that mattered.
//!
//! So the caller supplies a *tree*, [`crate::core::render`] turns it into
//! markup, and every string that reaches the wire from Lua passes through the
//! escaper on the way. There is no call site to get wrong, because there is no
//! call site that writes a `<`.
//!
//! # Failure convention
//!
//! The same split `efuns_document.rs` documents, applied here:
//!
//! * **Author error raises.** A malformed parts table, a command containing a
//!   `|`, a `javascript:` URL — all of these are bugs in mudlib code and the
//!   error names the field, the way `lua_to_json` does.
//! * **Delivery failure returns `false`.** A full output channel is not the
//!   caller's fault and there is nothing useful to do about it, so it is
//!   reported rather than raised.

use mlua::prelude::*;

use crate::core::network::telnet::mxp;
use crate::core::render::{Group, Node, RichLine};
use crate::core::session::{SessionId, SessionOutput};
use crate::core::lock::RwLockExt;

use super::efuns::EfunContext;

/// How deep a parts tree may nest.
///
/// Far past anything a line of prose needs, and the reason it exists at all is
/// that `local t = {} t.parts = {t}` would otherwise recurse until the Rust
/// stack is gone — the same failure `lua_to_json`'s depth cap was added for,
/// with the same silent process death.
const MAX_DEPTH: usize = 32;

/// How many nodes one line may contain.
const MAX_NODES: usize = 4096;

/// One step of the breadcrumb carried into errors, so a failure says *which*
/// part is at fault rather than only that one is.
enum Step {
    Key(&'static str),
    Index(usize),
}

fn render_path(path: &[Step]) -> String {
    if path.is_empty() {
        return "the parts table".to_string();
    }
    let mut rendered = String::from("parts");
    for step in path {
        match step {
            Step::Key(k) => {
                rendered.push('.');
                rendered.push_str(k);
            }
            Step::Index(i) => rendered.push_str(&format!("[{i}]")),
        }
    }
    format!("field `{rendered}`")
}

fn bad(path: &[Step], msg: &str) -> LuaError {
    LuaError::RuntimeError(format!("send_rich: {} — {msg}", render_path(path)))
}

// ─── marshalling ─────────────────────────────────────────────────────────────

/// A string that will be interpolated into an MXP attribute list.
///
/// `|` separates the items of a `<SEND>` menu and `"` ends an attribute value,
/// and neither has an escape inside that grammar. Mangling them silently would
/// turn one command into two, or one hint into a caption it was never given,
/// so this refuses instead. Escaping is not an option here the way it is for
/// display text: the client has to read these back as structure.
fn checked_action(s: String, path: &[Step]) -> LuaResult<String> {
    if s.contains('|') {
        return Err(bad(path, "a command or hint may not contain `|` — MXP uses it to separate the items of a menu, and there is no escape for it"));
    }
    if s.contains('"') {
        return Err(bad(path, "a command or hint may not contain a double quote"));
    }
    Ok(s)
}

/// A URL, restricted to schemes that mean "show the player something".
///
/// `javascript:` and `data:` are the reason. On telnet an `<A href>` opens the
/// player's browser; if the rich line ever reaches a browser-based client
/// directly — which is the obvious next step for this type — an unchecked
/// scheme is stored cross-site scripting. Checking it here, where the value
/// enters the system, means no renderer has to remember to.
fn checked_href(s: String, path: &[Step]) -> LuaResult<String> {
    let lower = s.to_ascii_lowercase();
    const ALLOWED: [&str; 3] = ["http://", "https://", "mailto:"];
    if !ALLOWED.iter().any(|p| lower.starts_with(p)) {
        return Err(bad(
            path,
            "href must begin with http://, https:// or mailto: — any other scheme is a way to run code on the player's machine",
        ));
    }
    checked_action(s, path)
}

/// A string, or an array of strings, into a `Vec`.
fn string_list(v: LuaValue, path: &[Step]) -> LuaResult<Vec<String>> {
    match v {
        LuaValue::Nil => Ok(Vec::new()),
        LuaValue::String(s) => Ok(vec![s.to_string_lossy()]),
        LuaValue::Table(t) => {
            let mut out = Vec::new();
            for (i, item) in t.sequence_values::<LuaValue>().enumerate() {
                match item? {
                    LuaValue::String(s) => out.push(s.to_string_lossy()),
                    LuaValue::Integer(n) => out.push(n.to_string()),
                    LuaValue::Number(n) => out.push(n.to_string()),
                    other => {
                        return Err(bad(
                            path,
                            &format!("item {} is a {}, expected a string", i + 1, other.type_name()),
                        ))
                    }
                }
            }
            Ok(out)
        }
        other => Err(bad(
            path,
            &format!("expected a string or an array of strings, got {}", other.type_name()),
        )),
    }
}

/// Turn one Lua value into a node.
///
/// A bare string is literal text. A table is a group, and must be a map: `text`
/// or `parts` for its content, `send`/`hint`/`href`/`prompt`/`expire`/`var` for
/// what it does. Never a mixed table — `{ send = "look", "here" }` is idiomatic
/// Lua and refused anyway, because `classify_table` refuses the same shape for
/// `lua_to_json` and two marshallers in one codebase with different table rules
/// is how a subtle mismatch survives review.
fn node_from_lua(
    value: LuaValue,
    depth: usize,
    budget: &mut usize,
    path: &mut Vec<Step>,
) -> LuaResult<Node> {
    if *budget == 0 {
        return Err(bad(path, &format!("a line may hold at most {MAX_NODES} parts")));
    }
    *budget -= 1;
    if depth >= MAX_DEPTH {
        return Err(bad(
            path,
            &format!("nesting is deeper than {MAX_DEPTH} — a table that refers to itself will always hit this"),
        ));
    }

    match value {
        LuaValue::String(s) => Ok(Node::Text(s.to_string_lossy())),
        LuaValue::Integer(n) => Ok(Node::Text(n.to_string())),
        LuaValue::Number(n) => Ok(Node::Text(n.to_string())),
        LuaValue::Table(t) => table_node(t, depth, budget, path),
        other => Err(bad(
            path,
            &format!(
                "expected a string or a table, got {} — a function or a userdata has no rendering",
                other.type_name()
            ),
        )),
    }
}

fn table_node(
    t: LuaTable,
    depth: usize,
    budget: &mut usize,
    path: &mut Vec<Step>,
) -> LuaResult<Node> {
    // `{ br = true }` and `{ expire = "exits", empty = true }` are commands
    // rather than containers, and are checked before anything else so a caller
    // is never asked for content they do not have.
    if t.get::<Option<bool>>("br")?.unwrap_or(false) {
        return Ok(Node::Break);
    }
    if t.get::<Option<bool>>("expire_now")?.unwrap_or(false) {
        return Ok(Node::Expire(t.get::<Option<String>>("expire")?));
    }

    let has_text = t.contains_key("text")?;
    let has_parts = t.contains_key("parts")?;
    if has_text && has_parts {
        return Err(bad(path, "give either `text` or `parts`, not both"));
    }

    let mut children = Vec::new();
    if has_text {
        path.push(Step::Key("text"));
        let node = node_from_lua(t.get::<LuaValue>("text")?, depth + 1, budget, path);
        path.pop();
        children.push(node?);
    } else if has_parts {
        let nested: LuaTable = t.get("parts")?;
        path.push(Step::Key("parts"));
        let result = children_from_lua(&nested, depth + 1, budget, path);
        path.pop();
        children = result?;
    }

    path.push(Step::Key("send"));
    let send = string_list(t.get("send")?, path)?
        .into_iter()
        .map(|s| checked_action(s, path))
        .collect::<LuaResult<Vec<_>>>()?;
    path.pop();

    path.push(Step::Key("hint"));
    let hint = string_list(t.get("hint")?, path)?
        .into_iter()
        .map(|s| checked_action(s, path))
        .collect::<LuaResult<Vec<_>>>()?;
    path.pop();

    path.push(Step::Key("href"));
    let href = match t.get::<Option<String>>("href")? {
        Some(u) => Some(checked_href(u, path)?),
        None => None,
    };
    path.pop();

    if href.is_some() && !send.is_empty() {
        return Err(bad(
            path,
            "give either `href` or `send` — a group with both is a link that does two different things depending on which client reads it",
        ));
    }

    let expire = t.get::<Option<String>>("expire")?;
    let var = t.get::<Option<String>>("var")?;
    if let Some(name) = &var {
        path.push(Step::Key("var"));
        // The name goes into the tag itself, where escaping it would change
        // which variable the client set.
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
            || name.is_empty()
        {
            let e = bad(path, "a variable name may only hold letters, digits and underscores");
            path.pop();
            return Err(e);
        }
        path.pop();
    }

    Ok(Node::Group(Group {
        children,
        send,
        hint,
        href,
        prompt: t.get::<Option<bool>>("prompt")?.unwrap_or(false),
        expire,
        var,
    }))
}

fn children_from_lua(
    t: &LuaTable,
    depth: usize,
    budget: &mut usize,
    path: &mut Vec<Step>,
) -> LuaResult<Vec<Node>> {
    let mut out = Vec::new();
    for (i, item) in t.clone().sequence_values::<LuaValue>().enumerate() {
        path.push(Step::Index(i + 1));
        let node = node_from_lua(item?, depth, budget, path);
        path.pop();
        out.push(node?);
    }
    Ok(out)
}

/// `opts.line` — a named line category, or a raw user tag.
fn line_tag_from(value: LuaValue) -> LuaResult<Option<u8>> {
    let n = match value {
        LuaValue::Nil => return Ok(None),
        LuaValue::String(s) => match s.to_string_lossy().as_str() {
            "room_name" => mxp::MODE_ROOM_NAME,
            "room_desc" => mxp::MODE_ROOM_DESC,
            "room_exits" => mxp::MODE_ROOM_EXITS,
            "welcome" => mxp::MODE_WELCOME,
            other => {
                return Err(LuaError::RuntimeError(format!(
                    "send_rich: unknown line category {other:?} — expected \"room_name\", \
                     \"room_desc\", \"room_exits\", \"welcome\", or a number from {}-{}",
                    mxp::USER_TAG_MIN,
                    mxp::USER_TAG_MAX
                )))
            }
        },
        LuaValue::Integer(i) => {
            if !(i64::from(mxp::USER_TAG_MIN)..=i64::from(mxp::USER_TAG_MAX)).contains(&i) {
                return Err(LuaError::RuntimeError(format!(
                    "send_rich: line tag {i} is out of range — a game's own tags are {}-{}; \
                     10, 11, 12 and 19 are reserved and have names",
                    mxp::USER_TAG_MIN,
                    mxp::USER_TAG_MAX
                )));
            }
            i as u8
        }
        other => {
            return Err(LuaError::RuntimeError(format!(
                "send_rich: `line` must be a string or a number, got {}",
                other.type_name()
            )))
        }
    };
    Ok(Some(n))
}

fn line_from_lua(nodes: Vec<Node>, opts: Option<&LuaTable>) -> LuaResult<RichLine> {
    let mut line = RichLine::new(nodes);
    let Some(o) = opts else { return Ok(line) };

    if let Some(mode) = o.get::<Option<String>>("mode")? {
        line.mode = match mode.as_str() {
            "secure" => mxp::LineMode::Secure,
            "open" => mxp::LineMode::Open,
            "locked" => mxp::LineMode::Locked,
            other => {
                return Err(LuaError::RuntimeError(format!(
                    "send_rich: unknown mode {other:?} — expected \"secure\", \"open\" or \"locked\""
                )))
            }
        };
    }
    line.tag = line_tag_from(o.get("line")?)?;
    if let Some(nl) = o.get::<Option<bool>>("newline")? {
        line.newline = nl;
    }
    Ok(line)
}

// ─── registration ────────────────────────────────────────────────────────────

pub fn register_render_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();

    // send_rich(session_id, parts, opts?) -> boolean
    //
    // Ungated, and that is the design working rather than an omission: this
    // cannot emit a `<` that did not come from the driver, so gating it would
    // be gating "send text".
    {
        let sh = ctx.session_handler.clone();
        let f = lua.create_function(
            move |_, (session_id, parts, opts): (String, LuaTable, Option<LuaTable>)| {
                let id: SessionId = session_id
                    .parse()
                    .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {e}")))?;
                let mut budget = MAX_NODES;
                let mut path = Vec::new();
                let nodes = children_from_lua(&parts, 0, &mut budget, &mut path)?;
                let line = line_from_lua(nodes, opts.as_ref())?;

                let handler = sh.read_recover();
                let Some(session) = handler.get(&id) else {
                    return Ok(false);
                };
                Ok(session.try_send(SessionOutput::Rich(Box::new(line))))
            },
        )?;
        globals.set("send_rich", f)?;
    }

    // mxp_var(session_id, name, value) -> boolean
    //
    // `<VAR hp>40</VAR>`: sets a client-side variable *and* displays the value,
    // which is the difference between `<VAR>` and `<!ENTITY>`. Session-scoped
    // and gone with the connection, so there is no registry to replay.
    {
        let sh = ctx.session_handler.clone();
        let f = lua.create_function(
            move |lua, (session_id, name, value): (String, String, LuaValue)| {
                let id: SessionId = session_id
                    .parse()
                    .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {e}")))?;
                let t = lua.create_table()?;
                t.set("var", name)?;
                t.set("text", value)?;
                let mut budget = MAX_NODES;
                let mut path = Vec::new();
                let node = table_node(t, 0, &mut budget, &mut path)?;

                let mut line = RichLine::new(vec![node]);
                // No terminator: a variable update is not a line of prose, and
                // appending one would put a blank line in the scrollback of
                // every client that renders the value.
                line.newline = false;

                let handler = sh.read_recover();
                let Some(session) = handler.get(&id) else {
                    return Ok(false);
                };
                Ok(session.try_send(SessionOutput::Rich(Box::new(line))))
            },
        )?;
        globals.set("mxp_var", f)?;
    }

    // mxp_expire(session_id, name?) -> boolean
    //
    // Retire the links tagged with `name`, or every named link if no name is
    // given. Links that never carried a name never expire.
    {
        let sh = ctx.session_handler.clone();
        let f = lua.create_function(move |_, (session_id, name): (String, Option<String>)| {
            let id: SessionId = session_id
                .parse()
                .map_err(|e| LuaError::RuntimeError(format!("Invalid session id: {e}")))?;
            let mut line = RichLine::new(vec![Node::Expire(name)]);
            line.newline = false;

            let handler = sh.read_recover();
            let Some(session) = handler.get(&id) else {
                return Ok(false);
            };
            Ok(session.try_send(SessionOutput::Rich(Box::new(line))))
        })?;
        globals.set("mxp_expire", f)?;
    }

    Ok(())
}
