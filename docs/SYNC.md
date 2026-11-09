# Cloud sync and the phone web app

Last updated: 2026-11-09

Token Counter can publish what it shows from one **main Mac** so that other Macs and a phone can see the
same numbers:

- **Main Mac** (role *This Mac uploads*): computes usage exactly as before and, about once a minute,
  uploads an **end-to-end encrypted** snapshot of every range (Session / Today / Week / Month / All) plus the
  official Claude and Codex limit rows.
- **Other Macs** (role *Show another Mac*): show the main Mac's snapshot by default, with a toggle back to
  *This Mac*'s own local numbers.
- **Phone web app**: a small home-screen web app (iPhone Safari → Share → Add to Home Screen) that renders the
  same snapshot with the same UI.

The server only ever stores ciphertext. Nothing changes for anyone who leaves sync off.

## Privacy model

- One secret **pairing code** per account: `tc1.` + base64url JSON `{v, u, k, s}` where `u`/`k` are the
  Supabase URL and its *publishable* key, and `s` is 32 random bytes `K`. Keep the code secret — whoever has it
  can read (and write) the snapshot.
- From `K` the clients derive three independent values (SHA-256 with distinct labels):
  `channel` (row id, 16 bytes hex), `token` (bearer secret; the server stores only its SHA-256) and
  `encKey` (AES-256 key, never leaves the devices).
- Payload: gzip(JSON) sealed with AES-256-GCM, fresh random 96-bit nonce per upload, AAD bound to the channel.
- The database table is not reachable with the public key at all (RLS on, no policies, privileges revoked).
  Clients call three `SECURITY DEFINER` RPCs (`tc_head`, `tc_get`, `tc_put`) that check the token first.
  Channels can only be created by the project owner, so the public key cannot be used to create rows.
- The phone web app is served with a strict Content-Security-Policy (scripts only from itself, network only to
  `*.supabase.co`), `no-referrer`, `nosniff`, no framing. The pairing code is kept in the browser's local storage
  on that device only; a `#pair=` link fragment is never sent to the web server.

## Setting up your own backend

1. Create a Supabase project (the free plan is enough) and run
   [`supabase/migrations/20261109000000_tc_sync.sql`](../supabase/migrations/20261109000000_tc_sync.sql)
   in the SQL editor.
2. Make a pairing code and register its channel:
   ```bash
   node scripts/sync-new-channel.mjs https://<project>.supabase.co <publishable key> ~/tc-pairing-code.txt
   ```
   It writes the code to the file (mode 0600) and prints an `insert` statement — run that in the SQL editor.
3. Host the phone web app (optional): `npm run build:web` produces `dist-web/` (static files, including a
   `vercel.json` with the security headers). Deploy it to any HTTPS static host; with Vercel:
   `TC_WEB_DEPLOY_DIR=<folder linked to your Vercel project> scripts/deploy-web.sh`.
   The web app needs HTTPS (WebCrypto and the service worker are unavailable on plain http).
4. On the **main Mac**: open the Sync sheet (cloud button in the title bar) → *This Mac uploads* → paste the
   code → optionally set *Phone web app address* (enables the QR code) → Save.
5. On **another Mac**: Sync sheet → *Show another Mac* → paste the same code → Save.
6. On the **phone**: scan the QR from the main Mac's Sync sheet (opens the web app already paired) and Add to
   Home Screen. iOS gives home-screen apps their own storage, so if the installed app asks for the code, copy it
   on the Mac (Copy button) and paste it on the phone (Universal Clipboard).

## Behaviour and limits

- The main Mac publishes while the app is running and the Mac is awake. It uploads when anything shown on
  screen changes, at most once a minute, plus a heartbeat every 10 minutes; failures back off up to 10 minutes.
  Viewers poll a tiny version check every 30 s while visible and download only when the version changes.
- Viewers show the snapshot's age ("updated 2 min ago") and turn it amber when the main Mac has not synced for
  15 minutes.
- Every paired device holds the same key, so any of them could technically upload. Only one Mac should be set to
  *This Mac uploads* per code.
- Supabase pauses free-plan projects after about a week of very low activity. Normal use (the main Mac uploading,
  or any viewer polling) keeps it active; after a long absence, resume the project from the Supabase dashboard.

## Protocol reference

| piece | value |
|---|---|
| channel | hex(SHA256("tc-sync/channel/v1" ‖ K)[0..16]) |
| token | base64url(SHA256("tc-sync/auth/v1" ‖ K)) |
| encKey | SHA256("tc-sync/enc/v1" ‖ K) |
| envelope | base64(0x01 ‖ nonce(12) ‖ AES-256-GCM(encKey, nonce, aad, gzip(json))) |
| aad | "tc-sync/snapshot/v1:" + channel |
| RPCs | `POST /rest/v1/rpc/tc_head` · `tc_get` · `tc_put`, header `apikey: <publishable key>` |

Snapshot JSON (schema 1): `{schema, generatedAt, device: {name, appVersion}, plan, usage: {session, today, week,
month, all}, claudeQuota, codexQuota}` where each `usage` entry is exactly what the `get_usage` command returns.

Both implementations are tested against the same fixed vectors (K = bytes 0x00…0x1f) — see the tests in
`src-tauri/src/sync.rs` and `node scripts/sync-selftest.mjs`, which also decrypts the Rust-generated fixture
`src-tauri/tests/fixtures/sync_envelope_vector.json` to prove the two sides interoperate.
