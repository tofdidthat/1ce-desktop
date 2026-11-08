import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowDownToLine, Check, CheckCircle2, ChevronRight, CircleAlert,
  CircleHelp, CloudUpload, FileAudio2, FolderClosed, FolderOpen,
  HardDrive, ListMusic, Plus, Settings2, ShieldCheck, Sparkles, X,
} from 'lucide-react';

type Section = 'organize' | 'upload' | 'settings';
type ScanState = 'ready' | 'missing' | 'multiple' | 'invalid';
type BeatFolder = {
  folder: string;
  folderName: string;
  projectFile: string | null;
  projectCount: number;
  state: ScanState;
  detail: string;
};
type AppStatus = { version: string; scanOnly: boolean; exportEngineReady: boolean };

const browserPreview = typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window);

const sampleFolders: BeatFolder[] = [
  { folder: 'preview/bigtruck', folderName: 'bigtruck 140 e @tofdidthat', projectFile: 'bigtruck 140 e @tofdidthat.flp', projectCount: 1, state: 'ready', detail: 'Example project' },
  { folder: 'preview/sup', folderName: 'sup 140 f @tofdidthat', projectFile: 'sup 140 f @tofdidthat.flp', projectCount: 1, state: 'ready', detail: 'Example project' },
  { folder: 'preview/spider', folderName: 'spider 140 f# @tofdidthat', projectFile: 'spider 140 f# @tofdidthat.flp', projectCount: 1, state: 'ready', detail: 'Example project' },
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
      const scanned = await invoke<BeatFolder[]>('scan_beat_folders', { paths });
      setFolders(scanned);
    } catch (cause) {
      setError(`Couldn't open or scan folders. ${String(cause)}`);
    } finally {
      setBusy(false);
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
          <div className="nav-label">WORKSPACE</div>
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
          <div className="page-content">
            <header className="page-heading">
              <div>
                <span className="overline">EXPORT WORKSPACE</span>
                <h1>Organize</h1>
                <p className="page-description">Everything ready for your next release.</p>
              </div>
              <button className="primary-button" type="button" disabled={busy} onClick={chooseFolders}>
                <Plus size={17} aria-hidden="true" />{busy ? 'Scanning…' : 'Add folders'}
              </button>
            </header>

            <div className="summary-strip" aria-label="Project summary">
              <div className="summary-metric"><span>Projects</span><strong>{displayedFolders.length}</strong></div>
              <div className="summary-metric"><span>Ready</span><strong>{ready}</strong></div>
              <div className="summary-divider" />
              <div className="summary-note"><ShieldCheck size={16} aria-hidden="true" /> Originals remain untouched</div>
            </div>

            {error && (
              <div className="error-banner" role="alert">
                <CircleAlert size={17} /><span>{error}</span>
                <button type="button" aria-label="Dismiss message" onClick={() => setError(null)}><X size={17} /></button>
              </div>
            )}

            <section className="project-surface" aria-labelledby="project-heading">
              <div className="surface-toolbar">
                <div className="surface-title"><h2 id="project-heading">Beat projects</h2><span className="count-pill">{displayedFolders.length}</span></div>
                {browserPreview && (
                  <button type="button" className="quiet-button" onClick={() => setShowExamples((current) => !current)}>
                    {showExamples ? 'Hide examples' : 'Show examples'}
                  </button>
                )}
              </div>

              {usingExamples && <div className="preview-notice"><span className="preview-mark" /> Sample projects for layout preview only — not files on your computer</div>}

              {displayedFolders.length === 0 ? (
                <div className="empty-state">
                  <div className="empty-art"><FolderClosed size={29} strokeWidth={1.4} /></div>
                  <h3>No projects added</h3>
                  <p>Choose folders containing FL Studio projects to start organizing your beats.</p>
                  <button type="button" className="secondary-button" onClick={chooseFolders} disabled={busy}>
                    <FolderOpen size={16} /> Select folders <ChevronRight size={15} />
                  </button>
                </div>
              ) : (
                <div className="project-list">
                  <div className="list-header"><span>NAME</span><span>PROJECT FILE</span><span>STATUS</span><span /></div>
                  {displayedFolders.map((beat) => (
                    <div className="project-row" key={beat.folder}>
                      <div className="project-identity">
                        <div className="project-icon"><FileAudio2 size={18} strokeWidth={1.5} /></div>
                        <div className="project-name-wrap"><strong title={beat.folderName}>{beat.folderName}</strong><span title={usingExamples ? 'Design example' : beat.folder}>{usingExamples ? 'Example beat' : beat.folder}</span></div>
                      </div>
                      <span className="project-file" title={beat.detail}>{beat.projectFile || beat.detail}</span>
                      <span className={`project-state ${beat.state}`}><i aria-hidden="true" />{prettyState(beat.state)}</span>
                      {usingExamples ? <span className="row-spacer" /> : (
                        <button type="button" className="remove-button" aria-label={`Remove ${beat.folderName}`} title="Remove from list" onClick={() => setFolders((items) => items.filter((item) => item.folder !== beat.folder))}><X size={17} /></button>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </section>

            <div className="action-dock">
              <div className="action-copy"><div className="action-copy-icon"><CheckCircle2 size={17} /></div><div><strong>{usingExamples ? 'Preview mode' : `${ready} of ${folders.length} ready`}</strong><span>{usingExamples ? 'Example projects cannot be exported.' : 'Export becomes available after the renderer is validated.'}</span></div></div>
              <button type="button" className="export-button" disabled title="Invisible batch exporting is not enabled yet"><ArrowDownToLine size={17} /> Export all</button>
            </div>
            <p className="page-footnote">Local-first. Original FL Studio files are never modified during scanning.</p>
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
              <div className="setting-row"><div className="setting-icon"><Sparkles size={19} /></div><div className="setting-copy"><strong>Smart Rename</strong><span>Read BPM from FLP, then estimate musical key from the audio.</span></div><span className="setting-state">Planned</span></div>
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
