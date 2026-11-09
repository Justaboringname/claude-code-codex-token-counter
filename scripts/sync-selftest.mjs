#!/usr/bin/env node
// Cloud-sync protocol selftest (SPEC §6/§7.8). Plain Node, no test framework:
//   node scripts/sync-selftest.mjs
// Imports src/sync/protocol.ts directly (Node ≥ 22.18 strips the types).
//
//   (a) §6 test vectors (pairing code, channel, token, encKey, AES-GCM "hello")
//   (b) seal → open round trip + rejection cases, snapshot validation
//       (newer schema refused, wrong types coerced, gzip bomb and oversize
//       payload refused), relative ages and error wording
//   (c) opens the Rust-made fixture (src-tauri/tests/fixtures/sync_envelope_vector.json,
//       or $TC_SYNC_FIXTURE) if present and deep-compares its JSON
//   (d) if $TC_SYNC_TEST_CODE is set: live tc_put of a demo snapshot to that
//       TEST channel, tc_get + open, compare; and a 401 check with a random code.
//
// Never prints a pairing code, the secret, the token or the key.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = new URL('..', import.meta.url);
const P = await import(new URL('src/sync/protocol.ts', root).href);
const { demoSnapshot } = await import(new URL('src/dev/demoSnapshot.ts', root).href);
const { relativeAge } = await import(new URL('src/sync/age.ts', root).href);
const { describeError, isRealSyncError } = await import(new URL('src/sync/describe.ts', root).href);

let passed = 0;
let skipped = 0;
const failures = [];
async function test(name, fn) {
  try {
    await fn();
    passed++;
    console.log(`  ok    ${name}`);
  } catch (e) {
    failures.push(name);
    console.log(`  FAIL  ${name}\n        ${String(e?.stack ?? e).split('\n').slice(0, 4).join('\n        ')}`);
  }
}
function skip(name, why) {
  skipped++;
  console.log(`  skip  ${name} (${why})`);
}
async function rejects(promiseOrFn, kind) {
  try {
    await (typeof promiseOrFn === 'function' ? promiseOrFn() : promiseOrFn);
  } catch (e) {
    assert.ok(P.isSyncError(e), `expected SyncError, got ${e}`);
    assert.equal(e.kind, kind, `expected kind ${kind}, got ${e.kind}: ${e.message}`);
    return e;
  }
  assert.fail(`expected SyncError(${kind}), but it resolved`);
}
const hex = (b) => Buffer.from(b).toString('hex');
const utf8 = (s) => new TextEncoder().encode(s);
const b64u = (s) => Buffer.from(s).toString('base64url');

// ---- §6 vectors ----
const V = {
  code: 'tc1.eyJ2IjoxLCJ1IjoiaHR0cHM6Ly9leGFtcGxlLnN1cGFiYXNlLmNvIiwiayI6InNiX3B1Ymxpc2hhYmxlX3Rlc3QiLCJzIjoiQUFFQ0F3UUZCZ2NJQ1FvTERBME9EeEFSRWhNVUZSWVhHQmthR3h3ZEhoOCJ9',
  url: 'https://example.supabase.co',
  apiKey: 'sb_publishable_test',
  channel: '0e8a2bff0cb4c93cd3e39875d39afdf0',
  token: 'sXBEM3Hi3FGry4uvVVu9X3DYi_HsyBYyg2pXQyVye5s',
  encKey: '18653618e6a8509563c53d8bfd2a0b3e715f14002cc1dce14fd22e781009669a',
  nonce: '000102030405060708090a0b',
  ctTag: 'f6bb5cb0871ede9f3d5f5de7245bcc101a85458c4f',
  envelope: 'AQABAgMEBQYHCAkKC/a7XLCHHt6fPV9d5yRbzBAahUWMTw==',
};
const K = Uint8Array.from({ length: 32 }, (_, i) => i);
const nonce = Uint8Array.from({ length: 12 }, (_, i) => i);

console.log('(a) §6 vectors');
const vk = await P.deriveKeys(V.code);
await test('parseCode reads url, key and the 32-byte secret', () => {
  const c = P.parseCode(V.code);
  assert.equal(c.url, V.url);
  assert.equal(c.apiKey, V.apiKey);
  assert.equal(hex(c.secret), hex(K));
});
await test('encodeCode reproduces the vector code', () => {
  assert.equal(P.encodeCode({ url: V.url, apiKey: V.apiKey, secret: K }), V.code);
});
await test('channel / token / encKey', () => {
  assert.equal(vk.channel, V.channel);
  assert.equal(vk.token, V.token);
  assert.equal(hex(vk.encKey), V.encKey);
});
await test('AES-GCM("hello") envelope with the fixed nonce', async () => {
  const env = await P.sealBytes(utf8('hello'), vk, nonce);
  assert.equal(env, V.envelope);
  const raw = Buffer.from(env, 'base64');
  assert.equal(raw[0], 1);
  assert.equal(hex(raw.subarray(1, 13)), V.nonce);
  assert.equal(hex(raw.subarray(13)), V.ctTag);
});
await test('openBytes(vector envelope) = "hello"', async () => {
  assert.equal(new TextDecoder().decode(await P.openBytes(V.envelope, vk)), 'hello');
});
await test('parseCode: whitespace, line wraps and unknown fields are fine', () => {
  const extra = 'tc1.' + b64u(JSON.stringify({ v: 1, u: V.url + '/', k: V.apiKey, s: b64u(K), note: 'x', z: [1] }));
  const wrapped = `  \n${extra.slice(0, 40)}\n${extra.slice(40)}  \t`;
  const c = P.parseCode(wrapped);
  assert.equal(c.url, V.url);
  assert.equal(hex(c.secret), hex(K));
  assert.equal(P.parseCode(`\n ${V.code} \n`).apiKey, V.apiKey);
});
await test('parseCode rejects bad codes with kind bad-code', async () => {
  const mk = (o) => 'tc1.' + b64u(JSON.stringify(o));
  const good = { v: 1, u: V.url, k: V.apiKey, s: b64u(K) };
  const cases = [
    '',
    'hello',
    V.code.replace('tc1.', 'tc2.'),
    'tc1.',
    'tc1.!!!!',
    V.code.slice(0, 60),
    mk({ ...good, v: 2 }),
    mk({ ...good, v: '1' }),
    mk({ u: V.url, k: V.apiKey, s: good.s }),
    mk({ ...good, u: undefined }),
    mk({ ...good, k: '' }),
    mk({ ...good, s: undefined }),
    mk({ ...good, s: b64u(K.subarray(0, 31)) }),
    mk({ ...good, s: b64u(new Uint8Array(33)) }),
    mk({ ...good, u: 'not a url' }),
    'tc1.' + b64u('[1,2]'),
  ];
  for (const c of cases) await rejects(() => P.parseCode(c), 'bad-code');
});

console.log('(b) envelope round trip');
const snap = demoSnapshot(new Date('2026-11-09T08:00:00Z'));
await test('seal → open returns the same snapshot', async () => {
  const env = await P.sealEnvelope(snap, vk);
  assert.deepEqual(await P.openEnvelope(env, vk), snap);
});
await test('every seal uses a fresh nonce', async () => {
  const a = Buffer.from(await P.sealEnvelope(snap, vk), 'base64').subarray(1, 13);
  const b = Buffer.from(await P.sealEnvelope(snap, vk), 'base64').subarray(1, 13);
  assert.notEqual(hex(a), hex(b));
});
await test('plaintext is gzip (RFC 1952 magic 1f 8b)', async () => {
  const plain = await P.openBytes(await P.sealEnvelope(snap, vk), vk);
  assert.equal(plain[0], 0x1f);
  assert.equal(plain[1], 0x8b);
});
await test('rejects: wrong key, other channel (AAD), tampering, version, truncation', async () => {
  const env = await P.sealEnvelope(snap, vk);
  const other = await P.deriveKeys(P.encodeCode({ url: V.url, apiKey: V.apiKey, secret: K.map((x) => x ^ 0xff) }));
  await rejects(P.openEnvelope(env, other), 'decrypt-failed');
  await rejects(P.openEnvelope(env, { ...vk, channel: other.channel }), 'decrypt-failed');
  const raw = Buffer.from(env, 'base64');
  const flip = Buffer.from(raw);
  flip[flip.length - 20] ^= 1;
  await rejects(P.openEnvelope(flip.toString('base64'), vk), 'decrypt-failed');
  const v2 = Buffer.from(raw);
  v2[0] = 2;
  await rejects(P.openEnvelope(v2.toString('base64'), vk), 'decrypt-failed');
  await rejects(P.openEnvelope(raw.subarray(0, 28).toString('base64'), vk), 'decrypt-failed');
  await rejects(P.openEnvelope('%%%not base64%%%', vk), 'decrypt-failed');
});
await test('non-gzip or non-snapshot plaintext → bad-snapshot', async () => {
  await rejects(P.openEnvelope(V.envelope, vk), 'bad-snapshot');
  const notSnap = await P.sealBytes(await P.gzip(utf8('{"hello":1}')), vk);
  await rejects(P.openEnvelope(notSnap, vk), 'bad-snapshot');
});
await test('missing optional fields are tolerated', async () => {
  const minimal = { schema: 1, generatedAt: snap.generatedAt, usage: { today: snap.usage.today } };
  const got = await P.openEnvelope(await P.sealEnvelope(minimal, vk), vk);
  assert.equal(got.device.name, 'Main Mac');
  assert.equal(got.usage.week, undefined);
});

await test('a newer schema major is refused with a clear message', async () => {
  for (const schema of [2, 3.0, 2.5]) {
    const e = await rejects(P.openEnvelope(await P.sealEnvelope({ ...snap, schema }, vk), vk), 'bad-snapshot');
    assert.equal(e.reason, 'newer-schema');
    assert.match(e.message, /newer Token Counter/);
    assert.match(describeError(e).long, /Update Token Counter here/);
  }
  // A minor bump of the same major still reads.
  assert.equal((await P.openEnvelope(await P.sealEnvelope({ ...snap, schema: 1.1 }, vk), vk)).schema, 1.1);
  for (const schema of [0, -1, '1', null]) {
    const e = await rejects(P.openEnvelope(await P.sealEnvelope({ ...snap, schema }, vk), vk), 'bad-snapshot');
    assert.equal(e.reason, null);
  }
});
await test('wrong-typed fields are coerced to what the UI expects', async () => {
  const today = structuredClone(snap.usage.today);
  Object.assign(today, {
    byTool: 1,
    sparkline: 'x',
    sparklineCodex: { a: 1 },
    totalCost: '12',
    totalTokens: null,
    deltaPct: 'up',
    rangeLabel: { x: 1 },
    sparkLeft: 5,
    priceHealth: { status: 'bogus' },
    codexQuota: 'nope',
  });
  today.byModel = { ok: today.byModel['claude-opus-5-5'], bad: 3, half: { in: '1', out: 2, cost: 'x' } };
  const poisoned = {
    ...structuredClone(snap),
    device: { name: 42, appVersion: 7 },
    plan: 9,
    usage: { today, week: 3, session: null, future: { kept: true } },
    claudeQuota: 7,
    codexQuota: { fiveHour: 'x', weekly: { usedPct: '9' }, status: 'weird', fetchedAt: 1, planType: 2, resetCredits: 'one' },
  };
  const got = await P.openEnvelope(await P.sealEnvelope(poisoned, vk), vk);
  const u = got.usage.today;
  assert.deepEqual(u.byTool, []);
  assert.deepEqual(u.sparkline, []);
  assert.equal('sparklineCodex' in u, false);
  assert.equal(u.totalCost, 0);
  assert.equal(u.totalTokens, 0);
  assert.equal(u.deltaPct, null);
  assert.equal(u.rangeLabel, '');
  assert.equal(u.sparkLeft, '');
  assert.equal('priceHealth' in u, false);
  assert.equal(u.codexQuota, null);
  assert.deepEqual(Object.keys(u.byModel).sort(), ['half', 'ok']);
  assert.deepEqual(u.byModel.ok, snap.usage.today.byModel['claude-opus-5-5']);
  assert.deepEqual(u.byModel.half, { in: 0, out: 2, cacheWrite: 0, cacheRead: 0 });
  assert.equal('week' in got.usage, false);
  assert.equal('session' in got.usage, false);
  assert.deepEqual(got.usage.future, { kept: true });
  assert.deepEqual(got.device, { name: 'Main Mac' });
  assert.equal('plan' in got, false);
  assert.equal(got.claudeQuota, null);
  assert.deepEqual(got.codexQuota, {
    fiveHour: null,
    weekly: { usedPct: 0, resetsAt: null },
    status: 'error',
    fetchedAt: '',
    planType: null,
    resetCredits: null,
  });
  // What PopoverContent does with a range must not throw now.
  [...(u.byTool ?? [])].sort((a, b) => b.calls - a.calls);
  Object.entries(u.byModel ?? {}).map(([, m]) => m.in + m.out + m.cacheRead);
  Math.max(...(u.sparkline ?? []), 0.0001);
});
await test('valid data comes back unchanged (no keys added, none lost)', async () => {
  const minimal = { schema: 1, generatedAt: snap.generatedAt, device: { name: 'Mac Studio' }, usage: {} };
  assert.deepEqual(P.asSnapshot(structuredClone(minimal)), minimal);
  assert.deepEqual(P.asSnapshot(structuredClone(snap)), snap);
});
await test('a gzip bomb stops at the unpacked-size cap (bad-snapshot)', async () => {
  const bomb = await P.gzip(new Uint8Array(P.MAX_SNAPSHOT_BYTES + 8 * 1024 * 1024));
  assert.ok(bomb.length < 200_000, `bomb is ${bomb.length} bytes`);
  const e = await rejects(P.openEnvelope(await P.sealBytes(bomb, vk), vk), 'bad-snapshot');
  assert.match(e.message, /unpacks to more than 32 MB/);
});
await test('a payload over the server limit is refused before decoding', async () => {
  const e = await rejects(P.openEnvelope('A'.repeat(P.MAX_PAYLOAD_CHARS + 4), vk), 'bad-snapshot');
  assert.match(e.message, /larger than the server allows/);
});
await test('relativeAge picks the unit from the rounded minutes', () => {
  const t0 = 10_000_000_000;
  const at = (s) => relativeAge(t0 - s * 1000, t0);
  assert.equal(at(30), 'just now');
  assert.equal(at(60), '1 min ago');
  assert.equal(at(3569), '59 min ago');
  for (const s of [3570, 3590, 3599, 3600, 5399]) assert.equal(at(s), '1 h ago', `${s} s`);
  assert.equal(at(7170), '2 h ago');
  assert.equal(at(48 * 3600 - 10), '2 days ago');
  assert.equal(at(47 * 3600), '47 h ago');
});
await test('error wording: no "last copy" without a snapshot; empty is not an error', () => {
  const net = new P.SyncError('network', "Can't reach the sync server.");
  assert.match(describeError(net, null, true).long, /Showing the last copy\.$/);
  assert.doesNotMatch(describeError(net, null, false).long, /last copy/);
  assert.doesNotMatch(describeError(net).long, /last copy/);
  assert.equal(isRealSyncError(new P.SyncError('empty', 'x')), false);
  assert.equal(isRealSyncError(net), true);
  assert.equal(isRealSyncError(null), false);
});

console.log('(c) Rust fixture');
const fixturePath = process.env.TC_SYNC_FIXTURE
  ? process.env.TC_SYNC_FIXTURE
  : fileURLToPath(new URL('src-tauri/tests/fixtures/sync_envelope_vector.json', root));
if (existsSync(fixturePath)) {
  await test(`opens ${fixturePath.replace(fileURLToPath(root), '')} and matches its JSON`, async () => {
    const f = JSON.parse(readFileSync(fixturePath, 'utf8'));
    const keys = await P.deriveKeys(f.code);
    const plain = await P.gunzip(await P.openBytes(f.payload, keys));
    assert.deepEqual(JSON.parse(new TextDecoder().decode(plain)), f.json);
    // Validation leaves real Rust output untouched.
    assert.deepEqual(await P.openEnvelope(f.payload, keys), f.json);
  });
} else {
  skip('Rust fixture', `${fixturePath} not found`);
}

console.log('(d) live round trip');
const live = process.env.TC_SYNC_TEST_CODE?.trim();
if (live) {
  const keys = await P.deriveKeys(live);
  const host = new URL(keys.url).host.replace(/^[^.]+/, '…');
  await test(`tc_head on the test channel (${host})`, async () => {
    const h = await P.head(keys);
    assert.equal(typeof h.version, 'number');
    console.log(`        version ${h.version}, updated ${h.updatedAt ?? 'never'}`);
  });
  let uploaded;
  let version;
  await test('tc_put a demo snapshot', async () => {
    uploaded = demoSnapshot(new Date(), process.env.TC_SYNC_DEVICE || 'Mac Studio');
    const env = await P.sealEnvelope(uploaded, keys);
    version = await P.put(keys, env, uploaded.generatedAt);
    assert.ok(Number.isInteger(version) && version > 0, `version ${version}`);
    console.log(`        → version ${version}, ${env.length} chars`);
  });
  await test('tc_head sees the new version', async () => {
    const h = await P.head(keys);
    assert.equal(h.version, version);
    assert.equal(Date.parse(h.generatedAt), Date.parse(uploaded.generatedAt));
  });
  await test('tc_get + open returns exactly what was uploaded', async () => {
    const { snapshot, version: v } = await P.fetchSnapshot(keys);
    assert.equal(v, version);
    assert.deepEqual(snapshot, uploaded);
  });
  await test('an unregistered code gets forbidden (401)', async () => {
    const stranger = await P.deriveKeys(
      P.encodeCode({ url: keys.url, apiKey: keys.apiKey, secret: crypto.getRandomValues(new Uint8Array(32)) }),
    );
    const e = await rejects(P.head(stranger), 'forbidden');
    assert.equal(e.status, 401);
  });
  await test('CORS preflight allows apikey + content-type (browser path)', async () => {
    const res = await fetch(`${keys.url}/rest/v1/rpc/tc_head`, {
      method: 'OPTIONS',
      headers: {
        Origin: 'https://example.github.io',
        'Access-Control-Request-Method': 'POST',
        'Access-Control-Request-Headers': 'apikey,content-type',
      },
    });
    const allowOrigin = res.headers.get('access-control-allow-origin');
    const allowHeaders = (res.headers.get('access-control-allow-headers') ?? '').toLowerCase();
    assert.ok(res.status < 300, `preflight status ${res.status}`);
    assert.ok(allowOrigin === '*' || allowOrigin === 'https://example.github.io', `allow-origin ${allowOrigin}`);
    assert.ok(allowHeaders === '*' || (allowHeaders.includes('apikey') && allowHeaders.includes('content-type')), `allow-headers ${allowHeaders}`);
  });
} else {
  skip('live round trip', 'TC_SYNC_TEST_CODE not set');
}

console.log(`\n${passed} passed, ${failures.length} failed, ${skipped} skipped`);
if (failures.length) process.exit(1);
