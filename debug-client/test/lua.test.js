// The Lua tokenizer. Not a parser: a run-splitter good enough that a source
// pane reads like source, and wrong only where Lua itself would reject the file.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { blockState, tokenize, withMatches } from '../src/lib/lua.js'

const kinds = (runs) => runs.map((r) => [r.text, r.kind])
const of = (line, startsIn = null) => kinds(tokenize(line, startsIn))

// Adjacent runs of the same kind are merged — fewer spans is less work for the
// renderer — so `x`, the spaces around it and the `=` arrive as one `plain`
// run. These expectations are written against that, not around it.

test('keywords, literals and plain names are told apart', () => {
  assert.deepEqual(of('local x = nil'), [
    ['local', 'keyword'],
    [' x = ', 'plain'],
    ['nil', 'literal'],
  ])
})

test('a name followed by ( is the thing you scan a file for', () => {
  assert.deepEqual(of('DAEMON.trait.all(entity)'), [
    ['DAEMON.trait.', 'plain'],
    ['all', 'ident'],
    ['(entity)', 'plain'],
  ])
})

test('whitespace between a name and its paren still makes it a call', () => {
  assert.ok(of('require  ("x")').some(([t, k]) => t === 'require' && k === 'ident'))
})

test('`self` is coloured as structure even though it is not reserved', () => {
  // The alternative is that the most important name in a method body looks like
  // every other local.
  assert.deepEqual(of('self'), [['self', 'keyword']])
})

test('a short comment eats the rest of the line', () => {
  assert.deepEqual(of('local x -- and "this" too'), [
    ['local', 'keyword'],
    [' x ', 'plain'],
    ['-- and "this" too', 'comment'],
  ])
})

test('a quoted string swallows what looks like code inside it', () => {
  assert.deepEqual(of('"local x" end'), [
    ['"local x"', 'string'],
    [' ', 'plain'],
    ['end', 'keyword'],
  ])
})

test('an escaped quote does not end the string', () => {
  assert.deepEqual(of('"a\\"b" x'), [
    ['"a\\"b"', 'string'],
    [' x', 'plain'],
  ])
})

test('an unterminated string ends at the line rather than swallowing the file', () => {
  assert.deepEqual(of('"never closed'), [['"never closed', 'string']])
})

test('numbers are literals, including hex and decimals', () => {
  assert.deepEqual(of('1 2.5 0xff'), [
    ['1', 'literal'],
    [' ', 'plain'],
    ['2.5', 'literal'],
    [' ', 'plain'],
    ['0xff', 'literal'],
  ])
})

// ─── long brackets ───────────────────────────────────────────────────────────

test('a long string on one line is a string', () => {
  assert.deepEqual(of('local s = [[hello]]'), [
    ['local', 'keyword'],
    [' s = ', 'plain'],
    ['[[hello]]', 'string'],
  ])
})

test('blockState tracks a long bracket across lines', () => {
  // `[[ ]]` for multi-line description strings is the house style, so this is
  // not an edge case — it is most of an area file.
  const lines = ['local d = [[', 'a description', 'over lines]]', 'local x = 1']
  assert.deepEqual(blockState(lines), [null, 0, 0, null])
})

test('only a matching level closes a long bracket', () => {
  const lines = ['local s = [==[', 'holds ]] and ]=] fine', 'done ]==]', 'after']
  assert.deepEqual(blockState(lines), [null, 2, 2, null])
})

test('a line inside a long bracket is all one run', () => {
  assert.deepEqual(of('anything at all "quoted" --not a comment', 0), [
    ['anything at all "quoted" --not a comment', 'comment'],
  ])
})

test('the line that closes a long bracket resumes code after it', () => {
  assert.deepEqual(of('tail]] .. x', 0), [
    ['tail]]', 'comment'],
    [' .. x', 'plain'],
  ])
})

test('a long comment opens a block, and a short one does not', () => {
  assert.deepEqual(blockState(['--[[ opens', 'still in', ']] out']), [null, 0, 0])
  assert.deepEqual(blockState(['-- [[ not an opener', 'out']), [null, null])
})

test('a long bracket inside a quoted string does not open a block', () => {
  assert.deepEqual(blockState(['local s = "[["', 'ordinary']), [null, null])
})

// ─── search overlay ──────────────────────────────────────────────────────────

test('a match is painted on top of the run it falls in, keeping the kind', () => {
  // Including inside a string or a comment, which is usually where you were
  // looking.
  const runs = withMatches(tokenize('-- find the thing'), 'thing')
  const hit = runs.find((r) => r.match)
  assert.equal(hit.text, 'thing')
  assert.equal(hit.kind, 'comment')
})

test('matching is case-insensitive and keeps the text as written', () => {
  const runs = withMatches(tokenize('local Thing = 1'), 'thing')
  assert.equal(runs.find((r) => r.match).text, 'Thing')
})

test('several matches in one run are each painted', () => {
  const runs = withMatches(tokenize('-- x x x'), 'x')
  assert.equal(runs.filter((r) => r.match).length, 3)
})

test('an empty pattern paints nothing', () => {
  const runs = tokenize('local x = 1')
  assert.deepEqual(withMatches(runs, ''), runs)
})
