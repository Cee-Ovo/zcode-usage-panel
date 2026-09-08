# 三项改造设计决策记录（2026-09）

本文件记录仪表盘三项改造（速度指标 / 状态防抖 / DSH 三分区）的设计决策、依据与放弃项。
配套调研证据见文末「数据管线调研结论」。

---

## 任务一：ZCode 响应速度（TTFT / tok/s）指标卡

### 数据源结论（以真实数据为准）

ZCode CLI 在 `~/.zcode/cli/db/db.sqlite` 中维护专表 **`model_usage`**（本机 3088 行）：

| 字段 | 含义 | 本机实测 |
| --- | --- | --- |
| `started_at` / `first_token_at` / `completed_at` | 请求开始 / 首 token / 完成（epoch ms） | 有 |
| `duration_ms` / `time_to_first_token_ms` | 总时长 / **TTFT 原值** | TTFT 有值 2229/3051（completed） |
| `status` | `running/completed/error/cancelled` | 3051 completed / 13 error / 24 cancelled |
| `input_tokens` / `output_tokens` / `reasoning_tokens` / `cache_read_input_tokens` / `cache_creation_input_tokens` | token 细分 | 有 |

现有 `sqlite.rs` 通用 reader **已经在读这张表**（有专门测试 `zcode_cli_model_usage_schema_reads_real_records`），但只映射 token 列、丢弃时间列。JSONL 侧（rollout / transcript）也有 `durationMs`、`startedAt` 与逐 chunk 事件，但当前解析路径不从中提取 usage；TTFT 原值只有 SQLite 里有。

TTFT 为 NULL 的行（774/2685 main_turn）集中在小输出 tool-calls 请求——客户端未记录首 token 时间，不可推导。

### 口径设计

- **样本定义**：`status = completed` 且对应字段非 NULL 的记录。`error/cancelled/running` 剔除。
- **首字延迟（TTFT）**：直接取 `time_to_first_token_ms` 原值（不自行用时间戳相减近似——原生字段更准）。统计均值与 P95，样本数如实展示（`样本 X/Y 条请求`，Y=范围内 completed 请求数）。
- **Token 速度（tok/s）**：`Σ(output_tokens + reasoning_tokens) ÷ Σ(生成阶段秒数)`，生成阶段秒数 = `(duration_ms − ttft_ms)/1000`；无 TTFT 但有 duration 的记录按全程 `duration` 计并计入另一口径？——**不**，为避免双口径混算：速度样本仅限「TTFT 与 duration 齐备」的记录，tooltip 写明。聚合用总量比（加权平均）而非逐条比率的均值，避免小请求主导。
- **衍生指标取舍**：展示「均值 + P95 + 样本覆盖」。放弃 min/max（对离群传输毛刺敏感、信息量低）与逐模型速度（样本常不足）。
- **降级**：样本为 0 时显示 `unavailable`（与 Reasoning/Cache 卡一致），绝不编造。

### 计算链路

Rust 侧：`UsageRecord` 增加可选 `duration_ms` / `ttft_ms` / `status`（缺失即 `None`，JSONL/其它 schema 不受影响）；sqlite 列别名集新增 duration/ttft/status；`aggregate.rs` 新增纯函数 `compute_speed_stats(&[UsageRecord])`（在 `dashboard_from_inner` 查询时对范围内记录现算，不进入 `Agg`——避免污染 BootSnapshot/bucket 序列化结构且可算精确分位数）。DTO `DashboardDto.speed` 随 `get_usage_view` 原子返回，天然与时间范围联动。

### UI

新卡「响应速度」放在「请求次数」右侧（同一行：Cache · 请求次数 · 响应速度），值为 `2.8 秒 · 87 tok/s` 风格；副文本 `首 token P95 5.4s · 样本 123/420`；ⓘ tooltip 写全口径。栅格：`活跃模型` 卡从整行改为 span 4，与 HitRate(2) 同行，保持 6 列栅格满排。精简视图保留该卡。

---

## 任务二：左下角状态防抖

### 根因（代码证据）

1. **engine 侧逐周期翻转**：`engine.rs` `refresh_once` 中 `inner.last_error = errors.into_iter().next()`——每个刷新周期的**任意一条**瞬时错误（某文件一次读失败、scan 抖动、单行 malformed）都会把 `last_error` 置为 `Some`，随 `usage-update` 事件（≤2 次/秒）推给前端；下一周期成功则清空。绿→红→绿随刷新周期震荡。
2. **前端直接放大**：`App.tsx` `monitoringError = !!(update?.error || refresh.error || initializationError)`——单次事件即翻转红点；顶部「数据源异常」pill 用同一来源 `dash.dataError`（= 同一个 `inner.last_error`），一起闪。
3. **并发路径叠加**：queryCoordinator 的 `refresh.error`（`get_usage_view` IPC 失败）是第二个独立翻转源；「立即刷新」与自动刷新并发时两路状态各自翻转。

### 方案（双向修复，状态语义统一）

- **后端迟滞**：`EngineInner` 增加 `error_streak`（连续出错周期数）。周期有错 → streak+1；无错 → 归零并清 error。`UsageUpdateEvent.error` / `DashboardDto.data_error` 仅在 **streak ≥ 2** 时对外可见（持续故障 ≈ 10s 内暴露，单次瞬时错误不上屏）。细节错误始终保留在 `运行详情`（engine `last_error` 原值仍可通过 diagnose 查到）。
- **前端健康推导**：新纯模块 `src/lib/health.ts`——`createHealthTracker()` 消费 engine 事件 + coordinator 状态 + 初始化错误，输出统一 `level: ok | error | paused | suspended`。规则：queryCoordinator 单次失败在「最近成功 ≤ 60s」内不降级；连续 2 次失败或后端 streak 门控错误 → error；成功即恢复（恢复要快，降级要慢——迟滞双向不对称）；`initializationError` 立即 error（确定性故障）。
- **一致性**：左下角状态卡、顶部 pill 全部改读 store 中同一份 health 派生结果，杜绝一处绿一处红。
- **可扩展**：health tracker 输入预留 providers 快照位（`provider degraded` → 汇总为 amber 提示，不翻红），任务三 DSH 即插即用。

放弃项：纯 UI debounce（setTimeout 遮丑但不改语义，pill 与卡仍可能不一致）；把瞬时错误显示为 amber 常驻（用户要求"不要跳"，amber 闪同样刺眼）。

---

## 任务三：三分区架构 + DSH 数据源

### DSH 本地日志调研结论

**本机（Linux 侧）未安装 DeepSeek Harness**，排查过：`~/.dsh`、`$DSH_HOME`、`~/.config|~/.local/share` 全部 deepseek/dsh/harness 命名目录、`which dsh`、cargo/go/npm 全局包、`/usr/local/bin`、`/opt`、全盘 maxdepth-4 find、`~/.zcode` 内部——零命中（仅 llama.cpp 的 deepseek 模型文件，无关）。DSH 应安装在用户 Windows 机器上。

据官方文档（deepseek.com/harness、GitHub deepseek-ai/deepseek-harness）：

- 日志目录：`~/.dsh/sessions`（`$DSH_HOME` 可覆盖），**默认 zstd 压缩 JSONL**（`.jsonl.zstd`，checksummed concatenated zstd frames），可配置为裸 `.jsonl`。
- 事件信封：`{ type, seq, time(unix ms), data }`；`assistant/message` 事件携带 `data.usage`（TokenUsage：`inputTokens`(未缓存输入)、`outputTokens`(含 reasoning)、`cacheReadTokens?`、`cacheWriteTokens?`、`reasoningTokens?`(⊂output，**不重复计入**)）与 `stream`（计时流记录，内部结构未公开文档化）。
- 模型归属：`request/header` 事件 `data.header.config.{provider, model}`（LlmCallConfig）。

### 方案

- **后端**：`providers/dsh.rs`（仿 codex.rs）：home 解析 `设置 > $DSH_HOME > ~/.dsh`；扫描 `sessions/**` 下 `.jsonl`（字节水位增量）与 `.jsonl.zstd`（文件变化时整读解压，按事件 `seq` 水位去重）；解析 `assistant/message.usage` + `request/header` 模型归属 + 路径推导 sessionId；产出与 Codex 同构的 `LocalUsage`（6 档范围）。防御式：未知事件/字段跳过不 panic；**DSH 不做速度指标**（stream 计时结构未公开，无法诚实计算——如实说明）。未找到 home → `NotInstalled` 空态 + 设置里可配路径。
- **花费**：DSH 分区不展示金额（官方公开单价核对成本高、且 DSH 本地日志无金额字段；拿不准就不展示）。
- **hub/settings**：`PROVIDER_DSH`、`dsh_enabled`(默认 true)/`dsh_home`/`dsh_refresh_ms`(默认 60s)；`overview()` 排序 zcode→codex→dsh→…。
- **前端三分区**：`CodexUsagePanel` 泛化为 `LocalUsagePanel`（props 驱动标题/口径文案/provider）；仪表盘统计区顶部新增分区切换器「ZCode | Codex | DSH」（LiquidSegmentedControl，OpenGlass 语言），ZCode 分区=现有指标栅格+模型排行+Session+趋势（行为/视觉零变化），Codex/DSH 分区=各自 LocalUsagePanel。**时间范围全局统一**：工具栏的时间范围标签同时驱动三个分区（Codex 原独立标签行移除——单一时间心智，切换分区不换口径）；「服务额度」区与告警区保持全局置底。
- **状态语义**：DSH provider 快照纳入任务二的 health 汇总。

放弃项：分区锚点堆叠（三段全展开页面过长，切换语义弱）；各分区独立时间范围（与全局工具栏并存易误导，操作绕）。

---

## 实施顺序与提交划分

1. `feat(providers): add DeepSeek Harness (DSH) local usage provider` —— 后端 DSH + 设置 + hub
2. `feat(ui): generalize local usage panel into switchable dashboard sections` —— 前端三分区 + devMock + 设置页
3. `feat(metrics): ZCode response speed (TTFT / tok-s) card` —— 任务一（落在 ZCode 分区内）
4. `fix(status): debounce monitoring status with streak gating + unified health` —— 任务二
5. `test/docs + 截图验收脚本` —— 收尾

## 数据管线调研结论（证据摘要）

- `~/.zcode/cli/db/db.sqlite` `model_usage` 表含逐请求 TTFT/duration/status/token 列（schema 与样本见上文）；`turn_usage` 为回合级汇总（无 model 列，不采用）。
- `~/.zcode/cli/rollout/model-io-sess_*.jsonl`：完整请求/响应快照（含 `durationMs`/`startedAt`/`response.usage`），当前未被解析提取 usage（`USAGE_OBJECT_PATHS` 不含 `.response.usage`），保持不动。
- `~/.zcode/cli/agents/**/transcript.jsonl`：`model_request/model_streaming/model_complete` 事件流可推导 TTFT，但与 SQLite 原生字段重复且解析成本高，不采用（SQLite 原生 `time_to_first_token_ms` 更准）。
- 速度实测样本：56–85 tok/s（GLM-5.3，本机），TTFT 均值约 1.2–2.2s——量级与参考 UI「2.8 秒 · 87 tok/s」一致。
