# ZCode Usage Panel

本地 **AI Coding Usage Dashboard**:ZCode Token 实时统计与等价 API 花费之外,同时管理 **OpenAI Codex 套餐额度、Antigravity/反重力额度、火山引擎 Token 包**,并提供 **ZCode 一键启动/唤醒**。Windows 桌面应用:仪表盘、系统托盘、QQ 式桌面边缘吸附小窗。浅色 Liquid Glass 设计(基于 [open-glass-ui](https://github.com/moekoelueker/open-glass-ui)),长期驻留、空闲近零开销。

![tech](https://img.shields.io/badge/Tauri%202-React%2018-TypeScript-blue) ![license](https://img.shields.io/badge/license-MIT-green)

---

## 目录

1. [功能总览](#功能总览)
2. [构建与安装](#构建与安装)
3. [ZCode 数据源与统计口径](#zcode-数据源与统计口径)(重要)
4. [边缘吸附实现](#边缘吸附实现qq-式贴边自动隐藏)
5. [系统托盘与 Popup](#系统托盘与-popup)
6. [无边框窗口缩放](#无边框窗口缩放)
7. [性能设计](#性能设计)
8. [复用 open-glass-ui 说明](#复用-open-glass-ui-说明)
9. [开发与测试](#开发与测试)
10. [已知限制](#已知限制)

---

## 功能总览

| 模块 | 说明 |
|---|---|
| **仪表盘** | 今天/60分钟/24小时/7天/30天/全部;ZCode 总 Token、API 等价花费、Input、Output、Reasoning、Cache、请求数、Cache Hit Rate、活跃模型 9 项核心指标;Codex 本地 Token 独立分区(今日/7天/累计 + 模型排行,与 ZCode 总 Token、官方套餐额度分开统计);Top 3 模型可展开全部 |
| **实时趋势** | 字段堆叠图 + 按模型折线,模型可单独显隐;当前 Session 消耗/增速/最近请求/模型切换记录;数字 220ms 补间动画 |
| **Sessions 页** | 最近 Sessions 列表(项目/模型/起止时间/五类 Token/命中率),点击查看 Session 内趋势 |
| **模型详情** | 今天/7天/30天/全部、平均每请求 Token、I/O/R 比例、命中率、30 天趋势、Session 分布 Top 10 |
| **等价花费** | 按官方 API 单价估算 USD/CNY 成本:UI 明确标注"按官方 API 单价估算 · 非实际 Billing"。内置 zai / DeepSeek / Anthropic / OpenAI / xAI / Moonshot / 字节 7 家价格表(编译进二进制),支持模型级用户覆盖、远程价格表拉取与促销价到期自动回落 |
| **峰谷计费** | DeepSeek 按北京时间(固定 UTC+8)工作日峰谷分时计价(峰段 9–12 / 14–18 时,周末为谷段;节假日按工作日处理);缓存读/写按独立单价计费,与输入/输出分项对齐 |
| **汇率换算** | USD→CNY 每日自动刷新(frankfurter.dev),失败时回落内置汇率;所有成本展示同时提供两种币种 |
| **智能挂起** | 主窗口与 Popup 全部隐藏(托盘/贴边滑出)时自动暂停监控轮询,进入手动刷新模式;任意窗口重新打开瞬间自动刷新一次;标题栏常驻手动刷新按钮 |
| **异常检测** | 10 分钟激增/单 Session 超阈/命中率骤降/模型连调/数据停滞,本地 Windows 通知,阈值全部可调可关 |
| **系统托盘** | 关窗最小化到托盘;菜单含显示主面板/今日用量/启动 ZCode/显示 ZCode/暂停监控/吸附开关/AOT/开机启动/设置/退出;左键弹 Glass Popup |
| **边缘吸附** | QQ 式贴边自动隐藏,详见下文 |
| **导出** | CSV/JSON × 时间范围/模型/Sessions/原始记录,系统保存对话框,导出位置完全由用户决定 |
| **价格设置页** | 查看/覆盖任意模型单价(峰/谷、缓存、单位),支持一键拉取远程价格表、恢复内置默认;Dashboard 成本汇总与模型成本明细弹窗同源 |
| **单实例** | 二次启动只唤出已有窗口,不产生第二个监控进程 |
| **服务额度** | 统一 Provider 框架:Codex(官方 rate_limits:5 小时窗口/周额度/credits,本地离线读取)、Antigravity(官方本地 RPC)、火山引擎 Token 包(费用中心 OpenAPI,多包聚合+到期提醒);失败自动降级,绝不伪造数据;Codex/Antigravity 本地 session 日志 Token(今日/7天/累计 + 按模型明细)独立展示,Codex 来源模型名带「（Codex）」标记(仅展示层,不改动原始数据与查询) |
| **额度趋势** | 额度快照本地持久化(SQLite,去重写入+400 天保留),变化趋势/每日消耗/**预计耗尽时间**(线性回归,明确标注「预测」,样本不足不显示) |
| **额度提醒** | 剩余 50/20/10%、Token 包 7 天到期、额度即将重置(用量 ≥80% 且 30 分钟内重置)、Provider 数据停更、API 成本阈值;同一事件冷却去重(6 小时/天级) |
| **ZCode 快捷启动** | 多路径自动检测 + 用户覆盖;未运行一键启动,已运行聚焦原窗口;状态/版本显示;托盘菜单直达;随本软件自动启动(可选) |
| **凭据安全** | 火山引擎 AK/SK 存入 **Windows 凭据管理器**(keyring),不写文件、不进日志、不出现在任何 UI/错误信息 |

## 构建与安装

### 方式一:GitHub Actions 自动构建(推荐)

推送到 GitHub 后,`.github/workflows/build-windows.yml` 会在 `windows-latest` 上:

1. 运行 vitest 前端测试、`cargo test` 数据层测试;
2. 运行 `cargo run --release --example bench`(100 万条合成记录基准);
3. 生成图标集并执行 `tauri build`(NSIS);
4. 产出 **`ZCode-Usage-Panel-Setup-<版本>.exe`** 与 **`ZCode-Usage-Panel-Portable-<版本>.zip`**,tag 构建自动挂到 Releases。

下载 `Setup.exe` 双击安装(NSIS per-user/machine 双模式,不需要管理员权限即可 per-user 安装;正常创建开始菜单项,可选桌面快捷方式,可正常卸载;卸载不会删除你主动导出的数据)。Portable 版解压即用。

### 方式二:本机构建(Windows)

```powershell
git clone https://github.com/Cee-Ovo/zcode-usage-panel.git
cd zcode-usage-panel
npm install          # 安装依赖;图标由 predev/prebuild 钩子在 dev/build 前自动生成
npm run tauri dev    # 开发调试
npm run tauri build  # 产出 src-tauri/target/release/bundle/nsis/*.exe
```

要求:Node ≥ 18、Rust stable(含 `x86_64-pc-windows-msvc`)、WebView2(Win11 自带)。

> 注:图标集由 `scripts/ensure-icons.mjs` 生成(纯 Node PNG 编码器,无额外依赖);仓库已包含生成产物,删除后 `pre*` 钩子也会在构建前自动重建。

## ZCode 数据源与统计口径

### 数据源(全程只读)

| 来源 | 处理方式 |
|---|---|
| 数据根目录 | 设置中手动指定 → `ZCODE_HOME` 环境变量 → `%USERPROFILE%\.zcode` |
| `*.jsonl`(transcripts/usage 流) | **增量追加读取**:每个文件维护字节水位,只解析新增的完整行;半行(正在写入)自动缓冲到下次刷新;文件截断/轮转换名自动重置;`projects/<项目目录>/<session>.jsonl` 结构自动提取 session/项目 |
| `*.db` / `*.sqlite`(如安装的 ZCode 版本使用 SQLite) | **只读打开**(`SQLITE_OPEN_READ_ONLY` + `busy_timeout`),表结构**运行时发现**(sqlite_master + PRAGMA table_info 打分),rowid/整数主键水位增量;`BEGIN EXCLUSIVE` 等锁竞争返回 Busy 自动稍后重试;WAL 孤儿库回退 `immutable=1` 快照 |
| 文件监控 | Windows ReadDirectoryChangesW(notify crate)+ 用户可调去抖(默认 600ms);每分钟一次目录级安全重扫(仅列目录,不重读内容) |

**不硬编码 schema**:字段通过别名集匹配(`input_tokens`/`promptTokens`/`prompt_tokens_details.cached_tokens` 等三十余种),无法识别的数据源会进入「设置 → 检测数据源」的诊断面板(含每文件 offset/跳过行数/最近错误/最近 3 条记录抽样),供你直接与 ZCode 自带 Usage 页核对。**首次使用请做一次核对**:对比"今天"总 Token 与 ZCode Usage 页数字。

### Token 统计口径

- **总 Token** = Input + Output + Reasoning + Cache(读+写),**仅累加数据源真实提供的字段**;字段缺失显示 `unavailable` 或 `—`,绝不推算。
- **Coverage 标注**:若某字段只有部分记录提供(如老版本无 reasoning),UI 显示"覆盖 n/m 条记录"。
- **Cache Hit Rate**(统一口径,tooltip 同文):
  - 逐条记录自动判定 schema:
    - *inclusive*(input 已含缓存,OpenAI 风格):单条 total = input;
    - *exclusive*(input 不含缓存,Claude 风格):单条 total = input + cache_read + cache_write;
  - **命中率 = Σ cached ÷ Σ total**;无缓存字段的记录不计入分子分母;全部缺失时显示 `unavailable`。
- **ZCode 未运行时**:内存中保留最后一次统计继续显示;重启后先显示持久化快照(标记"缓存快照 · 同步中"),后台完成增量同步后自动切换为实时数据。

## 边缘吸附实现(QQ 式贴边自动隐藏)

Rust 侧独立线程的"吸附引擎"(`src-tauri/src/windows/snap.rs`),事件驱动:

1. **贴边**:拖动窗口(Moved 事件流)靠近工作区边缘(阈值 × DPI 缩放,默认 24 逻辑像素,可调)时记录候选;拖动静止 160ms 后贴齐边缘(150–240ms 缓出动画,可调)。
2. **自动隐藏**:贴边后经过延迟(默认 600ms,可调),窗口向边缘滑出,仅留 **4px 触发条**。
3. **呼出**:仅在"已贴边且隐藏"状态下,以 60ms 间隔调用一次 `GetCursorPos`(单次 Win32 调用,无消息钩子、无高频轮询)检测光标进入边缘条 → 200ms 滑入。
4. **防误隐藏**:窗口重新隐藏需同时满足:鼠标不在窗口内(前端 mouseenter/mouseleave 上报)、无打开的菜单/下拉/tooltip(前端 focusin 检测 `[role=menu|listbox|combobox|tooltip]` 等上报)、窗口无焦点、且延迟已过。
5. **取消吸附**:拖离边缘超过阈值+16px 自动解除。
6. **多显示器/DPI/任务栏**:全部几何使用 Win32 物理像素(`MonitorFromWindow` + `GetMonitorInfoW` 的 rcWork),适配任务栏四边、100–200% DPI、多屏任意排列;`ScaleFactorChanged` 与每 4 秒的轻量校验在分辨率变更/睡眠恢复/拔插屏后自动重新贴齐。
7. **持久化**:贴边方向、隐藏状态、窗口矩形防抖(700ms)写入 settings.json,重启恢复。

设置项:启用开关、自动隐藏开关、吸附方向(左/右/上)、阈值、隐藏延迟、动画时长。

## 系统托盘与 Popup

- 关闭主窗口默认**最小化到托盘**(可改为退出);托盘菜单十项;`TaskbarCreated`(Explorer 重启)由 Tauri 托盘层处理图标重建。
- **左键单击托盘 → Glass Popup**:窗口定位依据 `TrayIconEvent` 的图标矩形 + `MonitorFromPoint` 工作区,自动适配任务栏上/下/左/右、多屏、DPI 并钳位;以 `SWP_NOACTIVATE` 显示**不抢占焦点**;光标离开 ~600ms(120ms 采样,仅在弹出期间)自动收起,Esc 或点击主窗口也会收起。

## 无边框窗口缩放

- 8 个 5–10px 透明命中区(四边 + 四角,CSS 光标 `ns/ew/nesw/nwse-resize`),`pointerdown` 调用 Tauri `startResizeDragging` → **手势交给 Win32 原生 resize**(光标、DPI、贴边行为全部原生)。
- 最小尺寸 400×540(tauri.conf `minWidth/minHeight` 兜底);贴边状态下的重新贴齐由吸附引擎的 `Resized` 处理,与缩放不冲突。
- 标题栏 `data-tauri-drag-region` 拖动移动;窗口位置/大小记忆并在启动时恢复。

## 性能设计

- **空闲近零开销**:无定时器轮询数据;全部由文件系统事件驱动(ReadDirectoryChangesW);唯一的周期性工作是 5 秒一次的空转唤醒(用于 busy 重试检查)与每分钟安全重扫、4 秒一次的贴边校验(仅贴边时)。
- **UI 隐藏即挂起**:主窗口与 Popup 均不可见时,引擎自动进入挂起态(唤醒间隔降为 30 秒,跳过全部数据刷新),只保留快照持久化等最低限度维护;任一窗口显示的边沿瞬间恢复并立即刷新一次。与用户手动"暂停监控"开关相互独立。
- **增量**:JSONL 字节水位 + SQLite rowid 水位;刷新只解析新增字节,绝不重扫历史。
- **聚合缓存**:记录按时间排序存储,范围查询二分切片;Session 汇总增量维护;`usage-update` 事件服务端节流 ≥500ms;前端 selector 级订阅(`useSyncExternalStore`),图表纯 SVG 零图表库、全 memo 化,数字动画组件自渲染。
- **基准**:`cargo run --release --example bench` — 100 万条合成记录/1 万 sessions:批量 ingest、乱序批、"今天"切片、30 天分桶、全量模型分组、session 重建、RSS。CI 会在每次构建时打印该报告(见 Actions 日志)。
- 内存策略:启动快照仅存聚合(不存原始记录);全量记录常驻内存(每条约 100–150B,百万条 ≈ 120MB 量级,以 bench 实测为准)。

## 复用 open-glass-ui 说明

[open-glass-ui](https://github.com/moekoelueker/open-glass-ui)(**MIT License**)以 **npm 依赖**方式直接引入(`open-glass-ui@0.3.x`,零运行时依赖),未复制源码,符合其许可与 attribution 要求(见 `THIRD-PARTY-NOTICES.md`)。

实际复用:

| 复用内容 | 用途 |
|---|---|
| `open-glass-ui/styles.css` + `--ogui-*` 设计 token 体系 | 全局玻璃材质、圆角、焦点环、深浅色 token 基座 |
| `GlassSystemProvider` | 根主题提供者(light/dark) |
| `SegmentedControl` | 时间范围/主题/默认范围切换 |
| `Switch` | 所有设置开关、实时监控开关 |
| `Button` / `SearchField` / `TextField` | 刷新/导出按钮、搜索框、路径/价格覆盖输入 |
| 设计语言(frosted glass、半透明分层、极细边框、柔和阴影、150–250ms 动效) | `src/styles/theme.css` 在其 token 之上扩展 ice-blue 环境渐变 |

## 开发与测试

```powershell
npm test                       # vitest:格式化/口径/store
cargo test --manifest-path src-tauri/Cargo.toml   # 数据层:JSONL 增量/半行/截断/垃圾行容错、SQLite 发现/水位/busy、聚合与命中率两种口径、激增告警冷却
npm run tauri dev              # 开发运行
```

覆盖的可靠性场景(测试或代码内建):ZCode 运行/未运行/刚启动、SQLite busy(WAL/EXCLUSIVE)、JSONL 半行、数据文件删除/移动(Gone)、schema 变化(重发现)、大量历史(增量水位)、中文/Unicode 路径、多实例(单实例插件)、托盘反复开关(Popup 幂等)、DPI/多屏(物理像素几何 + 校验)、Windows 睡眠/锁屏恢复(显示变更重校验)。

## 服务额度 Provider 说明

| Provider | 数据来源 | 可靠性 |
|---|---|---|
| **Codex** | Codex CLI 官方 session 文件(`<home>/.codex/sessions/**.jsonl`,append-only 增量读取)中的官方 `rate_limits`(5h 窗口/周额度 used_percent、reset 时间、credits、plan_type)+ `total_token_usage`(本地 Harness token 分项统计) | 完全离线、官方数据;Codex 未登录→未配置,目录缺失→未安装,额度数据超 6h→标注过期 |
| **Antigravity** | 官方 language_server 本地 Connect-RPC(`GetUserStatus`/`RetrieveUserQuotaSummary`,仅 127.0.0.1,端口/CSRF 取自官方日志) | 客户端未运行→降级「未找到运行中的本地服务」;无公开远程 API,未安装→未安装 |
| **火山引擎** | 费用中心官方 OpenAPI `ListResourcePackages`(HMAC-SHA256 签名,本地实现) | 需要 IAM AK/SK(建议 BillingCenterReadOnlyAccess);多包分页全量拉取,千/万/百万 Token 单位自动换算 |
| **ZCode 卡片** | 现有引擎聚合(今日 Token、API 等价成本、命中率)+ Launcher 状态 | 与现有统计同源 |

**官方额度与本地统计严格分离**:Codex 卡片的「官方额度」来自官方 rate_limits,「本地 Harness 用量」来自 session 文件 token 统计,二者永不合并为一个指标。所有无法获取的字段显示 unavailable,预测值(预计耗尽时间)永远标注「预测」。

## 已知限制

1. **构建产物**:本仓库设计为由 GitHub Actions(或本机 `npm run tauri build`)产出安装包;仓库内不含二进制。
1a. **Antigravity 额度**:官方无公开远程 API,仅当官方客户端在本机运行且日志可解析出本地 RPC 端点时可用;失败时显示 unavailable(设计如此,不伪造)。
1b. **Codex 额度更新时机**:官方 rate_limits 在 Codex 发起请求时刷新,本软件离线读取其落盘值;长时间不用 Codex 时该值会标注「数据过期」。
1c. **火山引擎**:需用户自行配置 AK/SK(系统凭据管理器);Token 包为费用中心口径(递减型资源包),与按量计费的 API 用量是两个体系。
2. SQLite 表若无 rowid 且无整数主键,退化为"全扫 + 行哈希去重"(上限 50 万行,超出重置并在诊断面板提示)。
3. JSONL 文件被截断重写时按"从头重读"处理,极端情况下该文件的 session 汇总可能短时双计(下一次全量刷新自愈)。
4. 等价花费为估算值:内置价格表 + 用户覆盖仅为官方单价快照,与任何实际 Billing 无关;峰谷判定为 DeepSeek 公开的北京时间规则,不随官方调价自动同步(可通过远程价格表或手动覆盖更新)。
5. 无 reasoning/cache 字段的旧数据源,相应指标显示 unavailable(设计如此,不推算)。
6. Windows 专属行为(吸附/托盘 Popup)在非 Windows 平台编译为空实现(便于数据层跨平台开发测试)。

## License

MIT(见 `LICENSE`)。UI 组件 open-glass-ui 同为 MIT,见 `THIRD-PARTY-NOTICES.md`。
