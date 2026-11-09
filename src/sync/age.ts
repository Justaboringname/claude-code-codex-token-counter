// Pure (no React), so scripts/sync-selftest.mjs can test it under Node.

/** "just now", "1 min ago", "12 min ago", "3 h ago", "2 days ago". The unit
 *  is picked from the same rounded minutes as the number, so 59.5 min reads
 *  "1 h ago", never "0 h ago". */
export function relativeAge(thenMs: number, nowMs: number = Date.now()): string {
  const s = Math.max(0, (nowMs - thenMs) / 1000);
  if (!Number.isFinite(s)) return 'at an unknown time';
  if (s < 45) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${Math.max(1, m)} min ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.floor(h / 24)} days ago`;
}
