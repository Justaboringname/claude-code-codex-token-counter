// The Sync sheet (SPEC §7.6), opened from the cloud button in the title bar.
//
// Desktop: status first (last upload / what this Mac shows), the iPhone QR
// when a web-app URL is set, then the settings: role, pairing code, device
// name, web-app URL, Save. Web app: which Mac this phone shows, last update,
// Check now, Unpair.
import { useEffect, useMemo, useState } from 'react';
import { Icon } from '../icons';
import type { Theme } from '../theme';
import type { SyncRole } from '../types';
import type { DesktopSync } from './desktop';
import { parseCode, isSyncError, type SyncError } from './protocol';
import type { PendingReplace } from './pairing';
import { QrCode } from './QrCode';
import type { RemoteView } from './remote';
import { clockTime, relativeAge, useNow } from './time';
import { Button, Divider, Layout, Note, OptionRow, SectionTitle, Sheet, Spinner, TextField } from './ui';
import { describeError, isRealSyncError } from './describe';

const ENCRYPTION_NOTE =
  'End-to-end encrypted with the pairing code: the server only ever stores ciphertext. Anyone with the code can read these numbers.';

function maskCode(code: string): string {
  return `tc1.${'•'.repeat(18)}${code.slice(-4)}`;
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // WKWebView without clipboard permission: the old execCommand path.
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand('copy');
      ta.remove();
      return ok;
    } catch {
      return false;
    }
  }
}

function Row({ t, children, style }: { t: Theme; children: React.ReactNode; style?: React.CSSProperties }) {
  return <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: t.ink, ...style }}>{children}</div>;
}

function StatusDot({ color }: { color: string }) {
  return <span style={{ width: 7, height: 7, borderRadius: 999, background: color, flexShrink: 0, display: 'inline-block' }} />;
}

// ---------------------------------------------------------------- desktop

export function DesktopSyncSheet({
  t,
  sync,
  remote,
  onClose,
  onMeasure,
}: {
  t: Theme;
  sync: DesktopSync;
  remote: RemoteView;
  onClose: () => void;
  onMeasure: (h: number) => void;
}) {
  const layout: Layout = 'desktop';
  const now = useNow(15_000);
  const cfg = sync.config;
  const savedRole: SyncRole = cfg?.role ?? 'off';
  const savedCode = cfg?.code ?? null;

  const [role, setRole] = useState<SyncRole>(savedRole);
  const [name, setName] = useState(cfg?.deviceName ?? '');
  const [webUrl, setWebUrl] = useState(cfg?.webUrl ?? '');
  const [pasted, setPasted] = useState('');
  const [showPaste, setShowPaste] = useState(!savedCode);
  const [revealed, setRevealed] = useState(false);
  const [copied, setCopied] = useState(false);
  const [showQr, setShowQr] = useState(false);
  const [saving, setSaving] = useState(false);
  const [publishing, setPublishing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedFlash, setSavedFlash] = useState(false);

  // Config arrives (or changes after a save): re-sync the drafts.
  useEffect(() => {
    setRole(cfg?.role ?? 'off');
    setName(cfg?.deviceName ?? '');
    setWebUrl(cfg?.webUrl ?? '');
    setShowPaste(!cfg?.code);
  }, [cfg?.role, cfg?.deviceName, cfg?.webUrl, cfg?.code]);

  const pastedTrim = pasted.replace(/\s+/g, '');
  const pastedError = useMemo(() => {
    if (!pastedTrim) return null;
    try {
      parseCode(pastedTrim);
      return null;
    } catch (e) {
      return isSyncError(e) ? e.message : 'Not a pairing code.';
    }
  }, [pastedTrim]);
  const codeForSave = pastedTrim || savedCode;
  const dirty =
    role !== savedRole ||
    (!!pastedTrim && pastedTrim !== savedCode) ||
    (role === 'primary' && name.trim() !== (cfg?.deviceName ?? '')) ||
    webUrl.trim() !== (cfg?.webUrl ?? '');
  const webUrlError = webUrl.trim() && !/^https:\/\/[^\s#]+$/i.test(webUrl.trim()) ? 'Use the full https:// address.' : null;
  // Off hides the code and address fields and saves neither (null keeps the
  // stored values), so a half-typed draft there can't block it.
  const canSave =
    role === 'off'
      ? role !== savedRole && !saving
      : dirty && !saving && !pastedError && !webUrlError && !!codeForSave;

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      // sync_set_config: null keeps the stored value, "" clears it. Off
      // sends no drafts: those fields aren't on screen.
      const off = role === 'off';
      await sync.save({
        role,
        code: off ? null : (codeForSave ?? null),
        deviceName: role === 'primary' ? name.trim() : null,
        webUrl: off ? null : webUrl.trim(),
      });
      setPasted('');
      setRevealed(false);
      setSavedFlash(true);
      setTimeout(() => setSavedFlash(false), 1800);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const publishNow = async () => {
    setPublishing(true);
    setError(null);
    try {
      await sync.publishNow();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setPublishing(false);
    }
  };

  const copy = async () => {
    if (!savedCode) return;
    if (await copyText(savedCode)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    }
  };

  const st = sync.status;
  const qrUrl = savedRole !== 'off' && savedCode && cfg?.webUrl ? `${cfg.webUrl.replace(/#.*$/, '')}#pair=${savedCode}` : null;

  // ---- status block (saved state) ----
  let status: React.ReactNode = null;
  if (savedRole === 'primary') {
    const uploading = publishing || !!st?.uploading;
    const lastUp = st?.lastUploadAt ? Date.parse(st.lastUploadAt) : NaN;
    const lastErr = st?.lastErrorAt ? Date.parse(st.lastErrorAt) : NaN;
    const failing = !!st?.lastError && (!Number.isFinite(lastUp) || lastErr >= lastUp);
    status = (
      <>
        <SectionTitle t={t}>Status</SectionTitle>
        <Row t={t} style={{ fontSize: 12.5, alignItems: 'flex-start' }}>
          <span style={{ marginTop: 5 }}>
            {uploading ? <Spinner size={9} color={t.inkSoft} /> : <StatusDot color={failing ? t.warn : t.ok} />}
          </span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontWeight: 600 }}>
              {uploading
                ? 'Uploading…'
                : Number.isFinite(lastUp)
                  ? `Uploaded ${relativeAge(lastUp, now)}`
                  : 'Not uploaded yet'}
            </div>
            <div style={{ fontSize: 11, color: t.inkSoft, marginTop: 2 }}>
              {Number.isFinite(lastUp) ? `${clockTime(lastUp)}${st?.lastVersion ? ` · version ${st.lastVersion}` : ''} · ` : ''}
              Checks every minute; uploads when the numbers change.
            </div>
            {failing && (
              <div style={{ fontSize: 11, color: t.warn, marginTop: 4, display: 'flex', gap: 5 }}>
                <span style={{ flexShrink: 0, marginTop: 1 }}>
                  <Icon.Warn size={11} color={t.warn} />
                </span>
                <span>
                  {Number.isFinite(lastErr) ? `Failed ${relativeAge(lastErr, now)}: ` : ''}
                  {st?.lastError}
                </span>
              </div>
            )}
          </div>
          <Button t={t} layout={layout} onClick={publishNow} disabled={uploading}>
            Sync now
          </Button>
        </Row>
      </>
    );
  } else if (savedRole === 'viewer') {
    const snap = remote.snapshot;
    const gen = snap ? Date.parse(snap.generatedAt) : NaN;
    const err = remote.error ? describeError(remote.error, snap?.device.name, !!snap) : null;
    // Nothing uploaded yet is a setup step, not a failure: neutral, not amber.
    const failing = isRealSyncError(remote.error);
    status = (
      <>
        <SectionTitle t={t}>Status</SectionTitle>
        <Row t={t} style={{ fontSize: 12.5, alignItems: 'flex-start' }}>
          <span style={{ marginTop: 5 }}>
            {remote.refreshing ? (
              <Spinner size={9} color={t.inkSoft} />
            ) : (
              <StatusDot color={failing ? t.warn : err ? t.inkMute : t.ok} />
            )}
          </span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontWeight: 600 }}>
              {snap ? `Showing ${snap.device.name}` : remote.refreshing ? 'Connecting…' : 'No snapshot yet'}
            </div>
            <div style={{ fontSize: 11, color: t.inkSoft, marginTop: 2 }}>
              {Number.isFinite(gen) ? `Updated ${relativeAge(gen, now)} (${clockTime(gen)}) · ` : ''}
              Checks every 30 seconds while open.
            </div>
            {err && (
              <div role={failing ? 'alert' : undefined} style={{ fontSize: 11, color: failing ? t.warn : t.inkSoft, marginTop: 4 }}>
                {err.long}
              </div>
            )}
          </div>
          <Button t={t} layout={layout} onClick={() => void remote.refresh(true)} disabled={remote.refreshing}>
            Check now
          </Button>
        </Row>
      </>
    );
  }

  return (
    <Sheet t={t} layout={layout} title="Sync" onClose={onClose} onMeasure={onMeasure}>
      {savedRole === 'off' && (
        <div style={{ fontSize: 12, lineHeight: 1.45, color: t.inkSoft, margin: '-4px 0 14px' }}>
          Show one Mac's numbers on your other Macs and on your iPhone. Everything is end-to-end encrypted.
        </div>
      )}
      {status}
      {status && qrUrl && <Divider t={t} />}
      {qrUrl && (
        <>
          <SectionTitle t={t}>iPhone</SectionTitle>
          {showQr ? (
            <div style={{ display: 'flex', gap: 14, alignItems: 'center' }}>
              <div style={{ padding: 6, background: '#FFFFFF', borderRadius: 14, boxShadow: t.tileInset }}>
                <QrCode text={qrUrl} size={148} />
              </div>
              <div style={{ fontSize: 11.5, lineHeight: 1.45, color: t.inkSoft }}>
                Scan with the iPhone camera → open → Share → <b style={{ color: t.ink }}>Add to Home Screen</b>.
                <div style={{ marginTop: 8 }}>
                  <Button t={t} layout={layout} kind="plain" onClick={() => setShowQr(false)} style={{ paddingLeft: 0 }}>
                    Hide code
                  </Button>
                </div>
              </div>
            </div>
          ) : (
            <Row t={t} style={{ fontSize: 12 }}>
              <Icon.Phone size={14} color={t.inkSoft} />
              <span style={{ flex: 1, color: t.inkSoft }}>Show these numbers on your phone.</span>
              <Button t={t} layout={layout} onClick={() => setShowQr(true)}>
                Show QR code
              </Button>
            </Row>
          )}
        </>
      )}
      {(status || qrUrl) && <Divider t={t} />}

      <SectionTitle t={t}>This Mac</SectionTitle>
      <div role="radiogroup" aria-label="Sync role" style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
        <OptionRow
          t={t}
          layout={layout}
          selected={role === 'off'}
          title="Off"
          hint="Numbers stay on this Mac."
          icon={<Icon.Cloud size={13} />}
          onSelect={() => setRole('off')}
        />
        <OptionRow
          t={t}
          layout={layout}
          selected={role === 'primary'}
          title="This Mac uploads (main)"
          hint="Publishes an encrypted snapshot for your other devices."
          icon={<Icon.Cloud size={13} glyph="up" />}
          onSelect={() => setRole('primary')}
        />
        <OptionRow
          t={t}
          layout={layout}
          selected={role === 'viewer'}
          title="Show another Mac"
          hint="Shows the main Mac's numbers here; switch back any time."
          icon={<Icon.Cloud size={13} glyph="down" />}
          onSelect={() => setRole('viewer')}
        />
      </div>

      {role !== 'off' && (
        <div style={{ marginTop: 14 }}>
          <SectionTitle t={t}>Pairing code</SectionTitle>
          {savedCode && (
            <Row t={t} style={{ alignItems: 'flex-start' }}>
              <div
                className={revealed ? 'tc-selectable' : undefined}
                style={{
                  flex: 1,
                  minWidth: 0,
                  fontFamily: 'ui-monospace, "SF Mono", Menlo, monospace',
                  fontSize: 11,
                  lineHeight: 1.45,
                  padding: '6px 10px',
                  borderRadius: 9,
                  background: t.fieldBg,
                  border: `0.5px solid ${t.fieldBorder}`,
                  color: t.inkStrong,
                  wordBreak: 'break-all',
                  WebkitUserSelect: revealed ? 'text' : 'none',
                  userSelect: revealed ? 'text' : 'none',
                  cursor: revealed ? 'text' : 'default',
                }}
              >
                {revealed ? savedCode : maskCode(savedCode)}
              </div>
              <Button t={t} layout={layout} onClick={() => setRevealed((r) => !r)} title={revealed ? 'Hide' : 'Reveal'}>
                <Icon.Eye size={12} />
                {revealed ? 'Hide' : 'Reveal'}
              </Button>
              <Button t={t} layout={layout} onClick={copy} title="Copy the code">
                {copied ? <Icon.Check size={12} color={t.ok} /> : <Icon.Copy size={12} />}
                {copied ? 'Copied' : 'Copy'}
              </Button>
            </Row>
          )}
          {savedCode && !showPaste && (
            <Button t={t} layout={layout} kind="plain" onClick={() => setShowPaste(true)} style={{ marginTop: 4, paddingLeft: 0 }}>
              Use a different code…
            </Button>
          )}
          {showPaste && (
            <div style={{ marginTop: savedCode ? 8 : 0 }}>
              <TextField
                t={t}
                layout={layout}
                label="Paste a pairing code"
                value={pasted}
                onChange={setPasted}
                placeholder="Paste a pairing code (tc1.…)"
                mono
                onEnter={save}
              />
              {pastedError && <div style={{ fontSize: 11, color: t.warn, marginTop: 5 }}>{pastedError}</div>}
              {!savedCode && !pastedTrim && (
                <div style={{ fontSize: 11, color: t.inkMute, marginTop: 5 }}>
                  Every device of yours uses the same code. Codes are issued for your sync server by its owner.
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {role === 'primary' && (
        <div style={{ marginTop: 14 }}>
          <SectionTitle t={t}>Name on other devices</SectionTitle>
          <TextField t={t} layout={layout} label="Device name" value={name} onChange={setName} placeholder="Mac Studio" onEnter={save} />
        </div>
      )}

      {role !== 'off' && (
        <div style={{ marginTop: 14 }}>
          <SectionTitle t={t}>Phone web app address</SectionTitle>
          <TextField
            t={t}
            layout={layout}
            label="Phone web app address"
            value={webUrl}
            onChange={setWebUrl}
            placeholder="https://… (optional, for the iPhone QR code)"
            inputMode="url"
            onEnter={save}
          />
          {webUrlError && <div style={{ fontSize: 11, color: t.warn, marginTop: 5 }}>{webUrlError}</div>}
        </div>
      )}

      {error && (
        <div
          role="alert"
          style={{ marginTop: 12, fontSize: 11.5, color: t.warn, display: 'flex', gap: 6, alignItems: 'flex-start' }}
        >
          <span style={{ flexShrink: 0, marginTop: 1 }}>
            <Icon.Warn size={11} color={t.warn} />
          </span>
          <span>{error}</span>
        </div>
      )}

      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 14 }}>
        <div style={{ flex: 1, fontSize: 11, color: t.ok, display: 'flex', alignItems: 'center', gap: 5 }}>
          {savedFlash && (
            <>
              <Icon.Check size={11} color={t.ok} /> Saved
            </>
          )}
        </div>
        <Button t={t} layout={layout} kind="primary" onClick={save} disabled={!canSave}>
          {saving && <Spinner size={10} color={t.accentInk} />}
          {saving ? 'Checking…' : 'Save'}
        </Button>
      </div>

      {role !== 'off' && (
        <>
          <Divider t={t} />
          <Note t={t} layout={layout} icon={<Icon.Lock size={11} color={t.inkMute} />}>
            {ENCRYPTION_NOTE}
          </Note>
        </>
      )}
    </Sheet>
  );
}

// ---------------------------------------------------------------- web app

export function WebSyncSheet({
  t,
  remote,
  serverHost,
  onUnpair,
  onClose,
}: {
  t: Theme;
  remote: RemoteView;
  /** The sync server this phone is paired with (shown, so a switch is visible). */
  serverHost: string | null;
  onUnpair: () => void;
  onClose: () => void;
}) {
  const layout: Layout = 'web';
  const now = useNow(15_000);
  const [confirming, setConfirming] = useState(false);
  useEffect(() => {
    if (!confirming) return;
    const id = setTimeout(() => setConfirming(false), 4000);
    return () => clearTimeout(id);
  }, [confirming]);

  const snap = remote.snapshot;
  const gen = snap ? Date.parse(snap.generatedAt) : NaN;
  const err = remote.error ? describeError(remote.error, snap?.device.name, !!snap) : null;
  // 'empty' (nothing uploaded yet) is a setup step: plain text, no alert.
  const failing = isRealSyncError(remote.error);
  const stale = Number.isFinite(gen) && now - gen > 15 * 60_000;

  return (
    <Sheet t={t} layout={layout} title="Sync" onClose={onClose}>
      <div
        style={{
          display: 'flex',
          gap: 14,
          alignItems: 'center',
          padding: '14px 16px',
          borderRadius: 18,
          background: t.tileBg,
          border: t.tileBorder,
          boxShadow: t.tileInset,
        }}
      >
        <span
          style={{
            width: 44,
            height: 44,
            borderRadius: 12,
            background: t.fieldBg,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: t.inkSoft,
            flexShrink: 0,
          }}
        >
          <Icon.Desktop size={22} />
        </span>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontSize: 17, fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {snap?.device.name ?? 'Main Mac'}
          </div>
          <div style={{ fontSize: 13, color: stale ? t.warn : t.inkSoft, marginTop: 2 }}>
            {Number.isFinite(gen) ? `Updated ${relativeAge(gen, now)} · ${clockTime(gen)}` : 'No upload yet'}
          </div>
        </div>
      </div>

      {serverHost && (
        <div
          title="Sync server"
          style={{ display: 'flex', alignItems: 'center', gap: 7, fontSize: 13, color: t.inkSoft, margin: '12px 2px 0' }}
        >
          <span style={{ display: 'flex', flexShrink: 0 }}>
            <Icon.Web size={13} />
          </span>
          <span style={{ minWidth: 0, overflowWrap: 'anywhere' }}>
            Via <span className="tc-selectable" style={{ color: t.ink, WebkitUserSelect: 'text', userSelect: 'text' }}>{serverHost}</span>
          </span>
        </div>
      )}

      <div style={{ fontSize: 13, color: t.inkSoft, margin: '8px 2px 0', lineHeight: 1.45 }}>
        {remote.fetchedAt ? `Checked ${relativeAge(remote.fetchedAt, now)}` : 'Not checked yet'}
        {remote.version ? ` · version ${remote.version}` : ''}. This phone checks every 30 seconds while open; the Mac uploads
        when its numbers change.
      </div>
      {err &&
        (failing ? (
          <div role="alert" style={{ fontSize: 13, color: t.warn, margin: '10px 2px 0', display: 'flex', gap: 6 }}>
            <span style={{ flexShrink: 0, marginTop: 2 }}>
              <Icon.Warn size={12} color={t.warn} />
            </span>
            <span>{err.long}</span>
          </div>
        ) : (
          <div style={{ fontSize: 13, color: t.inkSoft, margin: '10px 2px 0', lineHeight: 1.45 }}>{err.long}</div>
        ))}

      <Button
        t={t}
        layout={layout}
        onClick={() => void remote.refresh(true)}
        disabled={remote.refreshing}
        style={{ width: '100%', marginTop: 16 }}
      >
        {remote.refreshing ? <Spinner size={14} color={t.inkSoft} /> : <Icon.Refresh size={14} />}
        {remote.refreshing ? 'Checking…' : 'Check now'}
      </Button>

      <Divider t={t} gap={18} />
      <Note t={t} layout={layout} icon={<Icon.Lock size={12} color={t.inkMute} />}>
        {ENCRYPTION_NOTE}
      </Note>
      <Button
        t={t}
        layout={layout}
        kind="danger"
        onClick={() => (confirming ? onUnpair() : setConfirming(true))}
        style={{ width: '100%', marginTop: 16 }}
      >
        {confirming ? 'Tap again to unpair' : 'Unpair this phone'}
      </Button>
    </Sheet>
  );
}

/** A `#pair=` link for a different code than the one in use: nothing changes
 *  (and nothing is sent to the link's server) until the user confirms. */
export function ReplacePairingSheet({
  t,
  pending,
  currentName,
  busy,
  error,
  onConfirm,
  onCancel,
}: {
  t: Theme;
  pending: PendingReplace;
  currentName: string | null;
  busy: boolean;
  error: SyncError | null;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const layout: Layout = 'web';
  const sameServer = pending.currentHost === pending.host;
  const err = error ? describeError(error, null, false) : null;
  const row = (label: string, main: string, sub: string | null) => (
    <div style={{ display: 'flex', gap: 12, alignItems: 'baseline', padding: '11px 14px' }}>
      <span style={{ width: 54, flexShrink: 0, fontSize: 13, color: t.inkSoft }}>{label}</span>
      <span style={{ minWidth: 0, flex: 1 }}>
        <span style={{ display: 'block', fontSize: 15, fontWeight: 600, overflowWrap: 'anywhere' }}>{main}</span>
        {sub && <span style={{ display: 'block', fontSize: 13, color: t.inkSoft, marginTop: 1, overflowWrap: 'anywhere' }}>{sub}</span>}
      </span>
    </div>
  );
  return (
    <Sheet t={t} layout={layout} title="Replace pairing?" onClose={busy ? () => {} : onCancel}>
      <div style={{ fontSize: 15, lineHeight: 1.45, color: t.inkStrong }}>
        {sameServer
          ? 'This link has a different pairing code for the same sync server. This phone would then show whatever that code’s Mac uploads.'
          : 'This link pairs the phone with a different sync server. Only continue if it came from your own Mac’s QR code.'}
      </div>
      <div
        style={{
          marginTop: 14,
          borderRadius: 16,
          background: t.tileBg,
          border: t.tileBorder,
          boxShadow: t.tileInset,
        }}
      >
        {row('Now', currentName ?? 'Main Mac', pending.currentHost ? `via ${pending.currentHost}` : null)}
        <div style={{ height: 0.5, background: t.hairline, margin: '0 14px' }} />
        {row('Link', pending.host, sameServer ? 'same server, new code' : 'not checked yet')}
      </div>
      {err && (
        <div role="alert" style={{ fontSize: 13, color: t.warn, margin: '12px 2px 0', display: 'flex', gap: 6 }}>
          <span style={{ flexShrink: 0, marginTop: 2 }}>
            <Icon.Warn size={12} color={t.warn} />
          </span>
          <span>{err.long}</span>
        </div>
      )}
      <Button t={t} layout={layout} kind="primary" onClick={onCancel} disabled={busy} style={{ width: '100%', marginTop: 18 }}>
        Keep current pairing
      </Button>
      <Button t={t} layout={layout} onClick={onConfirm} disabled={busy} style={{ width: '100%', marginTop: 8 }}>
        {busy && <Spinner size={14} color={t.inkSoft} />}
        {busy ? 'Checking the link…' : err ? 'Try the link again' : 'Replace with this link'}
      </Button>
    </Sheet>
  );
}
