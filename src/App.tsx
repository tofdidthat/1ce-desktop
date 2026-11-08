import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowDownToLine, Check, CircleAlert,
  CircleHelp, CloudUpload, FileAudio2, FolderClosed,
  HardDrive, ListMusic, Plus, Settings2, ShieldCheck, Sparkles, X,
} from 'lucide-react';

type Section = 'organize' | 'upload' | 'settings';
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
  const [templateMessage, setTemplateMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<AppStatus | null>(null);

  useEffect(() => {
    invoke<AppStatus>('get_app_status').then(setStatus).catch(() => {
      // Browser is only a design preview. Native commands require Tauri.
    });
  }, []);

  const usingExamples = browserPreview && showExamples && folders.length === 0;
  const displayedFolders = usingExamples ? sampleFolders : folders;
  const ready = useMemo(() => displayedFolders.filter((item) => item.state === 'ready').length, [displayedFolders]);

  useEffect(() => {
    try {
      window.localStorage.setItem('onece.previewAudioFormat', audioFormat);
      window.localStorage.setItem('onece.previewStems', String(includeStems));
    } catch { /* Storage may be unavailable in embedded previews. */ }
  }, [audioFormat, includeStems]);

  async function chooseFolders() {
    if (browserPreview) {
      setError('This is a browser design preview. Open the 1CE Desktop app to select folders from your computer.');
      return;
    }
    setError(null);
    setBusy(true);
    try {
      const selection = await open({ directory: true, multiple: true, title: 'Select beat folders' });
      if (!selection) return;
      const picked = Array.isArray(selection) ? selection : [selection];
      const paths = [...new Set([...folders.map((item) => item.folder), ...picked])];
      const scanned = await invoke<BeatFolder[]>('scan_beat_folders', { paths, template: nameTemplate });
      setFolders(scanned);
    } catch (cause) {
      setError(`Couldn't open or scan folders. ${String(cause)}`);
    } finally {
      setBusy(false);
    }
  }

  async function applyTemplate() {
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
              <div className="heading-caption"><span className="caption-led" /> Scan-only preview</div>
            </header>

            {error && (
              <div className="error-banner" role="alert">
                <CircleAlert size={17} /><span>{error}</span>
                <button type="button" aria-label="Dismiss message" onClick={() => setError(null)}><X size={17} /></button>
              </div>
            )}

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
                        <button key={option} type="button" aria-pressed={audioFormat === option} className={audioFormat === option ? 'selected' : ''} onClick={() => setAudioFormat(option)}>
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
                    <button className="stems-switch" type="button" role="switch" aria-label="Include stems in export preferences" aria-checked={includeStems} onClick={() => setIncludeStems((current) => !current)}><span /></button>
                  </div>
                  <div className="setup-field">
                    <div className="setup-field-heading">
                      <label htmlFor="organize-template">Naming template</label>
                      <span className="setup-field-hint">Read-only preview</span>
                    </div>
                    <div className="setup-template-control">
                      <input id="organize-template" type="text" spellCheck={false} value={templateDraft} onChange={(event) => { setTemplateDraft(event.target.value); setTemplateMessage(null); }} />
                      <button type="button" onClick={applyTemplate}>Save</button>
                    </div>
                    <p className="setup-token-hint">Use {'{name}'}, {'{bpm}'}, {'{key}'} and {'{producer}'}.</p>
                    {templateMessage && <p className="setup-template-feedback" role="status">{templateMessage}</p>}
                  </div>
                  <div className="setup-field setup-location-field">
                    <div>
                      <strong>Output folder</strong>
                      <p>Choose a destination when the renderer is ready.</p>
                    </div>
                    <span className="setup-later"><FolderClosed size={14} /> Coming soon</span>
                  </div>
                </div>
                <div className="setup-footer">
                  <div className="setup-total">
                    <div><strong>{ready}</strong><span> / {displayedFolders.length} ready</span></div>
                    <p>{usingExamples ? 'Example projects · preview only' : 'Original FLPs remain untouched'}</p>
                  </div>
                  <button type="button" className="export-button" disabled title="Invisible batch exporting is not enabled yet">
                    <ArrowDownToLine size={16} /> Export all <span className="button-caption">Soon</span>
                  </button>
                  <p className="setup-safety-note"><ShieldCheck size={14} /> Format and stems are draft preferences. No files are rendered or modified.</p>
                </div>
              </section>

              <aside className="beat-queue-panel" aria-labelledby="beat-queue-heading">
                <div className="queue-heading">
                  <div className="queue-heading-title">
                    <span className="panel-overline">YOUR PROJECTS</span>
                    <h2 id="beat-queue-heading">Beat queue <span className="count-pill">{displayedFolders.length}</span></h2>
                  </div>
                  {browserPreview && <button type="button" className="quiet-button" onClick={() => setShowExamples((current) => !current)}>{showExamples ? 'Hide examples' : 'Show examples'}</button>}
                </div>
                {usingExamples && <div className="queue-preview-notice"><span className="preview-mark" /> Preview data, not local files</div>}
                <div className="queue-list">
                  {displayedFolders.length === 0 ? (
                    <div className="queue-empty">
                      <div className="empty-art"><FolderClosed size={23} strokeWidth={1.4} /></div>
                      <strong>No beats added</strong>
                      <span>Choose folders with FL Studio projects.</span>
                    </div>
                  ) : displayedFolders.map((beat, index) => (
                    <div className="queue-row" key={beat.folder}>
                      <div className="queue-file-symbol"><FileAudio2 size={16} strokeWidth={1.6} /></div>
                      <div className="queue-item-content">
                        <div className="queue-item-top">
                          <strong title={beat.folderName}>{beat.displayName || beat.folderName}</strong>
                          <span className={'queue-status-dot ' + beat.state} title={prettyState(beat.state)} />
                        </div>
                        <span className="queue-meta" title={beat.projectFile || beat.detail}>
                          {beat.bpm != null ? String(beat.bpm) + ' BPM' : 'BPM pending'}
                          <span className="queue-meta-separator">·</span>
                          {beat.keyHint || (beat.missingKey === false ? 'Key in title' : 'Key to review')}
                        </span>
                        {beat.needsTempoReview && <span className="queue-review-note">Review tempo mismatch</span>}
                      </div>
                      {usingExamples ? <span className="queue-index">{String(index + 1).padStart(2, '0')}</span> : (
                        <button type="button" className="remove-button" aria-label={'Remove ' + beat.folderName} title="Remove from queue" onClick={() => setFolders((items) => items.filter((item) => item.folder !== beat.folder))}><X size={15} /></button>
                      )}
                    </div>
                  ))}
                </div>
                <div className="queue-footer">
                  <div className="queue-count"><span className="queue-count-dot" /> {ready} of {displayedFolders.length} projects ready</div>
                  <button className="primary-button" type="button" disabled={busy} onClick={chooseFolders}>
                    <Plus size={16} aria-hidden="true" />{busy ? 'Scanning…' : 'Add folders'}
                  </button>
                  <p>{usingExamples ? 'Examples for layout review only.' : 'Folder scanning is read-only.'}</p>
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
        ) : (
          <div className="page-content">
            <header className="page-heading"><div><span className="overline">YOUR PREFERENCES</span><h1>Settings</h1><p className="page-description">A few essentials for your desktop workflow.</p></div></header>
            <section className="settings-surface" aria-label="Desktop settings">
              <div className="setting-row"><div className="setting-icon"><HardDrive size={19} /></div><div className="setting-copy"><strong>FL Studio</strong><span>Configure the FL Studio executable when rendering is available.</span></div><span className="setting-state">Not configured</span></div>
              <div className="setting-row"><div className="setting-icon"><Sparkles size={19} /></div><div className="setting-copy"><strong>Smart Rename</strong><span>Read BPM from FLP. Musical key detection is coming in a later stage.</span></div><span className="setting-state">Key planned</span></div>
              <div className="setting-row template-setting"><div className="setting-icon"><FileAudio2 size={19} /></div><div className="setting-copy"><strong>Export filename template</strong><span>Only a read-only name preview for now. Variables: {`{name}`}, {`{bpm}`}, {`{key}`}, {`{producer}`}.</span><div className="template-editor"><input aria-label="Export filename template" type="text" value={templateDraft} onChange={(e) => setTemplateDraft(e.target.value)} spellCheck={false} /><button type="button" onClick={applyTemplate}>Save template</button></div>{templateMessage && <span role="status" className="template-feedback">{templateMessage}</span>}</div></div>
              <div className="setting-row"><div className="setting-icon"><CircleHelp size={19} /></div><div className="setting-copy"><strong>About 1CE Desktop</strong><span>{status?.version || '0.2.0-alpha.1'} · Windows-first · Local by design</span></div><span className="setting-state">Alpha</span></div>
            </section>
            <div className="info-note"><ShieldCheck size={17} /><span>This early version only scans folders. No audio or project files are modified.</span></div>
          </div>
        )}
        <footer className="footer"><span>1CE Desktop <span className="footer-separator">·</span> Alpha</span><span>Made for producers</span></footer>
      </main>
    </div>
  );
}
