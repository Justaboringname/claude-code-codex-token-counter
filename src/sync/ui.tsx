// Small building blocks for the sync sheet and the pairing screen, drawn
// with the shared theme tokens (Liquid Glass look, light + dark).
import React, { CSSProperties, ReactNode, useEffect, useLayoutEffect, useRef } from 'react';
import type { Theme } from '../theme';

export type Layout = 'desktop' | 'web';

export function Spinner({ size = 12, color = 'currentColor' }: { size?: number; color?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 12 12" fill="none" className="tc-spin" aria-hidden>
      <circle cx="6" cy="6" r="4.6" stroke={color} strokeOpacity="0.25" strokeWidth="1.5" />
      <path d="M6 1.4A4.6 4.6 0 0 1 10.6 6" stroke={color} strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

export function SectionTitle({ t, children, right }: { t: Theme; children: ReactNode; right?: ReactNode }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        fontSize: 11,
        fontWeight: 600,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        color: t.inkSoft,
        margin: '0 0 8px',
      }}
    >
      <span>{children}</span>
      <div style={{ flex: 1 }} />
      {right}
    </div>
  );
}

type ButtonKind = 'primary' | 'secondary' | 'plain' | 'danger';

export function Button({
  t,
  kind = 'secondary',
  layout,
  onClick,
  disabled,
  children,
  style,
  title,
  type = 'button',
}: {
  t: Theme;
  kind?: ButtonKind;
  layout: Layout;
  onClick?: () => void;
  disabled?: boolean;
  children: ReactNode;
  style?: CSSProperties;
  title?: string;
  type?: 'button' | 'submit';
}) {
  const web = layout === 'web';
  const base: CSSProperties = {
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 6,
    minHeight: web ? 44 : 28,
    padding: web ? '0 18px' : '0 12px',
    borderRadius: 999,
    border: 'none',
    fontSize: web ? 16 : 12,
    fontWeight: 600,
    fontFamily: 'inherit',
    cursor: disabled ? 'default' : 'pointer',
    opacity: disabled ? 0.45 : 1,
    whiteSpace: 'nowrap',
  };
  const kinds: Record<ButtonKind, CSSProperties> = {
    primary: {
      background: t.accent,
      color: t.accentInk,
      boxShadow: 'inset 0 0.5px 0 rgba(255,255,255,0.35), 0 1px 3px rgba(0,0,0,0.18)',
    },
    secondary: { background: t.btnBg, color: t.ink, boxShadow: t.btnShadow },
    plain: { background: 'transparent', color: t.inkSoft, padding: web ? '0 10px' : '0 6px' },
    danger: { background: t.btnBg, color: t.danger, boxShadow: t.btnShadow },
  };
  return (
    <button type={type} title={title} onClick={onClick} disabled={disabled} style={{ ...base, ...kinds[kind], ...style }}>
      {children}
    </button>
  );
}

export function TextField({
  t,
  layout,
  value,
  onChange,
  placeholder,
  mono,
  autoFocus,
  onEnter,
  label,
  inputMode,
}: {
  t: Theme;
  layout: Layout;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  mono?: boolean;
  autoFocus?: boolean;
  onEnter?: () => void;
  label: string;
  inputMode?: React.HTMLAttributes<HTMLInputElement>['inputMode'];
}) {
  const web = layout === 'web';
  return (
    <input
      aria-label={label}
      value={value}
      autoFocus={autoFocus}
      placeholder={placeholder}
      inputMode={inputMode}
      spellCheck={false}
      autoCapitalize="off"
      autoComplete="off"
      autoCorrect="off"
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' && onEnter) {
          e.preventDefault();
          onEnter();
        }
      }}
      style={{
        width: '100%',
        boxSizing: 'border-box',
        // ≥16 px on the phone, or iOS zooms the page on focus.
        fontSize: web ? 16 : 12,
        lineHeight: web ? '22px' : '18px',
        padding: web ? '11px 14px' : '6px 10px',
        borderRadius: web ? 14 : 9,
        border: `0.5px solid ${t.fieldBorder}`,
        background: t.fieldBg,
        color: t.ink,
        outline: 'none',
        fontFamily: mono ? 'ui-monospace, "SF Mono", Menlo, monospace' : 'inherit',
        WebkitUserSelect: 'text',
        userSelect: 'text',
        cursor: 'text',
      }}
    />
  );
}

/** Radio row: a selectable glass tile with a title and a one-line hint. */
export function OptionRow({
  t,
  layout,
  selected,
  title,
  hint,
  icon,
  onSelect,
}: {
  t: Theme;
  layout: Layout;
  selected: boolean;
  title: string;
  hint: string;
  icon: ReactNode;
  onSelect: () => void;
}) {
  const web = layout === 'web';
  return (
    <button
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        width: '100%',
        textAlign: 'left',
        padding: web ? '12px 14px' : '8px 11px',
        minHeight: web ? 52 : 0,
        borderRadius: web ? 16 : 12,
        border: 'none',
        background: selected ? t.segThumbBg : 'transparent',
        boxShadow: selected ? t.segThumbShadow : 'none',
        color: t.ink,
        cursor: 'pointer',
        fontFamily: 'inherit',
      }}
    >
      <span
        style={{
          width: web ? 30 : 24,
          height: web ? 30 : 24,
          borderRadius: 999,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: selected ? t.accent : t.fieldBg,
          color: selected ? t.accentInk : t.inkSoft,
        }}
      >
        {icon}
      </span>
      <span style={{ display: 'flex', flexDirection: 'column', minWidth: 0, flex: 1 }}>
        <span style={{ fontSize: web ? 15 : 12.5, fontWeight: 600 }}>{title}</span>
        <span style={{ fontSize: web ? 13 : 11, color: t.inkSoft, marginTop: 1 }}>{hint}</span>
      </span>
    </button>
  );
}

/**
 * The sheet itself. Desktop: an overlay inside the panel (below the title
 * bar, so the traffic lights stay clear), reporting its height so the
 * auto-sizing window can grow to fit. Web: a bottom sheet over the page.
 */
export function Sheet({
  t,
  layout,
  title,
  onClose,
  onMeasure,
  children,
}: {
  t: Theme;
  layout: Layout;
  title: string;
  onClose: () => void;
  onMeasure?: (height: number) => void;
  children: ReactNode;
}) {
  const web = layout === 'web';
  const panelRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  useLayoutEffect(() => {
    const el = panelRef.current;
    if (!el || !onMeasure) return;
    const report = () => onMeasure(el.scrollHeight);
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    return () => {
      ro.disconnect();
      onMeasure(0);
    };
  }, [onMeasure]);

  const header = (
    <div style={{ display: 'flex', alignItems: 'center', marginBottom: web ? 14 : 12 }}>
      <span style={{ fontSize: web ? 20 : 14, fontWeight: 700, color: t.ink, letterSpacing: web ? '-0.01em' : 0 }}>
        {title}
      </span>
      <div style={{ flex: 1 }} />
      <button
        onClick={onClose}
        aria-label="Close"
        style={{
          width: web ? 36 : 24,
          height: web ? 36 : 24,
          borderRadius: 999,
          border: 'none',
          padding: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: t.btnBg,
          boxShadow: t.btnShadow,
          color: t.inkSoft,
          cursor: 'pointer',
        }}
      >
        <svg width={web ? 12 : 9} height={web ? 12 : 9} viewBox="0 0 10 10" fill="none">
          <path d="M2 2l6 6M8 2l-6 6" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      </button>
    </div>
  );

  if (web) {
    return (
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        style={{ position: 'fixed', inset: 0, zIndex: 50, display: 'flex', flexDirection: 'column', justifyContent: 'flex-end' }}
      >
        <div onClick={onClose} className="tc-fade-in" style={{ position: 'absolute', inset: 0, background: t.scrim }} />
        <div
          ref={panelRef}
          className="tc-sheet-up"
          style={{
            position: 'relative',
            width: '100%',
            maxWidth: 560,
            margin: '0 auto',
            maxHeight: 'calc(100dvh - env(safe-area-inset-top) - 24px)',
            overflowY: 'auto',
            boxSizing: 'border-box',
            padding: '10px 20px calc(env(safe-area-inset-bottom) + 20px)',
            borderRadius: '28px 28px 0 0',
            background: t.sheetBg,
            boxShadow: t.sheetShadow,
            backdropFilter: 'blur(40px) saturate(180%)',
            WebkitBackdropFilter: 'blur(40px) saturate(180%)',
            color: t.ink,
          }}
        >
          <div style={{ width: 36, height: 5, borderRadius: 999, background: t.trackBg, margin: '0 auto 12px' }} />
          {header}
          {children}
        </div>
      </div>
    );
  }

  return (
    <div role="dialog" aria-modal="true" aria-label={title} style={{ position: 'absolute', inset: 0, zIndex: 20 }}>
      <div onClick={onClose} className="tc-fade-in" style={{ position: 'absolute', inset: 0, background: t.scrim }} />
      <div
        ref={panelRef}
        className="tc-sheet-in"
        style={{
          position: 'absolute',
          top: SHEET_TOP,
          left: 10,
          right: 10,
          maxHeight: `calc(100% - ${SHEET_TOP + 10}px)`,
          overflowY: 'auto',
          boxSizing: 'border-box',
          padding: '14px 16px 16px',
          borderRadius: 16,
          background: t.sheetBg,
          boxShadow: t.sheetShadow,
          backdropFilter: 'blur(30px) saturate(180%)',
          WebkitBackdropFilter: 'blur(30px) saturate(180%)',
          color: t.ink,
        }}
      >
        {header}
        {children}
      </div>
    </div>
  );
}

/** Desktop sheet offset from the panel top: clears the title bar. */
export const SHEET_TOP = 48;

export function Divider({ t, gap = 14 }: { t: Theme; gap?: number }) {
  return <div style={{ height: 0.5, background: t.hairline, margin: `${gap}px 0` }} />;
}

export function Note({ t, layout, children, icon }: { t: Theme; layout: Layout; children: ReactNode; icon?: ReactNode }) {
  return (
    <div
      style={{
        display: 'flex',
        gap: 7,
        alignItems: 'flex-start',
        fontSize: layout === 'web' ? 13 : 11,
        lineHeight: 1.4,
        color: t.inkMute,
      }}
    >
      {icon && <span style={{ marginTop: layout === 'web' ? 2 : 1, flexShrink: 0, display: 'flex' }}>{icon}</span>}
      <span>{children}</span>
    </div>
  );
}
