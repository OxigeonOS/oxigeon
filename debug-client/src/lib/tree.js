// The file tree.
//
// A port of `build_rows` from `src/bin/tui/dap.rs`. The files pane is a
// collapsed tree rather than a list of paths: every `.lua` file under `mudlib/`
// and `game/` is several hundred rows, all of them beginning `mudlib/` — a list
// you read rather than navigate.

/// Flatten a sorted path list into the rows a collapsed tree shows.
///
/// Driven off the sorted list rather than a real tree structure: the paths
/// already carry the hierarchy, and sorting them puts every directory's
/// contents together and in order. A row is emitted only when every one of its
/// ancestors is expanded, which is what `visible` tracks.
export function buildRows(files, expanded) {
  const rows = []
  const seen = new Set()

  for (const file of files) {
    const parts = file.split('/').filter((p) => p !== '')
    if (parts.length === 0) continue
    let acc = ''
    let visible = true

    // Every component but the last is a directory.
    for (let depth = 0; depth < parts.length - 1; depth++) {
      acc = acc === '' ? parts[depth] : `${acc}/${parts[depth]}`
      if (visible && !seen.has(acc)) {
        seen.add(acc)
        rows.push({ path: acc, depth, isDir: true, expanded: expanded.has(acc) })
      }
      if (!expanded.has(acc)) visible = false
    }

    if (visible) rows.push({ path: file, depth: parts.length - 1, isDir: false, expanded: false })
  }
  return rows
}

/// Just this entry's own name — the tree shows the path by nesting.
export function label(path) {
  const at = path.lastIndexOf('/')
  return at === -1 ? path : path.slice(at + 1)
}

export function parentOf(path) {
  const at = path.lastIndexOf('/')
  return at === -1 ? null : path.slice(0, at)
}

/// Every directory above `path`, so it can be revealed.
export function ancestorsOf(path) {
  const parts = path.split('/')
  const out = []
  let acc = ''
  for (let i = 0; i < parts.length - 1; i++) {
    acc = acc === '' ? parts[i] : `${acc}/${parts[i]}`
    out.push(acc)
  }
  return out
}
