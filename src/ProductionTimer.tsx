import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { confirm } from '@tauri-apps/plugin-dialog';
import { ChevronLeft, ChevronRight, Clock3, Pin, PinOff, Plus, Trash2, X } from 'lucide-react';
import './production-timer.css';

type Kind = 'beat' | 'loop';
type ActiveTimer = { id: string; kind: Kind; localDate: string; startedAt: number; activeSeconds: number; running: boolean; interrupted: boolean };
type Session = { id: string; kind: Kind; localDate: string; activeSeconds: number; status: string; source: string; note?: string | null };
type DayTotal = { date: string; beatSeconds: number; loopSeconds: number };
export type TimerSnapshot = {
  enabled: boolean; active: ActiveTimer | null; sessions: Session[]; today: DayTotal; week: DayTotal[];
  beatAverageSeconds: number; beatAverageDays: number; loopAverageSeconds: number; loopAverageDays: number;
};

const empty: TimerSnapshot = { enabled: false, active: null, sessions: [], today: { date: '', beatSeconds: 0, loopSeconds: 0 }, week: [], beatAverageSeconds: 0, beatAverageDays: 0, loopAverageSeconds: 0, loopAverageDays: 0 };
const call = <T,>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args);
const time = (seconds: number) => [Math.floor(seconds / 3600), Math.floor(seconds % 3600 / 60), seconds % 60].map(value => String(value).padStart(2, '0')).join(':');
const minutes = (seconds: number) => Math.round(seconds / 60);


function useTimerSnapshot(poll = false) {
  const [data, setData] = useState<TimerSnapshot>(empty);
  const [error, setError] = useState('');
  const requestVersion = useRef(0);
  const refresh = async () => {
    const version = requestVersion.current;
    try {
      const value = await call<TimerSnapshot>('production_timer_snapshot');
      if (version === requestVersion.current) { setData(value); setError(''); }
    }
    catch (cause) { setError(String(cause)); }
  };
  useEffect(() => {
    void refresh();
    if (!poll) return;
    const id = window.setInterval(() => void refresh(), 1000);
    return () => window.clearInterval(id);
  }, [poll]);
  const act = async (command: string, args?: Record<string, unknown>) => {
    const version = ++requestVersion.current;
    try {
      const value = await call<TimerSnapshot>(command, args);
      if (version === requestVersion.current) { setData(value); setError(''); }
    }
    catch (cause) { setError(String(cause)); }
  };
  return { data, setData, error, refresh, act };
}

export function ProductionTimerPopover() {
  const { data, error, act } = useTimerSnapshot(true);
  const [pinned, setPinned] = useState(true);
  const [windowError, setWindowError] = useState('');
  const togglePin = async () => {
    try {
      await getCurrentWindow().setAlwaysOnTop(!pinned);
      setPinned(!pinned);
      setWindowError('');
    } catch (cause) { setWindowError(String(cause)); }
  };
  const [kind, setKind] = useState<Kind>('beat');
  useEffect(() => {
    const close = () => void call('production_timer_hide');
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape') close(); };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, []);
  const active = data.active;
  return <main className="timer-popover">
    <header className="timer-popover-head">
      <strong className="timer-drag" data-tauri-drag-region><i className="timer-live-dot" data-state={active ? active.running ? 'running' : 'paused' : 'idle'} />1CE Tracker</strong>
      <div className="timer-window-actions">
        <button type="button" aria-label={pinned ? 'Unpin timer' : 'Pin timer'} aria-pressed={pinned} title={pinned ? 'Unpin' : 'Keep on top'} onClick={() => void togglePin()}>{pinned ? <Pin size={13} /> : <PinOff size={13} />}</button>
        <button type="button" aria-label="Close timer" onClick={() => void call('production_timer_hide')}><X size={13} /></button>
      </div>
    </header>
    <div className="timer-kind" role="group" aria-label="Production type">
      {(['beat', 'loop'] as Kind[]).map(value => <button type="button" key={value} className={(active?.kind || kind) === value ? 'selected' : ''} disabled={!!active} onClick={() => setKind(value)}>{value === 'beat' ? 'Beat' : 'Loop'}</button>)}
    </div>
    <section className="timer-clock" aria-live="polite">
      <span>{active ? active.running ? 'Tracking now' : active.interrupted ? 'Interrupted · paused' : 'Paused' : 'Ready to create'}</span>
      <strong>{time(active?.activeSeconds || 0)}</strong>
      <div>
        {!active ? <button className="timer-primary" type="button" onClick={() => void act('production_timer_start', { kind })}>Start</button> :
          <button className="timer-primary" type="button" onClick={() => void act(active.running ? 'production_timer_pause' : 'production_timer_resume')}>{active.running ? 'Pause' : 'Resume'}</button>}
        <button type="button" disabled={!active || active.activeSeconds === 0} onClick={() => void act('production_timer_finish')}>Finish</button>
      </div>
    </section>
    <div className="timer-metrics">
      <section><span>Today average</span><p><i className="metric-dot beat" /><b>{minutes(data.today.beatSeconds / Math.max(1, data.sessions.filter(item => item.localDate === data.today.date && item.kind === 'beat').length))}</b></p><p><i className="metric-dot loop" /><b>{minutes(data.today.loopSeconds / Math.max(1, data.sessions.filter(item => item.localDate === data.today.date && item.kind === 'loop').length))}</b></p></section>
      <section><span>Week average</span><p><i className="metric-dot beat" /><b>{minutes(data.beatAverageSeconds)}</b></p><p><i className="metric-dot loop" /><b>{minutes(data.loopAverageSeconds)}</b></p></section>
    </div>
    {(error || windowError) && <p className="timer-error" role="alert">{error || windowError}</p>}
    <button className="timer-records-link" type="button" onClick={() => void call('production_timer_open_history')}>View history<ChevronRight size={11} /></button>
  </main>;
}

function startOfWeek(date: Date) {
  const value = new Date(date); const day = (value.getDay() + 6) % 7;
  value.setHours(12, 0, 0, 0); value.setDate(value.getDate() - day); return value;
}
const isoDate = (date: Date) => {
  const year = date.getFullYear(), month = String(date.getMonth() + 1).padStart(2, '0'), day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
};

export function ProductionTimerHistory() {
  const { data, error, act } = useTimerSnapshot();
  const [openError, setOpenError] = useState('');
  const [weekOffset, setWeekOffset] = useState(0);
  const [kind, setKind] = useState<Kind>('beat');
  const [date, setDate] = useState(isoDate(new Date()));
  const [manualMinutes, setManualMinutes] = useState('30');
  const [note, setNote] = useState('');
  const weekStart = useMemo(() => { const value = startOfWeek(new Date()); value.setDate(value.getDate() + weekOffset * 7); return value; }, [weekOffset]);
  const weekEnd = useMemo(() => { const value = new Date(weekStart); value.setDate(value.getDate() + 6); return value; }, [weekStart]);
  const visible = data.sessions.filter(session => session.localDate >= isoDate(weekStart) && session.localDate <= isoDate(weekEnd)).sort((a, b) => b.localDate.localeCompare(a.localDate));
  const add = () => {
    const parsed = Math.round(Number(manualMinutes) * 60);
    if (parsed > 0) void act('production_timer_add_manual', { kind, localDate: date, activeSeconds: parsed, note: note.trim() || null });
  };
  const edit = (session: Session) => {
    const value = window.prompt('Minutes', String(minutes(session.activeSeconds)));
    if (value == null || Number(value) <= 0) return;
    void act('production_timer_update', { id: session.id, kind: session.kind, localDate: session.localDate, activeSeconds: Math.round(Number(value) * 60), note: session.note || null });
  };
  const remove = async (session: Session) => {
    if (await confirm(`Delete ${minutes(session.activeSeconds)} minutes of ${session.kind} from ${session.localDate}?`, { title: 'Delete production record', kind: 'warning', okLabel: 'Delete', cancelLabel: 'Keep' })) {
      await act('production_timer_delete', { id: session.id });
    }
  };
  const total = (target: Kind) => visible.filter(item => item.kind === target).reduce((sum, item) => sum + item.activeSeconds, 0);
  return <div className="timer-history">
    <header className="page-heading timer-history-heading"><div><span className="overline">PRODUCTION / TIME</span><h1>Time Tracker</h1><p className="page-description">Beat and loop production records stored locally.</p></div><button type="button" onClick={() => void call('production_timer_show').then(() => setOpenError('')).catch(cause => setOpenError(String(cause)))}><Clock3 size={17} /> Open compact timer</button></header>
    <section className="timer-history-summary">
      <button type="button" aria-label="Previous week" onClick={() => setWeekOffset(value => value - 1)}><ChevronLeft size={18} /></button>
      <div><strong>{new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(weekStart)} – {new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(weekEnd)}</strong><span>{minutes(total('beat'))} min beat · {minutes(total('loop'))} min loop</span></div>
      <button type="button" aria-label="Next week" disabled={weekOffset >= 0} onClick={() => setWeekOffset(value => value + 1)}><ChevronRight size={18} /></button>
    </section>
    <section className="timer-manual-entry" aria-label="Add manual record">
      <select aria-label="Production type" value={kind} onChange={event => setKind(event.target.value as Kind)}><option value="beat">Beat</option><option value="loop">Loop</option></select>
      <input aria-label="Date" type="date" value={date} onChange={event => setDate(event.target.value)} />
      <input aria-label="Minutes" type="number" min="1" step="1" value={manualMinutes} onChange={event => setManualMinutes(event.target.value)} />
      <input aria-label="Note" placeholder="Optional note" value={note} onChange={event => setNote(event.target.value)} />
      <button type="button" onClick={add}><Plus size={15} /> Add</button>
    </section>
    <section className="timer-history-list" aria-label="Production records">
      {visible.length === 0 ? <p>No records in this week.</p> : visible.map(session => <article key={session.id}>
        <i className={session.kind} /><div><strong>{session.kind === 'beat' ? 'Beat' : 'Loop'} · {minutes(session.activeSeconds)} min</strong><span>{session.localDate}{session.note ? ` · ${session.note}` : ''} · {session.source}</span></div>
        <button type="button" onClick={() => edit(session)}>Edit</button><button type="button" aria-label="Delete record" onClick={() => void remove(session)}><Trash2 size={15} /></button>
      </article>)}
    </section>
    {(error || openError) && <div className="error-banner" role="alert">{error || openError}</div>}
  </div>;
}


