import { useId, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

type Key = { root: string; mode: string; confirmed: boolean };
type Preview = { id: string; finalName: string; sourceProject: string; key: Key; destinations: string[]; fileCount: number; bytes: number; resumed: boolean };
export type DeliveryReceipt = { id: string; finalName: string; key: Key; destinations: string[]; uploadReady: boolean };
type Busy = (busy: boolean) => void;
const roots = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
function displayPath(path: string) { return path.startsWith('\\\\?\\UNC\\') ? '\\\\' + path.slice(8) : path.startsWith('\\\\?\\') ? path.slice(4) : path; }

function Paths({ paths }: { paths: string[] }) {
  return <ul className="delivery-paths">{paths.map(path => <li key={path}>{displayPath(path)}</li>)}</ul>;
}
export function DeliveryPanel({ attempt, title, template, candidate, disabled, onBusy, delivered }: {
  attempt: string; title: string; template: string; candidate?: { root: string; mode: string } | null;
  disabled: boolean; onBusy: Busy; delivered?: DeliveryReceipt | null;
}) {
  const fieldId = useId();
  const [root, setRoot] = useState(candidate?.root ?? '');
  const [mode, setMode] = useState(candidate?.mode ?? 'minor');
  const [preview, setPreview] = useState<Preview | null>(null);
  const [receipt, setReceipt] = useState<DeliveryReceipt | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const [executionStarted, setExecutionStarted] = useState(false);
  const blocked = disabled || working;

  async function deliver() {
    if (blocked || !roots.includes(root) || !['minor', 'major'].includes(mode)) return;
    setWorking(true); onBusy(true); setError(null);
    try {
      // Keep a prepared plan after an execution failure so retry resumes it.
      const plan = preview ?? await invoke<Preview>('prepare_delivery', {
        request: { attempt, title, template, key: { root, mode, confirmed: true } },
      });
      setPreview(plan); setExecutionStarted(true);
      setReceipt(await invoke<DeliveryReceipt>('execute_delivery', { id: plan.id }));
    } catch (cause) { setError(String(cause)); }
    finally { setWorking(false); onBusy(false); }
  }
  const completed = receipt ?? delivered;
  if (completed) return <details className="delivery-result"><summary>Delivered: {completed.finalName}</summary><Paths paths={completed.destinations} /></details>;
  return <details className="delivery-panel">
    <summary>Set key and finish export</summary>
    <p>Choose the correct key, then finish this export without rendering again.</p>
    <div className="delivery-key-fields">
      <label htmlFor={`${fieldId}-root`}>Key<select id={`${fieldId}-root`} disabled={blocked || executionStarted} value={root} onChange={event => { setRoot(event.target.value); setPreview(null); }}><option value="">Choose</option>{roots.map(note => <option key={note}>{note}</option>)}</select></label>
      <label htmlFor={`${fieldId}-mode`}>Mode<select id={`${fieldId}-mode`} disabled={blocked || executionStarted} value={mode} onChange={event => { setMode(event.target.value); setPreview(null); }}><option value="minor">Minor</option><option value="major">Major</option></select></label>
    </div>
    {preview && <div className="delivery-preview">
      <strong>{preview.finalName}</strong><Paths paths={preview.destinations} />
    </div>}
    <p>The project moves to the configured destinations after verification; its source folder is removed.</p>
    <button type="button" disabled={blocked || !roots.includes(root)} onClick={() => void deliver()}>
      {working ? 'Processing...' : executionStarted ? 'Retry export with this key' : 'Export with this key'}
    </button>
    {working && <button type="button" onClick={() => void invoke('cancel_export').catch(cause => setError(String(cause)))}>Cancel</button>}
    {error && <p role="alert">{error}</p>}
  </details>;
}

export function DeliveryRecovery({ disabled, onBusy, defaultPath }: { disabled: boolean; onBusy: Busy; defaultPath: string }) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [receipt, setReceipt] = useState<DeliveryReceipt | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  async function recover() {
    if (disabled || working) return;
    setWorking(true); onBusy(true); setError(null); setReceipt(null); setPreview(null);
    try {
      const recoveryPath = await invoke<string>('export_recovery_directory');
      const attempt = await open({ title: 'Choose the 1ce-attempt folder to verify or resume', directory: true, multiple: false, defaultPath: recoveryPath || defaultPath || undefined });
      if (attempt && !Array.isArray(attempt)) setPreview(await invoke<Preview>('resume_delivery', { attempt }));
    } catch (cause) { setError(String(cause)); }
    finally { setWorking(false); onBusy(false); }
  }
  async function resume() {
    if (!preview || disabled || working) return;
    setWorking(true); onBusy(true); setError(null);
    try { setReceipt(await invoke<DeliveryReceipt>('execute_delivery', { id: preview.id })); }
    catch (cause) { setError(String(cause)); }
    finally { setWorking(false); onBusy(false); }
  }
  return <div className="delivery-recovery">
    <button type="button" disabled={disabled || working} onClick={() => void recover()}>Resume interrupted delivery</button>
    {preview && !receipt && <div><strong>{preview.finalName} · {preview.key.root} {preview.key.mode}</strong><p>Source project: {displayPath(preview.sourceProject)}</p><Paths paths={preview.destinations} /><button type="button" disabled={disabled || working} onClick={() => void resume()}>Resume these paths</button></div>}
    {receipt && <div role="status"><strong>Delivery verified</strong><Paths paths={receipt.destinations} /></div>}
    {working && <button type="button" onClick={() => void invoke('cancel_export').catch(cause => setError(String(cause)))}>Cancel</button>}
    {error && <p role="alert">{error}</p>}
  </div>;
}
