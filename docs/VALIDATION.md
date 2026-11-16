# Validation evidence and limits

Consolidated 2026-11-11. Historical results below were recorded during local development; they are not all rerun for every change.

## Previous recorded local evidence

- Rust unit suite before recovery/cleanup consolidation: 62 passed, 7 ignored.
- TypeScript check and Vite build passed after manual key finalization changes.
- Real disposable FL renders exercised WAV, MP3, both formats and stems on/off. Narrow no-audio/startup failures were also observed; FL/plugin compatibility is not universal.
- A real disposable render through checksums and verified project relocation passed; the external original input was unchanged. The test's reviewed key was not a musical accuracy claim.
- HOOD with original path spelling exited 0 without audio; a neutral working FLP copy rendered. The neutral-copy mitigation is implemented; the exact FL parser cause is unproven.
- Real retained E-minor audio formerly rejected solely by section disagreement was accepted by v2. SPENDALL remains inconclusive, with D# minor supplied by the user as ground truth.
- Synthetic tests cover 24 keys, unsuitable/noise/silence input and conflicting sections; these do not establish real-beat accuracy.

## Recovery/cleanup consolidation

New fixture tests cover completed recovery after source removal, destination/configuration tampering, preservation of a new folder at the former source path, holding-folder diagnostics, manifested internal cleanup, cancellation, unknown/changed files, missing manifests and retry after partial cleanup. Tests use disposable temporary trees, never production music destinations.

Executed on Windows after the final code changes:

- `cargo test --manifest-path src-tauri/Cargo.toml --quiet`: **67 passed, 0 failed, 7 ignored**. Ignored real FL/audio/preference tests were not run.
- `node --experimental-strip-types --test tests/smart-rename-preview.test.mjs`: **2 passed**.
- `npm run build`: TypeScript and Vite build passed.
- `git diff --check`: passed; Git reports line-ending conversion warnings for existing working-tree files.
- React recovery UI reviewed against the React best-practices checklist; no new effects, listeners or dependencies introduced. Native picker interaction was not exercised in the real GUI.

No production project or previous delivery was renamed, moved or cleaned. Fixture source moves/deletions stayed within newly created temporary test trees; build output was regenerated. No commit, push or release was performed.

## Not certified

Real UI click-through of this change, power-loss durability, cross-drive hardware behavior, musical accuracy, automatic repair of partial source cleanup, arbitrary FL/VST compatibility, release signing, licensing and upload.

Local test/build tools write build output and disposable temporary fixtures. They do not render, rename or move the user's production beats unless an explicit real test is requested with designated fixtures.
