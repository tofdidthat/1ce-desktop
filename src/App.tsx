import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { listen } from '@tauri-apps/api/event';
import { confirm, open } from '@tauri-apps/plugin-dialog';
import { smartRenamePreview } from './smart-rename-preview';
import { QueueTitle } from './QueueTitle';
import { DeliveryPanel, DeliveryRecovery, type DeliveryReceipt } from './DeliveryPanel';
import { ProductionTimerHistory, type TimerSnapshot } from './ProductionTimer';
import {
  ArrowDownToLine, Check, CircleAlert,
  CircleHelp, CloudUpload, FileAudio2, FolderClosed,
  Clock3, HardDrive, ListMusic, Settings2, ShieldCheck, Sparkles, X,
} from 'lucide-react';

type Section = 'organize' | 'upload' | 'timer' | 'settings';
type AudioFormat = 'wav' | 'mp3' | 'both';
type ScanState = 'ready' | 'missing' | 'multiple' | 'invalid';
type BeatFolder = {
  folder: string;
  folderName: string;
  projectFile: string | null;
  projectCount: number;
  state: ScanState;
  detail: string;
  bpm?: number | null;
  bpmSource?: string | null;
  bpmWarning?: string | null;
  namePreview?: string | null;
  missingKey?: boolean;
  needsTempoReview?: boolean;
  displayName?: string;
  keyHint?: string;
};
type AppStatus = { version: string; scanOnly: boolean; exportEngineReady: boolean };
type RenamePlan = { id: string; oldFolder: string; newFolder: string; oldFile: string; newFile: string };
type ExportConfig = { executable: string; mp3Folder: string; stemsFolder: string; projectsFolder: string; compatibilityMode: boolean };
type KeyCandidate = { root: string; mode: string; score: number };
type KeyAnalysis = { status: string; confidence: string; reason: string; candidate: KeyCandidate | null; alternatives: KeyCandidate[]; autoRenameAllowed: boolean; audioPath: string };
type ExportJob = { folder: string; state: string; message: string; output: string | null; audioExported: boolean; uploadReady: boolean; keyAnalysis?: KeyAnalysis | null; keyProgress?: number | null; delivery?: DeliveryReceipt | null };
type ExportStatus = { jobs: ExportJob[]; running: boolean };
const defaultTemplate = '{name} {bpm} {key} {producer}';

const browserPreview = typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window);

const sampleFolders: BeatFolder[] = [
  { folder: 'preview/bigtruck', folderName: 'bigtruck 140 e @tofdidthat', displayName: 'bigtruck', keyHint: 'E', bpm: 140, projectFile: 'bigtruck 140 e @tofdidthat.flp', projectCount: 1, state: 'ready', detail: 'Example project' },
  { folder: 'preview/sup', folderName: 'sup 140 f @tofdidthat', displayName: 'sup', keyHint: 'F', bpm: 140, projectFile: 'sup 140 f @tofdidthat.flp', projectCount: 1, state: 'ready', detail: 'Example project' },
  { folder: 'preview/spider', folderName: 'spider 140 f# @tofdidthat', displayName: 'spider', keyHint: 'F#', bpm: 140, projectFile: 'spider 140 f# @tofdidthat.flp', projectCount: 1, state: 'ready', detail: 'Example project' },
];

const sections = [
  { id: 'organize', label: 'Organize', Icon: ListMusic },
  { id: 'upload', label: 'Upload', Icon: CloudUpload },
] as const;

function prettyState(state: ScanState) {
  return state === 'ready' ? 'Ready' : state === 'missing' ? 'No FLP' :
    state === 'multiple' ? 'Multiple FLPs' : 'Invalid folder';
}

function KeyResult({ result }: { result: KeyAnalysis }) {
  return <div className="queue-key-result">
    <span>{result.candidate ? `Key estimate: ${result.candidate.root} ${result.candidate.mode} · review` : 'Key inconclusive'}</span>
    <details><summary>Details</summary><span>{result.reason}. Experimental; confidence is not calibrated.</span>
      {result.alternatives.length > 0 && <span>Alternatives: {result.alternatives.map(c => `${c.root} ${c.mode}`).join(' / ')}</span>}
      <span>Audio: {result.audioPath}</span>
    </details>
  </div>;
}

function NavItem({ label, Icon, active, onClick }: {
  label: string;
  Icon: typeof ListMusic;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button type="button" className={`nav-item${active ? ' active' : ''}`} onClick={onClick} aria-current={active ? 'page' : undefined} title={label}>
      <Icon size={18} strokeWidth={1.85} aria-hidden="true" />
      <span>{label}</span>
      {active && <span className="nav-active-indicator" aria-hidden="true" />}
    </button>
  );
}

export default function App() {
  const [section, setSection] = useState<Section>('organize');
  const [folders, setFolders] = useState<BeatFolder[]>([]);
  const [manualNames, setManualNames] = useState<Record<string, string>>({});
  const renameName = (beat: BeatFolder) => manualNames[beat.folder] || (smartRename ? beat.namePreview : null);
  const [showExamples, setShowExamples] = useState(browserPreview);
  const [nameTemplate, setNameTemplate] = useState(() => {
    try { return window.localStorage.getItem('onece.filenameTemplate') || defaultTemplate; }
    catch { return defaultTemplate; }
  });
  const [templateDraft, setTemplateDraft] = useState(nameTemplate);
  // Draft format/stems preferences are visual only until the Rust renderer is ready.
  const [audioFormat, setAudioFormat] = useState<AudioFormat>(() => {
    try {
      const value = window.localStorage.getItem('onece.previewAudioFormat');
      return value === 'wav' || value === 'mp3' || value === 'both' ? value : 'both';
    } catch { return 'both'; }
  });
  const [includeStems, setIncludeStems] = useState(() => {
    try { return window.localStorage.getItem('onece.previewStems') === 'true'; }
    catch { return false; }
  });
  // Editing previews never writes files; Rename explicitly applies the queue names.
  const [smartRename, setSmartRename] = useState(false);
  const [templateMessage, setTemplateMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [draggingFolders, setDraggingFolders] = useState(false);
  const [renameMessage, setRenameMessage] = useState<string | null>(null);
  const [recoveries, setRecoveries] = useState<RenamePlan[]>([]);
  const [exportConfig, setExportConfig] = useState<ExportConfig>({ executable: '', mp3Folder: '', stemsFolder: '', projectsFolder: '', compatibilityMode: true });
  const [exportJobs, setExportJobs] = useState<ExportJob[]>([]);
  const [exportBusy, setExportBusy] = useState(false);
  const [timerSettings, setTimerSettings] = useState<TimerSnapshot | null>(null);
  const [timerHistoryVersion, setTimerHistoryVersion] = useState(0);
  const exportRef = useRef(false);
  const queuePanelRef = useRef<HTMLElement | null>(null);
  const scanningRef = useRef(false);
  function deliveryBusy(value: boolean) { scanningRef.current = value; setBusy(value); }

  useEffect(() => {
    if (browserPreview) return;
    let disposed = false;
    let pending = false;
    const poll = async () => {
      if (pending) return;
      pending = true;
      try {
        const snapshot = await invoke<ExportStatus>('export_status');
        if (disposed) return;
        setExportJobs(snapshot.jobs);
        exportRef.current = snapshot.running; setExportBusy(snapshot.running);
      } catch { /* Older native binary may still be rebuilding. */ }
      finally { pending = false; }
    };
    void poll(); const timer = window.setInterval(() => void poll(), 1000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, []);

  useEffect(() => {
    if (browserPreview) return;
    void invoke<TimerSnapshot>('production_timer_snapshot').then(setTimerSettings).catch(cause => setError(String(cause)));
    let stop: undefined | (() => void);
    void listen('open-production-history', () => { setTimerHistoryVersion(value => value + 1); setSection('timer'); }).then(unlisten => { stop = unlisten; });
    return () => stop?.();
  }, []);

  async function startExport() {
    if (browserPreview || scanningRef.current || exportRef.current) return;
    const selected = folders.filter(beat => beat.state === 'ready' && beat.projectFile);
    if (!selected.length) return;
    scanningRef.current = true; setBusy(true); setError(null);
    try {
      await invoke('start_export', { inputs: selected.map(beat => ({ folder: beat.folder, expectedFile: beat.projectFile, title: manualNames[beat.folder] || beat.namePreview || beat.folderName })), format: audioFormat, stems: includeStems, template: nameTemplate });
      exportRef.current = true; setExportBusy(true);
    } catch (cause) { setError(String(cause)); }
    finally { scanningRef.current = false; setBusy(false); }
  }

  async function cancelExport() {
    try { await invoke('cancel_export'); }
    catch (cause) { setError(`Could not cancel: ${String(cause)}`); }
  }

  useEffect(() => {
    invoke<AppStatus>('get_app_status').then(setStatus).catch(() => {
      // Browser is only a design preview. Native commands require Tauri.
    });
  }, []);

  async function loadRecoveries() {
    if (browserPreview) return;
    try { setRecoveries(await invoke<RenamePlan[]>('pending_smart_renames')); }
    catch (cause) { setError(`Rename recovery needs attention: ${String(cause)}`); }
  }
  useEffect(() => { void loadRecoveries(); }, []);

  useEffect(() => {
    if (!browserPreview) void invoke<ExportConfig>('get_export_config').then(setExportConfig).catch(cause => setError(`Export configuration: ${String(cause)}`));
  }, []);

  async function chooseExportPath(kind: Exclude<keyof ExportConfig, 'compatibilityMode'>) {
    if (browserPreview || scanningRef.current) return;
    scanningRef.current = true; setBusy(true); setError(null);
    try {
      const picked = kind === 'executable'
        ? await open({ title: 'Select FL Studio FL64.exe', multiple: false, filters: [{ name: 'FL Studio', extensions: ['exe'] }] })
        : await open({ title: `Select ${kind === 'mp3Folder' ? 'MP3' : kind === 'stemsFolder' ? 'Stems + master WAV' : 'Projects'} destination`, directory: true, multiple: false });
      if (!picked || Array.isArray(picked)) return;
      setExportConfig(await invoke<ExportConfig>('save_export_config', { config: { ...exportConfig, [kind]: picked } }));
    } catch (cause) { setError(String(cause)); }
    finally { scanningRef.current = false; setBusy(false); }
  }

  async function renameProject(beat: BeatFolder, queue: BeatFolder[]): Promise<BeatFolder[] | null> {
    const proposed = renameName(beat);
    if (exportRef.current || scanningRef.current || browserPreview || !beat.projectFile || !proposed) return null;
    scanningRef.current = true;
    setBusy(true); setError(null); setRenameMessage(null);
    try {
      const plan = await invoke<RenamePlan>('prepare_smart_rename', {
        folder: beat.folder, expectedFile: beat.projectFile, proposed, template: nameTemplate, manual: !!manualNames[beat.folder],
      });
      const newFolder = await invoke<string>('execute_smart_rename', { id: plan.id });
      // Retain the new path even if the follow-up scanner fails.
      setFolders(items => items.map(item => item.folder === beat.folder ? { ...item, folder: newFolder, folderName: proposed, projectFile: plan.newFile } : item));
      setManualNames(items => { const next = { ...items }; delete next[beat.folder]; next[newFolder] = proposed; return next; });
      setRenameMessage(`Renamed folder and FLP to ${proposed}. No export was performed.`);
      const scanned = await invoke<BeatFolder[]>('scan_beat_folders', { paths: queue.map(item => item.folder === beat.folder ? newFolder : item.folder), template: nameTemplate });
      setFolders(scanned);
      return scanned;
    } catch (cause) { setError(String(cause)); return null; }
    finally { await loadRecoveries(); scanningRef.current = false; setBusy(false); }
  }

  const needsRename = (beat: BeatFolder) => beat.state === 'ready' && !!beat.projectFile && !!renameName(beat) &&
    (beat.folderName !== renameName(beat) || beat.projectFile !== `${renameName(beat)}.flp`);

  async function renameQueue() {
    if (exportRef.current || scanningRef.current || browserPreview || recoveries.length > 0) return;
    let queue = folders;
    const selected = folders.filter(needsRename).map(beat => beat.folder);
    for (const path of selected) {
      const beat = queue.find(item => item.folder === path);
      if (!beat || !needsRename(beat)) continue;
      const updated = await renameProject(beat, queue);
      if (!updated) break; // Failure stops the remaining queue.
      queue = updated;
    }
  }

  async function recoverRename(plan: RenamePlan) {
    if (exportRef.current || scanningRef.current) return;
    scanningRef.current = true; setBusy(true); setError(null);
    try {
      const yes = await confirm(`Restore the original names?\n\nFolder: ${plan.oldFolder}\nFLP: ${plan.oldFile}\n\nCurrent destination: ${plan.newFolder}\nFLP: ${plan.newFile}\n\nRecovery stops if files changed or either name would overwrite something.`, { title: 'Recover interrupted rename', kind: 'warning', okLabel: 'Restore original names', cancelLabel: 'Cancel' });
      if (!yes) return;
      const original = await invoke<string>('recover_smart_rename', { id: plan.id });
      setRenameMessage('Original folder and FLP names restored.');
      const paths = [...new Set([...folders.map(item => item.folder === plan.newFolder ? original : item.folder), original])];
      setFolders(await invoke<BeatFolder[]>('scan_beat_folders', { paths, template: nameTemplate }));
    } catch (cause) { setError(String(cause)); }
    finally { await loadRecoveries(); scanningRef.current = false; setBusy(false); }
  }

  const usingExamples = browserPreview && showExamples && folders.length === 0;
  const displayedFolders = usingExamples ? sampleFolders : folders;
  const exported = useMemo(() => displayedFolders.filter(item => exportJobs.some(job => job.folder === item.folder && job.uploadReady && !!job.delivery)).length, [displayedFolders, exportJobs]);

  useEffect(() => {
    try {
      window.localStorage.setItem('onece.previewAudioFormat', audioFormat);
      window.localStorage.setItem('onece.previewStems', String(includeStems));
    } catch { /* Storage may be unavailable in embedded previews. */ }
  }, [audioFormat, includeStems]);

  async function scanFolders(picked: string[]) {
    if (exportRef.current || scanningRef.current || picked.length === 0) return;
    scanningRef.current = true;
    setError(null);
    setBusy(true);
    try {
      const paths = [...new Set([...folders.map((item) => item.folder), ...picked])];
      if (paths.length > 250) {
        setError('The queue supports up to 250 folders. Remove some folders before adding more.');
        return;
      }
      const scanned = await invoke<BeatFolder[]>('scan_beat_folders', { paths, template: nameTemplate });
      setFolders(scanned);
    } catch (cause) {
      setError(`Couldn't scan folders. ${String(cause)}`);
    } finally {
      scanningRef.current = false;
      setBusy(false);
    }
  }

  useEffect(() => {
    if (browserPreview || section !== 'organize') return;
    let disposed = false;
    let stop: (() => void) | undefined;
    const contains = (position: { x: number; y: number }) => {
      const rect = queuePanelRef.current?.getBoundingClientRect();
      if (!rect) return false;
      const scale = window.devicePixelRatio || 1;
      const x = position.x / scale, y = position.y / scale;
      return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
    };
    void getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === 'leave') { setDraggingFolders(false); return; }
      if (payload.type === 'drop') {
        setDraggingFolders(false);
        if (contains(payload.position)) void scanFolders(payload.paths);
      } else setDraggingFolders(contains(payload.position));
    }).then((unlisten) => { if (disposed) unlisten(); else stop = unlisten; })
      .catch(() => { if (!disposed) setError('Drag and drop is unavailable. Use Add folders.'); });
    return () => { disposed = true; stop?.(); setDraggingFolders(false); };
  }, [section, folders, nameTemplate]);

  async function applyTemplate() {
    if (exportRef.current || scanningRef.current) return;
    const draft = templateDraft.trim();
    const unsafe = '<>:"/\\|?*';
    const outsideFields = draft.replace(/\{(?:name|bpm|key|producer)\}/g, '');
    const foundFields = draft.match(/\{(?:name|bpm|key|producer)\}/g) || [];
    if (!draft.includes('{name}') || draft.length > 200 ||
      [...draft].some((char) => unsafe.includes(char) || char.charCodeAt(0) < 32) ||
      outsideFields.includes('{') || outsideFields.includes('}') ||
      new Set(foundFields).size !== foundFields.length) {
      setTemplateMessage('Use {name}, {bpm}, {key} and {producer}. Avoid invalid filename characters.');
      return;
    }
    try {
      if (!browserPreview) {
        await invoke('preview_beat_name', { title: 'test 140 e @producer', flpBpm: 140, detectedKey: null, template: draft });
      }
      window.localStorage.setItem('onece.filenameTemplate', draft);
      setNameTemplate(draft);
      setTemplateMessage(browserPreview ? 'Saved for visual preview. Native renaming is not enabled.' : 'Template saved; future folder scans will use it.');
      if (!browserPreview && folders.length > 0) {
        const scanned = await invoke<BeatFolder[]>('scan_beat_folders', { paths: folders.map((folder) => folder.folder), template: draft });
        setFolders(scanned);
      }
    } catch (cause) {
      setTemplateMessage(`Template rejected: ${String(cause)}`);
    }
  }

  return (
    <div className="application">
      <aside className="sidebar">
        <div className="brand" aria-label="1CE Desktop">
          <div className="brand-mark"><img src="/logo.png" alt="1CE logo" /></div>
          <span className="brand-name">Desktop</span>
        </div>

        <div className="sidebar-navigation">
          <div className="nav-label">WORKFLOW</div>
          <nav aria-label="Workspace navigation">
            {sections.map(({ id, label, Icon }) => (
              <NavItem key={id} label={label} Icon={Icon} active={section === id} onClick={() => { setError(null); setSection(id); }} />
            ))}
          </nav>
        </div>

        <div className="sidebar-bottom">
          <NavItem label="Settings" Icon={Settings2} active={section === 'settings'} onClick={() => { setError(null); setSection('settings'); }} />
          <div className="sidebar-info">
            <span className="service-dot" aria-hidden="true" />
            <span>{status ? 'Rust connected' : 'Design preview'}</span>
            <span className="sidebar-version">v0.2</span>
          </div>
        </div>
      </aside>

      <main className="main-area">
        {section === 'organize' ? (
          <div className="page-content organize-page">
            <header className="page-heading organize-heading">
              <div>
                <span className="overline">LOCAL WORKSPACE / 01</span>
                <h1>Organize<span className="title-period">.</span></h1>
                <p className="page-description">From projects to finished beats. Quietly, in the background.</p>
              </div>
              <div className="heading-caption"><span className="caption-led" /> Local workspace</div>
            </header>

            {error && (
              <div className="error-banner" role="alert">
                <CircleAlert size={17} /><span>{error}</span>
                <button type="button" aria-label="Dismiss message" onClick={() => setError(null)}><X size={17} /></button>
              </div>
            )}
            {renameMessage && <p className="rename-feedback" role="status">{renameMessage}</p>}
            {recoveries.map(plan => <div className="rename-recovery" role="alert" key={plan.id}>
              <strong>Interrupted rename</strong>
              <p>{plan.oldFolder} → {plan.newFolder}</p>
              <button type="button" disabled={busy || exportBusy} onClick={() => void recoverRename(plan)}>Recover rename</button>
            </div>)}

            <div className="workspace-layout workspace-layout-inverted">
              <section className="export-settings-panel" aria-labelledby="export-setup-heading">
                <div className="setup-heading">
                  <div>
                    <span className="panel-overline">EXPORT CONFIGURATION</span>
                    <h2 id="export-setup-heading">Export setup</h2>
                    <p>Everything in one place before the batch starts.</p>
                  </div>
                  <span className="setup-draft">Preview</span>
                </div>
                <div className="setup-fields">
                  <div className="setup-field">
                    <div className="setup-field-heading">
                      <label id="format-label">Audio format</label>
                      <span className="setup-field-hint">Output preset</span>
                    </div>
                    <div className="format-switch" role="group" aria-labelledby="format-label">
                      {(['wav', 'mp3', 'both'] as const).map((option) => (
                        <button key={option} type="button" disabled={exportBusy} aria-pressed={audioFormat === option} className={audioFormat === option ? 'selected' : ''} onClick={() => setAudioFormat(option)}>
                          {option === 'both' ? 'WAV + MP3' : option.toUpperCase()}
                        </button>
                      ))}
                    </div>
                  </div>
                  <div className="setup-field setup-stems-field">
                    <div>
                      <strong>Export stems</strong>
                      <p>Include separated tracks with your mix.</p>
                    </div>
                    <button className="stems-switch" type="button" role="switch" disabled={exportBusy} aria-label="Include stems in export preferences" aria-checked={includeStems} onClick={() => setIncludeStems((current) => !current)}><span /></button>
                  </div>
                  <div className="setup-field setup-stems-field">
                    <div>
                      <strong>Smart Rename</strong>
                      <p>{smartRename ? 'Review names in the queue, then use Rename below it.' : 'Enable to preview names and rename projects without exporting.'}</p>
                    </div>
                    <button className="stems-switch" type="button" role="switch" aria-label="Smart Rename preview" disabled={busy || exportBusy} aria-checked={smartRename} onClick={() => setSmartRename((current) => !current)}><span /></button>
                  </div>
                </div>
              </section>

              <aside ref={queuePanelRef} className={`beat-queue-panel${draggingFolders ? ' is-dragging' : ''}`} aria-labelledby="beat-queue-heading">
                <div className="queue-heading">
                  <div className="queue-heading-title">
                    <span className="panel-overline">YOUR PROJECTS</span>
                    <h2 id="beat-queue-heading">Beat queue <span className="count-pill">{displayedFolders.length}</span></h2>
                  </div>
                  <div className="queue-heading-status">
                    <div className="queue-ready-count" aria-label={`${exported} of ${displayedFolders.length} projects finalized in this batch`}><strong>{exported}</strong><span> / {displayedFolders.length} finalized</span></div>
                    {browserPreview && <button type="button" className="quiet-button" onClick={() => setShowExamples((current) => !current)}>{showExamples ? 'Hide examples' : 'Show examples'}</button>}
                  </div>
                </div>
                {usingExamples && <div className="queue-preview-notice"><span className="preview-mark" /> Preview data, not local files</div>}
                <div className="queue-list">
                  {displayedFolders.length === 0 ? (
                    <div className="queue-empty">
                      <div className="empty-art"><FolderClosed size={23} strokeWidth={1.4} /></div>
                      <strong>No beats added</strong>
                      <span>Drop folders with FL Studio projects here.</span>
                    </div>
                  ) : displayedFolders.map((beat, index) => (
                    <div className="queue-row" key={beat.folder}>
                      <div className="queue-file-symbol"><FileAudio2 size={16} strokeWidth={1.6} /></div>
                      <div className="queue-item-content">
                        <div className="queue-item-top">
                          <QueueTitle value={exportJobs.find(job => job.folder === beat.folder)?.delivery?.finalName || manualNames[beat.folder] || (smartRename && smartRenamePreview(true, beat.projectFile, beat.namePreview)) || beat.displayName || beat.folderName}
                            disabled={busy || exportBusy || beat.state !== 'ready'} onSave={name => setManualNames(items => ({ ...items, [beat.folder]: name }))} />
                          <span className={'queue-status-dot ' + beat.state} title={prettyState(beat.state)} />
                        </div>
                        <span className="queue-meta" title={beat.projectFile || beat.detail}>
                          {beat.bpm != null ? String(beat.bpm) + ' BPM' : 'BPM pending'}
                          <span className="queue-meta-separator">·</span>
                          {beat.keyHint || (beat.missingKey === false ? 'Key in title' : 'Key to review')}
                        </span>
                        {beat.needsTempoReview && <span className="queue-review-note">Review tempo mismatch</span>}
                        {exportJobs.find(job => job.folder === beat.folder)?.keyAnalysis && <KeyResult result={exportJobs.find(job => job.folder === beat.folder)!.keyAnalysis!} />}
                        {exportJobs.filter(job => job.folder === beat.folder).map(job => <span className={`queue-export-state ${job.state}`} role="status" key={job.folder} title={job.output ? `${job.message}\n${job.output}` : job.message}>
                          {['queued','preparing_source','rendering','validating','key_queued','analyzing','preparing_delivery','delivering'].includes(job.state) && <span className="export-spinner" aria-hidden="true" />}{job.message}
                          {job.state === 'analyzing' && <progress className="queue-key-progress" aria-label="Find Key analysis progress" max={100} value={job.keyProgress ?? 0} />}
                        </span>)}
                        {exportJobs.filter(job => job.folder === beat.folder && job.audioExported && job.output && !['key_queued', 'analyzing'].includes(job.state)).map(job => <DeliveryPanel
                          key={JSON.stringify([job.output, manualNames[beat.folder] || beat.namePreview || beat.folderName, nameTemplate])}
                          attempt={job.output!} title={manualNames[beat.folder] || beat.namePreview || beat.folderName} template={nameTemplate}
                          candidate={job.keyAnalysis?.candidate} disabled={browserPreview || busy || exportBusy} onBusy={deliveryBusy} delivered={job.delivery} />)}
                      </div>
                      {usingExamples ? <span className="queue-index">{String(index + 1).padStart(2, '0')}</span> : (
                        <button type="button" disabled={busy || exportBusy} className="remove-button" aria-label={'Remove ' + beat.folderName} title="Remove from queue" onClick={() => { setFolders(items => items.filter(item => item.folder !== beat.folder)); setManualNames(items => { const next = { ...items }; delete next[beat.folder]; return next; }); }}><X size={15} /></button>
                      )}
                    </div>
                  ))}
                </div>
                <div className="queue-footer">
                  <div className="queue-actions">
                    <button className="primary-button" type="button" disabled={busy || exportBusy || browserPreview || recoveries.length > 0 || !folders.some(needsRename)} onClick={() => void renameQueue()} title="Apply queue names to folders and selected FLPs without exporting">
                      <Sparkles size={16} aria-hidden="true" /> Rename
                    </button>
                    <button className="primary-button" type="button" disabled={!exportBusy && (busy || browserPreview || !status?.exportEngineReady || recoveries.length > 0 || !folders.some(beat => beat.state === 'ready'))} onClick={() => void (exportBusy ? cancelExport() : startExport())} title={exportBusy ? 'Cancel this export batch' : 'Export, detect scale and move to configured destinations'}>
                      <ArrowDownToLine size={16} aria-hidden="true" /> {exportBusy ? 'Cancel' : 'Export'}
                    </button>
                  </div>
                  <p>{usingExamples ? 'Examples for layout review only.' : busy || exportBusy ? 'Processing…' : 'Drop project folders here.'}</p>
                </div>
              </aside>
            </div>
          </div>
        ) : section === 'upload' ? (
          <div className="page-content">
            <header className="page-heading"><div><span className="overline">DELIVERY WORKSPACE</span><h1>Upload</h1><p className="page-description">One place to send your exports to 1CE.</p></div><span className="phase-chip">Coming later</span></header>
            <section className="project-surface upload-surface">
              <div className="surface-toolbar"><div className="surface-title"><h2>Transfer queue</h2><span className="count-pill">0</span></div><span className="surface-hint">Nothing queued</span></div>
              <div className="empty-state upload-empty"><div className="empty-art"><CloudUpload size={30} strokeWidth={1.4} /></div><h3>Your queue is empty</h3><p>After export, send your beats to 1CE. The website will take care of publishing and delivery retries.</p></div>
            </section>
            <div className="info-note"><Check size={17} /><span>Uploads happen through the 1CE API. Social publishing stays on the website.</span></div>
          </div>
        ) : section === 'timer' ? (
          <div className="page-content"><ProductionTimerHistory key={timerHistoryVersion} /></div>
        ) : (
          <div className="page-content">
            <header className="page-heading"><div><span className="overline">YOUR PREFERENCES</span><h1>Settings</h1><p className="page-description">A few essentials for your desktop workflow.</p></div></header>
            <section className="settings-surface" aria-label="Desktop settings">
              <div className="quick-export-preferences" aria-label="Naming template and export destinations">
                <div className="quick-template">
                  <label htmlFor="top-filename-template">Naming template</label>
                  <div className="template-editor"><input id="top-filename-template" aria-label="Export filename template" type="text" value={templateDraft} onChange={(e) => setTemplateDraft(e.target.value)} spellCheck={false} /><button type="button" disabled={busy || exportBusy} onClick={applyTemplate}>Save</button></div>
                  <span>Variables: {'{name}'}, {'{bpm}'}, {'{key}'}, {'{producer}'}</span>
                  {templateMessage && <span role="status" className="template-feedback">{templateMessage}</span>}
                </div>
                <div className="quick-destinations">
                  <strong>Destinations</strong>
                  {([
                    ['mp3Folder', 'MP3'],
                    ['stemsFolder', 'Stems + WAV'],
                    ['projectsFolder', 'Projects'],
                  ] as const).map(([key, label]) => <div className="quick-destination" key={key}>
                    <span>{label}</span><span className="destination-path" title={exportConfig[key]}>{exportConfig[key] || 'Choose folder'}</span>
                    <button type="button" className="quiet-button" aria-label={`Choose ${label} destination`} disabled={browserPreview || busy || exportBusy} onClick={() => void chooseExportPath(key)}>Choose</button>
                  </div>)}
                </div>
              </div>
              <div className="setting-row"><div className="setting-icon"><HardDrive size={19} /></div><div className="setting-copy"><strong>FL Studio</strong><span title={exportConfig.executable}>{exportConfig.executable || 'Select your installed FL Studio executable.'}</span></div><button type="button" className="quiet-button" disabled={browserPreview || busy || exportBusy} onClick={() => void chooseExportPath('executable')}>Choose</button></div>
              <div className="setting-row"><button type="button" className="setting-icon setting-icon-button" aria-label="Open timer history" title="Open timer history" onClick={() => { setError(null); setSection('timer'); }}><Clock3 size={19} /></button><div className="setting-copy"><strong>Production Timer — Show in Windows tray</strong><span>Manual Beat / Loop timer. Closing its compact window keeps the current session available.</span></div><button className="stems-switch" type="button" role="switch" aria-label="Show Production Timer in Windows tray" aria-checked={timerSettings?.enabled || false} disabled={browserPreview || !timerSettings} onClick={() => {
                void invoke<TimerSnapshot>('set_production_timer_enabled', { enabled: !timerSettings?.enabled }).then(setTimerSettings).catch(cause => setError(String(cause)));
              }}><span /></button></div>
              <div className="setting-row"><div className="setting-icon"><HardDrive size={19} /></div><div className="setting-copy"><strong>Render compatibility mode</strong><span>Recommended for FL Studio 24. Its window may appear. Off uses experimental isolated rendering.</span></div><button className="stems-switch" type="button" role="switch" aria-label="Render compatibility mode" aria-checked={exportConfig.compatibilityMode} disabled={browserPreview || busy || exportBusy} onClick={() => {
                void invoke<ExportConfig>('save_export_config', { config: { ...exportConfig, compatibilityMode: !exportConfig.compatibilityMode } }).then(setExportConfig).catch(cause => setError(String(cause)));
              }}><span /></button></div>
              <div className="setting-row"><div className="setting-icon"><FolderClosed size={19} /></div><div className="setting-copy"><strong>Delivery recovery</strong><span>Verify or resume an export from its private recovery records. Music destinations remain unchanged.</span><DeliveryRecovery disabled={browserPreview || busy || exportBusy} onBusy={deliveryBusy} defaultPath={exportConfig.stemsFolder} /></div></div>
              <div className="setting-row"><div className="setting-icon"><CircleHelp size={19} /></div><div className="setting-copy"><strong>About 1CE Desktop</strong><span>{status?.version || '0.2.0-alpha.1'} · Windows-first · Local by design</span></div><span className="setting-state">Alpha</span></div>
            </section>
            {error && <div className="error-banner" role="alert">{error}</div>}
            <div className="info-note"><ShieldCheck size={17} /><span>Preferences stay local. Audio is validated, key estimates require review, and final delivery copies your files. Upload is not connected yet.</span></div>
          </div>
        )}
        <footer className="footer"><span>1CE Desktop <span className="footer-separator">·</span> Alpha</span><span>Made for producers</span></footer>
      </main>
    </div>
  );
}
