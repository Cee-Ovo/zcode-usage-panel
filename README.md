# ZCode Usage Panel

**一块面板，看尽你所有 AI 编程工具的用量。**

ZCode Usage Panel 是一个常驻 Windows 桌面的用量驾驶舱：把 ZCode、OpenAI Codex、Claude Code、DeepSeek Harness（DSH）四家的 token 消耗、响应速度、等价花费放进同一块毛玻璃仪表盘，并把 Codex 套餐额度、Antigravity 额度、火山引擎 Token 包也一网打尽。所有数据**只在你本机解析**——不上传、不联网统计、凭据进系统钥匙串。

![platform](https://img.shields.io/badge/platform-Windows%2010%2B-blue) ![tech](https://img.shields.io/badge/Tauri%202%20%C2%B7%20React%2018%20%C2%B7%20Rust-24c8db) ![license](https://img.shields.io/badge/license-MIT-green) ![release](https://img.shields.io/github/v/release/Cee-Ovo/zcode-usage-panel?color=8f7bff)

---

## ✨ 它能做什么

### 📊 四源统一仪表盘

| 数据源 | 说明 |
|---|---|
| **ZCode** | 总 Token、API 等价花费、Input / Output / Reasoning / Cache、请求次数、**响应速度（首字延迟 TTFT + tps，P95 与样本覆盖如实标注；另附近 24 小时 / 近 7 天固定窗口的 tps 对比行）**、Cache 命中率、活跃模型 |
| **Codex / DSH / Claude Code** | 各自客户端本地 session 日志的完整统计，**信息密度与 ZCode 分区对齐**——总量、缓存、请求、按模型排行、时间趋势、按官方单价估算的费用，一个不少 |

今天 / 60 分钟 / 24 小时 / 7 天 / 30 天 / 全局时间范围一键切换；字段堆叠趋势图 + 按模型折线，模型可单独显隐。

### 🗂 多源 Sessions 浏览

四个来源的会话合并在一个列表里，前缀一眼可辨：`cx-`（Codex）、`cc-`（Claude Code）、`dsh-`（DSH）、无前缀（ZCode）。会话名、项目、模型、起止时间、Token、命中率字段齐全，长值截断 + 悬停看全；全文搜索、分页、会话内趋势详情都开箱即用。历史归档（如 Codex `archived_sessions/`、DSH zstd 压缩流）一并覆盖。

### 🧮 四源合并的模型页

模型页把四个来源的模型行并成一张表，行尾括注来源（`（ZCode）` / `（Codex）` / `（DSH）` / `（Claude Code）`）——同一个模型被两个 agent 用也不会混淆。每行给出总量、Input / Output / Reasoning / Cache、命中率与请求数、按官方单价的估算花费、响应速度；点开是单模型详情：今天 / 7 天 / 30 天 / 全量、30 天趋势、Top 10 会话分布，且按来源取数，不会串口径。

### 🪙 服务额度与提醒

- **Codex**：官方 rate_limits（5 小时窗口 / 周额度 / credits），完全离线读取；
- **Antigravity**：官方本地 RPC 实时额度（客户端运行时自动发现端点）；
- **火山引擎**：费用中心 OpenAPI Token 包（多包聚合、千/万/百万 Token 单位自动换算、到期提醒），AK/SK 存 Windows 凭据管理器；
- **额度趋势**：快照本地持久化，变化趋势 / 每日消耗 / **预计耗尽时间**（线性回归，明确标注「预测」）；
- **额度提醒**：剩余 50/20/10%、Token 包 7 天到期、即将重置、数据停更、成本阈值，阈值全部可调可关。

**官方额度与本地统计严格分离**，永远不合并成一个含糊的数字。

### 💰 花费估算

内置智谱 / DeepSeek / Anthropic / OpenAI / xAI / Moonshot / Google / MiniMax / 字节等 30+ 模型官方单价（编译进二进制，来源与日期可查），支持模型级覆盖、远程价格表、促销价到期自动回落；DeepSeek 按北京时间峰谷分时计价；USD→CNY 每日自动刷新。所有金额都明确标注「按官方 API 单价估算 · 非实际 Billing」。

### 🖥 桌面体验

- **QQ 式贴边吸附**：窗口拖到屏幕边缘自动收起，只留 4px 触发条，光标一碰即滑出；多显示器 / 任务栏四边 / 100–200% DPI 全适配；
- **系统托盘**：关窗最小化到托盘，左键弹出 Glass 快览 Popup，右键十项菜单直达常用功能；
- **智能挂起**：窗口全部隐藏时自动暂停监控轮询，重新打开瞬间恢复——**空闲近零开销**；
- **Liquid Glass 视觉**：浅色毛玻璃分层、细腻动效、无边框原生缩放；
- **ZCode 一键启动**：多路径自动检测，未运行一键拉起，已运行聚焦原窗口；
- 单实例、窗口位置记忆、异常检测通知（用量激增 / 命中率骤降 / 模型连调等）。

### 🧾 诚实的统计口径

不编造，是这个项目的底线：

- 总 Token 按各数据源 schema 逐条判定：input 已含缓存的**不重复加**，reasoning 嵌在 output 里的**不重复计**；
- 数据源没提供的字段显示 `unavailable` 或 `—`，绝不推算；
- 可选字段带覆盖标注（"覆盖 n/m 条记录"），速度指标如实标注样本数；
- 签名、解析、聚合全部本地实现，凭据不落盘、不进日志。

## 📦 安装

从 [Releases](https://github.com/Cee-Ovo/zcode-usage-panel/releases) 下载最新版：

- **`ZCode-Usage-Panel-Setup-<版本>.exe`** — NSIS 安装包，双击安装（per-user 无需管理员），带开始菜单与可选桌面快捷方式，可正常卸载；
- **`ZCode-Usage-Panel-Portable-<版本>.zip`** — 便携版，解压即用。

要求：Windows 10+（WebView2，Win11 自带）。

## 🚀 快速上手

1. 安装后启动，面板会**自动发现**本机的 ZCode / Codex / Claude Code / DSH 数据目录（也支持 `ZCODE_HOME`、`CODEX_HOME`、`DSH_HOME`、`CLAUDE_CONFIG_DIR` 环境变量或在设置里手动指定）；
2. 首次使用建议核对一次「今天」总 Token 与各客户端自带 Usage 页数字；
3. 可选：设置 → 火山引擎 填入 IAM AccessKey/SecretKey 查询 Token 包；其余额度卡（Codex / Antigravity）零配置，装好即显示。

## 🛠 本机构建

```powershell
git clone https://github.com/Cee-Ovo/zcode-usage-panel.git
cd zcode-usage-panel
npm install
npm run tauri dev    # 开发调试
npm run tauri build  # 产出 src-tauri/target/release/bundle/nsis/*.exe
```

要求：Node ≥ 18、Rust stable（`x86_64-pc-windows-msvc`）、WebView2。推送 `v*` 标签会触发 GitHub Actions 自动构建安装包并挂到 Releases。

## 🧪 开发与测试

```powershell
npx vitest run                                   # 前端:格式化/口径/store/多源 mock
cargo test --manifest-path src-tauri/Cargo.toml  # 数据层:JSONL 增量/半行/截断容错、
                                                  # SQLite 发现/水位/busy、四源聚合口径、
                                                  # 签名已知答案向量、额度告警冷却
```

UI 可在无 Tauri 环境下用 `npm run dev`（DEV mock 数据，四源齐备）开发调试；Windows 专属行为（吸附 / 托盘 Popup）在非 Windows 平台编译为空实现，数据层可跨平台开发。

## 📐 已知限制

- Antigravity 额度仅当官方客户端在本机运行时可查（无公开远程 API，失败时诚实显示 unavailable）；
- Codex 官方 rate_limits 在其发起请求时刷新，长时间不用会标注「数据过期」；
- 等价花费为估算值，与任何实际 Billing 无关（内置价格表可远程更新 / 手动覆盖）；
- SQLite 表无 rowid 且无整数主键时退化全扫 + 行哈希去重（上限 50 万行）。

## License

MIT（见 [LICENSE](LICENSE)，第三方许可见 `THIRD-PARTY-NOTICES.md`）。
