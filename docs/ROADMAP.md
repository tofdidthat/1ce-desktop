# 1CE Desktop v2 · incremental roadmap

Updated: 2026-11-08. Prioritize correctness and protected local files over the number of PRs. Items below are sequential checkpoints, **not one PR each**.

| Stage | Scope | Exit gate | Status |
| --- | --- | --- | --- |
| A. Foundation | Tauri 2, Rust command interface, React/TS UI, Organize/Upload split, native folder picker | Frontend build; Rust check on Windows; inspect first-level FLP without writes | **In progress** |
| B. BPM and naming | Read-only FLP BPM parser, version support, name template and token parsing | 3 known FLPs at 140; missing/ambiguous BPM; tempo automation policy; no unintended renames | Next |
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

## Next task after foundation

Build a focused read-only Rust FLP tempo parser with unit tests and fixtures. Confirm with FL Studio's visible BPM for at least one project and check whether tempo automation exists, before using BPM in exported names.
