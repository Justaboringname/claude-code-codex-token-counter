// Desktop (Tauri) side of cloud sync: the Rust config/status commands
// (SPEC §5) and the viewer's "which Mac am I looking at" toggle.
//
// Every call degrades to "sync off": an older backend without these commands
// (or a failing one) must leave the app exactly as it was before sync.
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { SyncConfigView, SyncRole, SyncStatus } from '../types';

// Role of the last successful sync_get_config, so the first paint after a
// launch already shows the right source (a viewer would otherwise start a
// pointless local scan). Never holds the code.
const ROLE_HINT_KEY = 'sync-role-hint-v1';
// Viewer's title-bar toggle: 'remote' (the main Mac) or 'local' (This Mac).
const VIEW_SOURCE_KEY = 'sync-view-source-v1';

export type ViewSource = 'remote' | 'local';

export function readRoleHint(): SyncRole {
  try {
    const r = window.localStorage.getItem(ROLE_HINT_KEY);
    if (r === 'primary' || r === 'viewer') return r;
  } catch {}
  return 'off';
}

function writeRoleHint(r: SyncRole) {
  try {
    if (r === 'off') window.localStorage.removeItem(ROLE_HINT_KEY);
    else window.localStorage.setItem(ROLE_HINT_KEY, r);
  } catch {}
}

export function readViewSource(): ViewSource {
  try {
    return window.localStorage.getItem(VIEW_SOURCE_KEY) === 'local' ? 'local' : 'remote';
  } catch {
    return 'remote';
  }
}

export function writeViewSource(v: ViewSource) {
  try {
    window.localStorage.setItem(VIEW_SOURCE_KEY, v);
  } catch {}
}

// sync_set_config arguments: null keeps the stored value, "" clears it.
export interface SaveConfig {
  role: SyncRole;
  code: string | null;
  deviceName: string | null;
  webUrl: string | null;
  /** The standby switch; null keeps the stored one. */
  standby: boolean | null;
}

export interface DesktopSync {
  /** The backend has the sync commands. */
  available: boolean;
  /** Role in effect (the cached hint until the backend answers). */
  role: SyncRole;
  config: SyncConfigView | null;
  status: SyncStatus | null;
  save: (c: SaveConfig) => Promise<SyncStatus>;
  publishNow: () => Promise<SyncStatus>;
}

function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return 'Something went wrong.';
}

export function useDesktopSync(isTauri: boolean): DesktopSync {
  const [available, setAvailable] = useState(false);
  const [config, setConfig] = useState<SyncConfigView | null>(null);
  const [hint, setHint] = useState<SyncRole>(() => (isTauri ? readRoleHint() : 'off'));
  const [status, setStatus] = useState<SyncStatus | null>(null);

  const applyConfig = useCallback((c: SyncConfigView | null) => {
    setConfig(c);
    setAvailable(c !== null);
    const role = c?.role ?? 'off';
    setHint(role);
    writeRoleHint(role);
  }, []);

  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    invoke<SyncConfigView>('sync_get_config')
      .then((c) => !cancelled && applyConfig(c))
      .catch(() => !cancelled && applyConfig(null));
    invoke<SyncStatus>('sync_status')
      .then((s) => !cancelled && setStatus(s))
      .catch(() => {});
    const un = listen<SyncStatus>('sync-status', (e) => setStatus(e.payload));
    // The event covers uploads; this catches anything missed while asleep.
    const timer = setInterval(() => {
      invoke<SyncStatus>('sync_status')
        .then((s) => !cancelled && setStatus(s))
        .catch(() => {});
    }, 60_000);
    return () => {
      cancelled = true;
      clearInterval(timer);
      un.then((f) => f()).catch(() => {});
    };
  }, [isTauri, applyConfig]);

  const save = useCallback(
    async (c: SaveConfig): Promise<SyncStatus> => {
      try {
        const st = await invoke<SyncStatus>('sync_set_config', {
          role: c.role,
          code: c.code,
          deviceName: c.deviceName,
          webUrl: c.webUrl,
          standby: c.standby,
        });
        setStatus(st);
        const fresh = await invoke<SyncConfigView>('sync_get_config').catch(() => null);
        if (fresh) applyConfig(fresh);
        return st;
      } catch (e) {
        throw new Error(errorText(e));
      }
    },
    [applyConfig],
  );

  const publishNow = useCallback(async (): Promise<SyncStatus> => {
    try {
      const st = await invoke<SyncStatus>('sync_publish_now');
      setStatus(st);
      return st;
    } catch (e) {
      // Pick up the recorded lastError too.
      invoke<SyncStatus>('sync_status').then(setStatus).catch(() => {});
      throw new Error(errorText(e));
    }
  }, []);

  return { available, role: config?.role ?? hint, config, status, save, publishNow };
}
