# 1CE Desktop v2 · incremental roadmap

Updated: 2026-11-08. Prioritize correctness and protected local files over the number of PRs. Items below are sequential checkpoints, **not one PR each**.

| Stage | Scope | Exit gate | Status |
| --- | --- | --- | --- |
| A. Foundation | Tauri 2, Rust command interface, React/TS UI, Organize/Upload split, native folder picker | Frontend build; Rust check on Windows; inspect first-level FLP without writes | **Implemented; awaiting real-file native smoke** |
| B. BPM and naming | Read-only Rust FLP BPM parser, legacy support, configurable non-writing name preview and title-token parsing | Unit fixtures for modern/legacy/truncation and name conflicts; real Windows FLP smoke still required; no automatic renaming | **Implemented; real FLP smoke pending** |
| C. Invisible export | Windows isolated desktop `CreateDesktopW` + `CreateProcessW` via a carefully reviewed Rust abstraction | Reproduce 3/3 FLP render test; timeout, child-process handling, cleanup, plugin intervention, no stray FL processes | Next **sensitive** |
| D. Integrity & organization | Fresh staging per attempt, MP3/WAV validation, actual stem validation, SHA-256, ZIP checks, atomic delivery, collisions, journal/restart | Failure injection; checksum and integrity tests; source files preserved | Later **sensitive** |
| E. Find Key | Offline shared Rust audio key detector (harmonic/percussive processing, HPCP/chroma, alternatives and confidence) | Compare against known-key WAVs + Antares reference; calibrate uncertainties; license review | Later |
| F. Smart Rename | Shared name-parser + detector for existing WAV/MP3; configurable user template | No duplicated BPM/key; preview first; unknown key stays for review; rename collision tests | Later **sensitive** |
| G. Upload | Persistent queue sending only to 1CE API; site owns social publishing/retries | Authenticated protocol, resumable uploads, dedup and partial-failure tests | Later **sensitive** |
| H. Release | Signed Windows installer, testing, updater strategy, rollback | Windows smoke, dependency audit, clean install/uninstall behavior | Later |

## Migration reference & invariants

- Keep v0.1 available as behavioral reference. Do not drop its collision handling, zip integrity checks, retry strategy, or source safety.
- `Organize` is for **local** export, folder organization, stems packaging and renaming. `Upload` is a separate layout for delivery to the 1CE service, with persistent queue/history.
- Use one reusable key analysis implementation for both export-time automatic naming and Smart Rename.
- Choose folder conventions/templates without modifying the original `.flp` by default.
- A successful process exit alone is **not enough** to certify a completed export. Require fresh, structurally valid audio and integrity checks.
- Note major/minor mode separately from abbreviated root-key label; confidence must be measured and calibrated. Never infer high confidence from an arbitrary threshold without validation.

## Evidence to retain

- **BPM PowerShell test**: `bigtruck 140 e @tofdidthat.flp`, `sup 140 f @tofdidthat.flp`, `spider 140 f# @tofdidthat.flp` → 140 BPM each, modern FLP event 156, 0 errors. No tempo automation validation.
- **Invisible desktop PowerShell test**: FL Studio invisible during three consecutive renders; each returned `OK`. Audio quality, actual stems count, cancellation/timeout safety, and behavior across VSTs still require validation.

## Current gate before invisible rendering

- Run `cargo test --manifest-path src-tauri/Cargo.toml` on Windows (GitHub Actions).
- Read the three real FLP files (`bigtruck`, `sup`, `spider`) via the native Tauri app and check 140 BPM and name previews; previous 3/3 pass was PowerShell-only.
- Confirm FL Studio's displayed BPM for at least one file. Detecting tempo automation is not implemented; multiple tempo-setting events trigger a manual review warning, but absence of these events does **not** prove no automation.
- Name templates are previews only: no file rename, move, export, or deletion occurs. If a field is missing, the program never invents a Key.

Next sensitive batch: design and test Windows invisible process lifecycle before enabling Export All.
