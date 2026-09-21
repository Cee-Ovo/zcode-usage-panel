import { memo, useState } from "react";
import { Glass, SegmentedControl, Button } from "open-glass-ui";
import { AnimatePresence, motion } from "motion/react";
import { AnimatedNumber } from "../components/AnimatedNumber";
import { LiquidSegmentedControl } from "../components/LiquidSegmentedControl";
import { LocalSourceSection } from "../components/LocalSourceSection";
import { CostDetailModal } from "../components/CostDetailModal";
import { MetricCard, InfoDot, SpeedTrendLine } from "../components/MetricCard";
import { TrendChart } from "../components/TrendChart";
import { FxButton, useAction } from "../components/fx";
import { api } from "../lib/ipc";
import { listItemVariants, rowGestures, softSpring, staggerContainer } from "../lib/motion";
import { store, useStore } from "../lib/store";
import { modelDetailGate } from "../lib/modelDetail";
import type { ModelCost, ModelRow, SpeedStats, SpeedWindowStats } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { RANGE_KEYS, RANGE_LABELS } from "../lib/types";
import {
  formatCny,
  formatFull,
  formatLatency,
  formatModelSpeed,
  formatPercent,
  formatRate,
  formatRelative,
  formatTokens,
  formatTps,
} from "../lib/format";

const HIT_HINT =
  "Cache Hit Rate = cached input ÷ total input(逐条记录自动判定口径:inclusive schema 用 cached/input;exclusive schema 用 cache_read ÷ (input+cache_read+cache_write))。无 cache 字段的数据不计入,显示 unavailable。";
const TOTAL_HINT =
  "ZCode 总 Token = Input + Output + Reasoning + Cache(读+写),仅统计 ZCode 本地 usage 记录;\n与 Codex 本地 Token、DSH 本地 Token、Claude Code 本地 Token 分开统计,互不计入。";

const SPEED_HINT =
  "首字延迟(TTFT)与 Token 速度均来自 ZCode 本地 model_usage 记录的原始字段,不自行推算。\n" +
  "样本口径:仅统计状态为 completed(或未记录状态)的请求;error / cancelled / running 剔除。\n" +
  "TTFT 取每条记录的 time_to_first_token_ms 原值,未记录该字段的请求不计入(副文本标注覆盖样本数)。\n" +
  "tps = Σ(output+reasoning tokens) ÷ Σ生成时长,生成时长 = duration − TTFT;缺 TTFT 或无输出 token 的请求不计入,\n" +
  "因此不与「全程平均」混算。P95 为最近邻位次法。数据源不记录时间字段的记录存在时,整卡显示 unavailable,绝不编造。";

// ---- 四分区(数据源分区)定义 --------------------------------------------------

type SectionKey = "zcode" | "codex" | "dsh" | "claude";

const SECTION_KEYS: SectionKey[] = ["zcode", "codex", "dsh", "claude"];

const SECTION_LABELS: Record<SectionKey, string> = {
  zcode: "ZCode",
  codex: "Codex",
  dsh: "DSH",
  claude: "CC",
};

const SECTION_SUBTITLES: Record<SectionKey, string> = {
  zcode: "本地 usage 记录 · API 等价花费为官方单价估算",
  codex: "Codex 客户端 session 日志统计 · 不计入 ZCode 总 Token",
  dsh: "DeepSeek Harness session 日志统计 · 不计入 ZCode 总 Token",
  claude: "Claude Code 本地转写统计 · 不计入 ZCode 总 Token",
};

const CODEX_EXPLAIN =
  "Codex Token = 本地 Codex 客户端 session 日志(含 archived_sessions)中提供的 total_tokens 原值;Cached / Cache Write 作为分项展示,不会重复加到总量中。\n" +
  "它与「ZCode 总 Token」分开统计、互不计入;两者是独立的统计口径。\n" +
  "本地 Token 统计 ≠ 实际 Billing;速度类指标因日志无时间字段而不可用。";

const DSH_EXPLAIN =
  "DSH Token = DeepSeek Harness 本地 session 日志中 assistant 消息的 usage 统计:inputTokens 为未缓存输入,cacheRead / cacheWrite 单列;reasoning 已包含在 Output 中,总量不重复累计。\n" +
  "它与「ZCode 总 Token」「Codex 本地 Token」分开统计、互不计入。\n" +
  "本地 Token 统计 ≠ 实际 Billing;速度类指标因日志无时间字段而不可用。";

const CLAUDE_EXPLAIN =
  "Claude Code Token = 本地 ~/.claude/projects 转写中 assistant 消息的 usage 统计:Claude 官方口径 input_tokens 不含 cache(读/写单列),总量 = Input + Output + Cache 读 + Cache 写。\n" +
  "同一 message.id 的流式重复行按最后一条快照计数,不会重复累计。\n" +
  "它与「ZCode 总 Token」分开统计、互不计入;速度类指标因日志无时间字段而不可用。";

function readStoredSection(): SectionKey {
  try {
    const value = localStorage.getItem("zup.section");
    if (value && (SECTION_KEYS as string[]).includes(value)) return value as SectionKey;
  } catch { /* optional preference */ }
  return "zcode";
}

export const DashboardPage = memo(function DashboardPage({ onRangeChange }: { onRangeChange: (key: string) => void }) {
  const hasDash = useStore((s) => s.dash !== null);
  const [section, setSection] = useState<SectionKey>(readStoredSection);
  const [compact, setCompact] = useState(() => {
    try { return localStorage.getItem("zup.compact") === "true"; } catch { return false; }
  });
  /** 当前本地源标签对应的 provider id(同时用作组件 key)。 */
  const localProvider: "codex" | "dsh" | "claude-code" =
    section === "codex" ? "codex" : section === "dsh" ? "dsh" : "claude-code";

  if (!hasDash) {
    return <div className="empty-state">正在加载 ZCode 用量数据…</div>;
  }

  return (
    <motion.div
      className="zup-grid dashboard-page"
      variants={staggerContainer}
      initial="initial"
      animate="enter"
    >
      <header className="page-heading">
        <div>
          <span className="page-eyebrow">USAGE OVERVIEW</span>
          <h1>用量概览</h1>
          <p>本地用量，清晰掌握每一次使用。</p>
        </div>
        <span className="source-label"><span aria-hidden="true">▤</span> 本地数据面板</span>
      </header>
      {import.meta.env.DEV && !("__TAURI_INTERNALS__" in window) && <div className="sample-review" aria-label="磨玻璃样板预览">
        <span>材质样板 · 演示数据</span>
        <a href="https://github.com/moekoelueker/open-glass-ui" target="_blank" rel="noreferrer">OpenGlass UI 0.3.0 ↗</a>
        <span className="sample-theme-controls">
          {(["light", "dark"] as const).map((theme) => <Button key={theme} onClick={() => {
            const settings = store.get().settings;
            if (settings) store.set({ settings: { ...settings, theme } });
          }}>{theme === "light" ? "浅色样板" : "深色样板"}</Button>)}
        </span>
      </div>}
      {/* range selector */}
      <DashboardToolbar compact={compact} onCompactChange={setCompact} onRangeChange={onRangeChange} />

      {/* section switcher: ZCode / Codex / DSH 数据源分区 */}
      <div className="dashboard-sections" role="tablist" aria-label="数据源分区">
        <LiquidSegmentedControl
          aria-label="数据源分区"
          className="dashboard-section-tabs"
          value={section}
          onValueChange={(v) => {
            setSection(v);
            try { localStorage.setItem("zup.section", v); } catch { /* optional preference */ }
          }}
          items={SECTION_KEYS.map((k) => ({ value: k, label: SECTION_LABELS[k] }))}
        />
        <span className="muted dashboard-section-subtitle">
          {SECTION_SUBTITLES[section]}
        </span>
      </div>

      <div className="dashboard-section">
        {section === "zcode" ? (
          <ZCodeSection compact={compact} />
        ) : (
          <LocalSourceSection
            // 三个本地源共用同一组件:换源时必须重建实例,否则上一个源的
            // 展开态 / 模型显隐 / 成本弹窗会串到新源上。
            key={localProvider}
            provider={localProvider}
            totalLabel={
              section === "codex"
                ? "Codex 总 Token"
                : section === "dsh"
                  ? "DSH 总 Token"
                  : "Claude Code 总 Token"
            }
            explain={
              section === "codex" ? CODEX_EXPLAIN : section === "dsh" ? DSH_EXPLAIN : CLAUDE_EXPLAIN
            }
            modelSource={section === "codex" ? "codex" : section === "dsh" ? "dsh" : "claude-code"}
            compact={compact}
            emptyHint={
              section === "codex"
                ? "在「设置 → Codex」中开启后,这里会显示本地 session 日志统计。"
                : section === "dsh"
                  ? "在「设置 → DSH」中开启后,这里会显示 DeepSeek Harness 本地 session 日志统计。"
                  : "在「设置 → Claude Code」中开启后,这里会显示本地转写统计。"
            }
          />
        )}
      </div>
    </motion.div>
  );
});

const DashboardToolbar = memo(function DashboardToolbar({
  compact,
  onCompactChange,
  onRangeChange,
}: {
  compact: boolean;
  onCompactChange: (compact: boolean) => void;
  onRangeChange: (key: string) => void;
}) {
  const rangeKey = useStore((s) => s.rangeKey);
  const dashboardRestored = useStore((s) => s.dash?.restored ?? false);
  const dashboardDataError = useStore((s) => s.dash?.dataError ?? null);
  const healthDetail = useStore((s) => s.health.detail);
  const refreshError = useStore((s) => s.refresh.error);
  const initializationError = useStore((s) => s.initializationError);
  const dashboardRangeKey = useStore((s) => s.dash?.rangeKey ?? null);
  const healthLevel = useStore((s) => s.health.level);
  const refreshAction = useAction(
    async () => {
      await api.refreshNow();
    },
    { okText: "已刷新" },
  );

  return (
    <div className="dashboard-toolbar">
      <SegmentedControl
        aria-label="时间范围"
        value={rangeKey}
        onValueChange={(v) => onRangeChange(v)}
        items={RANGE_KEYS.map((k) => ({ value: k, label: RANGE_LABELS[k] }))}
      />
      {dashboardRestored && <span className="badge-note">缓存快照 · 同步中</span>}
      {dashboardRangeKey !== null && dashboardRangeKey !== rangeKey && (
        <span role="status" className="badge-note">
          待更新 · 仍显示{RANGE_LABELS[dashboardRangeKey as keyof typeof RANGE_LABELS] ?? dashboardRangeKey}数据
        </span>
      )}
      <FxButton variant="quiet" size="small" aria-pressed={compact} onClick={() => {
        const next = !compact;
        onCompactChange(next);
        try { localStorage.setItem("zup.compact", String(next)); } catch { /* optional preference */ }
      }}>
        {compact ? "显示详细指标" : "精简视图"}
      </FxButton>
      {healthLevel === "error" && (
        <span className="badge-note" title={dashboardDataError ?? healthDetail ?? refreshError ?? initializationError ?? undefined}>
          数据源异常
        </span>
      )}
      <span style={{ marginLeft: "auto" }}>
        <FxButton
          variant="quiet"
          size="small"
          action={refreshAction}
          busyLabel="刷新中…"
          title="立即刷新 ZCode 数据与所有 Provider"
        >
          立即刷新
        </FxButton>
      </span>
    </div>
  );
});

const ZCodeSection = memo(function ZCodeSection({ compact }: { compact: boolean }) {
  const dash = useStore((s) => s.dash);
  const rangeKey = useStore((s) => s.rangeKey);
  const trend = useStore((s) => s.trend);
  const visibleModels = useStore((s) => s.trendVisibleModels);
  const costSummary = useStore((s) => s.costSummary);
  const [expanded, setExpanded] = useState(false);
  const [costModalModel, setCostModalModel] = useState<string | null>(null);

  if (!dash) return null;

  const agg = dash.agg;
  const hit = cacheHitRate(agg);
  const models = expanded ? dash.models : dash.models.slice(0, 3);
  const costByModel = new Map<string, ModelCost>(
    (costSummary?.models ?? []).map((m) => [m.name, m]),
  );
  const unknownCount = costSummary?.unknownModels.length ?? 0;

  return (
    <>
      {/* core metrics (ZCode only — Codex / DSH local tokens get their own
          sections; official plan quotas live in the quota section) */}
      <div
        key={rangeKey}
        className={`zup-grid metrics-grid dashboard-metrics${compact ? " is-compact" : ""}`}
      >
        <MetricCard
          glass
          layoutEnabled={false}
          className="metric-card--primary"
          label="ZCode 总 Token"
          value={<AnimatedNumber value={totalTokens(agg)} format={formatTokens} />}
          sub={`${formatFull(totalTokens(agg))} tokens`}
          hint={TOTAL_HINT}
        />
        <MetricCard
          glass
          layoutEnabled={false}
          className="metric-card--cost"
          label="API 等价花费"
          value={
            // 缓存快照阶段:Token 卡显示的是持久化快照,花费却按尚未加载的
            // 记录集算(0),两者不一致 —— 此时如实显示「—」而不是 ¥0.00。
            costSummary && !dash?.restored ? (
              <span>≈ {formatCny(costSummary.totalCostCny)}</span>
            ) : (
              "—"
            )
          }
          sub={
            <span>
              {dash?.restored
                ? "缓存快照同步中 · 完成后按官方单价估算"
                : "按官方 API 单价估算 · 非实际 Billing"}
              {!dash?.restored && unknownCount > 0 ? ` · ${unknownCount} 个模型价格未知` : ""}
            </span>
          }
        />
        {!compact && <>
        <MetricCard
          glass
          layoutEnabled={false}
          label="Input Token"
          value={<AnimatedNumber value={agg.input} format={formatTokens} />}
        />
        <MetricCard
          glass
          layoutEnabled={false}
          label="Output Token"
          value={<AnimatedNumber value={agg.output} format={formatTokens} />}
        />
        <MetricCard
          glass
          layoutEnabled={false}
          label="Reasoning Token"
          value={
            agg.reasoning.present > 0 ? (
              <AnimatedNumber value={agg.reasoning.sum} format={formatTokens} />
            ) : (
              "unavailable"
            )
          }
          unavailable={agg.reasoning.present === 0}
          sub={
            agg.reasoning.present > 0 && agg.reasoning.present < agg.requests
              ? `覆盖 ${agg.reasoning.present}/${agg.requests} 条记录`
              : undefined
          }
          hint="数据源未提供 reasoning 字段的记录不会计入,也不会被推算。"
        />
        <MetricCard
          glass
          layoutEnabled={false}
          label="Cache Token"
          value={
            agg.cacheRead.present > 0 ? (
              <AnimatedNumber
                value={agg.cacheRead.sum + agg.cacheWrite.sum}
                format={formatTokens}
              />
            ) : (
              "unavailable"
            )
          }
          unavailable={agg.cacheRead.present === 0}
          sub={
            agg.cacheRead.present > 0
              ? `读 ${formatTokens(agg.cacheRead.sum)} · 写 ${formatTokens(agg.cacheWrite.sum)}`
              : undefined
          }
        />
        </>}
        <MetricCard
          glass
          layoutEnabled={false}
          label="请求次数"
          value={<AnimatedNumber value={agg.requests} format={formatFull} />}
        />
        <SpeedCard speed={dash.speed} windows={dash.speedWindows} />
        <MetricCard
          glass
          layoutEnabled={false}
          label="Cache Hit Rate"
          value={hit === null ? "unavailable" : formatPercent(hit)}
          unavailable={hit === null}
          hint={HIT_HINT}
        />
        <MetricCard
          glass
          layoutEnabled={false}
          className="metric-card--model"
          label="活跃模型"
          value={
            <span style={{ fontSize: 14.5 }}>
              {dash.activeSession?.activeModel ?? dash.models[0]?.name ?? "—"}
            </span>
          }
          sub={
            dash.models.length > 1 ? `共 ${dash.models.length} 个模型` : undefined
          }
        />
      </div>

      {/* Codex 本地 Token(独立于 ZCode 指标) */}
      {/* trend — 整行置顶,紧接指标卡 */}
      <Glass className="panel sample-glass" material="regular" renderer="css">
        <div className="panel-title">
          Token 趋势 · {RANGE_LABELS[(trend?.rangeKey ?? rangeKey) as keyof typeof RANGE_LABELS] ?? rangeKey}
          <span className="right">
            <InfoDot text="点击模型名可单独显示/隐藏该模型的曲线。" />
          </span>
        </div>
        <TrendChart trend={trend} visibleModels={visibleModels} />
      </Glass>

      {/* top models */}
      <Glass className="panel sample-glass" material="regular" renderer="css">
        <div className="panel-title">
          模型排行
          <span className="right muted">
            {dash.models.length > 3 && (
              <FxButton variant="quiet" size="small" onClick={() => setExpanded(!expanded)}>
                {expanded ? "收起" : `展开全部 (${dash.models.length})`}
              </FxButton>
            )}
          </span>
        </div>
        <div className="model-row model-head">
          <span>模型</span>
          <span style={{ textAlign: "right" }}>总 Token</span>
          <span style={{ textAlign: "right" }}>占比</span>
          <span style={{ textAlign: "right" }}>Input</span>
          <span style={{ textAlign: "right" }}>Output</span>
          <span style={{ textAlign: "right" }}>Reasoning</span>
          <span style={{ textAlign: "right" }}>Cached In</span>
          <span style={{ textAlign: "right" }}>命中率 / 请求</span>
          <span style={{ textAlign: "right" }}>API 花费</span>
        </div>
        {models.length === 0 && (
          <div className="empty-state">该时间范围内没有模型调用</div>
        )}
        <AnimatePresence initial={false}>
          {models.map((m) => (
            <ModelLine
              key={m.name}
              row={m}
              cost={costByModel.get(m.name)}
              onCostClick={() => setCostModalModel(m.name)}
            />
          ))}
        </AnimatePresence>
      </Glass>

      {/* live session strip */}
      <Glass className="panel sample-glass" material="regular" renderer="css">
        <div className="panel-title">当前 Session</div>
        {dash.activeSession ? (
          <div className="kv">
            <span className="k">Session Token</span>
            <span>
              <AnimatedNumber
                value={dash.activeSession.sessionTotalTokens}
                format={formatTokens}
              />
            </span>
            <span className="k">增长速度</span>
            <span>{formatRate(dash.activeSession.tokensPerMin)}(近 5 分钟)</span>
            <span className="k">最近请求</span>
            <span>{formatRelative(dash.activeSession.lastRequestMs)}</span>
            <span className="k">最近模型速度</span>
            <span title="当前 Session 最近模型的 TTFT 均值与生成速度(与响应速度卡同口径)">
              {formatModelSpeed(
                dash.models.find((m) => m.name === dash.activeSession?.activeModel)?.speed,
              )}
            </span>
            <span className="k">项目</span>
            <span>{dash.activeSession.project ?? "—"}</span>
            <span className="k">模型切换</span>
            <span>
              {dash.activeSession.modelSwitches
                .slice(-6)
                .map(
                  (s) =>
                    `${new Date(s.tsMs).toLocaleTimeString([], {
                      hour: "2-digit",
                      minute: "2-digit",
                    })} ${s.model}`,
                )
                .join(" → ") || "—"}
            </span>
          </div>
        ) : (
          <div className="muted">暂无活跃 Session(ZCode 未运行时显示最后一次统计)</div>
        )}
      </Glass>

      <AnimatePresence>
        {costModalModel && (
          <CostDetailModal
            glass
            key={costModalModel}
            model={costModalModel}
            rangeKey={rangeKey}
            fx={costSummary?.fx}
            priceUpdatedAt={costSummary?.priceUpdatedAt}
            onClose={() => setCostModalModel(null)}
          />
        )}
      </AnimatePresence>
    </>
  );
});

/** 响应速度卡:首字延迟均值 + 加权 tps;无样本时按惯例显示 unavailable。
 * 第二行给出近 24h / 近 7 天固定窗口的 tps 对比(样本不足自动隐藏)。 */
function SpeedCard({
  speed,
  windows,
}: {
  speed: SpeedStats | undefined;
  windows: SpeedWindowStats[] | undefined;
}) {
  const hasSamples = !!speed && speed.ttftSamples > 0;
  const hasSpeed = !!speed && (speed.speedSamples > 0 || speed.speedTps !== null);
  const available = hasSamples || hasSpeed;
  return (
    <MetricCard
      glass
      layoutEnabled={false}
      label="响应速度"
      value={
        available ? (
          <span>
            {hasSamples ? formatLatency(speed!.ttftAvgMs) : "—"}
            <span className="muted" style={{ fontWeight: 400 }}> · </span>
            {formatTps(speed!.speedTps)}
          </span>
        ) : (
          "unavailable"
        )
      }
      unavailable={!available}
      sub={
        available ? (
          <>
            <div>
              {`首 token P95 ${formatLatency(speed!.ttftP95Ms)} · 样本 ${speed!.ttftSamples}/${speed!.completedRequests} 条请求`}
            </div>
            <SpeedTrendLine windows={windows} />
          </>
        ) : undefined
      }
      hint={SPEED_HINT}
    />
  );
}

function ModelLine({
  row,
  cost,
  onCostClick,
}: {
  row: ModelRow;
  cost: ModelCost | undefined;
  onCostClick: () => void;
}) {
  const hit = cacheHitRate(row.agg);
  return (
    <motion.div
      variants={listItemVariants}
      initial="initial"
      animate="enter"
      exit="exit"
      {...rowGestures}
      transition={softSpring}
      className="model-row"
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault(); e.currentTarget.click();
        }
      }}
      onClick={() => {
        store.set({ page: "models" });
        // 序号保护:过期响应不会覆盖新选的模型,也不会重开已关闭的弹窗。
        modelDetailGate.open(() => api.modelDetail(row.name));
      }}
      title="点击查看模型详情"
    >
      <div>
        <div className="name">{row.name}</div>
        <div className="share-track">
          <div className="share-fill" style={{ width: `${Math.round(row.share * 100)}%` }} />
        </div>
      </div>
      <span className="num">{formatTokens(totalTokens(row.agg))}</span>
      <span className="num">{(row.share * 100).toFixed(1)}%</span>
      <span className="num">{formatTokens(row.agg.input)}</span>
      <span className="num">{formatTokens(row.agg.output)}</span>
      <span className="num">
        {row.agg.reasoning.present > 0 ? formatTokens(row.agg.reasoning.sum) : "—"}
      </span>
      <span className="num">
        {row.agg.cacheRead.present > 0 ? formatTokens(row.agg.cacheRead.sum) : "—"}
      </span>
      <span className="num">
        {hit === null ? "—" : `${(hit * 100).toFixed(0)}% · ${formatFull(row.agg.requests)}`}
      </span>
      <span
        className="num"
        title="点击查看成本明细"
        role="button"
        tabIndex={0}
        aria-label={`${row.name} 成本明细`}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault(); e.stopPropagation(); onCostClick();
          }
        }}
        onClick={(e) => {
          e.stopPropagation();
          onCostClick();
        }}
        style={{ cursor: "pointer", color: "var(--zup-blue-600)" }}
      >
        {cost?.priced ? (
          <>≈ {formatCny(cost.costCny)}</>
        ) : (
          <span className="cost-unknown" title="没有官方价格,可在设置中手动覆盖">
            价格未知
          </span>
        )}
      </span>
    </motion.div>
  );
}
