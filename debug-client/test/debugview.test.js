// The client half of the protocol.
//
// These assert the rules stated at the top of `src/bin/tui/dap.rs` — the ones
// that are not obvious from the DAP specification and that this client gets
// wrong silently if it gets them wrong at all.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { DebugView, stepCommand } from '../src/lib/debugview.js'

const FILES = [
  { rel: 'mudlib/cmds/who.lua', abs: 'C:/Code/oxigeon/mudlib/cmds/who.lua' },
  { rel: 'game/init.lua', abs: 'C:/Code/oxigeon/game/init.lua' },
]

/// A view with the wire captured rather than sent.
function view() {
  const sent = []
  const dbg = new DebugView({ send: (command, args) => sent.push({ command, args }) })
  dbg.setFiles(FILES)
  return { dbg, sent, last: (c) => [...sent].reverse().find((s) => s.command === c) }
}

/// Take a view all the way to attached, as a real session does.
function attached() {
  const v = view()
  v.dbg.onConnected()
  v.dbg.onMessage({ type: 'event', event: 'initialized' })
  return v
}

function stop(dbg, { line = 19, path = FILES[0].abs, allThreadsStopped = true } = {}) {
  dbg.onMessage({
    type: 'event',
    event: 'stopped',
    body: { reason: 'breakpoint', threadId: 1, allThreadsStopped },
  })
  dbg.onMessage({
    type: 'response',
    command: 'stackTrace',
    success: true,
    body: { stackFrames: [{ id: 1000, name: 'cmd_who', line, source: { path } }] },
  })
}

// ─── handshake ───────────────────────────────────────────────────────────────

test('initialize comes first, and attach only on initialized', () => {
  const { dbg, sent } = view()
  dbg.onConnected()
  assert.deepEqual(sent.map((s) => s.command), ['initialize'])

  dbg.onMessage({ type: 'event', event: 'initialized' })
  // Order matters: attach arms the hook, breakpoints are only honoured once
  // armed, and configurationDone releases it.
  assert.deepEqual(sent.map((s) => s.command), [
    'initialize',
    'attach',
    'setExceptionBreakpoints',
    'configurationDone',
  ])
  assert.equal(dbg.attached, true)
})

test('breakpoints set before the handshake are held, then re-sent on attach', () => {
  // A setBreakpoints queued before initialize would reach the adapter first and
  // be answered against a session that does not exist yet.
  const { dbg, sent, last } = view()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.toggleBreakpoint(19)
  assert.equal(sent.filter((s) => s.command === 'setBreakpoints').length, 0)

  dbg.onConnected()
  dbg.onMessage({ type: 'event', event: 'initialized' })
  const bp = last('setBreakpoints')
  assert.deepEqual(bp.args.breakpoints, [{ line: 19 }])
  // And it lands before configurationDone releases the hook.
  const order = sent.map((s) => s.command)
  assert.ok(order.indexOf('setBreakpoints') < order.indexOf('configurationDone'))
})

// ─── paths ───────────────────────────────────────────────────────────────────

test('setBreakpoints sends the absolute path require produced, not the tree path', () => {
  // A path that does not match the `@`-chunk name is still answered
  // `verified: true`, and then never stops.
  const { dbg, last } = attached()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.toggleBreakpoint(19)
  assert.equal(last('setBreakpoints').args.source.path, FILES[0].abs)
})

test('a frame path and a tree path are one identity, however they are spelled', () => {
  // Storing both gave the same file two identities: a breakpoint set before a
  // stop and one set after it landed on different keys, so the gutter dot
  // vanished on the line you were standing on.
  const { dbg } = attached()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.toggleBreakpoint(19)

  stop(dbg, { path: 'c:\\Code\\oxigeon\\mudlib\\cmds\\who.lua' })
  assert.equal(dbg.open, 'mudlib/cmds/who.lua', 'the backslashed drive-cased path folded')
  assert.equal(dbg.linesFor('mudlib/cmds/who.lua').size, 1)
  assert.equal(dbg.markedUnder('mudlib/cmds/who.lua', false), true)
})

test('a folder shows that something under it is marked, so collapsing hides nothing', () => {
  const { dbg } = attached()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.toggleBreakpoint(19)
  assert.equal(dbg.markedUnder('mudlib', true), true)
  assert.equal(dbg.markedUnder('game', true), false)
})

test('removing the last breakpoint unmarks the file, empty map or not', () => {
  const { dbg } = attached()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.toggleBreakpoint(19)
  dbg.toggleBreakpoint(19)
  assert.equal(dbg.markedUnder('mudlib/cmds/who.lua', false), false)
  assert.equal(dbg.markedUnder('mudlib', true), false)
})

// ─── logpoints ───────────────────────────────────────────────────────────────

test('a logpoint travels as logMessage, the protocol field VS Code also sets', () => {
  const { dbg, last } = attached()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.beginLogpoint(42)
  dbg.logpointEdit.text = 'hp={player.hp}'
  dbg.commitLogpoint()
  assert.deepEqual(last('setBreakpoints').args.breakpoints, [
    { line: 42, logMessage: 'hp={player.hp}' },
  ])
})

test('re-opening the editor pre-fills, so opening it twice is an edit', () => {
  const { dbg } = attached()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.beginLogpoint(42)
  dbg.logpointEdit.text = 'first'
  dbg.commitLogpoint()
  dbg.beginLogpoint(42)
  assert.equal(dbg.logpointEdit.text, 'first')
})

test('an empty message removes the logpoint', () => {
  // The only way to un-set one without clearing the breakpoint and starting
  // over.
  const { dbg, last } = attached()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.beginLogpoint(42)
  dbg.logpointEdit.text = 'x'
  dbg.commitLogpoint()
  dbg.beginLogpoint(42)
  dbg.logpointEdit.text = '   '
  dbg.commitLogpoint()
  assert.deepEqual(last('setBreakpoints').args.breakpoints, [])
  assert.equal(dbg.markedUnder('mudlib/cmds/who.lua', false), false)
})

test('abandoning the editor changes nothing', () => {
  const { dbg } = attached()
  dbg.openFile('mudlib/cmds/who.lua')
  dbg.beginLogpoint(42)
  dbg.logpointEdit.text = 'never committed'
  dbg.cancelLogpoint()
  assert.equal(dbg.linesFor('mudlib/cmds/who.lua').size, 0)
})

// ─── stopping and resuming ───────────────────────────────────────────────────

test('a stop asks for the stack and follows the frame into the file', () => {
  const { dbg, last } = attached()
  stop(dbg, { line: 19 })
  assert.equal(dbg.stopped, true)
  assert.equal(dbg.open, 'mudlib/cmds/who.lua')
  assert.equal(dbg.cursor, 18, '0-based cursor on the 1-based line')
  assert.equal(last('scopes').args.frameId, 1000)
})

test('allThreadsStopped is what says the world is frozen, and absent means it is', () => {
  // Kept apart from `stopped` because a lua55 build can suspend one dispatch —
  // and this pane used to draw its freeze banner over a game that was
  // demonstrably still being played.
  const a = attached()
  stop(a.dbg, { allThreadsStopped: false })
  assert.equal(a.dbg.stopped, true)
  assert.equal(a.dbg.worldFrozen, false)

  const b = attached()
  b.dbg.onMessage({ type: 'event', event: 'stopped', body: { reason: 'pause', threadId: 1 } })
  assert.equal(b.dbg.worldFrozen, true, 'absent means the old, always-frozen behaviour')
})

test('the stop id is taken from the event, not assumed to be 1', () => {
  const { dbg, last } = attached()
  dbg.onMessage({
    type: 'event',
    event: 'stopped',
    body: { reason: 'breakpoint', threadId: 7 },
  })
  assert.equal(dbg.stopId, 7)
  assert.equal(last('stackTrace').args.threadId, 7)
})

test('a continued event for another thread leaves this stop alone', () => {
  // Another dispatch may still be suspended behind it.
  const { dbg } = attached()
  stop(dbg)
  dbg.onMessage({ type: 'event', event: 'continued', body: { threadId: 99 } })
  assert.equal(dbg.stopped, true)
  dbg.onMessage({ type: 'event', event: 'continued', body: { threadId: 1 } })
  assert.equal(dbg.stopped, false)
})

test('a resume invalidates every variables handle', () => {
  // introspect.lua resets its handle table; reusing a reference would read an
  // unrelated value.
  const { dbg } = attached()
  stop(dbg)
  dbg.onMessage({
    type: 'response',
    command: 'scopes',
    success: true,
    body: { scopes: [{ name: 'Locals', variablesReference: 5 }] },
  })
  assert.equal(dbg.vars.length, 1)
  dbg.onMessage({ type: 'event', event: 'continued', body: { threadId: 1 } })
  assert.deepEqual(dbg.vars, [])
  assert.deepEqual(dbg.frames, [])
})

test('"not stopped" against a request in flight folds the state back', () => {
  // auto_continue_secs can resume the VM without us asking.
  const { dbg } = attached()
  stop(dbg)
  dbg.onMessage({
    type: 'response',
    command: 'variables',
    success: false,
    message: 'not stopped',
  })
  assert.equal(dbg.stopped, false)
})

// ─── variables ───────────────────────────────────────────────────────────────

test('the expensive scope is seeded as a header and never walked unasked', () => {
  // Expanding Globals reaches the entire daemon graph.
  const { dbg, sent } = attached()
  stop(dbg)
  dbg.onMessage({
    type: 'response',
    command: 'scopes',
    success: true,
    body: {
      scopes: [
        { name: 'Locals', variablesReference: 5 },
        { name: 'Globals', variablesReference: 6, expensive: true },
      ],
    },
  })
  assert.deepEqual(dbg.vars.map((v) => v.name), ['Locals', 'Globals'])
  const asked = sent.filter((s) => s.command === 'variables').map((s) => s.args.variablesReference)
  assert.deepEqual(asked, [5])
  assert.equal(dbg.vars[1].value, '(expensive)')
})

test('children land under the node that asked for them, at one more depth', () => {
  const { dbg } = attached()
  stop(dbg)
  dbg.onMessage({
    type: 'response',
    command: 'scopes',
    success: true,
    body: { scopes: [{ name: 'Locals', variablesReference: 5 }] },
  })
  dbg.onMessage({
    type: 'response',
    command: 'variables',
    success: true,
    __request: { command: 'variables', arguments: { variablesReference: 5 } },
    body: { variables: [{ name: 'player', value: 'table', variablesReference: 9 }] },
  })
  assert.deepEqual(dbg.vars.map((v) => [v.name, v.depth]), [
    ['Locals', 0],
    ['player', 1],
  ])
  assert.equal(dbg.vars[0].expanded, true)
})

test('a variables response for a stale handle is dropped, not inserted', () => {
  const { dbg } = attached()
  stop(dbg)
  dbg.onMessage({
    type: 'response',
    command: 'variables',
    success: true,
    __request: { command: 'variables', arguments: { variablesReference: 404 } },
    body: { variables: [{ name: 'ghost', value: '1' }] },
  })
  assert.deepEqual(dbg.vars, [])
})

test('collapsing removes the whole subtree, not just the first child', () => {
  const { dbg } = attached()
  stop(dbg)
  dbg.vars = [
    { name: 'Locals', depth: 0, varRef: 5, expanded: true },
    { name: 'player', depth: 1, varRef: 9, expanded: true },
    { name: 'hp', depth: 2, varRef: 0 },
    { name: 'other', depth: 1, varRef: 0 },
  ]
  dbg.collapse(1)
  assert.deepEqual(dbg.vars.map((v) => v.name), ['Locals', 'player', 'other'])
})

// ─── evaluate ────────────────────────────────────────────────────────────────

test('the REPL refuses without a paused frame rather than sending', () => {
  // `evaluate` is rejected outright while the VM is running; it does not queue.
  const { dbg, sent } = attached()
  dbg.replInput = 'player.hp'
  dbg.submitRepl()
  assert.equal(sent.filter((s) => s.command === 'evaluate').length, 0)
  assert.match(dbg.replLog.at(-1).answer, /needs a paused frame/)
})

test('the REPL evaluates in the selected frame', () => {
  const { dbg, last } = attached()
  stop(dbg)
  dbg.replInput = 'player.hp'
  dbg.submitRepl()
  assert.equal(last('evaluate').args.frameId, 1000)
  dbg.onMessage({
    type: 'response',
    command: 'evaluate',
    success: true,
    __request: { command: 'evaluate', arguments: {} },
    body: { result: '42' },
  })
  assert.deepEqual(dbg.replLog.at(-1), { expr: 'player.hp', answer: '42' })
})

test('inspect follows evaluate into one variables call', () => {
  // The payload is a table; a single string would be cut off at
  // introspect.lua's 256-character value limit.
  const { dbg, last } = attached()
  stop(dbg)
  dbg.requestInspect()
  assert.ok(last('evaluate').args.expression.includes('DAEMON.trait.all'))

  dbg.onMessage({
    type: 'response',
    command: 'evaluate',
    success: true,
    __request: { command: 'evaluate', arguments: {} },
    body: { result: 'table', variablesReference: 77 },
  })
  assert.equal(last('variables').args.variablesReference, 77)

  const SEP = '\u001f'
  dbg.onMessage({
    type: 'response',
    command: 'variables',
    success: true,
    __request: { command: 'variables', arguments: { variablesReference: 77 } },
    body: {
      variables: [
        { name: '1', value: ['T', 'max_hp', 'Health', 'derived', 'core', '0', '42', '', 'false'].join(SEP) },
        { name: '2', value: ['E', 'blessed', 'Blessed', '1', ''].join(SEP) },
      ],
    },
  })
  assert.equal(dbg.inspect.traits.length, 1)
  assert.equal(dbg.inspect.traits[0].value, '42')
  assert.equal(dbg.inspect.effects.length, 1)
  assert.equal(dbg.inspect.pending, false)
})

test('an inspect payload that is not a table says so rather than hanging', () => {
  const { dbg } = attached()
  stop(dbg)
  dbg.requestInspect()
  dbg.onMessage({
    type: 'response',
    command: 'evaluate',
    success: true,
    __request: { command: 'evaluate', arguments: {} },
    body: { result: 'nil', variablesReference: 0 },
  })
  assert.equal(dbg.inspect.pending, false)
  assert.match(dbg.inspect.error, /expected a table/)
})

test('an inspect variables response is not mistaken for the variables tree', () => {
  const { dbg } = attached()
  stop(dbg)
  dbg.onMessage({
    type: 'response',
    command: 'scopes',
    success: true,
    body: { scopes: [{ name: 'Locals', variablesReference: 5 }] },
  })
  dbg.requestInspect()
  dbg.onMessage({
    type: 'response',
    command: 'evaluate',
    success: true,
    __request: { command: 'evaluate', arguments: {} },
    body: { result: 'table', variablesReference: 77 },
  })
  dbg.onMessage({
    type: 'response',
    command: 'variables',
    success: true,
    __request: { command: 'variables', arguments: { variablesReference: 77 } },
    body: { variables: [{ name: '1', value: ['T', 'a', 'b', 'c', 'd', '1', '2', '', 'false'].join('\u001f') }] },
  })
  assert.deepEqual(dbg.vars.map((v) => v.name), ['Locals'], 'the tree did not absorb the payload')
  assert.equal(dbg.inspect.traits.length, 1)
})

// ─── console ─────────────────────────────────────────────────────────────────

test('a logpoint reporting and a condition that raised are told apart', () => {
  // When conditions were the only source every line was drawn as a warning, so
  // the first working logpoint looked like it had gone wrong.
  const { dbg } = attached()
  dbg.onMessage({ type: 'event', event: 'output', body: { output: 'hp=42\n' } })
  dbg.onMessage({
    type: 'event',
    event: 'output',
    body: { output: 'condition failed', category: 'important' },
  })
  assert.deepEqual(dbg.output, [
    { important: false, text: 'hp=42' },
    { important: true, text: 'condition failed' },
  ])
})

test('the console is capped, because a logpoint on a hot line never stops', () => {
  const { dbg } = attached()
  for (let i = 0; i < 600; i++) {
    dbg.onMessage({ type: 'event', event: 'output', body: { output: `line ${i}` } })
  }
  assert.ok(dbg.output.length <= 500)
  assert.equal(dbg.output.at(-1).text, 'line 599')
})

// ─── search ──────────────────────────────────────────────────────────────────

test('search wraps, is case-insensitive, and starts after the cursor', () => {
  const { dbg } = attached()
  dbg.source = ['Alpha', 'beta', 'ALPHA', 'gamma']
  dbg.cursor = 0
  dbg.search = 'alpha'
  dbg.find(true)
  assert.equal(dbg.cursor, 2, 'advanced past the line it was sitting on')
  dbg.find(true)
  assert.equal(dbg.cursor, 0, 'wrapped')
  dbg.find(false)
  assert.equal(dbg.cursor, 2, 'backwards wraps too')
})

test(':noh stops highlighting and keeps the pattern for n', () => {
  const { dbg } = attached()
  dbg.source = ['a', 'target', 'b']
  dbg.search = 'target'
  dbg.sourcePrompt = { kind: 'goto', text: 'noh' }
  dbg.commitSourcePrompt()
  assert.equal(dbg.highlight, false)
  assert.equal(dbg.search, 'target')
  dbg.cursor = 0
  dbg.find(true)
  assert.equal(dbg.cursor, 1)
})

test('an empty search pattern repeats the last one, as // does in vi', () => {
  const { dbg } = attached()
  dbg.source = ['a', 'hit', 'b', 'hit']
  dbg.search = 'hit'
  dbg.cursor = 1
  dbg.sourcePrompt = { kind: 'search', text: '' }
  dbg.commitSourcePrompt()
  assert.equal(dbg.search, 'hit')
  assert.equal(dbg.cursor, 3)
})

test(': goes to a 1-based line and clamps to the file', () => {
  const { dbg } = attached()
  dbg.source = ['a', 'b', 'c']
  dbg.sourcePrompt = { kind: 'goto', text: '2' }
  dbg.commitSourcePrompt()
  assert.equal(dbg.cursor, 1)
  dbg.sourcePrompt = { kind: 'goto', text: '900' }
  dbg.commitSourcePrompt()
  assert.equal(dbg.cursor, 2)
})

// ─── keys ────────────────────────────────────────────────────────────────────

test('every step has a Ctrl alias, because the browser owns the function keys', () => {
  // F11 is full-screen and F12 is developer tools; neither can be intercepted.
  const key = (k, mods = {}) => stepCommand({ key: k, ctrlKey: false, shiftKey: false, ...mods })
  assert.equal(key('F5'), 'continue')
  assert.equal(key('g', { ctrlKey: true }), 'continue')
  assert.equal(key('F10'), 'next')
  assert.equal(key('ArrowRight', { ctrlKey: true }), 'next')
  assert.equal(key('F11'), 'stepIn')
  assert.equal(key('ArrowDown', { ctrlKey: true }), 'stepIn')
  assert.equal(key('F11', { shiftKey: true }), 'stepOut')
  assert.equal(key('ArrowUp', { ctrlKey: true }), 'stepOut')
  assert.equal(key('x'), null)
})

// ─── auto-continue ───────────────────────────────────────────────────────────

test('the countdown runs from the stop and is absent when disabled', () => {
  const { dbg } = attached()
  assert.equal(dbg.autoContinueIn(), null, 'not stopped')
  stop(dbg)
  const left = dbg.autoContinueIn()
  assert.ok(left > 290 && left <= 300)

  dbg.autoContinueSecs = 0
  assert.equal(dbg.autoContinueIn(), null, '0 disables the safety valve')
})
