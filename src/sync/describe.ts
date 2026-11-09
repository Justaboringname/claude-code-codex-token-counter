import type { SyncError } from './protocol';

/** An error worth an amber mark. 'empty' (nothing uploaded yet) is a normal
 *  setup state, shown neutrally everywhere (badge, freshness line, sheets). */
export function isRealSyncError(e: SyncError | null | undefined): e is SyncError {
  return !!e && e.kind !== 'empty';
}

/** UI wording for a sync error: `short` fits the freshness line, `long` a sheet.
 *  `hasSnapshot`: a last good copy is on screen (network errors say so;
 *  without one they must not claim it). */
export function describeError(
  e: SyncError,
  deviceName?: string | null,
  hasSnapshot = false,
): { short: string; long: string } {
  const mac = deviceName || 'the main Mac';
  switch (e.kind) {
    case 'network': {
      const offline = typeof navigator !== 'undefined' && navigator.onLine === false;
      const then = hasSnapshot ? 'Showing the last copy.' : 'Check your connection and try again.';
      return { short: 'Offline', long: `${offline ? 'No internet connection.' : e.message} ${then}` };
    }
    case 'empty':
      return { short: 'Waiting for first upload', long: `Nothing uploaded yet. Turn on “This Mac uploads” on ${mac}.` };
    case 'forbidden':
      return { short: 'Pairing code not accepted', long: `${e.message} Pair again with a current code.` };
    case 'decrypt-failed':
      return { short: "Pairing code doesn't match", long: "The pairing code doesn't match the data on the server." };
    case 'bad-code':
      return { short: 'Invalid pairing code', long: e.message };
    case 'bad-snapshot':
      return e.reason === 'newer-schema'
        ? { short: 'Update needed', long: `${e.message} Update Token Counter here to show it.` }
        : { short: 'Unreadable data', long: `${e.message} Update Token Counter on both devices.` };
  }
}
