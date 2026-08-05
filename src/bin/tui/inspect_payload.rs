//! The Lua the Inspect tab evaluates, and the parser for what comes back.
//!
//! Deliberately free of any dependency on the rest of the TUI, so
//! `tests/tui_inspect_payload.rs` can compile this one file and run the real
//! expression against a real booted mudlib. A payload tested only against a
//! hand-written fixture would pass while the thing it names had been renamed.
//!
//! Two constraints from `src/core/scripting/debugger/introspect.lua` shape the
//! design, and neither is negotiable:
//!
//! - `MAX_STR = 256` — any single value is truncated at 256 characters, so one
//!   big concatenated string would be cut off.
//! - `MAX_CHILDREN = 200` — a table expands to at most 200 rows.
//!
//! Hence: **an array of short delimited strings**, one row per trait or effect.
//! `game/traits/` defines 27 across core and skills, comfortably inside 200.

/// Unit separator. A delimiter that cannot occur in a trait id, a label, or a
/// number, which `|` and `:` both can.
pub const SEP: char = '\u{1f}';

#[derive(Debug, Clone, PartialEq)]
pub struct TraitRow {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub group: String,
    pub base: String,
    pub value: String,
    pub max: String,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectRow {
    pub id: String,
    pub label: String,
    pub stacks: String,
    pub expires: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Trait(TraitRow),
    Effect(EffectRow),
}

/// Build the expression to evaluate. `target` is any Lua expression naming an
/// entity in the paused frame — `player` in most command frames, but a mob or
/// an item works just as well, because a trait is any numeric datum on any
/// entity rather than a character statistic.
///
/// Values are read through `DAEMON.trait.all` and `DAEMON.effect.active`, never
/// from `entity.stats`: for a derived or buffed trait the stored number is the
/// wrong answer, and showing the difference is the entire point of the pane.
pub fn expression(target: &str) -> String {
    format!(
        "(function() \
           local e = {target} \
           local o = {{}} \
           if e and DAEMON and DAEMON.trait then \
             local ok, list = pcall(DAEMON.trait.all, e) \
             if ok and list then for _, t in ipairs(list) do \
               o[#o+1] = table.concat({{\"T\", tostring(t.id), tostring(t.label or \"\"), \
                 tostring(t.kind or \"\"), tostring(t.group or \"\"), tostring(t.base), \
                 tostring(t.value), tostring(t.max or \"\"), tostring(t.failed or false)}}, \"\\31\") \
             end end \
           end \
           if e and DAEMON and DAEMON.effect then \
             local ok, list = pcall(DAEMON.effect.active, e) \
             if ok and list then for _, a in ipairs(list) do \
               o[#o+1] = table.concat({{\"E\", tostring(a.inst and a.inst.def or \"?\"), \
                 tostring((a.def and a.def.label) or \"\"), \
                 tostring((a.inst and a.inst.stacks) or 1), \
                 tostring((a.inst and a.inst.expires) or \"\")}}, \"\\31\") \
             end end \
           end \
           return o \
         end)()",
        target = target
    )
}

/// Parse one row. `raw` may arrive with the surrounding quotes `introspect.lua`
/// puts around a string value, so they are stripped here rather than at every
/// call site.
pub fn parse_row(raw: &str) -> Option<Row> {
    let fields: Vec<&str> = raw.trim().trim_matches('"').split(SEP).collect();
    match fields.first()? {
        &"T" if fields.len() >= 9 => Some(Row::Trait(TraitRow {
            id: fields[1].into(),
            label: fields[2].into(),
            kind: fields[3].into(),
            group: fields[4].into(),
            base: fields[5].into(),
            value: fields[6].into(),
            max: fields[7].into(),
            failed: fields[8] == "true",
        })),
        &"E" if fields.len() >= 5 => Some(Row::Effect(EffectRow {
            id: fields[1].into(),
            label: fields[2].into(),
            stacks: fields[3].into(),
            expires: fields[4].into(),
        })),
        _ => None,
    }
}
