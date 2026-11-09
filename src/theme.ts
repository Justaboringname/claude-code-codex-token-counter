import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

// Apple Liquid Glass (macOS 26) content palette. The glass itself is the
// NATIVE NSGlassEffectView backdrop (lib.rs) — everything here is deliberately
// colorless so the material, not the chrome, carries the look. Ink levels
// mirror the system label hierarchy (label/secondary/tertiary/quaternary);
// color is reserved for data (Claude orange ramp, Codex blue ramp).
export interface Theme {
  ink: string;
  inkStrong: string;
  inkSoft: string;
  inkMute: string;
  inkFaint: string;
  hairline: string;
  /** Recessed capsule track behind progress meters (system fill). */
  trackBg: string;
  /** Segmented-control trough — a shallow well cut into the glass. */
  segBg: string;
  segInset: string;
  /** The floating glass thumb of the active segment. */
  segThumbBg: string;
  segThumbShadow: string;
  /** Circular glass icon buttons in the title bar. */
  btnBg: string;
  btnShadow: string;
  /** Small floating glass chips (tool tiles). */
  tileBg: string;
  tileBorder: string;
  tileInset: string;
  /** Status text that needs attention (stale price table). Deeper amber in
   *  light mode so small text stays legible on pale glass. */
  warn: string;
  /** Stand-in panel background used ONLY in browser preview (no native
   *  glass there). In the real Tauri app the card is fully transparent. */
  previewBg: string;
  /** Sync sheet: the dimming layer over the panel, and the sheet slab
   *  (opaque: the numbers behind it bleed through any translucency). */
  scrim: string;
  sheetBg: string;
  sheetShadow: string;
  /** Text fields and secondary controls inside the sheet. */
  fieldBg: string;
  fieldBorder: string;
  /** Primary action (Claude orange) and its label. */
  accent: string;
  accentInk: string;
  /** Destructive action text, and a calm "all good" mark. */
  danger: string;
  ok: string;
  /** Keyboard focus ring of text fields (outer glow, and the edge over the border). */
  focusRing: string;
  focusEdge: string;
  /** Phone web app only: the wallpaper behind the glass card (no native
   *  glass in a browser), the solid color under it (overscroll, status bar)
   *  and the ink drawn straight on it (large title, freshness line). */
  webWallpaper: string;
  webBase: string;
  webInk: string;
  webInkSoft: string;
  webChipBg: string;
  webChipShadow: string;
  webCardShadow: string;
  /** Phone web app: the frosted card (no native glass behind it), and the
   *  inks for text on that card and on the sheets. The desktop inks are tuned
   *  for native glass; these keep small text at WCAG AA (4.5:1) over the
   *  wallpaper-backed card. Applied with `webTheme()`. */
  webCardBg: string;
  webInks: Pick<Theme, 'inkSoft' | 'inkMute' | 'inkFaint' | 'warn' | 'accent' | 'danger'>;
  /** The stale/error capsule of the freshness line on the wallpaper. */
  webWarnOnWall: string;
  webWarnChipBg: string;
  /** Phone: the "vs yesterday" delta (the desktop's system green/orange are
   *  too light for small text on the light card). */
  webDeltaUp: string;
  webDeltaDown: string;
}

/** The theme the phone web app draws with: the same palette, web inks. */
export function webTheme(t: Theme): Theme {
  return { ...t, ...t.webInks };
}

export const lightTheme: Theme = {
  ink: 'rgba(0, 0, 0, 0.88)',
  inkStrong: 'rgba(0, 0, 0, 0.8)',
  inkSoft: 'rgba(0, 0, 0, 0.58)',
  inkMute: 'rgba(0, 0, 0, 0.44)',
  inkFaint: 'rgba(0, 0, 0, 0.3)',
  hairline: 'rgba(0, 0, 0, 0.08)',
  trackBg: 'rgba(120, 120, 128, 0.2)',
  segBg: 'rgba(120, 120, 128, 0.14)',
  segInset: 'inset 0 0.5px 1.5px rgba(0, 0, 0, 0.1)',
  segThumbBg: 'rgba(255, 255, 255, 0.75)',
  segThumbShadow: [
    'inset 0 0.5px 0 rgba(255, 255, 255, 0.95)', // specular top lip
    '0 0 0 0.5px rgba(0, 0, 0, 0.04)',
    '0 1px 2.5px rgba(0, 0, 0, 0.12)',
    '0 2px 8px rgba(0, 0, 0, 0.08)', // lift off the trough
  ].join(', '),
  btnBg: 'rgba(255, 255, 255, 0.42)',
  btnShadow: [
    'inset 0 0.5px 0 rgba(255, 255, 255, 0.9)',
    '0 0 0 0.5px rgba(255, 255, 255, 0.45)',
    '0 1px 3px rgba(0, 0, 0, 0.1)',
  ].join(', '),
  tileBg: 'linear-gradient(180deg, rgba(255,255,255,0.5) 0%, rgba(255,255,255,0.3) 100%)',
  tileBorder: '0.5px solid rgba(255, 255, 255, 0.65)',
  tileInset: 'inset 0 1px 0 rgba(255,255,255,0.85), 0 1px 2.5px rgba(0,0,0,0.07)',
  warn: '#B35A00',
  previewBg: 'rgba(255, 255, 255, 0.5)',
  scrim: 'rgba(24, 22, 32, 0.22)',
  sheetBg: '#F7F7F9',
  sheetShadow: [
    'inset 0 1px 0 rgba(255, 255, 255, 0.95)',
    '0 0 0 0.5px rgba(0, 0, 0, 0.08)',
    '0 18px 48px rgba(0, 0, 0, 0.22)',
  ].join(', '),
  fieldBg: 'rgba(120, 120, 128, 0.1)',
  fieldBorder: 'rgba(0, 0, 0, 0.07)',
  accent: '#C25E3B',
  accentInk: '#FFFFFF',
  danger: '#D2372B',
  ok: '#248A4A',
  focusRing: 'rgba(194, 94, 59, 0.3)',
  focusEdge: 'rgba(194, 94, 59, 0.7)',
  webWallpaper: [
    // A dusk veil over the whole wallpaper: white text drawn straight on it
    // (subtitle, freshness line, notes) needs it for 4.5:1.
    'linear-gradient(rgba(38, 14, 30, 0.3), rgba(38, 14, 30, 0.3))',
    'radial-gradient(70% 45% at 8% 0%, rgba(214, 112, 76, 0.95) 0%, rgba(214, 112, 76, 0) 100%)',
    'radial-gradient(70% 50% at 100% 30%, rgba(98, 146, 214, 0.85) 0%, rgba(98, 146, 214, 0) 100%)',
    'radial-gradient(90% 60% at 20% 100%, rgba(140, 120, 214, 0.75) 0%, rgba(140, 120, 214, 0) 100%)',
    'linear-gradient(180deg, #C9775A 0%, #B48AA6 45%, #8C9BD6 100%)',
  ].join(', '),
  webBase: '#8E5C5E',
  webInk: '#FFFFFF',
  webInkSoft: 'rgba(255, 255, 255, 0.94)',
  webChipBg: 'rgba(255, 255, 255, 0.24)',
  webChipShadow: [
    'inset 0 0.5px 0 rgba(255, 255, 255, 0.6)',
    '0 0 0 0.5px rgba(255, 255, 255, 0.28)',
    '0 2px 8px rgba(0, 0, 0, 0.12)',
  ].join(', '),
  webCardShadow: [
    'inset 0 1px 0 rgba(255, 255, 255, 0.55)',
    'inset 0 0 0 0.5px rgba(255, 255, 255, 0.3)',
    '0 20px 50px rgba(40, 20, 60, 0.22)',
  ].join(', '),
  webCardBg: 'rgba(255, 255, 255, 0.66)',
  // Measured (Playwright WebKit, text hidden, backdrop composited): every
  // run ≥ 4.5:1 on the card, the sheet and the pairing card.
  webInks: {
    inkSoft: 'rgba(0, 0, 0, 0.7)',
    inkMute: 'rgba(0, 0, 0, 0.64)',
    inkFaint: 'rgba(0, 0, 0, 0.64)',
    warn: '#7A3B00',
    accent: '#B4532F',
    danger: '#B3261E',
  },
  webWarnOnWall: '#FFE0AD',
  webWarnChipBg: 'rgba(72, 28, 8, 0.34)',
  webDeltaUp: '#11572B',
  webDeltaDown: '#7A3B00',
};

export const darkTheme: Theme = {
  ink: 'rgba(255, 255, 255, 0.92)',
  inkStrong: 'rgba(255, 255, 255, 0.85)',
  inkSoft: 'rgba(255, 255, 255, 0.62)',
  inkMute: 'rgba(255, 255, 255, 0.46)',
  inkFaint: 'rgba(255, 255, 255, 0.32)',
  hairline: 'rgba(255, 255, 255, 0.09)',
  trackBg: 'rgba(120, 120, 128, 0.3)',
  segBg: 'rgba(120, 120, 128, 0.22)',
  segInset: 'inset 0 0.5px 1.5px rgba(0, 0, 0, 0.28)',
  segThumbBg: 'rgba(255, 255, 255, 0.24)',
  segThumbShadow: [
    'inset 0 0.5px 0 rgba(255, 255, 255, 0.32)', // specular top lip
    '0 0 0 0.5px rgba(255, 255, 255, 0.06)',
    '0 1px 2.5px rgba(0, 0, 0, 0.3)',
    '0 2px 8px rgba(0, 0, 0, 0.22)', // lift off the trough
  ].join(', '),
  btnBg: 'rgba(255, 255, 255, 0.13)',
  btnShadow: [
    'inset 0 0.5px 0 rgba(255, 255, 255, 0.28)',
    '0 0 0 0.5px rgba(255, 255, 255, 0.08)',
    '0 1px 3px rgba(0, 0, 0, 0.28)',
  ].join(', '),
  tileBg: 'linear-gradient(180deg, rgba(255,255,255,0.11) 0%, rgba(255,255,255,0.055) 100%)',
  tileBorder: '0.5px solid rgba(255, 255, 255, 0.14)',
  tileInset: 'inset 0 1px 0 rgba(255,255,255,0.14), 0 1px 2.5px rgba(0,0,0,0.25)',
  warn: '#FF9F0A',
  previewBg: 'rgba(28, 28, 30, 0.45)',
  scrim: 'rgba(0, 0, 0, 0.4)',
  sheetBg: '#26262A',
  sheetShadow: [
    'inset 0 1px 0 rgba(255, 255, 255, 0.1)',
    '0 0 0 0.5px rgba(255, 255, 255, 0.1)',
    '0 18px 48px rgba(0, 0, 0, 0.5)',
  ].join(', '),
  fieldBg: 'rgba(120, 120, 128, 0.24)',
  fieldBorder: 'rgba(255, 255, 255, 0.08)',
  accent: '#D97757',
  accentInk: '#FFFFFF',
  danger: '#FF6961',
  ok: '#32D74B',
  focusRing: 'rgba(217, 119, 87, 0.42)',
  focusEdge: 'rgba(217, 119, 87, 0.9)',
  webWallpaper: [
    'radial-gradient(70% 45% at 6% 0%, rgba(217, 119, 87, 0.42) 0%, rgba(217, 119, 87, 0) 100%)',
    'radial-gradient(70% 50% at 100% 38%, rgba(67, 127, 189, 0.4) 0%, rgba(67, 127, 189, 0) 100%)',
    'radial-gradient(90% 60% at 18% 100%, rgba(98, 78, 170, 0.38) 0%, rgba(98, 78, 170, 0) 100%)',
    'linear-gradient(180deg, #24171A 0%, #17161F 50%, #10131D 100%)',
  ].join(', '),
  webBase: '#141420',
  webInk: 'rgba(255, 255, 255, 0.95)',
  webInkSoft: 'rgba(255, 255, 255, 0.66)',
  webChipBg: 'rgba(255, 255, 255, 0.12)',
  webChipShadow: [
    'inset 0 0.5px 0 rgba(255, 255, 255, 0.24)',
    '0 0 0 0.5px rgba(255, 255, 255, 0.08)',
    '0 2px 8px rgba(0, 0, 0, 0.3)',
  ].join(', '),
  webCardShadow: [
    'inset 0 1px 0 rgba(255, 255, 255, 0.14)',
    'inset 0 0 0 0.5px rgba(255, 255, 255, 0.1)',
    '0 20px 50px rgba(0, 0, 0, 0.45)',
  ].join(', '),
  webCardBg: 'rgba(28, 28, 30, 0.45)',
  webInks: {
    inkSoft: 'rgba(255, 255, 255, 0.68)',
    inkMute: 'rgba(255, 255, 255, 0.6)',
    inkFaint: 'rgba(255, 255, 255, 0.6)',
    warn: '#FF9F0A',
    accent: '#B9583A',
    danger: '#FF9A92',
  },
  webWarnOnWall: '#FFC46B',
  webWarnChipBg: 'rgba(255, 159, 10, 0.14)',
  webDeltaUp: '#34C759',
  webDeltaDown: '#FF9F0A',
};

export type ThemeMode = 'auto' | 'light' | 'dark';

function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

// System appearance, read straight from the webview. At launch (and whenever
// the native window has no theme override) prefers-color-scheme mirrors the
// macOS system setting, so this is a correct first-paint value and the only
// signal available in browser preview.
function systemPrefersDark(): boolean {
  return (
    typeof window !== 'undefined' &&
    !!window.matchMedia &&
    window.matchMedia('(prefers-color-scheme: dark)').matches
  );
}

function readMode(): ThemeMode {
  if (typeof window === 'undefined') return 'auto';
  // URL override (?theme=dark|light) — lets browser previews and headless
  // screenshots pin a theme without touching persisted state.
  const q = new URLSearchParams(window.location.search).get('theme');
  if (q === 'dark' || q === 'light') return q;
  const stored = window.localStorage.getItem('theme');
  if (stored === 'dark' || stored === 'light' || stored === 'auto') return stored;
  // No stored choice → follow the macOS system appearance by default.
  return 'auto';
}

function initialDark(mode: ThemeMode): boolean {
  if (mode === 'dark') return true;
  if (mode === 'light') return false;
  return systemPrefersDark();
}

// Theme controller. `mode` is the user's choice (auto/light/dark, persisted);
// `isDark` is the resolved appearance actually in effect.
//   - auto       → follow the macOS system appearance. Event-driven via the
//                  native theme-changed event, which the OS fires even while
//                  the window is in the background (no polling to throttle).
//   - light/dark → pin explicitly and force the native window theme.
// `cycleMode` rotates Auto → Light → Dark → Auto.
export function useDarkMode(): { isDark: boolean; mode: ThemeMode; cycleMode: () => void } {
  const [mode, setMode] = useState<ThemeMode>(() => readMode());
  const [isDark, setIsDark] = useState<boolean>(() => initialDark(readMode()));

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let mql: MediaQueryList | undefined;
    let mqlHandler: ((e: MediaQueryListEvent) => void) | undefined;

    if (mode === 'light' || mode === 'dark') {
      // Pinned: force the native window (and its glass backdrop) to match.
      setIsDark(mode === 'dark');
      if (isTauri()) invoke('set_theme', { theme: mode }).catch(() => {});
    } else if (isTauri()) {
      // Auto: release any prior override so the native window (glass backdrop
      // + webview) tracks the system again, then mirror the system theme into
      // the CSS layer and keep it in sync via the OS theme-changed event.
      const win = getCurrentWindow();
      invoke('set_theme', { theme: 'auto' })
        .then(() => win.theme())
        .then((t) => {
          if (!cancelled && t) setIsDark(t === 'dark');
        })
        .catch(() => {});
      win
        .onThemeChanged(({ payload }) => setIsDark(payload === 'dark'))
        .then((u) => {
          if (cancelled) u();
          else unlisten = u;
        })
        .catch(() => {});
    } else if (typeof window !== 'undefined' && window.matchMedia) {
      // Browser preview: no native window, so track the system via the query.
      mql = window.matchMedia('(prefers-color-scheme: dark)');
      setIsDark(mql.matches);
      mqlHandler = (e) => setIsDark(e.matches);
      mql.addEventListener('change', mqlHandler);
    }

    return () => {
      cancelled = true;
      unlisten?.();
      if (mql && mqlHandler) mql.removeEventListener('change', mqlHandler);
    };
  }, [mode]);

  const cycleMode = () => {
    setMode((m) => {
      const next: ThemeMode = m === 'auto' ? 'light' : m === 'light' ? 'dark' : 'auto';
      window.localStorage.setItem('theme', next);
      return next;
    });
  };

  return { isDark, mode, cycleMode };
}
