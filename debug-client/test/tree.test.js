// The file tree, over a fixed path list.
//
// Fixed rather than discovered, exactly as `fixture_files()` in
// `src/bin/tui/dap.rs` is: what these assert is the tree-building code, not the
// contents of somebody's checkout — and `mudlib/` and `game/` are gitignored,
// so a discovered list is empty on a fresh clone and asserts nothing at all.

import assert from 'node:assert/strict'
import { test } from 'node:test'

import { ancestorsOf, buildRows, label } from '../src/lib/tree.js'

const FILES = [
  'mudlib/cmds/who.lua',
  'mudlib/cmds/look.lua',
  'mudlib/cmds/say.lua',
  'mudlib/daemons/ticker_d.lua',
  'mudlib/daemons/room_d.lua',
  'mudlib/lib/strings.lua',
  'mudlib/lib/color.lua',
  'game/areas/thornhollow/rooms.lua',
  'game/areas/thornhollow/mobs.lua',
  'game/init.lua',
].sort()

test('the tree starts at the roots, with fewer rows than there are files', () => {
  const rows = buildRows(FILES, new Set(['mudlib', 'game']))
  // The two roots and their immediate children, nothing deeper.
  assert.deepEqual(
    rows.map((r) => r.path),
    [
      'game',
      'game/areas',
      'game/init.lua',
      'mudlib',
      'mudlib/cmds',
      'mudlib/daemons',
      'mudlib/lib',
    ]
  )
  assert.ok(rows.length < FILES.length)
})

test('a collapsed root hides everything under it', () => {
  const rows = buildRows(FILES, new Set(['mudlib']))
  assert.ok(rows.some((r) => r.path === 'mudlib/cmds'))
  assert.ok(!rows.some((r) => r.path.startsWith('game/areas')))
  const game = rows.find((r) => r.path === 'game')
  assert.equal(game.isDir, true)
  assert.equal(game.expanded, false)
})

test('expanding a directory shows its files at the right depth', () => {
  const rows = buildRows(FILES, new Set(['mudlib', 'mudlib/cmds']))
  const cmds = rows.filter((r) => r.path.startsWith('mudlib/cmds/'))
  assert.deepEqual(
    cmds.map((r) => label(r.path)),
    ['look.lua', 'say.lua', 'who.lua']
  )
  assert.ok(cmds.every((r) => r.depth === 2 && !r.isDir))
})

test('a directory is emitted once however many files it holds', () => {
  const rows = buildRows(FILES, new Set(['mudlib']))
  assert.equal(rows.filter((r) => r.path === 'mudlib/cmds').length, 1)
})

test('a deep path needs every ancestor expanded, not just its parent', () => {
  // `game/areas/thornhollow` open but `game/areas` shut shows nothing.
  const rows = buildRows(FILES, new Set(['game', 'game/areas/thornhollow']))
  assert.ok(!rows.some((r) => r.path.includes('thornhollow/')))
  const all = buildRows(FILES, new Set(['game', 'game/areas', 'game/areas/thornhollow']))
  assert.ok(all.some((r) => r.path === 'game/areas/thornhollow/rooms.lua'))
})

test('ancestorsOf names every directory above a file and not the file', () => {
  assert.deepEqual(ancestorsOf('game/areas/thornhollow/rooms.lua'), [
    'game',
    'game/areas',
    'game/areas/thornhollow',
  ])
  assert.deepEqual(ancestorsOf('game'), [])
})
