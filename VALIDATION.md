# Reliability and usability validation — 2026-09-05

## Delivered

- One in-flight range query with one coalesced follow-up. Different page/range results are rejected; same-range completed snapshots are rendered to avoid starvation under continuous updates. Effect-scoped coordinators and late-listener cleanup support React StrictMode.
- `get_usage_view` returns dashboard, optional trend, cost summary and ingestion revision under one engine lock with one range boundary. The Models page omits trend computation; hidden/non-data pages do not request dashboard data. Legacy commands remain compatible.
- Active-session ID is incrementally maintained and its summary is directly looked up, without rebuilding and sorting the entire session list.
- JSONL lines over 8 MiB are counted once and discarded in bounded chunks through their terminating newline. Later valid records remain readable. Normal partial lines wait for completion; backlog chunks get a follow-up refresh. Exact-limit and truncation cases have tests. Source files are never edited.
- History health distinguishes persistent storage, memory fallback and failed writes. Checked transactions roll back failed multi-row writes; failed disk initialization leaves the original database intact and creates a usable in-memory schema. Diagnostics sent through the health IPC are generic, without paths or SQL details.
- Sessions queries search the complete summary set before stable sorting and paging (25/50/100 UI page sizes). Search by session/project/model, true matching totals, retry feedback, latest-detail protection, keyboard rows and visibility-aware refresh are included.
- Shared accessible dialogs provide Escape dismissal, Tab containment and focus restoration. Dashboard compact mode is optional and preserves the existing detailed default. Local diagnostic paths/details are hidden until explicitly revealed.
- CI installs locked frontend dependencies using `npm ci`. Development browser IPC uses synthetic mocks and is tree-shaken out of production builds; unsupported side effects fail explicitly.

## Automated verification

- `npm test`: 33 frontend tests passed.
- `npm run build`: TypeScript and production Vite build passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --offline`: 115 tests passed, including atomic view range/revision, full-history paging, JSONL boundary, transaction rollback and fallback storage checks.
- `git diff --check`: passed.
- Browser regression with 620 synthetic sessions: compact/detailed toggle, search for the 620th session, paging, Session/model/cost dialogs, Escape, Tab containment, focus restore, latest-range response, maximum one in-flight query, generic error display and retry passed. No page runtime errors in the successful run.

Reproduce the browser regression from the repository root (requires Playwright CLI and a supported browser):

```powershell
# Terminal 1
npm run dev -- --host 127.0.0.1

# Terminal 2; this uses DEV synthetic fixtures, not native desktop IPC.
npx --yes --package @playwright/cli playwright-cli -s=zup-test open http://127.0.0.1:5173/
npx --yes --package @playwright/cli playwright-cli -s=zup-test run-code --filename scripts/browser-smoke.js
npx --yes --package @playwright/cli playwright-cli -s=zup-test close
```

Browser artifacts are ignored under `output/playwright/` and `.playwright-cli/`.

## Performance observations

The release benchmark uses 1,000,000 synthetic records and 10,000 sessions. Three consecutive final runs measured:

| Measurement | Observed range |
| --- | --- |
| Batched ingest | 1,618–1,684 ms |
| 30-day trend | 106.5–108.0 ms |
| Full-history model grouping | 183.8–216.3 ms |
| Direct active-session lookup | 0.003–0.005 ms |
| Full session-list construction | 15.2–19.7 ms |
| Windows working set | 272.4–272.5 MiB |

Earlier measurements in the same work session were substantially faster even for unchanged aggregation code. Host load/conditions were not controlled, so these numbers are sizing observations, not a before/after speedup claim. The measured direct lookup is separate from full session-list construction. Windows memory reporting now reads the process working set instead of reporting a placeholder zero.

## Explicit boundaries

No installation, credential changes, real account calls, source-data migration, or source-log writes were performed. Native tray behavior, multi-monitor DPI, edge docking, sleep/resume and real provider/keyring/export dialogs were not end-to-end exercised; browser mocks do not certify those paths. Existing Rust dead-code/unused-result warnings remain outside this change.

The conditional deeper performance work from the proposal—provider concurrency, splitting the ingestion mutex, and cross-request price-aware caches—has not been enabled. This change removes known redundant work and establishes a repeatable benchmark first; a controlled lock-wait/provider-latency baseline is still needed before expanding those concurrency/cache boundaries.
