// Rendering the driver's styled spans.
//
// There is no ANSI decoder here, and there was one until the WebSocket
// transport landed. `?ansi=spans` makes the driver send structured runs
// instead of escape codes, and it is emphatic about why: the interesting cases
// in that state machine — a style accumulating across two sequences, an
// `ESC[m` that means reset, a sequence truncated at a buffer boundary — get
// found once there, with tests, rather than separately in every client.
//
// What is left is the half a browser genuinely has to do: turn a palette index
// and a few booleans into CSS.
//
// A span is `{text}` plus whichever of `fg`, `bg`, `bold`, `dim`, `italic`,
// `underline`, `blink`, `inverse`, `strike` apply. Absent means off, so an
// unstyled run is just `{text}`. See docs/src/protocols/websocket.md.

/// xterm's first 16. **Colours are xterm-256 palette indices, always** — the
/// sixteen basic ANSI colours are 0-15 in that palette, so one integer covers
/// everything `lib/color.lua` can emit and a client needs one lookup table
/// rather than three.
const BASIC = [
  '#000000', '#cd0000', '#00cd00', '#cdcd00', '#0000ee', '#cd00cd', '#00cdcd', '#e5e5e5',
  '#7f7f7f', '#ff0000', '#00ff00', '#ffff00', '#5c5cff', '#ff00ff', '#00ffff', '#ffffff',
]
const CUBE = [0, 95, 135, 175, 215, 255]

const hex2 = (n) => n.toString(16).padStart(2, '0')

/// Palette index → CSS colour. 0-15 basic, 16-231 the 6×6×6 cube, 232-255 the
/// greyscale ramp.
export function indexed(n) {
  if (!Number.isInteger(n) || n < 0 || n > 255) return null
  if (n < 16) return BASIC[n]
  if (n < 232) {
    const i = n - 16
    return `#${hex2(CUBE[Math.floor(i / 36) % 6])}${hex2(CUBE[Math.floor(i / 6) % 6])}${hex2(CUBE[i % 6])}`
  }
  const g = 8 + (n - 232) * 10
  return `#${hex2(g)}${hex2(g)}${hex2(g)}`
}

/// One span as inline CSS. `inverse` swaps the two colours rather than using a
/// filter, so it composes with everything else.
///
/// Total on purpose: anything that is not a span styles nothing, rather than
/// raising. A `style=` expression runs inside the render, so throwing here does
/// not lose a colour — it loses the pane, and every keystroke bound to it. That
/// is not theoretical. One call site was still passing `span.style` from the
/// shape this used to have, which is `undefined`, and the Play tab therefore
/// refused to mount from the moment the first `prompt` frame arrived — which is
/// to say from the moment you logged in, and never before.
export function css(span) {
  if (!span || typeof span !== 'object') return ''
  const fg = indexed(span.fg)
  const bg = indexed(span.bg)
  const [front, back] = span.inverse ? [bg ?? 'var(--bg)', fg ?? 'var(--fg)'] : [fg, bg]

  const out = []
  if (front) out.push(`color:${front}`)
  if (back) out.push(`background:${back}`)
  if (span.bold) out.push('font-weight:700')
  if (span.dim) out.push('opacity:.6')
  if (span.italic) out.push('font-style:italic')
  const lines = []
  if (span.underline) lines.push('underline')
  if (span.strike) lines.push('line-through')
  if (lines.length) out.push(`text-decoration:${lines.join(' ')}`)
  if (span.blink) out.push('animation:ansi-blink 1s steps(2,start) infinite')
  return out.join(';')
}

/// A `text` frame as lines of spans.
///
/// Exactly one of `spans` and `text` is present — `spans` omits `text` rather
/// than sending both, because `text` is the busiest frame in the protocol and
/// duplicating its content would double it. In `spans` mode the `text` branch
/// is what a `raw`/`none` server, or an older one, still sends.
///
/// The driver normalizes line endings: interior `\r\n` becomes `\n`, exactly
/// one trailing terminator is stripped, and no `\r` reaches the client. So
/// splitting on `\n` is safe, and a deliberate blank final line is content and
/// survives.
export function linesOf(frame) {
  if (Array.isArray(frame.spans)) return splitSpans(frame.spans)
  return String(frame.text ?? '')
    .split('\n')
    .map((text) => (text === '' ? [] : [{ text }]))
}

/// Break a run of spans wherever one carries a newline, keeping its style.
function splitSpans(spans) {
  const lines = [[]]
  for (const span of spans) {
    const parts = String(span.text ?? '').split('\n')
    parts.forEach((part, i) => {
      if (i > 0) lines.push([])
      if (part !== '') lines[lines.length - 1].push({ ...span, text: part })
    })
  }
  return lines
}
