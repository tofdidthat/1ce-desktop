# 1CE Desktop v2

Windows-first local workflow for FL Studio producers, built with Tauri 2, Rust, React and TypeScript.

**Version: 0.2.0-alpha.1.** Local export and organization work; musical key detection is experimental. This is not a release-ready installer. Upload, licensing and automatic updates are not connected.

## Current workflow

1. Drop project folders onto the Beat queue. The newest top-level `.flp` is selected by modification time; equal timestamps use a deterministic filename tie-breaker.
2. Configure the naming template, FL executable and three destinations in Settings: MP3, Stems + master WAV, and Projects.
3. Edit queue titles if needed. Smart Rename previews the name; **Rename** applies folder and selected-FLP names without per-beat confirmation. Other file contents are preserved.
4. **Export** starts without a confirmation dialog. FL renders sequentially; local key analysis and delivery of a finished beat can overlap the next render.
5. A usable key estimate replaces `{key}` automatically (`F#m`, `F#maj`). An inconclusive result pauses that beat; choose root/mode and **Export with this key** to finish the existing audio without rendering again.
6. Final audio and project copies are verified before the source project folder is removed. Existing destination files are never overwritten.

The queue scrolls within its panel. Rename/Export remain visible. The exported counter counts verified final deliveries, not imported projects or analysis reaching 100%.

## Destinations and file safety

- MP3: `<MP3 destination>/<final name>.mp3` when requested.
- WAV/stems: `<Stems destination>/<final name>/`; master WAV and requested stems share this folder. No separate master directory.
- Project: `<Projects destination>/<final name>/`, with the selected FLP named `<final name>.flp` and other assets preserved.
- Empty top-level `stems` is omitted from delivered projects; populated stems are preserved. No `.1ce-owner` marker is generated in music folders.
- Technical attempts and diagnostics live under `%LOCALAPPDATA%\1ce-desktop\exports`, outside music destinations.
- Export **moves** the project after verifying destinations and the source snapshot. Standalone Rename changes source names without exporting.
- Absolute sample references inside FLPs are not rewritten. Test reopening relocated projects; external samples may need relinking.

## Recovery and internal cleanup

Settings ? **Delivery recovery** opens private export records. Select the relevant `1ce-attempt-*` folder to verify or resume its saved paths. Old attempts in music destinations can be selected manually if compatible with current records.

Interrupted publication can resume while source/configuration still match. Completed source moves can be verified again even after the source is gone. An interruption during source removal may leave `.1ce-moving-*`; preserve it and the `source-move.json` journal for manual inspection. Do not delete or re-export blindly.

After verified source-move completion, only the manifest-checked private `render-input` copy is cleaned. Changed, extra or unmanifested files are retained. Diagnostics, verified processing audio and incomplete attempts are retained for recovery; there is no broad automatic cleanup of past attempts.

## Development on Windows

Install Node.js 22+, Rust/MSVC, Microsoft C++ Build Tools and WebView2. See [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```powershell
npm install --no-fund --no-audit
npm run tauri dev
```

Run only one development instance. Port 1420 is fixed; an existing Vite process must be identified before starting another. `npm run dev` alone is a browser preview without native file/export commands.

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

Real FL tests are ignored by default and require explicit disposable fixtures. Never run them against production music folders.

## Known limits

- Key estimates are uncalibrated. SPENDALL is reported by the user as D# minor; the detector previously returned A# major and currently remains inconclusive on that retained export. Do not treat estimates as certified musical truth.
- FL/plugin compatibility, listening acceptance, power loss, cross-drive behavior and real-project sample references need broader validation.
- Selected queue folders/jobs are not a persistent library. Internal journals support delivery recovery, not automatic restoration of the entire UI queue.
- No network upload or social publishing is performed. The website/API will own downstream publishing.
- Local development changes are not automatically a GitHub release or distributed installer.

## Documentation

- [Architecture and safety contract](docs/ARCHITECTURE.md)
- [Roadmap and remaining gates](docs/ROADMAP.md)
- [Development workflow](docs/WORKFLOW.md)
- [Validation evidence](docs/VALIDATION.md)
