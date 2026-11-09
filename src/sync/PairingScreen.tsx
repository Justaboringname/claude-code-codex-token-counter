// Phone web app, not paired yet: full-screen pairing (SPEC §7.6). A
// `#pair=<code>` link (the Mac's QR code) pairs by itself; pasting works too.
//
// Layout: WebShell owns the safe-area padding and is one viewport tall; this
// screen fills it (flex: 1) and centres its block, so nothing scrolls.
import { useState } from 'react';
import { webCardStyle } from '../GlassSurface';
import { Icon } from '../icons';
import type { Theme } from '../theme';
import { AppIconMark } from './AppIconMark';
import { describeError } from './describe';
import type { WebPairing } from './pairing';
import { Button, Note, Spinner, TextField } from './ui';

export function PairingScreen({ t, isDark, pairing }: { t: Theme; isDark: boolean; pairing: WebPairing }) {
  const [code, setCode] = useState('');
  const submit = () => {
    if (code.trim() && !pairing.busy) void pairing.pair(code);
  };
  // Nothing has been shown yet, so a network error must not say "last copy".
  const err = pairing.error ? describeError(pairing.error, null, false) : null;
  const cloud = (
    <span style={{ display: 'inline-flex', verticalAlign: '-3px' }}>
      <Icon.Cloud size={16} color={t.ink} />
    </span>
  );

  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: '18px 4px',
      }}
    >
      <div style={{ width: '100%', maxWidth: 420 }}>
        <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', textAlign: 'center' }}>
          <div style={{ borderRadius: 20, boxShadow: '0 10px 30px rgba(0,0,0,0.25)' }}>
            <AppIconMark size={84} dark={isDark} />
          </div>
          <h1
            style={{
              margin: '18px 0 6px',
              fontSize: 30,
              fontWeight: 700,
              letterSpacing: '-0.02em',
              color: t.webInk,
            }}
          >
            Token Counter
          </h1>
          <p style={{ margin: 0, fontSize: 16, lineHeight: 1.4, color: t.webInkSoft, maxWidth: 300 }}>
            Your Mac's Claude Code and Codex usage, on this phone.
          </p>
        </div>

        <div style={{ ...webCardStyle(t), marginTop: 26, padding: '20px 18px 18px', color: t.ink }}>
          {pairing.busy ? (
            <div
              role="status"
              style={{ display: 'flex', alignItems: 'center', gap: 10, justifyContent: 'center', padding: '22px 0', fontSize: 16 }}
            >
              <Spinner size={16} color={t.inkSoft} /> Pairing…
            </div>
          ) : (
            <>
              <ol style={{ margin: 0, paddingLeft: 20, fontSize: 15, lineHeight: 1.45, color: t.inkStrong }}>
                <li>On the Mac running Token Counter, click the {cloud} cloud button.</li>
                {pairing.standalone ? (
                  <li style={{ marginTop: 6 }}>
                    Click <b>Copy</b> next to the pairing code, then paste it here.
                  </li>
                ) : (
                  <li style={{ marginTop: 6 }}>Scan its QR code with the Camera — or paste the pairing code here.</li>
                )}
              </ol>
              {pairing.standalone && (
                // A Camera scan opens Safari, whose storage this home-screen app doesn't share.
                <div style={{ fontSize: 13, lineHeight: 1.4, color: t.inkSoft, margin: '8px 0 0 20px' }}>
                  The QR code opens Safari, not this app. To use it, delete this icon and add it again from that page.
                </div>
              )}
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  submit();
                }}
                style={{ marginTop: 16 }}
              >
                <TextField
                  t={t}
                  layout="web"
                  label="Pairing code"
                  value={code}
                  onChange={setCode}
                  placeholder="tc1.…"
                  mono
                  onEnter={submit}
                />
                {err && (
                  <div role="alert" style={{ fontSize: 14, color: t.warn, marginTop: 10, display: 'flex', gap: 6 }}>
                    <span style={{ flexShrink: 0, marginTop: 2 }}>
                      <Icon.Warn size={13} color={t.warn} />
                    </span>
                    <span>{err.long}</span>
                  </div>
                )}
                <Button
                  t={t}
                  layout="web"
                  kind="primary"
                  type="submit"
                  disabled={!code.trim()}
                  style={{ width: '100%', marginTop: 14 }}
                >
                  Pair
                </Button>
                {pairing.retry && (
                  <Button t={t} layout="web" onClick={pairing.retry} style={{ width: '100%', marginTop: 8 }}>
                    Try the link again
                  </Button>
                )}
                {pairing.relink && (
                  <Button
                    t={t}
                    layout="web"
                    kind="plain"
                    onClick={pairing.relink}
                    style={{ width: '100%', marginTop: 6, whiteSpace: 'normal', lineHeight: 1.3 }}
                  >
                    Pair again with this app's link
                  </Button>
                )}
              </form>
            </>
          )}
        </div>

        <div style={{ marginTop: 16, padding: '0 6px' }}>
          <Note t={{ ...t, inkMute: t.webInkSoft }} layout="web" icon={<Icon.Lock size={12} color={t.webInkSoft} />}>
            End-to-end encrypted. The server only stores ciphertext; the pairing code is the key.
          </Note>
        </div>
      </div>
    </div>
  );
}
