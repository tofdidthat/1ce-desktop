# Architecture & safety contract

## Technology

- **Desktop:** Tauri 2, Rust (Windows/MSVC); React 19, TypeScript, Vite.
- **Tauri IPC:** minimal explicitly registered commands. Start read-only; add scoped actions only after tests.
- **No backend dependencies for export/organization:** everything runs locally, including eventual key detection.
- **Web boundary:** desktop uploads to 1CE API only. The API manages YouTube/social publishing, confirmation, retry and external integrations.

## Modules as the application grows

```text
src/                       React + TS presentation, UI state, mode switches
src-tauri/src/lib.rs       Tauri app setup, narrow command exposure
src-tauri/src/scanner.rs   Current: read-only single-FLP beat-folder detection
src-tauri/src/flp_metadata.rs  Read-only FLP BPM event parser (156 modern, 66/93 legacy)
src-tauri/src/naming.rs        Safe title tokenization + configurable filename previews
future: render/win32.rs    Isolated-desktop Windows process launching + cleanup
future: render/queue.rs    Durable sequential jobs; timeout and event reporting
future: audio.rs           Validate output files and stems
future: storage.rs         Staging, hashes, transaction journal, safe delivery
future: key.rs             Shared local Find Key engine + confidence/candidates
future: upload.rs          Transfer queue to first-party 1CE API only
```

## Foundation boundaries (current)

`scan_beat_folders()` receives user-selected local paths and an optional naming template. It inspects immediate children for exactly one `.flp`, reads the base tempo **read-only**, and returns a naming preview and warnings. It is limited to 250 entries and never edits an FLP. No implicit folder operations. The `preview_beat_name` IPC command is pure and produces no filesystem writes. A scanned FLP with unreadable tempo is reported with a warning; it is **not** proof the project will render.

## Important invariants for later engine work

1. Never edit an original `.flp` or its parent directory during scanning or experimental export.
2. Stage rendering outputs in unique attempts outside source folders. Never mark stale prior output successful.
3. No overwrite without explicitly selected conflict policy. Do not follow reparse points, symlinks or junctions when moving/deleting source trees.
4. Validate MP3 and WAV content, identify the main mix independently of stems, and verify compressed archive entries/hashes.
5. No automatic source deletion on export. If reorganization is enabled later, require separate confirmed, audited, restart-safe copying/moving stage.
6. Keep process lifecycle tied to a specific Windows job: preserve handles, timeout, process-tree cleanup, and desktop-handle lifetime. Never kill unrelated user FL Studio processes.
7. In case of VST pop-ups or an interactive license prompt, provide a way to cancel or switch to visible desktop mode, instead of freezing silently.
8. No automatic key renaming on low-confidence or ambiguous results. Mode (`major`/`minor`) must be retained alongside root (`F#`).
9. The desktop does not distribute third-party DSP licensed under incompatible terms; evaluate TONART licensing separately if needed.

## Platform & version caveats

The Windows hidden desktop technique was confirmed in a PowerShell prototype, not a guarantee across all FL Studio/VST versions. It will be adapted in Rust only after safety, cancellation and process ownership tests. Current foundation intentionally does **not** launch FL Studio.

## Naming preview contract

- Supported tokens: `{name}`, `{bpm}`, `{key}`, `{producer}` (only once each; `{name}` required).
- Name parser recognizes trailing producer handles, tempo metadata and a note/scale suffix; user-provided BPM/Key are kept rather than silently overridden. Title-vs-FLP BPM conflicts flag manual review.
- Root note and scale mode remain separate in structured metadata, even if the template shows just a letter. An absent Key stays absent until the later Find Key stage; no guessed Key is inserted.
- Default: `{name} {bpm} {key} {producer}`. Template saved in the native webview's local storage. Existing selected folders are re-scanned on explicit Save.
- Current parser reads the FLP *base* BPM only. It does not guarantee detection of tempo automation or changes over time. More than one tempo-setting event is flagged but is not an automation detector.
- **No disk mutation:** this stage exposes suggestions, not a file renamer.
