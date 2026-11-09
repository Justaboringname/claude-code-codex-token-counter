// Snapshot validation (SPEC §4): every decrypted or cached snapshot passes
// through here before anything renders or stores it.
//
// A snapshot is trusted to be *authentic* (AES-GCM under the pairing code) but
// not to be *well-formed*: a newer primary may change a field's type, and a
// wrong type in a field the UI reads (say `byTool: 1`) would throw during
// render. So:
//   - a newer schema major is refused outright (SnapshotShapeError.newerSchema);
//   - fields the UI reads are coerced when present with the wrong type
//     (numbers → finite or 0, strings → string, arrays → arrays of the right
//     items), or dropped when optional;
//   - valid data comes back unchanged (no keys added, none reordered), and
//     unknown fields are left alone (viewers tolerate them).
//
// Pure module (Node imports it through protocol.ts under type stripping).
import type { Range, Snapshot } from '../types.ts';

/** The snapshot format this build understands. */
export const SNAPSHOT_SCHEMA = 1;

const RANGES: readonly Range[] = ['session', 'today', 'week', 'month', 'all'];
// Longer series than any real range has (All is per day); keeps a hostile
// snapshot from turning into a million-point SVG path.
const MAX_SERIES = 2000;

export class SnapshotShapeError extends Error {
  /** Set when the snapshot is from a newer format this build can't read. */
  newerSchema: number | null;
  constructor(why: string, newerSchema: number | null = null) {
    super(why);
    this.name = 'SnapshotShapeError';
    this.newerSchema = newerSchema;
  }
}

type Obj = Record<string, unknown>;

const isObj = (v: unknown): v is Obj => !!v && typeof v === 'object' && !Array.isArray(v);
const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const isStr = (v: unknown): v is string => typeof v === 'string';

// Each helper looks at o[k] on a copy `o`. A missing key (JSON has no
// `undefined`) stays missing unless `req`, so valid data gets no new keys.

function num(o: Obj, k: string, mode: 'zero' | 'null' | 'drop' = 'zero', req = false) {
  const v = o[k];
  if (v === undefined) {
    if (req) o[k] = mode === 'null' ? null : 0;
    return;
  }
  if (isNum(v) || (v === null && mode === 'null')) return;
  if (mode === 'drop') delete o[k];
  else o[k] = mode === 'null' ? null : 0;
}

function str(o: Obj, k: string, mode: 'empty' | 'null' | 'drop' = 'empty', req = false) {
  const v = o[k];
  if (v === undefined) {
    if (req) o[k] = mode === 'null' ? null : '';
    return;
  }
  if (isStr(v) || (v === null && mode === 'null')) return;
  if (mode === 'drop') delete o[k];
  else o[k] = mode === 'null' ? null : '';
}

function series(o: Obj, k: string, drop: boolean) {
  const v = o[k];
  if (v === undefined) return;
  if (!Array.isArray(v)) {
    if (drop) delete o[k];
    else o[k] = [];
    return;
  }
  if (v.length <= MAX_SERIES && v.every(isNum)) return;
  o[k] = v.slice(-MAX_SERIES).map((x) => (isNum(x) ? x : 0));
}

function limitWindow(v: unknown): Obj | null {
  if (!isObj(v)) return null;
  const o = { ...v };
  num(o, 'usedPct', 'zero', true);
  str(o, 'resetsAt', 'null', true);
  return o;
}

const QUOTA_STATUS = new Set(['ok', 'stale', 'no_token', 'error']);

/** ClaudeQuota / CodexLiveQuota (same core shape). */
function liveQuota(v: unknown, codex: boolean): Obj | null {
  if (!isObj(v)) return null;
  const o = { ...v };
  for (const k of ['fiveHour', 'weekly']) if (o[k] !== undefined && o[k] !== null) o[k] = limitWindow(o[k]);
  // Anything but 'ok' reads as stale, which is the safe reading.
  if (!isStr(o.status) || !QUOTA_STATUS.has(o.status)) o.status = 'error';
  str(o, 'message', 'null');
  str(o, 'fetchedAt');
  if (codex) {
    str(o, 'planType', 'null');
    num(o, 'resetCredits', 'null');
  } else {
    str(o, 'subscription', 'null');
  }
  return o;
}

/** Usage.codexQuota: the rollout-log fallback (codex.rs CodexQuota). */
function logQuota(v: unknown): Obj | null {
  if (!isObj(v)) return null;
  const o = { ...v };
  str(o, 'asOf');
  for (const k of ['primaryUsedPct', 'primaryWindowMinutes', 'secondaryUsedPct', 'secondaryWindowMinutes']) num(o, k, 'zero', true);
  num(o, 'primaryResetsAt', 'null');
  num(o, 'secondaryResetsAt', 'null');
  str(o, 'planType', 'null');
  return o;
}

const PRICE_STATUS = new Set(['fresh', 'aging', 'stale', 'error', 'builtin']);

function priceHealth(v: unknown): Obj | undefined {
  if (!isObj(v) || !isStr(v.status) || !PRICE_STATUS.has(v.status)) return undefined;
  const o = { ...v };
  for (const k of ['fetchedAt', 'lastError', 'lastErrorAt']) str(o, k, 'null');
  return o;
}

function modelUsage(v: unknown): Obj | null {
  if (!isObj(v)) return null;
  const o = { ...v };
  // ByModel reads all four token counts unguarded.
  for (const k of ['in', 'out', 'cacheWrite', 'cacheRead']) num(o, k, 'zero', true);
  // Absent/invalid cost → the frontend's own pricing fallback.
  num(o, 'cost', 'drop');
  num(o, 'cacheWrite1h', 'drop');
  num(o, 'outEst', 'drop');
  num(o, 'costEst', 'drop');
  str(o, 'priceSource', 'drop');
  return o;
}

function usage(v: Obj): Obj {
  const o = { ...v };
  if (o.byModel !== undefined) {
    const out: Obj = {};
    if (isObj(o.byModel)) {
      for (const [id, m] of Object.entries(o.byModel)) {
        const fixed = modelUsage(m);
        if (fixed) out[id] = fixed;
      }
    }
    o.byModel = out;
  }
  if (o.byTool !== undefined) {
    o.byTool = Array.isArray(o.byTool)
      ? o.byTool.flatMap((t) => {
          if (!isObj(t) || !isStr(t.name) || !t.name) return [];
          const c = { ...t };
          num(c, 'calls', 'zero', true);
          num(c, 'tokens', 'zero', true);
          return [c];
        })
      : [];
  }
  series(o, 'sparkline', false);
  series(o, 'sparklineCodex', true);
  if (o.codexQuota !== undefined && o.codexQuota !== null) o.codexQuota = logQuota(o.codexQuota);
  for (const k of ['sparkLeft', 'sparkRight', 'rangeLabel', 'planLabel', 'planId']) str(o, k);
  for (const k of ['sessionStart', 'activeBlockStart']) str(o, k, 'null');
  for (const k of [
    'totalCost',
    'totalTokens',
    'creditUsed5h',
    'creditUsedWeek',
    'creditLimit5h',
    'creditLimitWeek',
    'tokens5h',
    'tokensWeek',
    'priorityEvents5h',
    'tokenLimit5h',
  ])
    num(o, k);
  num(o, 'totalCostEst', 'drop');
  num(o, 'deltaPct', 'null');
  if (o.priceHealth !== undefined) {
    const ph = priceHealth(o.priceHealth);
    if (ph) o.priceHealth = ph;
    else delete o.priceHealth;
  }
  return o;
}

/** Validate a parsed snapshot. Throws SnapshotShapeError; returns a copy in
 *  which every field the UI reads has the type it expects. */
export function sanitizeSnapshot(v: unknown): Snapshot {
  if (!isObj(v)) throw new SnapshotShapeError('not an object');
  if (!isNum(v.schema) || v.schema < 1) throw new SnapshotShapeError('no schema');
  const major = Math.floor(v.schema);
  if (major > SNAPSHOT_SCHEMA) throw new SnapshotShapeError(`format ${major}`, major);
  if (!isObj(v.usage)) throw new SnapshotShapeError('no usage');
  if (!isStr(v.generatedAt)) throw new SnapshotShapeError('no generatedAt');

  const o: Obj = { ...v };
  const device: Obj = isObj(v.device) ? { ...v.device } : {};
  if (!isStr(device.name) || !device.name.trim()) device.name = 'Main Mac';
  str(device, 'appVersion', 'drop');
  o.device = device;
  str(o, 'plan', 'drop');

  const u: Obj = { ...v.usage };
  for (const r of RANGES) {
    if (u[r] === undefined) continue;
    if (isObj(u[r])) u[r] = usage(u[r]);
    else delete u[r];
  }
  o.usage = u;

  for (const [k, codex] of [
    ['claudeQuota', false],
    ['codexQuota', true],
  ] as const) {
    if (o[k] !== undefined && o[k] !== null) o[k] = liveQuota(o[k], codex);
  }
  return o as unknown as Snapshot;
}
