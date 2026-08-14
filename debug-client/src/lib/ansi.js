// ANSI SGR → styled spans.
//
// A port of `src/bin/tui/ansi.rs`. `mudlib/lib/color.lua` emits a known, small
// set: `ESC[Nm` for the 16 colours, the bright pair, the styles and reset, plus
// `ESC[38;5;Nm` / `ESC[48;5;Nm` for its xterm-256 aliases. Everything here
// covers that, plus 24-bit colour and the "turn it off again" codes for
// completeness, and ignores any other CSI sequence rather than printing it.
//
// The decoder is a state machine because output arrives in arbitrary chunks: an
// escape sequence or a line can be split across two frames. Style carries
// across line breaks, as it does on a real terminal.

const NORMAL = 0
const ESCAPE = 1
const CSI = 2

/// xterm's first 16, the palette every terminal disagrees about slightly. These
/// are the values the driver's own client uses.
const BASIC = [
  '#000000', '#cd0000', '#00cd00', '#cdcd00', '#0000ee', '#cd00cd', '#00cdcd', '#e5e5e5',
]
const BRIGHT = [
  '#7f7f7f', '#ff0000', '#00ff00', '#ffff00', '#5c5cff', '#ff00ff', '#00ffff', '#ffffff',
]
const CUBE = [0, 95, 135, 175, 215, 255]

const hex2 = (n) => n.toString(16).padStart(2, '0')

/// xterm-256 index → CSS colour.
export function indexed(n) {
  if (n < 8) return BASIC[n]
  if (n < 16) return BRIGHT[n - 8]
  if (n < 232) {
    const i = n - 16
    return `#${hex2(CUBE[Math.floor(i / 36) % 6])}${hex2(CUBE[Math.floor(i / 6) % 6])}${hex2(CUBE[i % 6])}`
  }
  const g = 8 + (n - 232) * 10
  return `#${hex2(g)}${hex2(g)}${hex2(g)}`
}

const EMPTY = Object.freeze({
  fg: null,
  bg: null,
  bold: false,
  dim: false,
  italic: false,
  underline: false,
  blink: false,
  reverse: false,
  strike: false,
})

function sameStyle(a, b) {
  for (const k of Object.keys(EMPTY)) if (a[k] !== b[k]) return false
  return true
}

export class AnsiDecoder {
  constructor() {
    this.state = NORMAL
    this.params = ''
    this.style = { ...EMPTY }
    this.spans = []
    this.text = ''
  }

  /// Feed a chunk of game output. Returns every line completed by it; any
  /// trailing partial line stays buffered and is readable via `partial()`.
  feed(chunk) {
    const lines = []
    for (const ch of chunk) {
      switch (this.state) {
        case NORMAL:
          if (ch === '\x1b') this.state = ESCAPE
          else if (ch === '\n') lines.push(this.#takeLine())
          // The mudlib sends CRLF; a bare CR is a carriage return we have no
          // use for either, since we rebuild the line anyway.
          else if (ch === '\r') { /* drop */ }
          // Telnet NUL padding after CR, and the BEL we do not ring.
          else if (ch === '\x00' || ch === '\x07') { /* drop */ }
          else this.text += ch
          break

        case ESCAPE:
          if (ch === '[') {
            this.params = ''
            this.state = CSI
          } else {
            // Not a CSI — ESC ) 0 and friends. Drop the introducer and treat
            // the next byte as ordinary text.
            this.state = NORMAL
          }
          break

        case CSI: {
          // A CSI ends at the first byte in 0x40..=0x7e; everything before it
          // is parameter or intermediate bytes.
          const code = ch.charCodeAt(0)
          if (code >= 0x40 && code <= 0x7e) {
            if (ch === 'm') this.#applySgr()
            this.state = NORMAL
          } else {
            this.params += ch
          }
          break
        }
      }
    }
    return lines
  }

  /// The partial line currently buffered — the prompt, which the driver sends
  /// without a trailing newline. Returns `null` when nothing is pending.
  partial() {
    if (this.text === '' && this.spans.length === 0) return null
    const spans = this.spans.slice()
    if (this.text !== '') spans.push({ text: this.text, style: { ...this.style } })
    return spans
  }

  #closeSpan() {
    if (this.text !== '') {
      this.spans.push({ text: this.text, style: { ...this.style } })
      this.text = ''
    }
  }

  #takeLine() {
    this.#closeSpan()
    const line = this.spans
    this.spans = []
    return line
  }

  #applySgr() {
    // An empty parameter list — a bare `ESC[m` — means reset, same as `0`.
    const codes =
      this.params === ''
        ? [0]
        : this.params.split(';').map((p) => {
            const n = Number.parseInt(p.trim(), 10)
            return Number.isFinite(n) ? n : 0
          })

    const before = { ...this.style }
    const s = this.style
    for (let i = 0; i < codes.length; i++) {
      const c = codes[i]
      if (c === 0) Object.assign(s, EMPTY)
      else if (c === 1) s.bold = true
      else if (c === 2) s.dim = true
      else if (c === 3) s.italic = true
      else if (c === 4) s.underline = true
      else if (c === 5) s.blink = true
      else if (c === 7) s.reverse = true
      else if (c === 9) s.strike = true
      else if (c === 22) { s.bold = false; s.dim = false }
      else if (c === 23) s.italic = false
      else if (c === 24) s.underline = false
      else if (c === 25) s.blink = false
      else if (c === 27) s.reverse = false
      else if (c === 29) s.strike = false
      else if (c >= 30 && c <= 37) s.fg = BASIC[c - 30]
      else if (c === 38) {
        const ext = extended(codes, i)
        if (ext) { s.fg = ext.color; i += ext.used - 1 }
      } else if (c === 39) s.fg = null
      else if (c >= 40 && c <= 47) s.bg = BASIC[c - 40]
      else if (c === 48) {
        const ext = extended(codes, i)
        if (ext) { s.bg = ext.color; i += ext.used - 1 }
      } else if (c === 49) s.bg = null
      else if (c >= 90 && c <= 97) s.fg = BRIGHT[c - 90]
      else if (c >= 100 && c <= 107) s.bg = BRIGHT[c - 100]
      // Anything else is a code the mudlib does not emit; ignoring it is better
      // than rendering the escape as text.
    }

    // Only break the span if the style actually moved. A no-op `ESC[m` in the
    // middle of a word should not fragment it.
    if (!sameStyle(this.style, before)) {
      const after = { ...this.style }
      this.style = before
      this.#closeSpan()
      this.style = after
    }
  }
}

/// `38`/`48` take a sub-form: `;5;N` for indexed, `;2;R;G;B` for 24-bit.
function extended(codes, i) {
  const kind = codes[i + 1]
  if (kind === 5 && codes.length > i + 2) return { color: indexed(codes[i + 2] & 0xff), used: 3 }
  if (kind === 2 && codes.length > i + 4) {
    const [r, g, b] = [codes[i + 2] & 0xff, codes[i + 3] & 0xff, codes[i + 4] & 0xff]
    return { color: `#${hex2(r)}${hex2(g)}${hex2(b)}`, used: 5 }
  }
  return null
}

/// A span's style as inline CSS. `reverse` is done by swapping the two colours
/// rather than with a filter, so it composes with everything else.
export function css(style) {
  const fg = style.reverse ? (style.bg ?? 'var(--bg)') : style.fg
  const bg = style.reverse ? (style.fg ?? 'var(--fg)') : style.bg
  const out = []
  if (fg) out.push(`color:${fg}`)
  if (bg) out.push(`background:${bg}`)
  if (style.bold) out.push('font-weight:700')
  if (style.dim) out.push('opacity:.6')
  if (style.italic) out.push('font-style:italic')
  const lines = []
  if (style.underline) lines.push('underline')
  if (style.strike) lines.push('line-through')
  if (lines.length) out.push(`text-decoration:${lines.join(' ')}`)
  if (style.blink) out.push('animation:ansi-blink 1s steps(2,start) infinite')
  return out.join(';')
}
