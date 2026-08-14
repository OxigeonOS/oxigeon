// Ports come from `config/driver.toml`, so a server on non-standard ports needs
// no flags here either — the same bargain `oxigeon-tui` makes.
//
// This is **not** a TOML parser. It is a scanner for four scalar keys under two
// known tables, which is all that is wanted, and it treats anything it does not
// understand as absent. A real parser would be a dependency to keep current for
// a file we read four numbers out of; the failure mode of getting it wrong here
// is a default port and a connection error that says so.

import fs from 'node:fs'

const DEFAULTS = { telnetPort: 4000, dapPort: 4711, autoContinueSecs: 300 }

export function loadDriverConfig(file) {
  let text
  try {
    text = fs.readFileSync(file, 'utf8')
  } catch {
    return { ...DEFAULTS, found: false }
  }

  let section = ''
  const found = {}
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim()
    if (line.startsWith('#') || line === '') continue
    const header = line.match(/^\[([^\]]+)\]/)
    if (header) {
      section = header[1].trim()
      continue
    }
    const kv = line.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.+?)\s*(?:#.*)?$/)
    if (!kv) continue
    found[`${section}.${kv[1]}`] = kv[2]
  }

  const num = (key, fallback) => {
    const n = Number.parseInt(found[key] ?? '', 10)
    return Number.isFinite(n) ? n : fallback
  }

  return {
    telnetPort: num('servers.telnet.port', DEFAULTS.telnetPort),
    dapPort: num('servers.debug.port', DEFAULTS.dapPort),
    autoContinueSecs: num('servers.debug.auto_continue_secs', DEFAULTS.autoContinueSecs),
    debugEnabled: (found['servers.debug.enabled'] ?? 'false').trim() === 'true',
    found: true,
  }
}
