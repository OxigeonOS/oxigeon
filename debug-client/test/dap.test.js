// The DAP wire codec.
//
// `Content-Length` counts **bytes, not characters** — a body with any non-ASCII
// in it (a room description, a player name) desynchronizes the stream if this
// is measured in string length. That is the test that matters here.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { DapDecoder, encode } from '../bridge/dap.js'

test('a whole message decodes', () => {
  const d = new DapDecoder()
  const out = d.feed(encode({ seq: 1, type: 'request', command: 'attach' }))
  assert.equal(out.length, 1)
  assert.equal(out[0].command, 'attach')
})

test('two messages in one chunk both arrive', () => {
  const d = new DapDecoder()
  const out = d.feed(Buffer.concat([encode({ seq: 1 }), encode({ seq: 2 })]))
  assert.deepEqual(out.map((m) => m.seq), [1, 2])
})

test('a message split across chunks is held until whole', () => {
  const d = new DapDecoder()
  const whole = encode({ seq: 7, command: 'stackTrace' })
  for (let i = 0; i < whole.length - 1; i++) {
    assert.equal(d.feed(whole.subarray(i, i + 1)).length, 0, `byte ${i} completed early`)
  }
  const out = d.feed(whole.subarray(whole.length - 1))
  assert.equal(out.length, 1)
  assert.equal(out[0].seq, 7)
})

test('a non-ASCII body is framed in bytes, not characters', () => {
  // A room name with a dash in it is enough: the encoder must count 3 bytes for
  // an em dash, and the decoder must take 3 back.
  const message = { body: { name: 'Thornhollow — the square', emoji: '🜁' } }
  const frame = encode(message)
  const header = frame.subarray(0, frame.indexOf('\r\n\r\n')).toString()
  const declared = Number(header.split(':')[1].trim())
  assert.equal(declared, frame.length - header.length - 4)
  assert.notEqual(declared, JSON.stringify(message).length, 'the test is vacuous if these match')

  const d = new DapDecoder()
  const out = d.feed(frame)
  assert.equal(out.length, 1)
  assert.equal(out[0].body.name, 'Thornhollow — the square')
})

test('extra headers are ignored and the case of Content-Length does not matter', () => {
  const d = new DapDecoder()
  const body = '{"seq":3}'
  const out = d.feed(
    Buffer.from(`content-length: ${body.length}\r\nX-Whatever: yes\r\n\r\n${body}`)
  )
  assert.equal(out[0].seq, 3)
})

test('a header with no usable Content-Length is refused rather than guessed', () => {
  const d = new DapDecoder()
  assert.throws(() => d.feed(Buffer.from('X-Nothing: 1\r\n\r\n{}')), /Content-Length/)
})

test('an absurd Content-Length is refused rather than pre-allocated from', () => {
  const d = new DapDecoder()
  assert.throws(() => d.feed(Buffer.from('Content-Length: 99999999\r\n\r\n')), /exceeds the limit/)
})
