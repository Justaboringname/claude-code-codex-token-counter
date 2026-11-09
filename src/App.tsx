import { PopoverContent } from './PopoverContent';
import { IS_WEB_BUILD, isTauri, previewWantsRemote } from './env';
import { ErrorBoundary } from './ErrorBoundary';
import { GlassSurface } from './GlassSurface';
import { Icon } from './icons';
import { darkTheme, lightTheme, webTheme } from './theme';
import { readRoleHint, writeViewSource } from './sync/desktop';
import { CODE_STORAGE_KEY } from './sync/pairing';
import { clearSnapshotCache } from './sync/remote';
import { Button } from './sync/ui';
import './App.css';

function App() {
  // The phone web app draws its own wallpaper (light/dark, see WebShell).
  const browserPreview = !IS_WEB_BUILD && typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window);
  return (
    <div
      style={{
        background: browserPreview
          ? 'radial-gradient(120% 140% at 18% -8%, #8EC5F5 0%, #5B8FE8 32%, #7A68D6 58%, #35306E 88%, #1E1B45 100%)'
          : 'transparent',
      }}
    >
      {/* Last resort: the data panes have their own boundary; this catches
          anything else, so a render error never leaves an empty window. */}
      <ErrorBoundary label="app" fallback={() => <Recovery />}>
        <PopoverContent />
      </ErrorBoundary>
    </div>
  );
}

// Plain recovery screen: no app state, only the stored keys, then a reload.
function Recovery() {
  const q = new URLSearchParams(window.location.search).get('theme');
  const dark = q === 'dark' || (q !== 'light' && window.matchMedia?.('(prefers-color-scheme: dark)').matches === true);
  const base = dark ? darkTheme : lightTheme;
  const t = IS_WEB_BUILD ? webTheme(base) : base;
  const pairsInBrowser = IS_WEB_BUILD || (!isTauri() && previewWantsRemote());
  const viewer = isTauri() && readRoleHint() === 'viewer';
  const reload = () => window.location.replace(window.location.pathname + window.location.search);
  const layout = IS_WEB_BUILD ? 'web' : 'desktop';
  const btn = IS_WEB_BUILD ? { width: '100%' } : {};

  const content = (
    <div
      role="alert"
      style={{
        padding: IS_WEB_BUILD ? '36px 22px 28px' : '56px 28px 40px',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        textAlign: 'center',
        gap: 8,
        color: t.inkSoft,
        fontFamily: '-apple-system, "SF Pro Text", "Helvetica Neue", system-ui, sans-serif',
      }}
    >
      <Icon.Warn size={26} color={t.warn} />
      <div style={{ fontSize: IS_WEB_BUILD ? 19 : 15, fontWeight: 600, color: t.ink, marginTop: 4 }}>
        Token Counter hit a problem
      </div>
      <div style={{ fontSize: IS_WEB_BUILD ? 15 : 12, lineHeight: 1.45, maxWidth: 320 }}>
        {pairsInBrowser || viewer
          ? 'Synced data from another Mac couldn’t be shown. Clearing the saved copy usually fixes it; updating Token Counter on both devices does too.'
          : 'Something went wrong drawing this window. Reloading usually fixes it.'}
      </div>
      <div
        style={{
          display: 'flex',
          flexDirection: IS_WEB_BUILD ? 'column' : 'row',
          flexWrap: 'wrap',
          justifyContent: 'center',
          gap: 8,
          marginTop: 14,
          width: '100%',
        }}
      >
        <Button
          t={t}
          layout={layout}
          kind="primary"
          style={btn}
          onClick={() => {
            clearSnapshotCache();
            reload();
          }}
        >
          {pairsInBrowser || viewer ? 'Clear saved copy and reload' : 'Reload'}
        </Button>
        {viewer && (
          <Button
            t={t}
            layout={layout}
            style={btn}
            onClick={() => {
              writeViewSource('local');
              reload();
            }}
          >
            Show This Mac
          </Button>
        )}
        {pairsInBrowser && (
          <Button
            t={t}
            layout={layout}
            kind="danger"
            style={btn}
            onClick={() => {
              try {
                window.localStorage.removeItem(CODE_STORAGE_KEY);
              } catch {}
              clearSnapshotCache();
              reload(); // without any #pair= fragment
            }}
          >
            Unpair this phone
          </Button>
        )}
      </div>
    </div>
  );

  if (IS_WEB_BUILD) {
    return (
      <div
        style={{
          minHeight: '100dvh',
          boxSizing: 'border-box',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          padding: 'calc(env(safe-area-inset-top) + 16px) 16px calc(env(safe-area-inset-bottom) + 16px)',
          background: t.webWallpaper,
        }}
      >
        <div
          style={{
            width: '100%',
            maxWidth: 420,
            borderRadius: 26,
            background: t.sheetBg,
            boxShadow: t.webCardShadow,
          }}
        >
          {content}
        </div>
      </div>
    );
  }
  return <GlassSurface theme={t}>{content}</GlassSurface>;
}

export default App;
