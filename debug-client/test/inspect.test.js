// The Inspect tab's payload.
//
// The Rust equivalent is `src/bin/tui/inspect_payload.rs` plus
// `tests/demo_world/tui_inspect_payload.rs`, which runs the expression against
// a booted mudlib — a payload tested only against a hand-written fixture would
// pass while the thing it names had been renamed. That half cannot be done from
// here; what is testable is the shape and the parser.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { SEP, expression, parseRow } from '../src/lib/inspect.js'

test('the separator is the unit separator, which cannot occur in a trait id', () => {
  assert.equal(SEP.charCodeAt(0), 31)
})

test('the expression names the two daemons the values must come from', () => {
  const expr = expression('player')
  // Never `entity.stats`: for a derived or buffed trait the stored number is
  // the wrong answer, and showing the difference is the entire point.
  assert.ok(expr.includes('DAEMON.trait.all'))
  assert.ok(expr.includes('DAEMON.effect.active'))
  assert.ok(!expr.includes('.stats['))
})

test('the target is substituted, so any expression naming an entity works', () => {
  assert.ok(expression('mobs[1]').includes('local e = mobs[1]'))
})

test('every read is wrapped in pcall, so a missing daemon does not raise', () => {
  const expr = expression('player')
  assert.equal(expr.match(/pcall/g).length, 2)
})

test('rows are emitted one per trait, not one big concatenated string', () => {
  // introspect.lua truncates any single value at MAX_STR = 256, so a single
  // string would be cut off. An array of short rows is the shape that survives.
  const expr = expression('player')
  assert.ok(expr.includes('o[#o+1]'))
  assert.ok(expr.includes('return o'))
})

test('a trait row parses', () => {
  const raw = ['T', 'max_hp', 'Health', 'derived', 'core', '0', '42', '', 'false'].join(SEP)
  assert.deepEqual(parseRow(raw), {
    row: 'trait',
    id: 'max_hp',
    label: 'Health',
    kind: 'derived',
    group: 'core',
    base: '0',
    value: '42',
    max: '',
    failed: false,
  })
})

test('an effect row parses', () => {
  const raw = ['E', 'blessed', 'Blessed', '2', '1690000000'].join(SEP)
  assert.deepEqual(parseRow(raw), {
    row: 'effect',
    id: 'blessed',
    label: 'Blessed',
    stacks: '2',
    expires: '1690000000',
  })
})

test('the quotes introspect.lua puts around a string value are stripped', () => {
  const raw = `"${['T', 'a', 'b', 'c', 'd', '1', '2', '', 'true'].join(SEP)}"`
  const row = parseRow(raw)
  assert.equal(row.id, 'a')
  assert.equal(row.failed, true)
})

test('a short or unknown row is dropped rather than half-parsed', () => {
  assert.equal(parseRow(['T', 'only', 'three'].join(SEP)), null)
  assert.equal(parseRow(['X', 'a', 'b', 'c', 'd'].join(SEP)), null)
  assert.equal(parseRow(''), null)
})
