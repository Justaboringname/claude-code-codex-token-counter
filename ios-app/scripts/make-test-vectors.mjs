#!/usr/bin/env node
// Parity vectors for the iOS app's unit tests, computed by the web app's own
// TypeScript (Node strips the types):
//   node ios-app/scripts/make-test-vectors.mjs
// writes ios-app/Tests/Fixtures/vectors.json. Test keys only (K = 0x00…0x1f).
import { deflateRawSync, gzipSync, crc32 } from 'node:zlib';
import { writeFileSync } from 'node:fs';
import * as P from '../../src/sync/protocol.ts';
import { sanitizeSnapshot } from '../../src/sync/snapshot.ts';
import { relativeAge } from '../../src/sync/age.ts';
import * as F from '../../src/format.ts';
import { demoSnapshot } from '../../src/dev/demoSnapshot.ts';

const out = {};
const enc = new TextEncoder();
const hex = (b) => Buffer.from(b).toString('hex');
const b64u = (b) => Buffer.from(b).toString('base64url');
const K = Uint8Array.from({ length: 32 }, (_, i) => i);
const code = P.encodeCode({ url: 'https://example.supabase.co', apiKey: 'sb_publishable_test', secret: K });
const keys = await P.deriveKeys(code);
out.keys = { code, channel: keys.channel, token: keys.token, encKeyHex: hex(keys.encKey) };

// ---- pairing codes ----
const mk = (obj) => 'tc1.' + b64u(enc.encode(JSON.stringify(obj)));
const good = { v: 1, u: 'https://example.supabase.co', k: 'sb_publishable_test', s: b64u(K) };
const inputs = [
  code,
  `  ${code.slice(0, 20)}\n ${code.slice(20)}\t`,
  code + '==',
  mk({ ...good, u: 'https://example.supabase.co///' }),
  mk({ ...good, extra: [1, 2] }),
  mk({ ...good, u: 'http://localhost:54321' }),
  'tc2.' + code.slice(4),
  'tc1.%%%',
  'tc1.' + b64u(enc.encode('[1,2]')),
  'tc1.' + b64u(enc.encode('not json')),
  mk({ ...good, v: 2 }),
  mk({ ...good, v: true }),
  mk({ ...good, v: '1' }),
  mk({ ...good, u: '' }),
  mk({ ...good, u: 'ftp://example.com' }),
  mk({ ...good, u: 'not a url' }),
  mk({ ...good, k: '' }),
  mk({ ...good, s: b64u(K.slice(0, 31)) }),
  mk({ ...good, s: 'a+b/' }),
  mk({ v: 1, u: 'https://x.supabase.co', k: 'k' }),
  '',
];
out.parseCode = inputs.map((input) => {
  try {
    const c = P.parseCode(input);
    return { input, ok: true, url: c.url, apiKey: c.apiKey, secretHex: hex(c.secret) };
  } catch (e) {
    return { input, ok: false, kind: e.kind, message: e.message };
  }
});

// ---- extractCode (pairing.ts; reimplemented there without DOM deps) ----
const extractCode = (raw) => {
  const s = (raw ?? '').replace(/\s+/g, '');
  const m = s.match(/[#&?]pair=([^&#]+)/);
  if (!m) return s;
  try {
    return decodeURIComponent(m[1]);
  } catch {
    return m[1];
  }
};
out.extractCode = [
  code,
  `https://tokens.example.com/#pair=${code}`,
  `https://tokens.example.com/?x=1&pair=${encodeURIComponent(code)}#y`,
  ` https://a.b/#pair=${code}&other=1 `,
  'https://a.b/#pair=%E0%A4%A',
  'hello world',
].map((input) => ({ input, output: extractCode(input) }));

// ---- envelopes ----
const nonce = Uint8Array.from({ length: 12 }, (_, i) => 100 + i);
const demo = demoSnapshot(new Date('2026-11-09T12:00:00Z'));
out.envelope = {
  nonceHex: hex(nonce),
  payload: await P.sealEnvelope(demo, keys, nonce),
  json: demo,
  sanitized: sanitizeSnapshot(demo),
};
out.helloEnvelope = await P.sealBytes(enc.encode('hello'), keys, Uint8Array.from({ length: 12 }, (_, i) => i));
// The same snapshot under another key: must fail as decrypt-failed.
const otherKeys = await P.deriveKeys({ url: 'https://example.supabase.co', apiKey: 'k', secret: Uint8Array.from({ length: 32 }, () => 7) });
out.wrongKeyPayload = await P.sealEnvelope(demo, otherKeys, nonce);
out.newerSchemaPayload = await P.sealEnvelope({ ...demo, schema: 2 }, keys, nonce);
out.notGzipPayload = await P.sealBytes(enc.encode('{"schema":1}'), keys, nonce);

// ---- gzip framing variants (base64 of whole members) ----
const body = enc.encode(JSON.stringify(demo));
const raw = deflateRawSync(body);
const trailer = (data) => {
  const t = Buffer.alloc(8);
  t.writeUInt32LE(crc32(data) >>> 0, 0);
  t.writeUInt32LE(data.length >>> 0, 4);
  return t;
};
function member(data, flags, { extra, name, comment } = {}) {
  const parts = [Buffer.from([0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 3])];
  if (flags & 4) {
    const x = Buffer.from(extra);
    const l = Buffer.alloc(2);
    l.writeUInt16LE(x.length);
    parts.push(l, x);
  }
  if (flags & 8) parts.push(Buffer.from(name + '\0', 'latin1'));
  if (flags & 16) parts.push(Buffer.from(comment + '\0', 'latin1'));
  if (flags & 2) {
    const h = Buffer.concat(parts);
    const c = Buffer.alloc(2);
    c.writeUInt16LE(crc32(h) & 0xffff);
    parts.push(c);
  }
  parts.push(deflateRawSync(data), trailer(data));
  return Buffer.concat(parts);
}
const half = body.length >> 1;
const badCrc = Buffer.from(gzipSync(body));
badCrc[badCrc.length - 8] ^= 0xff;
out.gzip = {
  expectedB64: Buffer.from(body).toString('base64'),
  plain: Buffer.from(gzipSync(body)).toString('base64'),
  allFields: member(body, 4 | 8 | 16 | 2, { extra: 'xy', name: 'snapshot.json', comment: 'hi' }).toString('base64'),
  twoMembers: Buffer.concat([member(body.slice(0, half), 0), member(body.slice(half), 8, { name: 'b' })]).toString('base64'),
  badCrc: badCrc.toString('base64'),
  truncated: Buffer.from(gzipSync(body)).subarray(0, 40).toString('base64'),
  junkAfter: Buffer.concat([Buffer.from(gzipSync(body)), Buffer.from('junk')]).toString('base64'),
  // 33 MB of zeros: over the 32 MB cap.
  bomb: Buffer.from(gzipSync(Buffer.alloc(33 * 1024 * 1024))).toString('base64'),
  rawDeflateLength: raw.length,
};

// ---- snapshot validation ----
out.sanitize = [
  { schema: 1, generatedAt: 'x', usage: { today: { byModel: { a: { in: '1', out: 2, cacheWrite: null, cacheRead: true, cost: 'x' }, b: 3 }, byTool: [{ name: 'Read', calls: '4' }, { calls: 1 }, 5], sparkline: [1, 'a', null, 2], sparklineCodex: 'nope', totalCost: 'NaN', deltaPct: 'x', rangeLabel: 7 }, week: 3 }, claudeQuota: { status: 'weird', fiveHour: 5, weekly: { usedPct: 'x' } }, device: { name: '   ' } },
  { schema: 1.9, generatedAt: '2026-11-09T00:00:00Z', usage: {}, device: { name: 'Mac Studio', appVersion: 3 }, plan: 5 },
].map((input) => ({ input, output: sanitizeSnapshot(input) }));
out.sanitizeRejects = [null, [], { schema: 0, usage: {}, generatedAt: 'x' }, { schema: 1, generatedAt: 'x' }, { schema: 1, usage: {} }, { schema: true, usage: {}, generatedAt: 'x' }].map((input) => {
  try {
    sanitizeSnapshot(input);
    return { input, ok: true };
  } catch (e) {
    return { input, ok: false, newerSchema: e.newerSchema ?? null, message: e.message };
  }
});

// ---- relative age ----
const now = Date.parse('2026-11-09T12:00:00Z');
out.relativeAge = [0, 10, 44.9, 45, 89, 90, 600, 3569, 3570, 3600, 7199, 7200, 86400, 172799, 172800, 400000, -50].map((s) => ({
  seconds: s,
  text: relativeAge(now - s * 1000, now),
}));

// ---- format.ts ----
const moneys = [0, 0.004, 0.005, 1.005, 2.675, 9.999, 26.158, 999.994, 999.995, 1000, 1234.5, 16551.49, 1e6];
out.fmtMoney = moneys.map((n) => ({ n, text: F.fmtMoney(n) }));
out.fmtMoney0 = moneys.map((n) => ({ n, text: F.fmtMoney(n, 0) }));
out.fmtTokens = [0, 0.0004, 0.0005, 0.5, 0.9995, 1, 1.5, 999.4, 999.5, 1000, 27265.8, 20951392.8].map((k) => ({ k, text: F.fmtTokens(k) }));
out.fmtCredits = [0, 42.5, 999, 1000, 9999, 10000, 412345, 999999, 1000000, 3312345].map((n) => ({ n, text: F.fmtCredits(n) }));
out.modelMeta = [
  'claude-opus-5-5', 'claude-fable-5-1', 'claude-opus-4-8-fast', 'claude-sonnet-5', 'claude-haiku-5-5', 'claude-mythos-5-1',
  'claude-cowork', 'claude-duet', 'claude-chat', 'gpt-6-astra', 'gpt-5.5-codex', 'gpt-5.6-sol-long-fast', 'gpt-5.4-mini',
  'codex-auto-review', 'gpt-daybreak-ember', 'gpt-5-codex-max-flex', 'o3', 'codex',
].map((id) => ({ id, ...F.modelMeta(id), isCodex: F.isCodexModel(id) }));
out.liftForDark = ['#C86F49', '#265A94', '#D97757', 'bad'].map((h) => ({ hex: h, out: F.liftForDark(h) }));
out.displayToolName = ['Read', 'shell', 'apply_patch', 'mcp__Claude_Browser__navigate', 'mcp__x', 'Bash'].map((n) => ({ name: n, out: F.displayToolName(n) }));
out.costForModel = [
  ['claude-opus-5-5', { in: 1, out: 2, cacheWrite: 3, cacheRead: 4 }],
  ['claude-opus-5-5', { in: 1, out: 2, cacheWrite: 3, cacheRead: 4, cost: 9.5 }],
  ['gpt-6-astra', { in: 10, out: 1, cacheWrite: 0, cacheRead: 100 }],
].map(([id, u]) => ({ id, u, cost: F.costForModel(id, u) }));

// ---- toFixed (exact ties round away from zero in JS, to even in printf) ----
out.toFixed = [
  [0.5, 0], [1.5, 0], [2.5, 0], [12.5, 0], [0.125, 2], [0.375, 2], [1.25, 1], [12.625, 2], [-0.4, 0], [-0.5, 0], [-2.5, 0],
  [-12.5, 0], [1.005, 2], [2.675, 2], [0, 0], [0, 2], [99.95, 1], [0.045, 2], [1e-7, 2], [123456.789, 1], [66.8, 0], [8.5, 0],
  [999.995, 2], [0.05, 1], [0.25, 1], [0.75, 1], [26.158, 2],
].map(([x, d]) => ({ x, d, text: x.toFixed(d) }));

// ---- what the screen shows (PopoverContent.tsx) ----
// Those helpers are locals of a React file, so they are copied here verbatim,
// with Date.now() turned into a `now` argument.
const { describeError, isRealSyncError } = await import('../../src/sync/describe.ts');
const { readFileSync } = await import('node:fs');
const CANONICAL_TOOLS = ['Read', 'Edit', 'Bash', 'Grep', 'WebFetch', 'Glob'];
const STALE_SNAPSHOT_MS = 15 * 60_000;
function models(usage, isDark) {
  if (!usage) return [];
  return Object.entries(usage.byModel ?? {})
    .map(([id, u]) => {
      const meta = F.modelMeta(id);
      return { id, ...meta, color: isDark ? F.liftForDark(meta.color) : meta.color, usage: u, cost: F.costForModel(id, u) };
    })
    .filter((m) => m.cost > 0)
    .sort((a, b) => b.cost - a.cost)
    .slice(0, 10);
}
function modelRows(usage, isDark) {
  const totalCost = usage?.totalCost ?? 0;
  return models(usage, isDark).map((m) => {
    const pct = totalCost > 0 ? (m.cost / totalCost) * 100 : 0;
    const detail = [];
    if (m.usage.in + m.usage.out > 0) detail.push(`in ${F.fmtTokens(m.usage.in)}`, `out ${F.fmtTokens(m.usage.out)}`);
    if (m.usage.cacheRead > 0) detail.push(`cache ${F.fmtTokens(m.usage.cacheRead)}`);
    return { id: m.id, name: m.name, color: m.color, costText: F.fmtMoney(m.cost), pctText: `${pct.toFixed(0)}%`, pct, detail };
  });
}
function toolTiles(usage) {
  const used = [...(usage?.byTool ?? [])].sort((a, b) => b.calls - a.calls);
  const usedNames = new Set(used.map((t) => t.name));
  const filler = CANONICAL_TOOLS.filter((n) => !usedNames.has(n)).map((n) => ({ name: n, calls: 0, tokens: 0 }));
  return [...used, ...filler].slice(0, 6);
}
const iconBySemantic = (name) => {
  const n = name.toLowerCase();
  if (n === 'read') return 'read';
  if (n === 'edit' || n === 'write' || n === 'multiedit') return 'edit';
  if (n === 'bash' || n === 'bashoutput' || n === 'killshell') return 'bash';
  if (n === 'webfetch' || n === 'websearch') return 'web';
  return 'search';
};
const clampPct = (pct) => Math.min(100, Math.max(0, pct));
const liveEpochPct = (resetsAt, pct, now) => (resetsAt !== null && resetsAt * 1000 <= now ? 0 : clampPct(pct));
function liveIsoPct(window, now) {
  if (!window) return null;
  const resetMs = window.resetsAt ? Date.parse(window.resetsAt) : Number.NaN;
  return Number.isFinite(resetMs) && resetMs <= now ? 0 : clampPct(window.usedPct);
}
function limits(snapshot, usage, now) {
  const cq = snapshot.claudeQuota ?? null;
  const xq = snapshot.codexQuota ?? null;
  const claude =
    cq && (cq.fiveHour || cq.weekly)
      ? { five: liveIsoPct(cq.fiveHour, now), weekly: liveIsoPct(cq.weekly, now), staleAsOf: cq.status !== 'ok' ? cq.fetchedAt : null }
      : null;
  const live = xq && (xq.fiveHour || xq.weekly) ? xq : null;
  const log = usage?.codexQuota ?? null;
  const codex = live
    ? { five: liveIsoPct(live.fiveHour, now), weekly: liveIsoPct(live.weekly, now), staleAsOf: live.status !== 'ok' ? live.fetchedAt : null }
    : log
      ? {
          five: log.primaryWindowMinutes ? liveEpochPct(log.primaryResetsAt, log.primaryUsedPct, now) : null,
          weekly: log.secondaryWindowMinutes ? liveEpochPct(log.secondaryResetsAt, log.secondaryUsedPct, now) : null,
          staleAsOf: null,
        }
      : null;
  return { claude, codex };
}
function freshness(snapshot, error, now) {
  const gen = Date.parse(snapshot.generatedAt);
  const name = snapshot.device.name;
  const stale = Number.isFinite(gen) && now - gen > STALE_SNAPSHOT_MS;
  const age = Number.isFinite(gen) ? relativeAge(gen, now) : 'at an unknown time';
  const err = isRealSyncError(error) ? describeError(error, name, true) : null;
  const lead = stale ? name : `From ${name}`;
  const tail = stale ? ` last synced ${age}` : ` · updated ${age}`;
  return { text: `${lead}${tail}${err ? ` · ${err.short}` : ''}`, alert: stale || !!err };
}
function fmtAge(iso, now) {
  const ms = iso ? now - Date.parse(iso) : Number.NaN;
  if (!Number.isFinite(ms) || ms < 0) return null;
  const h = ms / 3_600_000;
  if (h < 1) return `${Math.max(1, Math.round(ms / 60_000))}m`;
  if (h < 48) return `${Math.floor(h)}h`;
  return `${Math.floor(h / 24)}d`;
}
function priceNote(h, now) {
  if (!h || h.status === 'fresh') return null;
  const age = fmtAge(h.fetchedAt, now);
  switch (h.status) {
    case 'builtin':
      return { text: 'Built-in prices', warn: !!h.lastError };
    case 'error':
      return { text: 'Price update failed', warn: true };
    case 'aging':
    case 'stale':
      return { text: age ? `Prices ${age} old` : 'Prices out of date', warn: h.status === 'stale' };
  }
  return null;
}
function screen(snapshot, now) {
  const ranges = {};
  for (const r of ['session', 'today', 'week', 'month', 'all']) {
    const usage = snapshot.usage?.[r] ?? null;
    const tiles = toolTiles(usage);
    const codex = usage?.sparklineCodex;
    ranges[r] = {
      present: !!usage,
      heroLabel: `${usage?.rangeLabel ?? '—'} spend`.toUpperCase(),
      money: F.fmtMoney(usage?.totalCost ?? 0),
      tokens: `${F.fmtTokens(usage?.totalTokens ?? 0)} tokens`,
      delta:
        usage?.deltaPct !== null && usage?.deltaPct !== undefined
          ? { up: usage.deltaPct >= 0, text: `${usage.deltaPct >= 0 ? '+' : ''}${usage.deltaPct.toFixed(0)}%` }
          : null,
      codexOverlay: !!codex?.some((v) => v > 0),
      sparkCount: (usage?.sparkline ?? new Array(14).fill(0)).length,
      light: modelRows(usage, false),
      dark: modelRows(usage, true),
      tiles: tiles.map((t) => ({ name: t.name, label: F.displayToolName(t.name), calls: t.calls, icon: iconBySemantic(t.name), color: F.toolColor(t.name) })),
      toolTotal: tiles.reduce((s, t) => s + t.calls, 0),
      limits: limits(snapshot, usage, now),
      priceNote: priceNote(usage?.priceHealth, now),
    };
  }
  return ranges;
}
const rust = JSON.parse(readFileSync(new URL('../../src-tauri/tests/fixtures/sync_envelope_vector.json', import.meta.url), 'utf8')).json;
const demoNow = Date.parse('2026-11-09T12:00:00Z');
// Edge cases the fixtures don't reach: ties, cost-only rows, MCP tools, a
// rolled-over window, a stale quota, a log-only Codex quota, price notes.
const edge = {
  schema: 1,
  generatedAt: '2026-11-09T11:40:00Z',
  device: { name: 'Mac Studio' },
  usage: {
    today: {
      rangeLabel: 'Today', totalCost: 8, totalTokens: 2.5, deltaPct: -0.4, sparkline: [], sparkLeft: '12am', sparkRight: 'now',
      byModel: {
        'claude-opus-5-5': { in: 2.5, out: 0.0004, cacheWrite: 9, cacheRead: 1250, cost: 1 },
        'claude-cowork': { in: 0, out: 0, cacheWrite: 0, cacheRead: 0, cost: 5 },
        'gpt-6-astra': { in: 1, out: 1, cacheWrite: 0, cacheRead: 0 },
        'claude-haiku-5-5': { in: 5, out: 5, cacheWrite: 0, cacheRead: 0, cost: 0 },
        'gpt-5.5-codex': { in: 999.5, out: 0.9995, cacheWrite: 0, cacheRead: 999999, cost: 1999.995 },
      },
      byTool: [{ name: 'mcp__Claude_Browser__navigate', calls: 12345, tokens: 1 }, { name: 'shell', calls: 7, tokens: 0 }, { name: 'Read', calls: 7, tokens: 0 }],
      codexQuota: { asOf: '2026-11-09T11:00:00Z', primaryUsedPct: 12.5, primaryWindowMinutes: 300, primaryResetsAt: 1794229200, secondaryUsedPct: 140, secondaryWindowMinutes: 0, secondaryResetsAt: null, planType: 'plus' },
      priceHealth: { status: 'stale', fetchedAt: '2026-11-06T11:00:00Z', lastError: 'boom', lastErrorAt: '2026-11-09T11:30:00Z' },
    },
    week: {
      rangeLabel: 'This week', totalCost: 0, totalTokens: 0, deltaPct: null, sparkline: [0, 0], sparklineCodex: [0, 0.01],
      byModel: {}, byTool: [],
      codexQuota: { asOf: 'x', primaryUsedPct: 50, primaryWindowMinutes: 300, primaryResetsAt: 1794225000, secondaryUsedPct: -5, secondaryWindowMinutes: 10080, secondaryResetsAt: null, planType: null },
      priceHealth: { status: 'builtin', fetchedAt: null, lastError: null, lastErrorAt: null },
    },
    month: { rangeLabel: 'This month', totalCost: 1, totalTokens: 1, deltaPct: 0, sparkline: [1], byModel: {}, byTool: [], priceHealth: { status: 'aging', fetchedAt: '2026-11-09T11:30:00Z', lastError: null, lastErrorAt: null } },
    all: { rangeLabel: 'All-time', totalCost: 1, totalTokens: 1, deltaPct: 12.5, sparkline: [1], byModel: {}, byTool: [], priceHealth: { status: 'error', fetchedAt: 'garbage', lastError: 'x', lastErrorAt: null } },
  },
  claudeQuota: { fiveHour: { usedPct: 62.5, resetsAt: '2026-11-09T11:50:00.612317+00:00' }, weekly: null, status: 'stale', message: 'x', fetchedAt: '2026-11-08T03:12:00Z', subscription: 'max' },
  codexQuota: { fiveHour: null, weekly: null, status: 'ok', message: null, fetchedAt: '2026-11-09T11:40:00Z', planType: 'plus', resetCredits: null },
};
const edgeClean = sanitizeSnapshot(edge);
out.screens = [
  { name: 'demo', json: demo, now: demoNow, ranges: screen(sanitizeSnapshot(demo), demoNow) },
  { name: 'rust', json: rust, now: Date.parse('2026-11-09T08:05:00Z'), ranges: screen(sanitizeSnapshot(rust), Date.parse('2026-11-09T08:05:00Z')) },
  { name: 'rust-later', json: rust, now: Date.parse('2026-11-12T08:05:00Z'), ranges: screen(sanitizeSnapshot(rust), Date.parse('2026-11-12T08:05:00Z')) },
  { name: 'edge', json: edge, now: demoNow, ranges: screen(edgeClean, demoNow) },
];
const E = (kind, message, reason = null) => ({ kind, message, status: null, reason });
out.freshness = [
  [demo, null, 0], [demo, null, 14.9 * 60], [demo, null, 15 * 60], [demo, null, 15 * 60 + 1], [demo, null, 3 * 3600],
  [demo, E('empty', 'Nothing has been uploaded yet.'), 60],
  [demo, E('network', "Can't reach the sync server."), 60],
  [demo, E('forbidden', 'The sync server rejected this pairing code.'), 20 * 60],
  [demo, E('decrypt-failed', 'x'), 60], [demo, E('bad-snapshot', 'x', 'newer-schema'), 60], [demo, E('bad-snapshot', 'x'), 60],
  [{ ...demo, generatedAt: 'garbage' }, null, 60],
].map(([snap, error, afterSec]) => {
  const s = sanitizeSnapshot(snap);
  const gen = Date.parse(demo.generatedAt);
  return { generatedAt: s.generatedAt, error, now: gen + afterSec * 1000, ...freshness(s, error, gen + afterSec * 1000) };
});
out.describeError = [
  [E('network', "Can't reach the sync server."), null, false], [E('network', 'The sync server answered 500.'), 'Mac Studio', true],
  [E('empty', 'x'), null, false], [E('empty', 'x'), 'Mac Studio', false], [E('forbidden', 'The sync server rejected this pairing code.'), null, false],
  [E('decrypt-failed', 'x'), null, false], [E('bad-code', 'Not a Token Counter pairing code (no payload).'), null, false],
  [E('bad-snapshot', 'The main Mac runs a newer Token Counter (data format 2).', 'newer-schema'), null, true], [E('bad-snapshot', 'y'), null, true],
].map(([error, deviceName, hasSnapshot]) => ({ error, deviceName, hasSnapshot, ...describeError(error, deviceName, hasSnapshot) }));
out.fmtAge = ['2026-11-09T11:59:50Z', '2026-11-09T11:58:30Z', '2026-11-09T11:00:01Z', '2026-11-09T11:00:00Z', '2026-11-07T12:00:01Z', '2026-11-07T12:00:00Z', '2026-11-01T00:00:00Z', '2026-11-09T12:00:01Z', 'garbage', null].map((iso) => ({ iso, text: fmtAge(iso, demoNow) }));

writeFileSync(new URL('../Tests/Fixtures/vectors.json', import.meta.url), JSON.stringify(out, null, 1) + '\n');
console.log('wrote ios-app/Tests/Fixtures/vectors.json');
