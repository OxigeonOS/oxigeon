// The seam a sent command leaves in the scrollback.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { BREAK, appendBreak, isBreak } from '../src/lib/scrollback.js'

const line = (text) => [{ text }]

test('a break is told apart from a line of spans by its shape', () => {
  assert.equal(isBreak(BREAK), true)
  assert.equal(isBreak(line('You see a well.')), false)
  assert.equal(isBreak([]), false, 'a blank line the game sent is still a line')
})

test('a break carries nothing that was typed', () => {
  // It is emitted while a password prompt is up, so this is the property that
  // makes that safe rather than a thing to remember not to do.
  assert.deepEqual(Object.keys(BREAK), ['rule'])
  assert.equal(Object.isFrozen(BREAK), true)
})

test('a break is appended after existing output', () => {
  const scrollback = [line('one')]
  assert.equal(appendBreak(scrollback), true)
  assert.equal(scrollback.length, 2)
  assert.equal(isBreak(scrollback[1]), true)
})

test('nothing is separated from nothing', () => {
  // A rule above the first line of a session is a rule with one side.
  const scrollback = []
  assert.equal(appendBreak(scrollback), false)
  assert.deepEqual(scrollback, [])
})

test('two commands with no output between them leave one seam', () => {
  // Two rules touching read as a wider gap rather than a seam, which is the
  // opposite of the point.
  const scrollback = [line('one')]
  appendBreak(scrollback)
  assert.equal(appendBreak(scrollback), false)
  assert.equal(scrollback.filter(isBreak).length, 1)
})

test('output between two commands earns a second seam', () => {
  const scrollback = [line('one')]
  appendBreak(scrollback)
  scrollback.push(line('a reply'))
  assert.equal(appendBreak(scrollback), true)
  assert.equal(scrollback.filter(isBreak).length, 2)
  assert.equal(isBreak(scrollback.at(-1)), true)
})
