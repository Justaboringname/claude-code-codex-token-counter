import { useEffect, useState } from 'react';

/** "just now", "1 min ago", "12 min ago", "3 h ago", "2 days ago". */
export function relativeAge(thenMs: number, nowMs: number = Date.now()): string {
  const s = Math.max(0, (nowMs - thenMs) / 1000);
  if (!Number.isFinite(s)) return 'at an unknown time';
  if (s < 45) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${Math.max(1, m)} min ago`;
  const h = Math.floor(s / 3600);
  if (h < 48) return `${h} h ago`;
  return `${Math.floor(h / 24)} days ago`;
}

/** Local time, or date + time when not today: "3:12 AM", "Nov 8, 11:40 PM". */
export function clockTime(ms: number): string {
  const d = new Date(ms);
  const sameDay = d.toDateString() === new Date().toDateString();
  return sameDay
    ? d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
    : d.toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
}

/** Re-render every `everyMs` so relative times tick. */
export function useNow(everyMs = 15_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), everyMs);
    const onVis = () => document.visibilityState === 'visible' && setNow(Date.now());
    document.addEventListener('visibilitychange', onVis);
    return () => {
      clearInterval(id);
      document.removeEventListener('visibilitychange', onVis);
    };
  }, [everyMs]);
  return now;
}
