// The Rust suite in `src/bin/tui/ansi.rs`, against the JS decoder.
//
// Ported case for case on purpose: what these assert is that the port answers
// what the original answered, and a test rewritten to suit the port would agree
// with whatever it happens to do.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { AnsiDecoder, indexed } from '../src/lib/ansi.js'

const parts = (line) => line.map((s) => [s.text, s.style])

test('plain text splits on newlines and drops CR', () => {
  const d = new AnsiDecoder()
  const lines = d.feed('one\r\ntwo\r\n')
  assert.equal(lines.length, 2)
  assert.equal(parts(lines[0])[0][0], 'one')
  assert.equal(parts(lines[1])[0][0], 'two')
  assert.equal(d.partial(), null)
})

test('a trailing partial line is the prompt', () => {
  const d = new AnsiDecoder()
  const lines = d.feed('scrolled\r\n42h 20m> ')
  assert.equal(lines.length, 1)
  const prompt = d.partial()
  assert.ok(prompt, 'prompt should be buffered')
  assert.equal(parts(prompt)[0][0], '42h 20m> ')
})

test('colour codes become styled spans', () => {
  const d = new AnsiDecoder()
  // What `{red}danger{/} ok` colorizes to.
  const p = parts(d.feed('\x1b[31mdanger\x1b[0m ok\r\n')[0])
  assert.equal(p.length, 2)
  assert.equal(p[0][0], 'danger')
  assert.equal(p[0][1].fg, '#cd0000')
  assert.equal(p[1][0], ' ok')
  assert.equal(p[1][1].fg, null)
})

test('xterm-256 foreground and background', () => {
  const d = new AnsiDecoder()
  // `{orange}` is 208; `{bg:17}` is the midnight background.
  const p = parts(d.feed('\x1b[38;5;208mA\x1b[48;5;17mB\r\n')[0])
  assert.equal(p[0][1].fg, indexed(208))
  assert.equal(p[1][1].fg, indexed(208))
  assert.equal(p[1][1].bg, indexed(17))
})

test('truecolor is understood even though the mudlib does not emit it', () => {
  const d = new AnsiDecoder()
  assert.equal(parts(d.feed('\x1b[38;2;10;20;30mx\r\n')[0])[0][1].fg, '#0a141e')
})

test('modifiers accumulate and clear', () => {
  const d = new AnsiDecoder()
  const p = parts(d.feed('\x1b[1m\x1b[4mboth\x1b[24monly bold\r\n')[0])
  assert.equal(p[0][1].bold, true)
  assert.equal(p[0][1].underline, true)
  assert.equal(p[1][1].bold, true)
  assert.equal(p[1][1].underline, false)
})

test('a sequence split across reads still parses', () => {
  // The failure this guards: an escape straddling a frame boundary rendering as
  // literal "[31m" in the game pane.
  const d = new AnsiDecoder()
  assert.equal(d.feed('\x1b[3').length, 0)
  assert.equal(d.feed('1mred').length, 0)
  const p = parts(d.feed('\r\n')[0])
  assert.equal(p[0][0], 'red')
  assert.equal(p[0][1].fg, '#cd0000')
})

test('style carries across a line break', () => {
  const d = new AnsiDecoder()
  const lines = d.feed('\x1b[32mgreen\r\nstill green\r\n')
  assert.equal(parts(lines[0])[0][1].fg, '#00cd00')
  assert.equal(parts(lines[1])[0][1].fg, '#00cd00')
})

test('non-SGR CSI sequences are swallowed, not printed', () => {
  const d = new AnsiDecoder()
  assert.equal(parts(d.feed('a\x1b[2Jb\r\n')[0])[0][0], 'ab')
})

test('a no-op reset does not fragment a word', () => {
  const d = new AnsiDecoder()
  assert.equal(parts(d.feed('unbro\x1b[0mken\r\n')[0]).length, 1)
})

test('bright colours are distinct from their basic pair', () => {
  const d = new AnsiDecoder()
  const p = parts(d.feed('\x1b[31ma\x1b[91mb\r\n')[0])
  assert.notEqual(p[0][1].fg, p[1][1].fg)
})

test('the 256-colour cube and greyscale ramp resolve', () => {
  assert.equal(indexed(0), '#000000')
  assert.equal(indexed(15), '#ffffff')
  assert.equal(indexed(16), '#000000') // cube origin
  assert.equal(indexed(231), '#ffffff') // cube apex
  assert.equal(indexed(232), '#080808') // ramp start
  assert.equal(indexed(255), '#eeeeee') // ramp end
})
