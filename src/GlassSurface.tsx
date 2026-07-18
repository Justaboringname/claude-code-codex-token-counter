import { CSSProperties, ReactNode, useEffect, useRef, useState } from 'react';
import { Theme } from './theme';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';

interface Props {
  theme: Theme;
  children: ReactNode;
}

// Corner radius of the panel. Kept in lockstep with the native
// NSGlassEffectView corner radius set in `lib.rs` (set_effect) — the
// native Liquid Glass slab IS the background; this only clips the
// webview content so it doesn't spill past the glass's rounded corners.
const RADIUS = 20;

const FULLSCREEN_CARD_W = 900;
const FULLSCREEN_ZOOM = 2.0;

function detectFullscreen(): boolean {
  if (typeof window === 'undefined') return false;
  const sw = window.screen?.availWidth ?? 0;
  const sh = window.screen?.availHeight ?? 0;
  if (!sw || !sh) return false;
  return window.innerWidth >= sw - 10 && window.innerHeight >= sh - 10;
}

export function GlassSurface({ theme: t, children }: Props) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [isFullscreen, setFullscreen] = useState<boolean>(() => detectFullscreen());

  // In the real app the background glass is the native NSGlassEffectView
  // (see lib.rs). In a plain browser preview there's no native layer, so
  // fall back to a simple translucent card just so the layout is visible.
  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    if (!isTauri) return;

    // Compact mode: window height tracks content; width preserved.
    // Fullscreen mode: never touch window sizing — OS owns it.
    let raf = 0;
    const apply = (contentH: number) => {
      if (detectFullscreen()) return; // hands off in fullscreen
      const target = Math.max(200, Math.ceil(contentH));
      const w = window.innerWidth;
      const h = window.innerHeight;
      if (Math.abs(h - target) < 2) return;
      getCurrentWindow()
        .setSize(new LogicalSize(w, target))
        .catch(() => {});
    };
    const schedule = (h: number) => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => apply(h));
    };
    const ro = new ResizeObserver((entries) => {
      const h = entries[0]?.contentRect.height ?? el.scrollHeight;
      schedule(h);
    });
    ro.observe(el);
    const onWindowResize = () => {
      setFullscreen(detectFullscreen());
      schedule(el.scrollHeight);
    };
    window.addEventListener('resize', onWindowResize);
    apply(el.scrollHeight);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      window.removeEventListener('resize', onWindowResize);
    };
  }, [isTauri]);

  // The card. In Tauri it's fully transparent — the native Liquid Glass
  // shows through; we only round-clip the content. In browser preview we
  // paint a light translucent stand-in so the layout is still legible.
  const card = (
    <div
      ref={rootRef}
      style={{
        position: 'relative',
        borderRadius: RADIUS,
        width: isFullscreen ? FULLSCREEN_CARD_W : '100vw',
        overflow: 'hidden',
        background: isTauri ? 'transparent' : t.previewBg,
        // Preview-only frosted approximation of the native glass: blur +
        // saturate is what WKWebView/Chrome CAN do (refraction is not), plus
        // a specular rim so the slab reads. The real app leaves all of this
        // to NSGlassEffectView.
        ...(isTauri
          ? {}
          : {
              backdropFilter: 'blur(48px) saturate(180%)',
              WebkitBackdropFilter: 'blur(48px) saturate(180%)',
              boxShadow:
                'inset 0 1px 0 rgba(255,255,255,0.5), inset 0 0 0 0.5px rgba(255,255,255,0.25), 0 24px 60px rgba(0,0,0,0.35)',
            }),
      }}
    >
      <div style={contentLayer(t)}>{children}</div>
    </div>
  );

  // Compact (default): card fills window width 1:1, window auto-sizes
  // to content height.
  if (!isFullscreen) return card;

  // Fullscreen: an explicit ENLARGED view, zoomed 2× and centered.
  return (
    <div
      style={{
        width: '100vw',
        minHeight: '100vh',
        display: 'flex',
        justifyContent: 'center',
        alignItems: 'center',
        overflow: 'hidden',
      }}
    >
      <div style={{ zoom: FULLSCREEN_ZOOM }}>{card}</div>
    </div>
  );
}

function contentLayer(t: Theme): CSSProperties {
  return {
    position: 'relative',
    zIndex: 1,
    fontFamily: '-apple-system, "SF Pro Text", "Helvetica Neue", system-ui, sans-serif',
    color: t.ink,
  };
}
