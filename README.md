# 1CE Desktop

Desktop-first beat workflow for FL Studio: **Export locally → Organize → Send to 1CE**.

> Status: v2 rebuild in progress. The current repository is being initialized incrementally. Do not use this branch as a production exporter yet.

## Product boundaries

- **1CE Export (local):** batch-render FL Studio projects, verify audio, organize outputs, and support Smart Rename.
- **1CE Upload (local):** queue and transfer approved outputs to the 1CE API. It does not publish directly to social platforms.
- **1CE website / API:** manages destinations, publishing, retries, and final delivery status.

## Migration policy

The existing C# Windows Forms application remains the behavioral reference until each capability is replaced and validated. The successful Windows proof of concept (invisible desktop batch rendering of three FLP projects) must be reproduced in Rust before replacing the exporter.

Work is broken into small, reviewable PRs. Read [docs/ROADMAP.md](docs/ROADMAP.md) after the first scaffold PR is opened.

**Important:** do not modify original `.flp` files or render into their source folders during early migration tests.
