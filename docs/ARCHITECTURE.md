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
future: flp_metadata.rs    Parse BPM directly from FLP (no project writes)
future: naming.rs          Parse existing tokens, format naming templates safely
future: render/win32.rs    Isolated-desktop Windows process launching + cleanup
future: render/queue.rs    Durable sequential jobs; timeout and event reporting
future: audio.rs           Validate output files and stems
future: storage.rs         Staging, hashes, transaction journal, safe delivery
future: key.rs             Shared local Find Key engine + confidence/candidates
future: upload.rs          Transfer queue to first-party 1CE API only
```

## Foundation boundaries (current)

`scan_beat_folders()` receives user-selected local paths, inspects immediate children for exactly one `.flp`, and returns status. It is limited to 250 entries and never opens/edits an FLP. No implicit folder operations.

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
