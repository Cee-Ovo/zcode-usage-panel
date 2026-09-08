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


# 2026-09-09 三任务改造验证(速度指标 / 状态防抖 / DSH 三分区)

## Delivered

- **三分区架构**:仪表盘统计区由 ZCode / Codex / DSH 三个数据源分区组成(LiquidSegmentedControl 切换,选择持久化)。全局时间范围同时驱动三个分区;Codex/DSH 复用泛化的 `LocalUsagePanel`(原 CodexUsagePanel);服务额度/告警区保持全局。`providers/local_usage.rs` 抽取 Codex/DSH 共用的六档范围聚合(N provider 泛化)。
- **DSH provider**:`providers/dsh.rs` 离线读取 `~/.dsh/sessions`(设置/DSH_HOME 可覆盖),raw `.jsonl` 字节水位增量 + `.jsonl.zstd` 尺寸变化触发整读、事件 `seq` 水位去重;usage 取自 `assistant/message` 事件(TokenUsage 字段按官方文档:inputTokens 未缓存输入、reasoning ⊂ output 不重复累计);模型归属 `request/header`。防御式:未知事件跳过、坏文件降级 note、缺目录 NotInstalled 空态。本机未安装 DSH,空态 + 可配置路径交付(排查记录见 DESIGN-NOTES.md)。
- **响应速度指标**:`UsageRecord` 增加可选 `duration_ms/ttft_ms/status`(JSONL 别名探测 + SQLite 列映射,来自 ZCode `model_usage` 表原生字段);`aggregate::compute_speed_stats` 在查询期按范围现算:TTFT 原值均值 + 最近邻 P50/P95,tok/s = Σ(output+reasoning)÷Σ(duration−TTFT) 加权;error/cancelled/running 剔除;样本覆盖如实展示;无样本显示 unavailable。
- **状态防抖**:engine `error_streak` 连续失败计数,`gate_error` 在 ≥2 个失败周期后才对外暴露错误(成功即清零;数据根消失改为每周期重检);前端 `HealthTracker`(episode 计数,修复 coordinator catch+finally 双发导致的重复计数)统一驱动状态点、pill、运行详情与重试入口;sidebar-smoke 更新为新语义。

## Automated verification

- `npm test`:47 frontend tests passed(新增 health 8 项、format 6 项)。
- `cargo test --manifest-path src-tauri/Cargo.toml`:134 tests passed(新增 DSH 11 项、local_usage 2 项、speed stats 4 项、engine streak 3 项、sqlite timing fixture 扩展)。
- `npx tsc --noEmit`:passed。
- `cargo run --example speed_smoke`:对本机真实 `model_usage` 表验证(today: 112 TTFT 样本/164 请求,均值 1.87s,P95 2.63s,75 tok/s;7d: 1285/1310 completed,错误行正确剔除)。
- Browser(playwright-core, 系统 Chrome):三分区切换双主题截图、DSH 空态(`?dsh=missing`)、DSH 详情弹窗、700px 无横向溢出、精简视图满排、sidebar-smoke(暂停/恢复/瞬时失败不翻转/连续失败转红/重试恢复)全部通过;截图留档 `output/playwright/`(gitignored,可用 `scripts/dev-shot-*.cjs` 复现)。
