// Application state, and the wiring between the bridge and the four tabs.
//
// A port of `src/bin/tui/app.rs`. The bridge talks to this through `onFrame`;
// the UI talks back through the methods. Nothing else is shared.
//
// `DebugView` is a plain class rather than a rune-annotated one: it holds Maps
// and Sets and mutates deeply, which is exactly what deep reactivity is worst
// at. It reports through `onChange` and this bumps `dbgVersion`, so a pane that
// reads `app.dbgVersion` redraws when the debugger moves and not otherwise.

import { AnsiDecoder } from './ansi.js'
import { Bridge } from './bridge.js'
import { DebugView } from './debugview.js'
import { blockState } from './lua.js'

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
  /// True while the server holds ECHO — mask what we render.
  masked = $state(false)
  history = []
  historyPos = null
  vitals = $state({ hp: null, maxhp: null, mp: null, maxmp: null, level: null, xp: null, gold: null })
  room = $state({ id: '', name: '', area: '', exits: [] })
  effects = $state([])
  telnet = $state({ state: 'connecting', why: '' })

  // ─── Debug ─────────────────────────────────────────────────────────────
  dap = $state({ state: 'connecting', why: '' })
  dbgVersion = $state(0)

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
    this.ansi = new AnsiDecoder()
    this.dbg = new DebugView({
      send: (command, args) => this.bridge.dap(command, args),
      onChange: () => {
        this.dbgVersion++
        // A file the view has decided to open but does not have yet.
        if (this.dbg.pendingFile) {
          const wanted = this.dbg.pendingFile
          if (this.requestedFile !== wanted) {
            this.requestedFile = wanted
            this.bridge.file(wanted)
          }
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
        this.telnet = { state: 'down', why: 'bridge closed' }
        this.dap = { state: 'down', why: 'bridge closed' }
        this.dbg.onDisconnected()
      },
      onRetry: (why, delay) => {
        this.link = { state: 'down', why: `${why} — retrying in ${Math.round(delay / 1000)}s` }
      },
      onProtocolError: (why) => this.pushSystem(why),
      onFrame: (frame) => this.onFrame(frame),
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
  }

  // ─── inbound ───────────────────────────────────────────────────────────

  onFrame(frame) {
    switch (frame.t) {
      case 'hello':
        this.info = frame
        this.dbg.autoContinueSecs = frame.autoContinueSecs ?? 300
        this.dbg.setFiles(frame.files ?? [])
        break

      case 'game': {
        for (const line of this.ansi.feed(frame.text)) this.pushLine(line)
        // Whatever is left unterminated is the prompt. The driver sends it with
        // no newline, which is exactly how we tell them apart.
        this.prompt = this.ansi.partial()
        break
      }

      case 'gmcp':
        this.onGmcp(frame.package, frame.json)
        break

      case 'echo':
        this.masked = frame.on
        break

      case 'telnet.up':
        this.telnet = { state: 'up', why: '' }
        break
      case 'telnet.down':
        this.telnet = { state: 'down', why: frame.why }
        break

      case 'dap.up':
        this.dap = { state: 'up', why: '' }
        this.dbg.onConnected()
        break
      case 'dap.down':
        this.dap = { state: 'down', why: frame.why }
        this.dbg.onDisconnected()
        break
      case 'dap.msg':
        this.dbg.onMessage(frame.msg)
        break

      case 'file':
        this.requestedFile = null
        this.dbg.setSource(frame.path, frame.lines ?? [], frame.error, blockState)
        break

      case 'files':
        this.dbg.setFiles(frame.files ?? [])
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
  /// game saying it.
  pushSystem(text) {
    this.pushLine([{ text: `— ${text}`, style: { fg: '#cdcd00', dim: true } }])
  }

  onGmcp(pkg, json) {
    let v
    try {
      v = JSON.parse(json)
    } catch {
      return
    }
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
    this.bridge.input(line)
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

  /// Drives the in-game `trace` command on your session. This is text, not
  /// data, and is labelled as such: the trace rings live in a thread-local on
  /// the Lua thread and are exposed only as pre-rendered strings.
  trace(what) {
    this.bridge.input(`trace ${what}`)
  }

  size(w, h) {
    this.bridge.naws(w, h)
  }
}
