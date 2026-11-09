// DEV ONLY (loaded from main.tsx behind import.meta.env.DEV, never bundled):
// run the desktop UI in a plain browser against a fake Tauri backend, to
// check the sync states without the Rust side.
//
//   ?mock=off       sync off (the cloud button opens the sheet on "Off")
//   ?mock=primary   this Mac uploads; &mockerr=1 adds a failed upload
//   ?mock=viewer    shows another Mac (the main Mac's snapshot comes from a
//                   fake server: fetches to example.supabase.co are answered
//                   locally with a demo snapshot sealed under the SPEC §6
//                   vector code — no real code or server involved)
//   &stale=1        the fake snapshot is 3 h old
//   &nosync=1       a backend without the sync commands (must look like today)
//
// Window size, theme and events are mocked too; the page backdrop stands in
// for the native glass.
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import { sampleClaudeQuota, sampleCodexQuota, sampleUsage } from '../sampleData';
import type { PlanId, Range, SyncConfigView, SyncRole, SyncStatus } from '../types';
import { deriveKeys, parseCode, sealEnvelope } from '../sync/protocol';
import { demoSnapshot } from './demoSnapshot';

// Public test vector (SPEC §6) — points at example.supabase.co.
const VECTOR_CODE =
  'tc1.eyJ2IjoxLCJ1IjoiaHR0cHM6Ly9leGFtcGxlLnN1cGFiYXNlLmNvIiwiayI6InNiX3B1Ymxpc2hhYmxlX3Rlc3QiLCJzIjoiQUFFQ0F3UUZCZ2NJQ1FvTERBME9EeEFSRWhNVUZSWVhHQmthR3h3ZEhoOCJ9';

export async function installTauriMock(kind: string) {
  const q = new URLSearchParams(window.location.search);
  const role: SyncRole = kind === 'primary' || kind === 'viewer' ? kind : 'off';
  const stale = q.get('stale') === '1';
  const minutes = (m: number) => new Date(Date.now() - m * 60_000).toISOString();

  let config: SyncConfigView = {
    role,
    code: role === 'off' ? null : VECTOR_CODE,
    deviceName: role === 'viewer' ? 'MacBook Pro' : 'Mac Studio',
    plan: 'max5x',
    webUrl: role === 'off' ? null : 'https://tokens.example.com/',
  };
  let status: SyncStatus = {
    role,
    paired: role !== 'off',
    deviceName: config.deviceName,
    lastUploadAt: role === 'primary' ? minutes(2) : null,
    lastVersion: role === 'primary' ? 128 : null,
    lastError: q.get('mockerr') ? "Can't reach the sync server (timed out after 20 s)." : null,
    lastErrorAt: q.get('mockerr') ? minutes(0.5) : null,
    uploading: false,
  };

  // Backdrop: roughly what the native glass looks like over a desktop.
  const dark = q.get('theme') === 'dark' || (q.get('theme') !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.style.background = dark
    ? 'linear-gradient(180deg, #2C2C31 0%, #1F1F24 100%)'
    : 'linear-gradient(180deg, #F3F2F5 0%, #E4E3E9 100%)';
  document.documentElement.style.minHeight = '100%';

  mockWindows('main');
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      const calls = ((window as unknown as { __tcMockCalls?: Record<string, number> }).__tcMockCalls ??= {});
      calls[cmd] = (calls[cmd] ?? 0) + 1;
      // &nosync=1: an older backend without the sync commands.
      if (q.get('nosync') && cmd.startsWith('sync_')) throw `Command ${cmd} not found`;
      switch (cmd) {
        case 'get_usage':
          return sampleUsage(a.range as Range, ((a.plan as PlanId) ?? 'max5x') as PlanId);
        case 'get_claude_quota':
          return sampleClaudeQuota();
        case 'get_codex_quota':
          return sampleCodexQuota();
        case 'set_theme':
          return null;
        case 'sync_get_config':
          return config;
        case 'sync_status':
          return status;
        case 'sync_set_config': {
          const nextRole = a.role as SyncRole;
          const code = (a.code as string | null) ?? null;
          await new Promise((r) => setTimeout(r, 600));
          if (nextRole !== 'off') {
            try {
              parseCode(code ?? '');
            } catch {
              throw 'Not a Token Counter pairing code.';
            }
            if (code !== VECTOR_CODE) throw "This pairing code isn't registered on the server.";
          }
          // Like sync.rs merge_field: null keeps, "" clears.
          const merge = (stored: string | null, v: unknown) =>
            v === null || v === undefined ? stored : String(v).trim() || null;
          config = {
            ...config,
            role: nextRole,
            code: merge(config.code, code),
            deviceName: merge(config.deviceName, a.deviceName) ?? 'Mac Studio',
            webUrl: merge(config.webUrl, a.webUrl),
          };
          status = { ...status, role: nextRole, paired: nextRole !== 'off', deviceName: config.deviceName };
          return status;
        }
        case 'sync_publish_now': {
          status = { ...status, uploading: true };
          void emit('sync-status', status);
          await new Promise((r) => setTimeout(r, 1200));
          status = {
            ...status,
            uploading: false,
            lastUploadAt: new Date().toISOString(),
            lastVersion: (status.lastVersion ?? 0) + 1,
            lastError: null,
            lastErrorAt: null,
          };
          return status;
        }
        default:
          if (cmd.startsWith('plugin:window|')) {
            if (cmd.endsWith('|theme')) return dark ? 'dark' : 'light';
            if (cmd.endsWith('|inner_size')) return { width: window.innerWidth, height: window.innerHeight };
            return null;
          }
          return null;
      }
    },
    { shouldMockEvents: true },
  );

  // Fake sync server for the vector code.
  const keys = await deriveKeys(VECTOR_CODE);
  const generated = new Date(Date.now() - (stale ? 3 * 60 : 3) * 60_000);
  const payload = await sealEnvelope(demoSnapshot(generated, 'Mac Studio'), keys);
  const realFetch = window.fetch.bind(window);
  window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    if (!url.startsWith(`${keys.url}/rest/v1/rpc/`)) return realFetch(input, init);
    await new Promise((r) => setTimeout(r, 250));
    const head = { version: 7, updatedAt: generated.toISOString(), generatedAt: generated.toISOString(), device: null };
    const body = url.endsWith('/tc_get') ? { ...head, payload } : head;
    return new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
  };
}
