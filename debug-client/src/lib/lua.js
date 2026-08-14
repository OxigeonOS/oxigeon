// Lua syntax highlighting.
//
// A port of `src/bin/tui/lua_syntax.rs`. Not a parser: a run-splitter good
// enough that a source pane reads like source, and wrong only in places Lua
// itself would reject the file.

/// Reserved words. `self` is not one, but reads as structure and is coloured
/// like it — the alternative is that the most important name in a method body
/// looks like every other local.
const KEYWORDS = new Set([
  'and', 'break', 'do', 'else', 'elseif', 'end', 'false', 'for', 'function', 'goto', 'if', 'in',
  'local', 'nil', 'not', 'or', 'repeat', 'return', 'then', 'true', 'until', 'while', 'self',
])

const LITERALS = new Set(['nil', 'true', 'false'])

/// `[`, `n` equals signs, `[` — returns `n`, or `null`.
function openAt(b, i) {
  if (b[i] !== '[') return null
  let n = 0
  while (b[i + 1 + n] === '=') n++
  return b[i + 1 + n] === '[' ? n : null
}

/// `]`, `n` equals signs, `]` — returns `n`, or `null`.
function closeAt(b, i) {
  if (b[i] !== ']') return null
  let n = 0
  while (b[i + 1 + n] === '=') n++
  return b[i + 1 + n] === ']' ? n : null
}

/// Index just past a `"`- or `'`-quoted string starting at `i`.
function skipQuoted(b, i) {
  const quote = b[i]
  let j = i + 1
  while (j < b.length) {
    if (b[j] === '\\') { j += 2; continue }
    if (b[j] === quote) return j + 1
    j++
  }
  // Unterminated: Lua would reject the file, so treat it as ending here rather
  // than swallowing everything after it.
  return b.length
}

/// Follow one line's long-bracket transitions, returning the state after it.
function scanLineBlocks(line, open) {
  const b = [...line]
  let i = 0
  while (i < b.length) {
    if (open !== null) {
      const n = closeAt(b, i)
      if (n === open) {
        open = null
        i += n + 2
        continue
      }
      i++
    } else {
      // A short comment eats the rest of the line — unless it opens a long one,
      // which `--[[` does.
      if (b[i] === '-' && b[i + 1] === '-') {
        const n = openAt(b, i + 2)
        if (n !== null) {
          open = n
          i += n + 4
          continue
        }
        return null
      }
      const n = openAt(b, i)
      if (n !== null) {
        open = n
        i += n + 2
        continue
      }
      // A quoted string cannot contain a long bracket opener that matters, so
      // skip over it wholesale.
      if (b[i] === '"' || b[i] === "'") {
        i = skipQuoted(b, i)
        continue
      }
      i++
    }
  }
  return open
}

/// Whether each line *starts* inside a long bracket, and at what level.
///
/// `null` means ordinary code; a number means inside `[=*[` with that many `=`
/// signs, so only a matching `]=*]` closes it.
///
/// Computed once when a file opens rather than per frame: it needs a scan from
/// the top of the file, and the source pane redraws on every keystroke.
export function blockState(lines) {
  const out = []
  let open = null
  for (const line of lines) {
    out.push(open)
    open = scanLineBlocks(line, open)
  }
  return out
}

const isDigit = (c) => c >= '0' && c <= '9'
const isAlpha = (c) => /\p{L}/u.test(c)
const isAlnum = (c) => /[\p{L}\p{N}]/u.test(c)

/// Whether the next non-space character opens a call.
function isCall(b, from) {
  for (let j = from; j < b.length; j++) {
    if (!/\s/.test(b[j])) return b[j] === '('
  }
  return false
}

/// Append a run, merging it into the previous one when the kind matches — fewer
/// spans is less work for the renderer.
function push(out, text, kind) {
  if (text === '') return
  const last = out[out.length - 1]
  if (last && last.kind === kind) last.text += text
  else out.push({ text, kind })
}

/// Split one line into `{text, kind}` runs. Kinds are `plain`, `keyword`,
/// `literal` (`nil`/`true`/`false` and numbers), `string`, `comment`, `ident`.
///
/// `startsIn` comes from `blockState` and says whether this line begins inside
/// a long bracket.
export function tokenize(line, startsIn = null) {
  const b = [...line]
  const out = []
  let i = 0

  // Everything up to the closing bracket belongs to the block that was open
  // when the line started.
  if (startsIn !== null) {
    let j = 0
    while (j < b.length) {
      if (closeAt(b, j) === startsIn) {
        j += startsIn + 2
        break
      }
      j++
    }
    j = Math.min(j, b.length)
    push(out, b.slice(0, j).join(''), 'comment')
    i = j
  }

  while (i < b.length) {
    const c = b[i]

    // ── comments ──────────────────────────────────────────────────────────
    if (c === '-' && b[i + 1] === '-') {
      const n = openAt(b, i + 2)
      if (n !== null) {
        let j = i + n + 4
        while (j < b.length && closeAt(b, j) !== n) j++
        const end = Math.min(j + n + 2, b.length)
        push(out, b.slice(i, end).join(''), 'comment')
        i = end
        continue
      }
      push(out, b.slice(i).join(''), 'comment')
      break
    }

    // ── strings ───────────────────────────────────────────────────────────
    if (c === '"' || c === "'") {
      const end = skipQuoted(b, i)
      push(out, b.slice(i, end).join(''), 'string')
      i = end
      continue
    }
    {
      const n = openAt(b, i)
      if (n !== null) {
        let j = i + n + 2
        while (j < b.length && closeAt(b, j) !== n) j++
        const end = Math.min(j + n + 2, b.length)
        push(out, b.slice(i, end).join(''), 'string')
        i = end
        continue
      }
    }

    // ── numbers ───────────────────────────────────────────────────────────
    if (isDigit(c)) {
      let j = i
      while (j < b.length && (isAlnum(b[j]) || b[j] === '.')) j++
      push(out, b.slice(i, j).join(''), 'literal')
      i = j
      continue
    }

    // ── words ─────────────────────────────────────────────────────────────
    if (isAlpha(c) || c === '_') {
      let j = i
      while (j < b.length && (isAlnum(b[j]) || b[j] === '_')) j++
      const word = b.slice(i, j).join('')
      const kind = LITERALS.has(word)
        ? 'literal'
        : KEYWORDS.has(word)
          ? 'keyword'
          : // Followed by `(` — a call or a definition, and the thing you scan
            // for when you are looking for where something happens.
            isCall(b, j)
            ? 'ident'
            : 'plain'
      push(out, word, kind)
      i = j
      continue
    }

    push(out, c, 'plain')
    i++
  }

  return out
}

/// Overlay search hits on top of the syntax runs.
///
/// Painted *on top* rather than instead: a hit inside a string or a comment is
/// usually where you were looking, and a highlighter that dropped the colour
/// underneath would make the match harder to find, not easier.
export function withMatches(runs, needle) {
  if (!needle) return runs
  const lower = needle.toLowerCase()
  const out = []
  for (const run of runs) {
    let rest = run.text
    let at = rest.toLowerCase().indexOf(lower)
    if (at === -1) {
      out.push(run)
      continue
    }
    while (at !== -1) {
      if (at > 0) out.push({ text: rest.slice(0, at), kind: run.kind })
      out.push({ text: rest.slice(at, at + lower.length), kind: run.kind, match: true })
      rest = rest.slice(at + lower.length)
      at = rest.toLowerCase().indexOf(lower)
    }
    if (rest) out.push({ text: rest, kind: run.kind })
  }
  return out
}
