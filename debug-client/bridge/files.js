// Lua file discovery, reading, and the path normalisation a breakpoint depends
// on.
//
// This is why the debug client needs a bridge at all rather than a WebSocket
// server in the driver: the adapter has **no `source` request**, so a debug
// client reads the files itself. A browser cannot, and cannot canonicalize a
// path either.
//
// A port of `discover_lua_files`/`walk` from `src/bin/tui/dap.rs` and of
// `src/core/scripting/debugger/paths.rs`.

import fs from 'node:fs'
import path from 'node:path'

/// Strip Windows' verbatim `\\?\` prefix, which canonicalization adds but
/// neither Lua nor any debug client understands.
function stripVerbatim(s) {
  return s.startsWith('//?/') ? s.slice(4) : s
}

/// Render `p` in the same textual form Lua's `require` produces: absolute,
/// forward slashes, no verbatim prefix.
///
/// Falls back to the path as given if it cannot be resolved (e.g. it does not
/// exist yet), so this never throws on a missing file.
export function absLuaPath(p) {
  let resolved
  try {
    resolved = fs.realpathSync(p)
  } catch {
    resolved = path.resolve(p)
  }
  return stripVerbatim(resolved.replaceAll('\\', '/'))
}

/// Fold a client-supplied path into a comparable key. Lowercased on Windows,
/// whose filesystem is case-insensitive and where debug clients are
/// inconsistent about the drive-letter case.
export function normalize(raw) {
  const s = stripVerbatim(String(raw).replaceAll('\\', '/'))
  return process.platform === 'win32' ? s.toLowerCase() : s
}

function walk(dir, out) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const entry of entries) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) walk(full, out)
    else if (entry.name.endsWith('.lua')) out.push(full)
  }
}

/// Every `.lua` file a breakpoint could apply to, under the roots the server
/// loads.
///
/// Each entry carries both forms deliberately. `rel` is what the tree, the
/// breakpoint map and the UI speak; `abs` is what `setBreakpoints` must send —
/// the same textual form `require` produced. Sending the browser only one of
/// them would mean either a tree full of absolute paths or a breakpoint the
/// adapter answers `verified: true` and then never stops on.
export function discoverLuaFiles(root, roots = ['mudlib', 'game']) {
  const found = []
  for (const sub of roots) walk(path.join(root, sub), found)
  found.sort()
  return found.map((full) => ({
    rel: path.relative(root, full).replaceAll('\\', '/'),
    abs: absLuaPath(full),
  }))
}

/// Read one file, as lines. Refuses anything outside the roots — the bridge is
/// a localhost dev tool, but "reads any file the user can read" is not a thing
/// to hand a web page regardless.
export function readLuaFile(root, rel) {
  const full = path.resolve(root, rel)
  const inside = ['mudlib', 'game'].some((sub) => {
    const base = path.resolve(root, sub)
    return full === base || full.startsWith(base + path.sep)
  })
  if (!inside) return { error: `refused: ${rel} is outside mudlib/ and game/` }
  try {
    return { lines: fs.readFileSync(full, 'utf8').split(/\r?\n/) }
  } catch (e) {
    return { error: `cannot read ${rel}: ${e.message}` }
  }
}
