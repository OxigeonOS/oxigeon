// The xterm-256 palette, as CSS colours.
//
// The driver sends every colour as a palette index — the sixteen basic ANSI
// colours are 0-15 in this palette, so one integer covers everything the mudlib
// can emit and a client needs exactly this one table.

/** The first sixteen are a convention, not a formula. */
const BASIC = [
  '#000000', '#cd0000', '#00cd00', '#cdcd00',
  '#0000ee', '#cd00cd', '#00cdcd', '#e5e5e5',
  '#7f7f7f', '#ff0000', '#00ff00', '#ffff00',
  '#5c5cff', '#ff00ff', '#00ffff', '#ffffff',
]

/** The 6×6×6 cube's levels are not evenly spaced. */
const CUBE = [0, 95, 135, 175, 215, 255]

const hex = (n) => n.toString(16).padStart(2, '0')

function build() {
  const table = [...BASIC]
  // 16-231: the colour cube.
  for (let r = 0; r < 6; r++) {
    for (let g = 0; g < 6; g++) {
      for (let b = 0; b < 6; b++) {
        table.push(`#${hex(CUBE[r])}${hex(CUBE[g])}${hex(CUBE[b])}`)
      }
    }
  }
  // 232-255: the grey ramp, which is far finer than the cube's greys.
  for (let i = 0; i < 24; i++) {
    const v = 8 + i * 10
    table.push(`#${hex(v)}${hex(v)}${hex(v)}`)
  }
  return table
}

export const PALETTE = build()

export function colour(index) {
  return PALETTE[index] ?? null
}

/**
 * Turn one span from the wire into an inline style string.
 *
 * `inverse` is applied here rather than left to CSS: swapping the two colours
 * is what a terminal does, and doing it in the style keeps the DOM a flat list
 * of spans with no special cases downstream.
 */
export function spanStyle(span, defaults) {
  let fg = span.fg != null ? colour(span.fg) : defaults.fg
  let bg = span.bg != null ? colour(span.bg) : defaults.bg
  if (span.inverse) [fg, bg] = [bg, fg]

  const out = []
  if (fg) out.push(`color:${fg}`)
  // Only paint a background when there is one to paint — a span with a
  // background on every character makes selection and line spacing look wrong.
  if (span.bg != null || span.inverse) out.push(`background-color:${bg}`)
  if (span.bold) out.push('font-weight:700')
  // Dim and bold are separate flags and can both be set; opacity is the only
  // one of the two that composes.
  if (span.dim) out.push('opacity:0.65')
  if (span.italic) out.push('font-style:italic')

  const decor = []
  if (span.underline) decor.push('underline')
  if (span.strike) decor.push('line-through')
  if (decor.length) out.push(`text-decoration:${decor.join(' ')}`)

  if (span.blink) out.push('animation:ox-blink 1s step-start infinite')
  return out.join(';')
}
