# Development and merge workflow

Keep changes manageable without unnecessary PR overhead.

- Default: work in **coherent batches** and merge after checks. UI, copy, docs, and small self-contained features can be committed directly to `main` where sensible.
- Use a **separate branch/PR for sensitive work**: original file mutation, Windows process manipulation, deleting/moving files, data recovery, authentication, uploads, distribution, security, and migrations.
- For sensitive changes, write down rollback options and expected behavior, run unit/build/Windows smoke tests, inspect diff, then merge. No force-push to `main`.
- Prefer at most one active PR for ordinary work; never split PRs just to increase counts.
- Keep the reference C# engine untouched until Rust behavior matches verified requirements.
- Never claim CI/Windows validation unless the actual runner completed. Browser preview does not validate native Tauri API.
- Tag risky functionality disabled by default until it passes the Windows smoke matrix.

## Local development

```powershell
npm install --no-fund --no-audit
npm run build
npm run tauri dev
```

Native Rust checks and Windows packaging need Windows + Rust toolchain (CI or local). Before a release, test with real FLP samples and a sacrificial output location, not the user's production beat folder.
