# Architecture and safety contract

Current local behavior, consolidated 2026-11-11. This document replaces the former incremental phase descriptions.

## Responsibilities

React/TypeScript presents Organize, Upload and Settings. Tauri exposes explicit Rust commands. Rust owns filesystem validation, FL process lifecycle, audio checks, analysis, delivery and journals. Export/organization run locally without a service dependency. Upload is not implemented; social publishing belongs to the 1CE API/web service.

## Modules

- `scanner.rs`: read-only top-level FLP scan and newest-file selection.
- `flp_metadata.rs`: base BPM from modern event 156 and legacy 66/93. This is not complete tempo-automation detection.
- `naming.rs`: title tokens, templates, conflict checks and final root/mode formatting.
- `rename.rs`: no-replace standalone folder/selected-FLP rename, snapshots and recovery journal.
- `render/mod.rs`: configuration, destination checks, render orchestration and fresh audio collection.
- `render/win32.rs`: owned Windows child process/job, cancellation and timeout.
- `render/settings.rs`: scoped FL split-track preference override and restoration.
- `render/source.rs`: neutral private project copy and manifest-checked working-copy cleanup.
- `render/wave.rs`, `integrity.rs`: audio structure, SHA-256 and checked reads.
- `render/jobs.rs`, `pipeline.rs`: sequential render with bounded background key/delivery work.
- `render/finalize.rs`: saved delivery plan, checked staging, no-replace publication, verified source move and explicit recovery.
- `key.rs`: experimental offline WAV key detector.
- `DeliveryPanel.tsx`: manual scale exception and Settings delivery recovery.

## Scan and standalone rename

Scan is read-only, limited to selected folders, and chooses the latest top-level regular FLP. Equal timestamps have a deterministic tie-breaker. Invalid BPM/metadata produces warnings, not proof a render will work.

Queue titles can be edited. Rename applies sequentially without a per-beat confirmation and stops on errors. It rechecks the selected FLP and source snapshot, uses non-replacing Windows moves, and renames only the folder and selected FLP. Other FLPs retain names; FLP bytes and absolute sample paths are not rewritten.

Rename uses flushed write-ahead records. Pending operations block new renames. Recover rename asks before restoring original names and refuses changed snapshots/collisions. This is separate from delivery recovery.

## Export lifecycle

1. Validate FL executable, inputs, template and three non-nested destinations separate from the source.
2. Copy the project tree into private `attempt/render-input`. Relative assets are preserved; the selected working FLP gets a neutral name to avoid observed FL command/path compatibility failures. Copies are hash-checked. Reject reparse points, excessive depth/entries and changed inputs.
3. Run FL with scoped stems on/off. Compatibility mode may display FL. The owned process/job supports cancellation/timeout; never terminate independent FL sessions. Only narrowly identified early startup/no-audio failures get bounded retries.
4. FL writes new uniquely prefixed WAV/MP3 into the original beat's `stems`. Existing files remain untouched. After FL exits, collect only this prefix's audio into the private attempt using copy/hash verification before removing the generated files from the source stems directory.
5. Exit code 0 is insufficient. Require actual valid master, requested MP3/stems and integrity records.
6. Analyze the validated master locally, possibly while the next beat renders. Inconclusive analysis leaves the beat pending; manual root/mode finishes existing audio.
7. Prepare final names and immutable delivery inputs. Stage and verify every destination before publishing with same-volume, no-replace moves. Publication across three roots is not globally atomic.
8. Verify final destinations, compare the complete source snapshot, move source to a unique sibling holding folder, compare again, then remove only individually verified files and empty directories. No recursive source deletion. Unexpected mutations preserve files; mismatches before deletion attempt to restore the source name.
9. Write source-move completion; only then clean the private manifested working copy. Retain validated processing audio and journals.

A source move can invalidate external absolute sample references. Reopening real relocated projects remains an acceptance gate.

## Layout and journals

Private root: `%LOCALAPPDATA%\1ce-desktop\exports\1ce-attempt-*`.

Important records: `renderer-command.json`, `audio-validated.json`, `key-analysis.json`, `working-copy.json`, `delivery-plan.json`, `delivery-intent-<kind>.json`, `delivery-complete.json`, `source-move.json`, `source-move-complete.json`, private `stage-<path hash>.json`.

`delivery-complete.json` records verified publication before source cleanup; its copy-stage sourceAction alone does not prove the project moved. `source-move-complete.json` records that later boundary. Returned successful receipts describe the completed move.

Final paths: MP3/name.mp3, Stems/name/(name.wav + requested stems), Projects/name/name.flp plus original assets. The roots are user-configurable and may be on different drives. Internal master.wav is a processing filename, never a master directory. Omit empty top-level project stems; preserve populated ones. Never generate ownership markers inside music folders.

## Recovery

All native export/delivery operations share an operation reservation. Explicit recovery opens the private root by default and uses the stored request, not a newly guessed title/key.

- Interrupted staging/publication: rebuild from original inputs/current config, require the saved plan to match, verify published targets and intents, then resume without overwriting.
- Completed source move: the source is no longer required. Verify completion records, key/name/id, current configured destination paths, publication intents, exact target trees and hashes before returning a moved receipt. Never remove a new folder subsequently created at the old source path.
- Interrupted source cleanup: if the holding folder survives, return its path and journal location for manual inspection. Do not automatically delete or restore partially removed trees.
- Legacy/incompatible/tampered records, edited destinations, changed configuration or sources stop recovery. Preserve evidence. There is no automatic migration for every historical owner-marker/empty-stems plan.

## Internal cleanup boundaries

The working-copy manifest is written only after copying succeeds. After delivery and source-move completion, compare every remaining private render-input entry against that manifest. Unknown/changed entries block cleanup. Missing entries allow retry after an interrupted cleanup. Hash-check files again before removal; remove directories individually and reject links/reparse points. Never recursively sweep attempts or music destinations.

Cleanup failure does not invalidate already verified delivery. Keep the private copy and a best-effort `working-copy-cleanup-warning.json` for inspection; explicit completed-delivery recovery retries cleanup. Warnings can remain as historical diagnostics after a successful retry. Incomplete/unmanifested working copies, processing audio and old attempts remain retained; automatic retention quotas/garbage collection are future work.

## Key detection limits

`1ce-key-v2-experimental`: local WAV PCM/float input, up to three sections, FFT/chroma and major/minor profile comparison. Section support accepts a candidate close to the local winner; global margin and agreement gates still reject ambiguity. Scores are not calibrated probabilities. Synthetic 24-key tests do not establish real-beat accuracy. No song-specific key replacements are allowed. The user's automatic naming workflow accepts a usable estimate, not musical certification.

## Release boundary

Upload/authentication, license activation, signed installer and verified automatic updates are future work. Do not describe local file safeguards or use of Rust as piracy protection. No paid provider or billing has been enabled.
