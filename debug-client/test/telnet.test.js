// The Rust suite in `src/bin/tui/telnet.rs`, against the bridge's parser and
// negotiation answers.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  DO, DONT, IAC, OPT_ECHO, OPT_GMCP, OPT_MCCP2, OPT_NAWS, OPT_SGA, OPT_TTYPE, SB, SE, WILL, WONT,
  TelnetParser, encodeGmcp, encodeText, respond,
} from '../bridge/telnet.js'

/// Feed bytes through a fresh parser and collect what a client would write back
/// and what it would tell the UI.
function drive(input) {
  const parser = new TelnetParser()
  const answered = new Set()
  const out = []
  const emitted = []
  for (const event of parser.feed(Buffer.from(input))) {
    const { reply, emit } = respond(event, answered, { w: 80, h: 24 })
    if (reply) out.push(reply)
    emitted.push(...emit)
  }
  return { out: Buffer.concat(out), emit: emitted }
}

test('GMCP offer is accepted and answered with hello and supports', () => {
  const { out } = drive([IAC, WILL, OPT_GMCP])
  assert.deepEqual([...out.subarray(0, 3)], [IAC, DO, OPT_GMCP])
  const text = out.toString('latin1')
  assert.ok(text.includes('Core.Hello'))
  assert.ok(text.includes('Core.Supports.Set'))
  assert.ok(text.includes('Char 1'))
})

test('MCCP2 is refused, because the driver never actually compresses', () => {
  const { out } = drive([IAC, WILL, OPT_MCCP2])
  assert.deepEqual([...out], [IAC, DONT, OPT_MCCP2])
})

test('ECHO toggles masking in both directions and is never de-duplicated', () => {
  // The driver toggles ECHO around every password prompt, so the second WILL
  // must still mask. De-duplicating it would silently expose a password on a
  // re-login.
  const { out, emit } = drive([IAC, WILL, OPT_ECHO, IAC, WONT, OPT_ECHO, IAC, WILL, OPT_ECHO])
  assert.deepEqual([...out], [IAC, DO, OPT_ECHO, IAC, DONT, OPT_ECHO, IAC, DO, OPT_ECHO])
  assert.deepEqual(
    emit.filter((e) => e.t === 'echo').map((e) => e.on),
    [true, false, true]
  )
})

test('NAWS is offered with the current size', () => {
  const { out } = drive([IAC, DO, OPT_NAWS])
  assert.deepEqual([...out.subarray(0, 3)], [IAC, WILL, OPT_NAWS])
  // 80x24 big-endian, wrapped in a subnegotiation.
  assert.deepEqual([...out.subarray(3)], [IAC, SB, OPT_NAWS, 0, 80, 0, 24, IAC, SE])
})

test('terminal type is reported when asked', () => {
  const { out } = drive([IAC, SB, OPT_TTYPE, 1, IAC, SE])
  assert.deepEqual([...out.subarray(0, 3)], [IAC, SB, OPT_TTYPE])
  assert.equal(out[3], 0) // TTYPE IS
  assert.ok(out.toString('latin1').includes('OXIGEON-WEB'))
})

test('an unknown option is refused rather than ignored', () => {
  const { out } = drive([IAC, WILL, 99, IAC, DO, 98])
  assert.deepEqual([...out], [IAC, DONT, 99, IAC, WONT, 98])
})

test('a repeated offer is answered once', () => {
  const { out } = drive([IAC, WILL, OPT_SGA, IAC, WILL, OPT_SGA])
  assert.deepEqual([...out], [IAC, DO, OPT_SGA])
})

test('game text and GMCP interleaved in one read both arrive', () => {
  const input = Buffer.concat([
    Buffer.from('You see a well.\r\n'),
    encodeGmcp('Char.Vitals', '{"hp":42,"maxhp":50}'),
    Buffer.from('42h> '),
  ])
  const { emit } = drive(input)
  const text = emit
    .filter((e) => e.t === 'game')
    .map((e) => e.bytes.toString('utf8'))
    .join('')
  assert.ok(text.includes('well'))
  assert.ok(text.includes('42h>'))
  const gmcp = emit.find((e) => e.t === 'gmcp')
  assert.equal(gmcp.package, 'Char.Vitals')
  assert.ok(gmcp.json.includes('42'))
})

test('an escaped IAC in the data stream is one 0xFF byte, not a command', () => {
  const { out, emit } = drive([0x41, IAC, IAC, 0x42])
  assert.equal(out.length, 0, 'nothing to answer')
  const bytes = Buffer.concat(emit.filter((e) => e.t === 'game').map((e) => e.bytes))
  assert.deepEqual([...bytes], [0x41, 0xff, 0x42])
})

test('a negotiation split across two reads still parses', () => {
  const parser = new TelnetParser()
  assert.deepEqual(parser.feed(Buffer.from([IAC])), [])
  assert.deepEqual(parser.feed(Buffer.from([WILL])), [])
  assert.deepEqual(parser.feed(Buffer.from([OPT_GMCP])), [
    { kind: 'negotiate', verb: WILL, option: OPT_GMCP },
  ])
})

test('a subnegotiation split across reads keeps its payload', () => {
  const parser = new TelnetParser()
  const whole = encodeGmcp('Room.Info', '{"id":"a.b"}')
  parser.feed(whole.subarray(0, 6))
  const events = parser.feed(whole.subarray(6))
  const sub = events.find((e) => e.kind === 'subnegotiation')
  assert.equal(sub.option, OPT_GMCP)
  assert.equal(sub.data.toString('utf8'), 'Room.Info {"id":"a.b"}')
})

test('outbound text is CRLF terminated and escapes a high byte', () => {
  assert.deepEqual([...encodeText('hi\n')], [0x68, 0x69, 0x0d, 0x0a])
  // A player typing 0xFF must not be able to inject a command.
  assert.deepEqual([...encodeText('ÿ')], [0xc3, 0xbf], 'UTF-8 encodes it below IAC')
  assert.deepEqual([...encodeText(Buffer.from([0xff]).toString('latin1'))], [0xc3, 0xbf])
})
