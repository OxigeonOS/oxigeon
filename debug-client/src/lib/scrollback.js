// Where the player typed something.
//
// The driver does not echo input and this client does not either — what you
// typed is in the box you typed it in, and a password must never reach the
// scrollback. But without an echo there is nothing between one command's output
// and the next, so a session reads as one undifferentiated wall.
//
// So a **break** goes in instead: no text, no record of what was typed, just the
// seam. It carries none of the input, which is what makes it safe to emit while
// a password prompt is up.

/// The marker itself. Frozen and shared — every break is the same break, and
/// nothing downstream should be writing to one.
export const BREAK = Object.freeze({ rule: true })

/// Scrollback entries are arrays of spans; a break is the one that is not.
export function isBreak(entry) {
  return !Array.isArray(entry)
}

/// Append a break, if one belongs there.
///
/// Returns whether it was added, so a caller can tell a no-op from a change.
/// Two rules with nothing between them are a wider gap rather than a seam, and
/// a rule above the first line of the session separates it from nothing.
export function appendBreak(scrollback) {
  if (scrollback.length === 0) return false
  if (isBreak(scrollback[scrollback.length - 1])) return false
  scrollback.push(BREAK)
  return true
}
