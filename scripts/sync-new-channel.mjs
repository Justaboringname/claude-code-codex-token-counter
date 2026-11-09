#!/usr/bin/env node
// Create a new sync channel for a self-hosted Supabase backend (docs/SYNC.md).
//
//   node scripts/sync-new-channel.mjs <supabase-url> <publishable-key> <code-file>
//
// Writes the pairing code (a secret) to <code-file> with mode 0600 and prints
// only the SQL that registers the channel; run that SQL as the project owner.
import { webcrypto as crypto } from 'node:crypto';
import { writeFileSync, chmodSync, existsSync } from 'node:fs';

const [url, key, out] = process.argv.slice(2);
if (!url || !key || !out) {
  console.error('usage: node scripts/sync-new-channel.mjs <supabase-url> <publishable-key> <code-file>');
  process.exit(2);
}
if (!/^https:\/\//.test(url)) throw new Error('supabase url must be https://');
if (existsSync(out)) throw new Error(`${out} already exists; refusing to overwrite a pairing code`);

const enc = new TextEncoder();
const b64u = (b) => Buffer.from(b).toString('base64url');
const sha = async (b) => new Uint8Array(await crypto.subtle.digest('SHA-256', b));
const cat = (a, b) => { const o = new Uint8Array(a.length + b.length); o.set(a); o.set(b, a.length); return o; };

const K = crypto.getRandomValues(new Uint8Array(32));
const code = 'tc1.' + b64u(enc.encode(JSON.stringify({ v: 1, u: url.replace(/\/+$/, ''), k: key, s: b64u(K) })));
const channel = Buffer.from((await sha(cat(enc.encode('tc-sync/channel/v1'), K))).slice(0, 16)).toString('hex');
const token = b64u(await sha(cat(enc.encode('tc-sync/auth/v1'), K)));
const tokenHash = Buffer.from(await sha(enc.encode(token))).toString('hex');

writeFileSync(out, code + '\n', { mode: 0o600 });
chmodSync(out, 0o600);
console.log(`pairing code written to ${out} (keep it secret)\n`);
console.log(`insert into public.tc_channels (channel, token_hash, label)\nvalues ('${channel}', '${tokenHash}', 'my main Mac');`);
