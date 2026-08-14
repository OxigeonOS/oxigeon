// Tail of `logs/journal.log`.
//
// The driver writes one JSON object per line there
// (`src/core/logging/game_logger.rs`) and captures *every* Lua error with its
// traceback, so this shows a mudlib crash whether or not a debugger was
// attached when it happened. It needs no cooperation from the server at all —
// it is a file.
//
// A port of `src/bin/tui/journal.rs`.

import fs from 'node:fs'

/// How much history to show on startup.
const BACKFILL = 200
const POLL_MS = 400

/// Parse one journal line. Anything unparseable is still shown — a half-written
/// line during a crash is exactly when you want to see it.
export function parseEntry(line) {
  try {
    const v = JSON.parse(line)
    const get = (k) => (typeof v[k] === 'string' ? v[k] : '')
    return { ts: get('ts'), level: get('level'), source: get('source'), msg: get('msg') }
  } catch {
    return { ts: '', level: 'raw', source: '', msg: line }
  }
}

// Formatting a timestamp for display lives in `src/lib/journalfmt.js`, on the
// side that displays it. A second copy here would be a second answer to one
// question, and this one had already drifted: it raised on an entry with no
// `ts`, which is exactly the half-written line during a crash that `parseEntry`
// goes out of its way to keep.

export function tail(file, onEntry) {
  let offset = 0
  let stopped = false

  // Position after backfill, so the first poll does not replay the whole file.
  try {
    const content = fs.readFileSync(file, 'utf8')
    const lines = content.split('\n').filter((l) => l.trim() !== '')
    for (const line of lines.slice(Math.max(0, lines.length - BACKFILL))) onEntry(parseEntry(line))
    offset = Buffer.byteLength(content, 'utf8')
  } catch {
    offset = 0 // not created yet — the driver opens it lazily on first write
  }

  let carry = ''
  const timer = setInterval(() => {
    if (stopped) return
    let size
    try {
      size = fs.statSync(file).size
    } catch {
      return
    }
    // Truncated or rotated out from under us; start over.
    if (size < offset) {
      offset = 0
      carry = ''
    }
    if (size === offset) return

    let fd
    try {
      fd = fs.openSync(file, 'r')
      const length = size - offset
      const buf = Buffer.alloc(length)
      const read = fs.readSync(fd, buf, 0, length, offset)
      offset += read
      const text = carry + buf.subarray(0, read).toString('utf8')
      const parts = text.split('\n')
      // A line without a terminator is still being written; hold it and pick it
      // up whole next time.
      carry = parts.pop() ?? ''
      for (const line of parts) {
        const trimmed = line.replace(/\r$/, '')
        if (trimmed.trim() !== '') onEntry(parseEntry(trimmed))
      }
    } catch {
      /* transient; try again on the next poll */
    } finally {
      if (fd !== undefined) fs.closeSync(fd)
    }
  }, POLL_MS)

  return () => {
    stopped = true
    clearInterval(timer)
  }
}
