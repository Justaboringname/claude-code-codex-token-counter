import type { SyncError } from './protocol';

/** UI wording for a sync error: `short` fits the freshness line, `long` a sheet. */
export function describeError(e: SyncError, deviceName?: string | null): { short: string; long: string } {
  const mac = deviceName || 'the main Mac';
  switch (e.kind) {
    case 'network':
      return {
        short: 'Offline',
        long:
          typeof navigator !== 'undefined' && navigator.onLine === false
            ? 'No internet connection. Showing the last copy.'
            : `${e.message} Showing the last copy.`,
      };
    case 'empty':
      return { short: 'Waiting for first upload', long: `Nothing uploaded yet. Turn on “This Mac uploads” on ${mac}.` };
    case 'forbidden':
      return { short: 'Pairing code not accepted', long: `${e.message} Pair again with a current code.` };
    case 'decrypt-failed':
      return { short: "Pairing code doesn't match", long: "The pairing code doesn't match the data on the server." };
    case 'bad-code':
      return { short: 'Invalid pairing code', long: e.message };
    case 'bad-snapshot':
      return { short: 'Unreadable data', long: `${e.message} Update Token Counter on both devices.` };
  }
}
