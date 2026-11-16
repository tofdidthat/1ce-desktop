# Development roadmap

Current checkpoint: local alpha 0.2.0-alpha.1, consolidated 2026-11-11. Implemented does not mean release-certified.

## Implemented locally

- Native folder drop, newest-FLP selection and read-only base BPM.
- Editable queue titles, Smart Rename preview and standalone source rename without per-beat confirmation.
- Rust-owned FL rendering, scoped stems setting, WAV/MP3 checks, bounded cancellation/startup handling.
- Automatic post-export key analysis overlapping the next render; manual scale exception without rerender.
- Three configurable destinations, verified no-replace publication and journaled source move.
- Clean project layout: no generated ownership marker; omit empty stems, preserve existing populated folders.
- Interrupted publication recovery; reverify completed moves without the old source; clear guidance for surviving holding folders.
- Manifest-checked private working-copy cleanup after success, with conservative retry/preservation.

## Next: correctness before release

1. Improve Find Key against labeled real music, including the user's SPENDALL D# minor case. Measure wrong estimates and inconclusive rates; do not force uncertain output.
2. Test the complete UI with real disposable projects: multiple beats, manual key exception, cancel/restart, changed source/destinations and folder locks.
3. Validate cross-drive moves, sample references, large projects, disk-full and power-loss scenarios. Implement guarded recovery for partially removed holding folders; currently manual inspection is required.
4. Design persistent queue/history and storage retention. Retain incomplete/unknown attempts; a cleanup policy needs explicit ownership, recoverability and disk-space tests.
5. Add upload: account authentication, named/keyed artifacts, destination hash reverification, resumable transfer and deduplication. Website/API owns social publishing.
6. Add licensing: account/device activation and signed offline entitlement. Define offline grace and recovery before enforcing it. No licensing exists yet.
7. Prepare release: dependency audit, Windows installer signing, verified updater/rollback and clean install/uninstall testing.

## Acceptance rules

- A process exit, analysis reaching 100%, or a valid audio file alone is not a completed beat.
- Final readiness requires verified named destinations and successful source-move completion.
- Never overwrite collisions or remove changed/unverified originals.
- Real FL/VST and listening checks must be reported separately from synthetic/unit tests.
- No release/merge/deploy without the user's authorization.

See [validation evidence](VALIDATION.md) for what has actually been exercised.
