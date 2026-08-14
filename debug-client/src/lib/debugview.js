// The Debug Adapter Protocol client, as state.
//
// A port of `DebugView` from `src/bin/tui/dap.rs`. The transport lives in the
// bridge; what is here is the client half of the protocol, and the rules it has
// to respect are unusual enough to be worth restating:
//
// - `stackTrace`, `scopes`, `variables`, `evaluate` and every step request are
//   **rejected outright while the VM is running** — they do not queue. So the
//   client tracks `stopped` itself and never sends them speculatively.
// - `attach` is mandatory. It is what sets `clients = 1` and arms the
//   breakpoint machinery; without it nothing ever stops and nothing says why.
// - `disconnect` clears every breakpoint server-side, so the breakpoint set
//   here is the source of truth and is re-sent on each attach.
// - `auto_continue_secs` can resume the VM without us asking, so an unsolicited
//   `continued` is normal and invalidates every variables handle.

import { expression as inspectExpression, parseRow } from './inspect.js'
import { ancestorsOf, buildRows } from './tree.js'

/// Console lines kept. A logpoint reporting once a combat round fills this in
/// minutes, and only the recent ones are worth anything.
const MAX_OUTPUT_LINES = 500

/// Which execution-control request a key means, if any.
///
/// Every step has a `Ctrl` alias because the function keys are not ours to
/// take — and in a browser that is not a preference, it is the rule: **F11 is
/// full-screen and F12 is developer tools, and neither can be intercepted**.
/// The arrows say what they do: down *into* a call, up *out* of it, right
/// *along* the line.
export function stepCommand(event) {
  const { key, ctrlKey: ctrl, shiftKey: shift } = event
  if (key === 'F5' && !ctrl) return 'continue'
  if (key === 'F10') return 'next'
  if (key === 'F11') return shift ? 'stepOut' : 'stepIn'
  if (ctrl && (key === 'g' || key === 'G')) return 'continue'
  if (ctrl && key === 'ArrowRight') return 'next'
  if (ctrl && key === 'ArrowDown') return 'stepIn'
  if (ctrl && key === 'ArrowUp') return 'stepOut'
  return null
}

export class DebugView {
  /// `send(command, arguments)` puts a DAP request on the wire. `onChange` is
  /// called whenever anything a pane draws has moved.
  constructor({ send, onChange }) {
    this.send = send
    this.onChange = onChange ?? (() => {})

    this.attached = false
    /// Something is stopped: the whole VM, or one dispatch.
    this.stopped = false
    /// **The whole game is held.** Straight off the `stopped` event's
    /// `allThreadsStopped`.
    ///
    /// Kept apart from `stopped` because the server may be built to suspend one
    /// dispatch and keep serving everyone else, and this pane used to draw its
    /// freeze banner over a game that was demonstrably still being played.
    this.worldFrozen = false
    /// The stop the client is looking at, and the one it resumes.
    this.stopId = 1
    this.stopReason = ''
    this.stoppedAt = null
    this.autoContinueSecs = 300

    this.frames = []
    this.frameSel = 0
    this.scopes = []
    this.vars = []
    this.varSel = 0

    /// Client-owned truth: the adapter forgets these on every disconnect.
    /// `Map<relPath, Map<line, message|null>>`. A message makes one a
    /// **logpoint**: it reports and keeps running instead of stopping.
    this.breakpoints = new Map()
    /// The logpoint message being typed, and the line it is for. `null` when
    /// not editing — the editor takes the keyboard while it is open.
    this.logpointEdit = null

    /// `[{rel, abs}]` from the bridge. Both forms, because the tree speaks
    /// `rel` and `setBreakpoints` must send `abs` — the same textual form
    /// `require` produced.
    this.files = []
    this.absByRel = new Map()
    this.relByNorm = new Map()
    this.expanded = new Set()
    this.rows = []
    this.fileSel = 0
    this.open = null
    this.source = []
    this.blocks = []
    this.cursor = 0
    /// A `:` or `/` line editor over the source pane, vi-style.
    this.sourcePrompt = null
    /// The last `/` pattern, for `n`/`N` and for highlighting.
    this.search = ''
    /// Whether matches are painted. `:noh` turns it off without forgetting the
    /// pattern, so `n` still works — the same split vi makes.
    this.highlight = true

    this.replInput = ''
    this.replLog = []
    /// `output` events, newest last, with whether each is a problem.
    ///
    /// Two sources, and they must not look alike: a **logpoint** reporting —
    /// ordinary, expected, possibly once a round — and a breakpoint condition
    /// that raised, which is a mistake.
    this.output = []

    this.focus = 'files'
    this.inspect = { target: 'player', traits: [], effects: [], error: null, selected: 0, pending: false }
    /// Which `variables` response belongs to the Inspect tab rather than to the
    /// variables tree. Held here, not in `inspect`, so `setRunning` clears it
    /// alongside the rest of the handle state a resume invalidates.
    this.inspectRef = null

    /// Set when a file has been asked for and not yet answered, so a second
    /// `stopped` in the same file does not re-request it.
    this.pendingFile = null
  }

  changed() {
    this.onChange(this)
  }

  request(command, args = {}) {
    this.send(command, args)
  }

  // ─── files ─────────────────────────────────────────────────────────────

  /// Take the file list from the bridge. The roots open, everything under them
  /// closed — the same first impression NERDTree gives, and a screen you can
  /// read at a glance.
  setFiles(files) {
    this.files = files
    this.absByRel = new Map(files.map((f) => [f.rel, f.abs]))
    this.relByNorm = new Map(files.map((f) => [normalize(f.abs), f.rel]))
    for (const f of files) {
      const root = f.rel.split('/')[0]
      if (root) this.expanded.add(root)
    }
    this.rebuildRows()
    this.changed()
  }

  rebuildRows() {
    const was = this.rows[this.fileSel]?.path
    this.rows = buildRows(
      this.files.map((f) => f.rel),
      this.expanded
    )
    const at = was ? this.rows.findIndex((r) => r.path === was) : -1
    this.fileSel = Math.min(at === -1 ? this.fileSel : at, Math.max(0, this.rows.length - 1))
  }

  toggleDir(path) {
    if (!this.expanded.delete(path)) this.expanded.add(path)
    this.rebuildRows()
    this.changed()
  }

  /// Open every directory above `path`, so it is on screen.
  reveal(path) {
    for (const dir of ancestorsOf(path)) this.expanded.add(dir)
    this.rebuildRows()
    const at = this.rows.findIndex((r) => r.path === path)
    if (at !== -1) this.fileSel = at
  }

  /// Fold whatever path we were handed onto the one the tree uses.
  ///
  /// The adapter reports frames with an absolute, forward-slashed path — the
  /// form Lua's `require` produced — while the tree, the breakpoint map and
  /// `setBreakpoints` all speak in paths relative to the roots. Storing both
  /// gave the same file two identities: a breakpoint set before a stop and one
  /// set after it landed on different keys, so the gutter dot vanished on the
  /// line you were standing on and the tree kept a mark for a breakpoint you
  /// had just removed.
  knownPath(path) {
    if (this.absByRel.has(path)) return path
    return this.relByNorm.get(normalize(path)) ?? path
  }

  /// Ask the bridge for a file. The adapter has no `source` request, so what
  /// you see is what is on disk — reload after a change like you always would.
  openFile(path, { focus = false } = {}) {
    const rel = this.knownPath(path)
    if (this.open === rel) {
      if (focus) this.focus = 'source'
      return
    }
    this.pendingFile = rel
    this.open = rel
    this.source = []
    this.blocks = []
    this.cursor = 0
    if (focus) this.focus = 'source'
    // Expand the directories above it. A stop opens whatever file it landed in,
    // and a tree that did not show where that was would be worse than the flat
    // list it replaced.
    this.reveal(rel)
    this.changed()
  }

  /// The bridge answered a `file` request.
  setSource(path, lines, error, blockState) {
    if (this.open !== path) return // superseded by a later open
    this.pendingFile = null
    this.source = error ? [`<${error}>`] : lines
    this.blocks = blockState(this.source)
    this.changed()
  }

  // ─── protocol ──────────────────────────────────────────────────────────

  onConnected() {
    this.request('initialize', {
      clientID: 'oxigeon-web',
      clientName: 'oxigeon debug-client',
      adapterID: 'oxigeon-lua',
      linesStartAt1: true,
      columnsStartAt1: true,
      pathFormat: 'path',
    })
  }

  onDisconnected() {
    this.attached = false
    this.setRunning()
    this.changed()
  }

  onMessage(msg) {
    if (msg.type === 'event') this.onEvent(msg)
    else if (msg.type === 'response') this.onResponse(msg)
    this.changed()
  }

  onEvent(msg) {
    switch (msg.event) {
      case 'initialized':
        // Order matters: attach arms the hook, breakpoints are only honoured
        // once armed, and configurationDone releases it.
        this.request('attach', {})
        this.attached = true
        this.sendAllBreakpoints()
        this.request('setExceptionBreakpoints', { filters: [] })
        this.request('configurationDone', {})
        break

      case 'stopped': {
        const body = msg.body ?? {}
        this.stopped = true
        // Absent means the old, always-frozen behaviour.
        this.worldFrozen = body.allThreadsStopped ?? true
        this.stopId = body.threadId ?? 1
        this.stoppedAt = Date.now()
        this.stopReason = body.reason ?? 'stopped'
        this.request('stackTrace', { threadId: this.stopId, levels: 64 })
        break
      }

      // Only clear the view if *this* stop is the one that continued; another
      // dispatch may still be suspended behind it.
      case 'continued':
        if ((msg.body?.threadId ?? this.stopId) === this.stopId) this.setRunning()
        break

      case 'output': {
        const text = msg.body?.output
        if (typeof text !== 'string') break
        const important = msg.body?.category === 'important'
        // A logpoint on a hot line can produce a lot of these, and holding
        // every one for the life of the session is a leak with a scrollback
        // nobody reads.
        if (this.output.length >= MAX_OUTPUT_LINES) {
          this.output.splice(0, this.output.length - MAX_OUTPUT_LINES + 1)
        }
        this.output.push({ important, text: text.replace(/\s+$/, '') })
        break
      }
    }
  }

  onResponse(msg) {
    const command = msg.__request?.command ?? msg.command ?? ''
    const args = msg.__request?.arguments ?? {}

    if (msg.success === false) {
      const why = msg.message ?? 'failed'
      if (command === 'evaluate') {
        if (this.inspect.pending) {
          this.inspect.pending = false
          this.inspect.error = why
        }
        this.answerRepl(`! ${why}`)
      } else if (why.includes('not stopped')) {
        // "not stopped" against anything else means the VM resumed underneath
        // a request we had already sent. Fold the state back.
        this.setRunning()
      } else {
        this.output.push({ important: true, text: `${command}: ${why}` })
      }
      return
    }

    switch (command) {
      case 'stackTrace':
        this.frames = (msg.body?.stackFrames ?? []).map((f) => ({
          id: f.id ?? 0,
          name: f.name ?? '?',
          path: f.source?.path ?? null,
          line: f.line ?? 0,
        }))
        this.frameSel = 0
        this.followFrame()
        break

      case 'scopes':
        this.scopes = (msg.body?.scopes ?? []).map((s) => ({
          name: s.name ?? '?',
          varRef: s.variablesReference ?? 0,
          expensive: s.expensive ?? false,
        }))
        // One collapsed header per scope, so Locals and Upvalues stay visibly
        // apart and the expensive Globals scope is reachable without being
        // walked on every stop — expanding it otherwise reaches the entire
        // daemon graph.
        this.vars = this.scopes.map((s) => ({
          name: s.name,
          value: s.expensive ? '(expensive)' : '',
          ty: '',
          varRef: s.varRef,
          depth: 0,
          expanded: false,
        }))
        this.varSel = 0
        for (const scope of this.scopes) {
          if (!scope.expensive) this.request('variables', { variablesReference: scope.varRef })
        }
        break

      case 'variables': {
        const asked = args.variablesReference ?? 0
        const rows = parseVariables(msg.body?.variables)
        if (this.inspect.pending && this.inspectRef === asked) this.absorbInspect(rows)
        else this.insertChildren(asked, rows)
        break
      }

      case 'evaluate': {
        const body = msg.body ?? {}
        const result = body.result ?? ''
        const varRef = body.variablesReference ?? 0
        if (this.inspect.pending) {
          // The payload is a table; its rows come back through one `variables`
          // call, because a single string would be cut off at introspect.lua's
          // 256-character value limit.
          if (varRef > 0) {
            this.inspectRef = varRef
            this.request('variables', { variablesReference: varRef })
          } else {
            this.inspect.pending = false
            this.inspect.error = `expected a table, got: ${result}`
          }
        } else {
          this.answerRepl(result)
        }
        break
      }
    }
  }

  setRunning() {
    this.stopped = false
    this.worldFrozen = false
    this.stoppedAt = null
    this.frames = []
    this.scopes = []
    // Every variablesReference is invalidated by a resume — introspect.lua
    // resets its handle table. Reusing one would read an unrelated value.
    this.vars = []
    this.varSel = 0
    this.inspectRef = null
    this.inspect.pending = false
  }

  /// Seconds left before the adapter resumes the VM on its own. `null` when not
  /// stopped, or when `autoContinueSecs = 0` disables the safety valve.
  autoContinueIn() {
    if (this.autoContinueSecs === 0 || this.stoppedAt === null) return null
    const gone = Math.floor((Date.now() - this.stoppedAt) / 1000)
    return Math.max(0, this.autoContinueSecs - gone)
  }

  answerRepl(text) {
    const last = this.replLog[this.replLog.length - 1]
    if (last) last.answer = text
  }

  followFrame() {
    const frame = this.frames[this.frameSel]
    if (!frame) return
    if (frame.path) {
      this.openFile(frame.path)
      this.cursor = Math.max(0, frame.line - 1)
    }
    this.request('scopes', { frameId: frame.id })
  }

  insertChildren(parentRef, rows) {
    // Every node the response can belong to is already in the list — the scope
    // headers are seeded when `scopes` arrives — so a reference that matches
    // nothing is a stale handle from before a resume. Drop it.
    const i = this.vars.findIndex((v) => v.varRef === parentRef)
    if (i === -1) return
    const depth = this.vars[i].depth + 1
    this.vars[i].expanded = true
    this.vars.splice(i + 1, 0, ...rows.map((r) => ({ ...r, depth })))
  }

  collapse(index) {
    const depth = this.vars[index].depth
    this.vars[index].expanded = false
    let end = index + 1
    while (end < this.vars.length && this.vars[end].depth > depth) end++
    this.vars.splice(index + 1, end - index - 1)
  }

  // ─── breakpoints ───────────────────────────────────────────────────────

  linesFor(rel) {
    return this.breakpoints.get(rel) ?? new Map()
  }

  /// Whether anything is set at or under `path`. Directories included, so a
  /// collapsed folder still shows that it holds one.
  ///
  /// `breakpoints` keeps an empty map behind when the last line is removed, so
  /// "is this path a key" is not the same question as "does this file have
  /// anything set". Everything that draws a marker goes through here, so the
  /// tree and the gutter cannot drift apart.
  markedUnder(path, isDir) {
    for (const [p, lines] of this.breakpoints) {
      if (lines.size === 0) continue
      if (isDir ? p.startsWith(`${path}/`) : p === path) return true
    }
    return false
  }

  sendBreakpoints(rel) {
    const lines = this.linesFor(rel)
    // Absolute and forward-slashed, the same textual form `require` produced —
    // `paths::normalize` on the far side folds the rest. Sending the relative
    // path instead is answered `verified: true` and then never stops.
    const abs = this.absByRel.get(rel) ?? rel
    const breakpoints = [...lines.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([line, message]) =>
        // `logMessage` is the protocol's own field for this, so the adapter
        // needs nothing bespoke and VS Code sets the same thing.
        message === null ? { line } : { line, logMessage: message }
      )
    this.request('setBreakpoints', { source: { path: abs }, breakpoints })
  }

  sendAllBreakpoints() {
    for (const [rel, lines] of this.breakpoints) if (lines.size) this.sendBreakpoints(rel)
  }

  toggleBreakpoint(line = this.cursor + 1) {
    if (!this.open) return
    const lines = this.breakpoints.get(this.open) ?? new Map()
    this.breakpoints.set(this.open, lines)
    if (!lines.delete(line)) lines.set(line, null)
    // Only transmit once the handshake has run. A `setBreakpoints` queued
    // before `initialize` would reach the adapter first and be answered against
    // a session that does not exist yet — and the local set is re-sent on
    // `initialized` regardless, so nothing is lost by waiting.
    if (this.attached) this.sendBreakpoints(this.open)
    this.changed()
  }

  /// Open the logpoint editor on a line.
  ///
  /// Pre-filled with the message already there, so opening it twice is "edit"
  /// rather than "start again".
  beginLogpoint(line = this.cursor + 1) {
    if (!this.open) return
    this.logpointEdit = { line, text: this.linesFor(this.open).get(line) ?? '' }
    this.changed()
  }

  /// Commit the logpoint editor. **An empty message removes the logpoint
  /// entirely** — which is the only way to un-set one without clearing the
  /// breakpoint and starting over.
  commitLogpoint() {
    const edit = this.logpointEdit
    this.logpointEdit = null
    if (!edit || !this.open) return this.changed()
    const lines = this.breakpoints.get(this.open) ?? new Map()
    this.breakpoints.set(this.open, lines)
    if (edit.text.trim() === '') lines.delete(edit.line)
    else lines.set(edit.line, edit.text)
    if (this.attached) this.sendBreakpoints(this.open)
    this.changed()
  }

  cancelLogpoint() {
    this.logpointEdit = null
    this.changed()
  }

  // ─── source motions ────────────────────────────────────────────────────

  /// Move to the next (or previous) line matching `search`, wrapping.
  ///
  /// Case-insensitive, because a search you have to get the case right for is
  /// one you type twice. Wrapping, because stopping at the end of the file
  /// looks like "no more matches" when there are several above you.
  find(forward) {
    if (this.search === '' || this.source.length === 0) return
    const needle = this.search.toLowerCase()
    const n = this.source.length
    for (let step = 1; step <= n; step++) {
      const i = forward ? (this.cursor + step) % n : (this.cursor + n - (step % n)) % n
      if (this.source[i].toLowerCase().includes(needle)) {
        this.cursor = i
        this.changed()
        return
      }
    }
  }

  commitSourcePrompt() {
    const prompt = this.sourcePrompt
    this.sourcePrompt = null
    if (!prompt) return this.changed()
    if (prompt.kind === 'goto') {
      const t = prompt.text.trim()
      // `:noh` — vi's own spelling for "stop highlighting that". Clearing the
      // pattern would also lose what `n` repeats, so only the highlight goes.
      if (['noh', 'nohl', 'nohls', 'nohlsearch'].includes(t)) {
        this.highlight = false
      } else {
        const n = Number.parseInt(t, 10)
        if (Number.isFinite(n)) {
          this.cursor = Math.min(Math.max(0, n - 1), Math.max(0, this.source.length - 1))
        }
      }
    } else {
      // An empty pattern repeats the last one, as `//` does in vi — which makes
      // `//` the shortest "next match" there is, without having to retype.
      if (prompt.text !== '') this.search = prompt.text
      this.highlight = true
      // Starts from the line *after* the cursor, so pressing `/` on a term you
      // are already sitting on advances.
      this.find(true)
    }
    this.changed()
  }

  // ─── REPL ──────────────────────────────────────────────────────────────

  submitRepl() {
    const expr = this.replInput
    if (expr === '') return
    this.replInput = ''
    if (!this.stopped) {
      this.replLog.push({ expr, answer: '! evaluate needs a paused frame' })
      return this.changed()
    }
    const frameId = this.frames[this.frameSel]?.id ?? 0
    this.replLog.push({ expr, answer: '…' })
    this.request('evaluate', { frameId, expression: expr, context: 'repl' })
    this.changed()
  }

  // ─── Inspect tab ───────────────────────────────────────────────────────

  requestInspect() {
    if (!this.stopped) {
      this.inspect.error = 'attach and break somewhere — evaluate needs a paused frame'
      return this.changed()
    }
    const frameId = this.frames[this.frameSel]?.id ?? 0
    this.inspect.pending = true
    this.inspect.error = null
    this.request('evaluate', {
      frameId,
      expression: inspectExpression(this.inspect.target),
      context: 'repl',
    })
    this.changed()
  }

  absorbInspect(rows) {
    this.inspect.pending = false
    this.inspectRef = null
    this.inspect.traits = []
    this.inspect.effects = []
    for (const row of rows) {
      const parsed = parseRow(row.value)
      if (parsed?.row === 'trait') this.inspect.traits.push(parsed)
      else if (parsed?.row === 'effect') this.inspect.effects.push(parsed)
    }
    if (this.inspect.traits.length === 0 && this.inspect.effects.length === 0) {
      this.inspect.error = `\`${this.inspect.target}\` resolved to nothing with traits in this frame`
    }
    this.inspect.selected = 0
  }
}

function parseVariables(value) {
  if (!Array.isArray(value)) return []
  return value.map((v) => ({
    name: v.name ?? '?',
    value: v.value ?? '',
    ty: v.type ?? '',
    varRef: v.variablesReference ?? 0,
    depth: 0,
    expanded: false,
  }))
}

/// Fold a path into a comparable key, as `paths::normalize` does. Lowercased
/// unconditionally: the browser cannot know the server's platform, and the only
/// cost of folding case on a case-sensitive filesystem is that two files
/// differing only in case would collide — which no Lua tree has.
function normalize(raw) {
  let s = String(raw).replaceAll('\\', '/')
  if (s.startsWith('//?/')) s = s.slice(4)
  return s.toLowerCase()
}
