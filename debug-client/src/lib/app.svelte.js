// Application state, and the wiring between the two sockets and the four tabs.
//
// There are two, and they are different kinds of thing:
//
//   - the **game**, straight to the driver's own WebSocket listener. Its client
//     is `client/src/lib/connection.js` — the repo's reference implementation
//     of the envelope, imported rather than copied, because two clients in one
//     repo with their own idea of the wire format is how they drift.
//   - the **bridge**, for the debugger, the file tree and the journal: the
//     three things a browser still cannot reach.
//
// `DebugView` is a plain class rather than a rune-annotated one: it holds Maps
// and Sets and mutates deeply, which is what deep reactivity is worst at, and
// `tests/` drives it under bare `node --test`, where runes do not exist. It
// reports through `onChange`, which bumps `dbgVersion`; the `dbg` getter is
// what turns that into a dependency a pane can hold. See the getter.

import { Connection } from '../../../client/src/lib/connection.js'

import { Bridge } from './bridge.js'
import { DebugView } from './debugview.js'
import { blockState } from './lua.js'
import { appendBreak } from './scrollback.js'
import { linesOf } from './spans.js'

/// How many rendered lines of game text to keep. The mudlib pages long output
/// itself, so this only has to cover scrollback a human would actually walk.
const SCROLLBACK = 5000

/// Journal lines kept for the bottom strip.
const JOURNAL_LINES = 2000

export const TABS = ['Play', 'Debug', 'Inspect', 'Trace']

export class App {
  tab = $state('Play')

  // ─── Play ──────────────────────────────────────────────────────────────
  scrollback = $state([])
  prompt = $state(null)
  input = $state('')
  /// True while the server is doing the echoing — a password is being asked
  /// for. Named for the player-visible effect, as the frame is.
  masked = $state(false)
  history = []
  historyPos = null
  vitals = $state({ hp: null, maxhp: null, mp: null, maxmp: null, level: null, xp: null, gold: null })
  room = $state({ id: '', name: '', area: '', exits: [] })
  effects = $state([])
  game = $state({ state: 'connecting', why: '' })

  // ─── Debug ─────────────────────────────────────────────────────────────
  dap = $state({ state: 'connecting', why: '' })
  /// Bumped by `DebugView.onChange`. Read through the `dbg` getter below,
  /// never directly by a pane.
  dbgVersion = $state(0)

  /// The debugger, as something a template can depend on.
  ///
  /// `DebugView` is a plain class — it has to be, because `tests/` drives it
  /// under bare `node --test`, and runes are a compile step. So the getter is
  /// what joins it to Svelte: reading `app.dbg` touches `dbgVersion`, and any
  /// effect that read it is invalidated when the debugger moves.
  ///
  /// **Do not alias this into a `$derived`.** `deriveds.js` compares with
  /// `value === this.v` and this getter answers the same object every time, so
  /// a derived recomputes and then declines to propagate. That is not a
  /// hypothetical: it is why the Debug tab redrew only when something else on
  /// the page happened to change, which reads as a pane that has frozen and as
  /// clicks that do not take.
  get dbg() {
    this.dbgVersion
    return this._dbg
  }

  // ─── Journal ───────────────────────────────────────────────────────────
  journal = $state([])
  journalFilter = $state('')
  showJournal = $state(true)

  // ─── Bridge ────────────────────────────────────────────────────────────
  link = $state({ state: 'connecting', why: '' })
  info = $state(null)
  /// Ticks once a second, so the auto-continue countdown moves even when the
  /// server is frozen and nothing else is arriving.
  now = $state(Date.now())

  constructor() {
    this.conn = null
    this._dbg = new DebugView({
      send: (command, args) => this.bridge.dap(command, args),
      onChange: () => {
        this.dbgVersion++
        // A file the view has decided to open but does not have yet.
        if (this._dbg.pendingFile && this.requestedFile !== this._dbg.pendingFile) {
          this.requestedFile = this._dbg.pendingFile
          this.bridge.file(this.requestedFile)
        }
      },
    })
    this.requestedFile = null

    this.bridge = new Bridge({
      onOpen: () => {
        this.link = { state: 'up', why: '' }
      },
      onClose: () => {
        this.link = { state: 'down', why: 'bridge closed' }
        this.dap = { state: 'down', why: 'bridge closed' }
        this._dbg.onDisconnected()
      },
      onRetry: (why, delay) => {
        this.link = { state: 'down', why: `${why} — retrying in ${Math.round(delay / 1000)}s` }
      },
      onProtocolError: (why) => this.pushSystem(why),
      onFrame: (frame) => this.onBridgeFrame(frame),
    })
  }

  start() {
    this.bridge.connect()
    this.timer = setInterval(() => {
      this.now = Date.now()
    }, 1000)
  }

  stop() {
    clearInterval(this.timer)
    this.conn?.close()
  }

  // ─── the game ──────────────────────────────────────────────────────────

  /// Open the driver's listener. Called once the bridge has said where it is,
  /// so a non-default port needs no flag here either.
  connectGame(url) {
    if (this.conn) return
    this.conn = new Connection(url, {
      onOpen: () => {
        this.game = { state: 'up', why: '' }
      },
      onClose: (code, reason) => {
        this.game = { state: 'down', why: reason || `closed (${code})` }
      },
      onText: (frame) => {
        for (const line of linesOf(frame)) this.pushLine(line)
      },
      // A separate frame type, so there is no "an unterminated line is the
      // prompt" heuristic to get wrong.
      onPrompt: (frame) => {
        this.prompt = linesOf(frame).flat()
      },
      // `data` is a nested JSON value, not a string — nothing to parse.
      onGmcp: (pkg, data) => this.onGmcp(pkg, data),
      // `masked: true` means the server has taken over echoing. The polarity is
      // inverted relative to the efuns that produce it, and getting it backwards
      // puts a password in the DOM.
      onEcho: (masked) => {
        this.masked = masked
      },
      onBye: (reason) => {
        this.game = { state: 'down', why: reason ?? 'server ended the session' }
      },
      onServerError: (message) => this.pushSystem(message),
      onProtocolError: (why) => this.pushSystem(why),
      // Forward-compatible: a newer server may send a type this client has
      // never heard of, and that is not a reason to break.
      onUnknown: (frame) => this.pushSystem(`unknown frame type '${frame.type}'`),
    })
    this.conn.connect()
  }

  // ─── the bridge ────────────────────────────────────────────────────────

  onBridgeFrame(frame) {
    switch (frame.t) {
      case 'hello':
        this.info = frame
        this._dbg.autoContinueSecs = frame.autoContinueSecs ?? 300
        this._dbg.setFiles(frame.files ?? [])
        if (frame.gameEnabled === false) {
          this.pushSystem('[servers.websocket] is disabled in the driver config')
          this.game = { state: 'down', why: 'disabled in the driver config' }
        } else if (frame.game) {
          this.connectGame(frame.game)
        }
        break

      case 'dap.up':
        this.dap = { state: 'up', why: '' }
        this._dbg.onConnected()
        break
      case 'dap.down':
        this.dap = { state: 'down', why: frame.why }
        this._dbg.onDisconnected()
        break
      case 'dap.msg':
        this._dbg.onMessage(frame.msg)
        break

      case 'file':
        this.requestedFile = null
        this._dbg.setSource(frame.path, frame.lines ?? [], frame.error, blockState)
        break

      case 'files':
        this._dbg.setFiles(frame.files ?? [])
        break

      case 'journal':
        if (this.journal.length >= JOURNAL_LINES) this.journal.shift()
        this.journal.push(frame.entry)
        break

      case 'error':
        this.pushSystem(frame.why)
        break
    }
  }

  pushLine(spans) {
    if (this.scrollback.length >= SCROLLBACK) this.scrollback.shift()
    this.scrollback.push(spans)
  }

  /// A line from the client itself, marked so it cannot be mistaken for the
  /// game saying it. `3` is yellow in the palette the driver sends.
  pushSystem(text) {
    this.pushLine([{ text: `— ${text}`, fg: 3, dim: true }])
  }

  onGmcp(pkg, v) {
    const num = (o, k) => (typeof o?.[k] === 'number' ? o[k] : null)

    // Package names are matched case-insensitively: gmcp_d lowercases on the
    // way in and clients disagree about capitalisation.
    switch (String(pkg).toLowerCase()) {
      case 'char.vitals':
        this.vitals = {
          ...this.vitals,
          hp: num(v, 'hp'),
          maxhp: num(v, 'maxhp'),
          mp: num(v, 'mp'),
          maxmp: num(v, 'maxmp'),
        }
        break
      case 'char.status':
        this.vitals = {
          ...this.vitals,
          level: num(v, 'level'),
          xp: num(v, 'xp'),
          gold: num(v, 'gold'),
        }
        break
      case 'char.effects':
        this.effects = Array.isArray(v)
          ? v.map((e) => ({
              label: typeof e?.label === 'string' ? e.label : '?',
              remaining: num(e, 'remaining') ?? -1,
              stacks: num(e, 'stacks') ?? 1,
            }))
          : []
        break
      case 'room.info': {
        const s = (k) => (typeof v?.[k] === 'string' ? v[k] : '')
        this.room = {
          id: s('id'),
          name: s('name'),
          area: s('area'),
          exits: Array.isArray(v?.exits) ? v.exits.filter((e) => typeof e === 'string') : [],
        }
        break
      }
    }
  }

  // ─── outbound ──────────────────────────────────────────────────────────

  submit() {
    const line = this.input
    // Never put a password into the recallable history.
    if (!this.masked && line !== '') this.history.push(line)
    this.historyPos = null
    this.input = ''
    this.command(line)
  }

  recall(delta) {
    if (this.history.length === 0) return
    const last = this.history.length - 1
    if (delta < 0) {
      this.historyPos = this.historyPos === null ? last : Math.max(0, this.historyPos - 1)
    } else if (this.historyPos !== null) {
      this.historyPos = this.historyPos >= last ? null : this.historyPos + 1
    }
    this.input = this.historyPos === null ? '' : this.history[this.historyPos]
  }

  /// Send a line to the game. One place, so nothing has to know that the game
  /// is a different socket from the bridge — and so the seam below is marked
  /// wherever a command comes from, the input box or an exit button alike.
  ///
  /// The break goes in *before* the send, so it lands above the output it is
  /// separating rather than after it.
  command(text) {
    if (text !== '') appendBreak(this.scrollback)
    this.conn?.send(text)
  }

  /// Drives the in-game `trace` command on your session. This is text, not
  /// data, and is labelled as such: the trace rings live in a thread-local on
  /// the Lua thread and are exposed only as pre-rendered strings.
  trace(what) {
    this.command(`trace ${what}`)
  }

  /// This transport's NAWS. A `hello` updates only the fields it carries.
  size(width, height) {
    this.conn?.hello({ width, height })
  }
}
