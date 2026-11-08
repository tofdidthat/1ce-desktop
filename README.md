# 1CE Desktop · v2

The next generation of **1CE Desktop**, built with **Tauri 2 + Rust + React + TypeScript**. Native-first, Windows-first and designed for music producers using FL Studio.

**Current milestone:** foundation / safe read-only folder scanning. **This is not yet a replacement for the v0.1 exporter.**

## Product boundaries

| Mode | This desktop app | 1CE website / API |
| --- | --- | --- |
| Organize | Locally render FLP beats, verify files, organize folders, and Smart Rename | Optional storage of uploaded library artifacts |
| Upload | Queue files and transfer reliably to the 1CE API | Select and publish to social platforms, verify publishing, retry failed destinations |

The desktop app never assumes responsibility for publishing directly to YouTube or other social networks. The web app owns downstream delivery status.

## Start on Windows

Install **Node.js 22+**, **Rust stable** with MSVC toolchain, **Microsoft C++ Build Tools** and **WebView2** following the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Run in PowerShell:

```powershell
npm install --no-fund --no-audit
npm run tauri dev
```

`npm run dev` launches a **browser-only visual preview**; it cannot open native folder dialogs or scan local files. Native features require `npm run tauri dev`.

## What actually works today

- Interface for Organize / Upload / Settings, adapted to desktop dimensions, using the original 1CE logo.
- Native multi-folder selection in Organize (Tauri dialog).
- **Read-only** Rust scan of selected folders; one `.flp` at the top level is considered eligible for a future export task.
- Remove selections in the interface. No state persisted yet, and no disk mutations.
- Export button intentionally disabled until the invisible Rust renderer and file-safety suite are validated.

## Validated outside this repository

On the user's Windows workstation, the earlier scripts produced successful results on `bigtruck`, `sup`, and `spider`:

- FLP base tempo parsing: **140 BPM for all three** (event 156), matching folder titles; tempo automation not evaluated.
- Windows hidden-desktop FL Studio rendering: **3/3 reported OK**, running sequentially without visible FL Studio windows.

These were PowerShell proof-of-concept tests, **not a Rust implementation** and **not yet independent WAV/stem or key accuracy verification**.

## Documentation

- [Development roadmap](docs/ROADMAP.md)
- [Architecture and safety rules](docs/ARCHITECTURE.md)
- [Git and release workflow](docs/WORKFLOW.md)

Source migration reference: legacy v0.1 implementation from `codigo-fonte.zip` (C# Windows Forms). Source is intentionally **not committed wholesale** while the new modules are built and verified.
