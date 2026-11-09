// Where this UI is running:
//   tauri   — the desktop app (WKWebView with the Rust commands)
//   web     — the phone web app (`vite build --mode web`): remote data only
//   preview — plain `vite dev` in a browser: sample data, or the remote path
//             with ?source=remote
export type AppEnv = 'tauri' | 'web' | 'preview';

export const IS_WEB_BUILD = import.meta.env.MODE === 'web';

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export function appEnv(): AppEnv {
  if (IS_WEB_BUILD) return 'web';
  return isTauri() ? 'tauri' : 'preview';
}

/** Browser preview exercising the remote (web app) data path. */
export function previewWantsRemote(): boolean {
  return typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('source') === 'remote';
}
