// The Rust suite in `src/bin/tui/journal.rs`.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { parseEntry } from '../bridge/journal.js'
import { clock } from '../src/lib/journalfmt.js'

test('parses a driver-written entry', () => {
  const raw =
    '{"ts":"2026-07-15T18:30:00Z","level":"error","source":"login.lua:42","msg":"attempt to index a nil value","meta":{"sid":"f3a2b1c0"}}'
  const e = parseEntry(raw)
  assert.equal(e.level, 'error')
  assert.equal(e.source, 'login.lua:42')
  assert.equal(clock(e), '18:30:00')
  assert.equal(e.msg, 'attempt to index a nil value')
})

test('an unparseable line is shown rather than dropped', () => {
  // A half-written line during a crash is exactly when you want to see it.
  const e = parseEntry('{half written')
  assert.equal(e.level, 'raw')
  assert.equal(e.msg, '{half written')
})

test('a non-string field does not become the string "undefined"', () => {
  const e = parseEntry('{"ts":null,"level":"warn","source":7,"msg":"x"}')
  assert.equal(e.source, '')
  assert.equal(e.ts, '')
})

test('a timestamp shorter than expected falls back to itself', () => {
  assert.equal(clock({ ts: '18:30' }), '18:30')
  assert.equal(clock({}), '')
})
