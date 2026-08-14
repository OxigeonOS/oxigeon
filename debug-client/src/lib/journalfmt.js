/// `2026-08-03T18:31:02Z` → `18:31:02`. Timestamps are always 20 chars ending
/// in `Z`, hand-formatted by the driver.
export function clock(entry) {
  const ts = entry?.ts ?? ''
  return ts.length >= 19 ? ts.slice(11, 19) : ts
}
