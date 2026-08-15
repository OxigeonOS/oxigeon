// Rendering the driver's styled spans.
//
// This replaces the ANSI-decoder suite, which was a port of
// `src/bin/tui/ansi.rs`. That state machine is gone: `?ansi=spans` makes the
// driver do it, once, with its own tests. What is left to check is the half a
// browser still owns — the palette, the CSS, and splitting a run of spans into
// lines.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { css, indexed, linesOf } from '../src/lib/spans.js'

// ─── palette ─────────────────────────────────────────────────────────────────

test('the sixteen basic colours are palette indices 0-15', () => {
  // `{red}` is 1, `{bright_blue}` is 12 — one integer covers everything
  // lib/color.lua can emit, so a client needs one lookup table rather than three.
  assert.equal(indexed(0), '#000000')
  assert.equal(indexed(1), '#cd0000')
  assert.equal(indexed(12), '#5c5cff')
  assert.equal(indexed(15), '#ffffff')
})

test('the cube and the greyscale ramp resolve', () => {
  assert.equal(indexed(16), '#000000') // cube origin
  assert.equal(indexed(208), '#ff8700') // `{orange}`
  assert.equal(indexed(231), '#ffffff') // cube apex
  assert.equal(indexed(232), '#080808') // ramp start
  assert.equal(indexed(255), '#eeeeee') // ramp end
})

test('an absent or out-of-range colour is no colour, not a broken one', () => {
  // Absent means off, so an unstyled run is just `{text}`.
  for (const bad of [undefined, null, -1, 256, 1.5, '1']) assert.equal(indexed(bad), null)
})

// ─── css ─────────────────────────────────────────────────────────────────────

test('an unstyled span produces no css at all', () => {
  assert.equal(css({ text: 'plain' }), '')
})

test('anything that is not a span styles nothing rather than raising', () => {
  // A `style=` expression runs inside the render, so throwing loses the pane
  // and every keystroke bound to it — not just a colour.
  for (const bad of [undefined, null, 'red', 42]) assert.equal(css(bad), '')
})

test('every span linesOf produces is one css accepts', () => {
  // The contract between the two, stated once. It was broken by a call site
  // still passing `span.style` — the shape this had before the driver started
  // sending structured runs — and the Play tab would not mount from the first
  // `prompt` frame onwards.
  const frames = [
    { spans: [{ text: 'a', fg: 1, bold: true }, { text: 'b\nc' }] },
    { text: 'plain\ntext' },
    { text: 'HP:40/40 > ' },
    { spans: [] },
    {},
  ]
  for (const frame of frames) {
    for (const line of linesOf(frame)) {
      for (const span of line) {
        assert.doesNotThrow(() => css(span))
        assert.equal(typeof span.text, 'string', 'a span always carries its text')
        assert.equal(span.style, undefined, 'spans are flat — there is no nested `style`')
      }
    }
  }
})

test('colour and weight become properties', () => {
  const out = css({ text: 'You are bleeding', fg: 1, bold: true })
  assert.ok(out.includes('color:#cd0000'))
  assert.ok(out.includes('font-weight:700'))
})

test('underline and strike share one text-decoration', () => {
  const out = css({ underline: true, strike: true })
  assert.equal(out.match(/text-decoration/g).length, 1)
  assert.ok(out.includes('underline line-through'))
})

test('inverse swaps the two colours', () => {
  const out = css({ fg: 1, bg: 4, inverse: true })
  assert.ok(out.includes('color:#0000ee'))
  assert.ok(out.includes('background:#cd0000'))
})

test('inverse with only a foreground falls back to the pane colours', () => {
  // Otherwise inverted text on a default background renders as nothing at all.
  const out = css({ fg: 1, inverse: true })
  assert.ok(out.includes('color:var(--bg)'))
  assert.ok(out.includes('background:#cd0000'))
})

// ─── frames to lines ─────────────────────────────────────────────────────────

test('a spans frame becomes one line when it has no newline', () => {
  const lines = linesOf({ spans: [{ text: 'a' }, { text: 'b', fg: 2 }] })
  assert.equal(lines.length, 1)
  assert.deepEqual(lines[0].map((s) => s.text), ['a', 'b'])
})

test('a newline inside a span splits the line and keeps the style', () => {
  const lines = linesOf({ spans: [{ text: 'one\ntwo', fg: 2, bold: true }] })
  assert.equal(lines.length, 2)
  assert.deepEqual(lines[0], [{ text: 'one', fg: 2, bold: true }])
  assert.deepEqual(lines[1], [{ text: 'two', fg: 2, bold: true }])
})

test('a style spanning a line break carries onto the next line', () => {
  const lines = linesOf({ spans: [{ text: 'green\nstill', fg: 2 }, { text: ' plain' }] })
  assert.equal(lines[1][0].fg, 2)
  assert.equal(lines[1][1].fg, undefined)
})

test('a text frame is split on newlines, since no CR ever reaches the client', () => {
  // The driver normalizes: interior `\r\n` becomes `\n`, exactly one trailing
  // terminator is stripped, and no `\r` arrives.
  const lines = linesOf({ text: 'one\ntwo' })
  assert.deepEqual(lines, [[{ text: 'one' }], [{ text: 'two' }]])
})

test('a deliberate blank line is content and survives as an empty line', () => {
  assert.deepEqual(linesOf({ text: 'a\n\nb' }), [[{ text: 'a' }], [], [{ text: 'b' }]])
  assert.deepEqual(linesOf({ spans: [{ text: 'a\n\nb' }] }), [[{ text: 'a' }], [], [{ text: 'b' }]])
})

test('spans wins over text, since exactly one of the two is ever present', () => {
  // `spans` omits `text` rather than sending both — `text` is the busiest frame
  // in the protocol and duplicating its content would double it.
  assert.deepEqual(linesOf({ spans: [{ text: 'structured' }], text: 'stale' }), [
    [{ text: 'structured' }],
  ])
})
