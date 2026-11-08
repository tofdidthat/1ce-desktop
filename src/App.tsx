import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import {
  ArrowDownToLine, ArrowRight, Check, ChevronDown, CircleAlert, CircleHelp,
  CirclePlus, Clock3, CloudUpload, Disc3, FileAudio2, FolderClosed,
  FolderOpen, HardDrive, ListMusic, Music2, PanelLeft,
  Settings2, ShieldCheck, Sparkles, X,
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

const sections = [
  { id: 'organize', label: 'Organize', Icon: ListMusic },
  { id: 'upload', label: 'Upload', Icon: CloudUpload },
] as const;

function MenuButton({ label, Icon, active, onClick }: {
  label: string;
  Icon: typeof ListMusic;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <button className={`nav-item ${active ? 'active' : ''}`} onClick={onClick} type="button">
      <Icon size={18} strokeWidth={1.9} />
      <span>{label}</span>
      {active && <span className="nav-dot" />}
    </button>
  );
}

function prettyState(state: ScanState) {
  return state === 'ready' ? 'Ready' : state === 'missing' ? 'No FLP' :
    state === 'multiple' ? 'Multiple FLPs' : 'Invalid folder';
}

export default function App() {
  const [section, setSection] = useState<Section>('organize');
  const [folders, setFolders] = useState<BeatFolder[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<AppStatus | null>(null);

  useEffect(() => {
    invoke<AppStatus>('get_app_status').then(setStatus).catch(() => {
      // Browser previews are visual-only: native features require Tauri.
    });
  }, []);

  const ready = useMemo(() => folders.filter((item) => item.state === 'ready').length, [folders]);

  async function chooseFolders() {
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
      setError(`Couldn't open or scan folders. ${String(cause)}. Use the desktop app for local file access.`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="application">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><img src="/logo.png" alt="1CE" /></div>
          <div className="brand-copy"><strong>1CE</strong><span>DESKTOP</span></div>
          <PanelLeft size={17} className="sidebar-hint" />
        </div>
        <div className="navigation-label">WORKSPACE</div>
        <nav className="navigation" aria-label="Workspace">
          {sections.map(({ id, label, Icon }) => (
            <MenuButton key={id} label={label} Icon={Icon} active={section === id} onClick={() => setSection(id)} />
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="help-card">
            <div className="help-icon"><Sparkles size={18} /></div>
            <strong>A better workflow.</strong>
            <p>Export and organize beats, without leaving your creative space.</p>
          </div>
          <MenuButton label="Settings" Icon={Settings2} active={section === 'settings'} onClick={() => setSection('settings')} />
          <div className="sidebar-version"><span className="status-circle" /> v0.2 · Alpha</div>
        </div>
      </aside>

      <main className="main-area">
        <div className="window-bar"><div className="window-crumb">1CE Desktop <span>/</span> {section === 'organize' ? 'Organize' : section === 'upload' ? 'Upload' : 'Settings'}</div><div className="window-right"><span className="local-dot" /> Local workspace <ChevronDown size={13} /></div></div>
        {section === 'organize' ? (
          <div className="content">
            <div className="heading-row">
              <div><div className="eyebrow">YOUR WORKSPACE</div><h1>Organize your sound<span className="purple-stop">.</span></h1><p className="description">From FL Studio projects to a perfectly organized library.</p></div>
              <div className="heading-actions"><span className="alpha-pill"><span className="alpha-dot" /> Scan-only alpha</span></div>
            </div>

            <section className="intro-card">
              <div className="intro-copy"><span className="eyebrow light">1CE EXPORT</span><h2>Your creativity, uninterrupted.</h2><p>Prepare beat folders now. Invisible batch rendering, BPM and key detection will be enabled in the next development stages.</p><button type="button" onClick={chooseFolders} disabled={busy} className="primary-button"><FolderOpen size={17} /> {busy ? 'Scanning...' : 'Add beat folders'} <ArrowRight size={16} /></button></div>
              <div className="intro-art" aria-hidden="true"><div className="disc-ring ring-one" /><div className="disc-ring ring-two" /><div className="disc-center"><Disc3 size={64} strokeWidth={0.8} /></div><span className="disc-glow" /></div>
            </section>

            <div className="section-heading"><div><h3>Beat folders <span className="item-count">{folders.length}</span></h3><p>Select folders containing a single FL Studio project.</p></div><button type="button" className="outline-button" onClick={chooseFolders} disabled={busy}><CirclePlus size={16} /> Add folders</button></div>

            {error && <div className="error-banner" role="alert"><CircleAlert size={17} /> <span>{error}</span><button type="button" aria-label="Dismiss error" onClick={() => setError(null)}><X size={16} /></button></div>}
            {folders.length === 0 ? (
              <div className="empty-state"><div className="empty-icon"><FolderClosed size={31} strokeWidth={1.35} /></div><strong>No beat folders yet</strong><p>Add your folders to see which projects are ready for the export pipeline.</p><button className="text-button" type="button" onClick={chooseFolders} disabled={busy}>Choose folders <ArrowRight size={15} /></button></div>
            ) : (
              <div className="folder-list">
                <div className="folder-table-head"><span>PROJECT</span><span>STATUS</span><span>FLP</span><span /></div>
                {folders.map((beat) => (
                  <div className="folder-row" key={beat.folder}>
                    <div className="folder-main"><div className="folder-symbol"><Music2 size={17} /></div><div><strong>{beat.folderName}</strong><p title={beat.folder}>{beat.folder}</p></div></div>
                    <span className={`folder-state ${beat.state}`}><span />{prettyState(beat.state)}</span>
                    <span className="folder-file" title={beat.detail}>{beat.projectFile || beat.detail}</span>
                    <button className="delete-button" type="button" aria-label={`Remove ${beat.folderName}`} onClick={() => setFolders((items) => items.filter((item) => item.folder !== beat.folder))}><X size={17} /></button>
                  </div>
                ))}
              </div>
            )}
            <div className="bottom-panel"><div className="bottom-panel-title"><div className="bottom-icon"><ShieldCheck size={18} /></div><div><strong>Safe by design</strong><p>Read-only inspection. Your original FLP projects remain untouched.</p></div></div><div className="bottom-right"><span>{ready} of {folders.length} ready</span><button className="disabled-export" type="button" disabled title="Invisible batch export will be added in a dedicated, Windows-validated stage"><ArrowDownToLine size={16} /> Export all <span className="coming-soon">Coming next</span></button></div></div>
          </div>
        ) : section === 'upload' ? (
          <div className="content"><div className="eyebrow">YOUR WORKSPACE</div><h1>Upload<span className="purple-stop">.</span></h1><p className="description">Your local delivery queue, connected to 1CE.</p><div className="mode-header"><CloudUpload size={20} /><span>Upload mode</span><span className="alpha-pill">Planned</span></div><div className="feature-empty"><div className="feature-icon"><CloudUpload size={36} strokeWidth={1.3} /></div><h2>Publish from one place.</h2><p>This mode will send exported beats to the 1CE API. The website will manage publishing to destinations, delivery status and retries.</p><div className="planned-features"><span><Clock3 size={14} /> Persistent queue</span><span><Check size={14} /> Confirmed deliveries</span><span><HardDrive size={14} /> No duplicate storage</span></div></div></div>
        ) : (
          <div className="content"><div className="eyebrow">PREFERENCES</div><h1>Settings<span className="purple-stop">.</span></h1><p className="description">Local application setup and future export preferences.</p><div className="settings-group"><div className="settings-item"><div className="setting-icon"><FileAudio2 size={21}/></div><div><strong>FL Studio executable</strong><p>Executable selection and validation will be added with the export engine.</p></div><span className="settings-value">Not configured</span></div><div className="settings-item"><div className="setting-icon"><Sparkles size={21}/></div><div><strong>Smart Rename</strong><p>Missing BPM from FLP; key analysis from rendered audio.</p></div><span className="settings-value">Planned</span></div><div className="settings-item"><div className="setting-icon"><CircleHelp size={21}/></div><div><strong>About</strong><p>1CE Desktop {status?.version || '0.2.0-alpha.1'} · Windows-first</p></div><span className="settings-value">Scan-only</span></div></div></div>
        )}
        <footer className="footer"><span><span className="footer-led" /> {status ? 'Rust service connected' : 'Preview · desktop services unavailable'}</span><span>Made for producers <span className="footer-heart">✳</span> 1CE</span></footer>
      </main>
    </div>
  );
}
