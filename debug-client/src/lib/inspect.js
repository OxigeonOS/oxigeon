// The Lua the Inspect tab evaluates, and the parser for what comes back.
//
// A port of `src/bin/tui/inspect_payload.rs`. Two constraints from
// `src/core/scripting/debugger/introspect.lua` shape the design, and neither is
// negotiable:
//
// - `MAX_STR = 256` — any single value is truncated at 256 characters, so one
//   big concatenated string would be cut off.
// - `MAX_CHILDREN = 200` — a table expands to at most 200 rows.
//
// Hence: **an array of short delimited strings**, one row per trait or effect.
// `game/traits/` defines 27 across core and skills, comfortably inside 200.

/// Unit separator. A delimiter that cannot occur in a trait id, a label, or a
/// number, which `|` and `:` both can.
export const SEP = '\u001f'

/// Build the expression to evaluate. `target` is any Lua expression naming an
/// entity in the paused frame — `player` in most command frames, but a mob or
/// an item works just as well, because a trait is any numeric datum on any
/// entity rather than a character statistic.
///
/// Values are read through `DAEMON.trait.all` and `DAEMON.effect.active`, never
/// from `entity.stats`: for a derived or buffed trait the stored number is the
/// wrong answer, and showing the difference is the entire point of the pane.
export function expression(target) {
  return (
    `(function() ` +
    `local e = ${target} ` +
    `local o = {} ` +
    `if e and DAEMON and DAEMON.trait then ` +
    `local ok, list = pcall(DAEMON.trait.all, e) ` +
    `if ok and list then for _, t in ipairs(list) do ` +
    `o[#o+1] = table.concat({"T", tostring(t.id), tostring(t.label or ""), ` +
    `tostring(t.kind or ""), tostring(t.group or ""), tostring(t.base), ` +
    `tostring(t.value), tostring(t.max or ""), tostring(t.failed or false)}, "\\31") ` +
    `end end ` +
    `end ` +
    `if e and DAEMON and DAEMON.effect then ` +
    `local ok, list = pcall(DAEMON.effect.active, e) ` +
    `if ok and list then for _, a in ipairs(list) do ` +
    `o[#o+1] = table.concat({"E", tostring(a.inst and a.inst.def or "?"), ` +
    `tostring((a.def and a.def.label) or ""), ` +
    `tostring((a.inst and a.inst.stacks) or 1), ` +
    `tostring((a.inst and a.inst.expires) or "")}, "\\31") ` +
    `end end ` +
    `end ` +
    `return o ` +
    `end)()`
  )
}

/// Parse one row. `raw` may arrive with the surrounding quotes `introspect.lua`
/// puts around a string value, so they are stripped here rather than at every
/// call site.
export function parseRow(raw) {
  const fields = String(raw).trim().replace(/^"|"$/g, '').split(SEP)
  if (fields[0] === 'T' && fields.length >= 9) {
    return {
      row: 'trait',
      id: fields[1],
      label: fields[2],
      kind: fields[3],
      group: fields[4],
      base: fields[5],
      value: fields[6],
      max: fields[7],
      failed: fields[8] === 'true',
    }
  }
  if (fields[0] === 'E' && fields.length >= 5) {
    return {
      row: 'effect',
      id: fields[1],
      label: fields[2],
      stacks: fields[3],
      expires: fields[4],
    }
  }
  return null
}
