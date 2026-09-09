import { memo, useState } from "react";
import { Glass, SegmentedControl, Button } from "open-glass-ui";
import { AnimatePresence, motion } from "motion/react";
import { AnimatedNumber } from "../components/AnimatedNumber";
import { LiquidSegmentedControl } from "../components/LiquidSegmentedControl";
import { LocalSourceSection } from "../components/LocalSourceSection";
import { CostDetailModal } from "../components/CostDetailModal";
import { MetricCard, InfoDot } from "../components/MetricCard";
import { QuotaSection } from "../components/QuotaSection";
import { TrendChart } from "../components/TrendChart";
import { FxButton, useAction } from "../components/fx";
import { api } from "../lib/ipc";
import { listItemVariants, rowGestures, softSpring, staggerContainer } from "../lib/motion";
import { store, useStore } from "../lib/store";
import type { ModelCost, ModelRow, SpeedStats } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { RANGE_KEYS, RANGE_LABELS } from "../lib/types";
import {
  formatCny,
  formatFull,
  formatLatency,
  formatPercent,
  formatRate,
  formatRelative,
  formatTokens,
  formatTps,
} from "../lib/format";

const HIT_HINT =
  "Cache Hit Rate = cached input ÷ total input(逐条记录自动判定口径:inclusive schema 用 cached/input;exclusive schema 用 cache_read ÷ (input+cache_read+cache_write))。无 cache 字段的数据不计入,显示 unavailable。";
const TOTAL_HINT =
  "ZCode 总 Token = Input + Output + Reasoning + Cache(读+写),仅统计 ZCode 本地 usage 记录;\n与 Codex 本地 Token、DSH 本地 Token、服务额度区的官方套餐额度分开统计,互不计入。";

const SPEED_HINT =
  "首字延迟(TTFT)与 Token 速度均来自 ZCode 本地 model_usage 记录的原始字段,不自行推算。\n" +
  "样本口径:仅统计状态为 completed(或未记录状态)的请求;error / cancelled / running 剔除。\n" +
  "TTFT 取每条记录的 time_to_first_token_ms 原值,未记录该字段的请求不计入(副文本标注覆盖样本数)。\n" +
  "tok/s = Σ(output+reasoning tokens) ÷ Σ生成时长,生成时长 = duration − TTFT;缺 TTFT 或无输出 token 的请求不计入,\n" +
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
  "它与「ZCode 总 Token」分开统计、互不计入;与服务额度区的 Codex 官方套餐额度(5 小时/周 rate_limits)也是两个独立指标。\n" +
  "本地 Token 统计 ≠ 官方剩余额度 ≠ 实际 Billing;速度类指标因日志无时间字段而不可用。";

const DSH_EXPLAIN =
  "DSH Token = DeepSeek Harness 本地 session 日志中 assistant 消息的 usage 统计:inputTokens 为未缓存输入,cacheRead / cacheWrite 单列;reasoning 已包含在 Output 中,总量不重复累计。\n" +
  "它与「ZCode 总 Token」「Codex 本地 Token」分开统计、互不计入。\n" +
  "本地 Token 统计 ≠ 实际 Billing;速度类指标因日志无时间字段而不可用。";

const CLAUDE_EXPLAIN =
  "Claude Code Token = 本地 ~/.claude/projects 转写中 assistant 消息的 usage 统计:Claude 官方口径 input_tokens 不含 cache(读/写单列),总量 = Input + Output + Cache 读 + Cache 写。\n" +
  "同一 message.id 的流式重复行按最后一条快照计数,不会重复累计。\n" +
  "它与「ZCode 总 Token」分开统计、互不计入;Claude Code 无本地可查的官方套餐额度,本分区不展示任何官方额度;速度类指标因日志无时间字段而不可用。";

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
          <p>本地用量与服务额度，清晰掌握每一次使用。</p>
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
            provider={section === "codex" ? "codex" : section === "dsh" ? "dsh" : "claude-code"}
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

      {/* AI service quotas (Codex / Antigravity / Volcengine + ZCode card) */}
      <QuotaSection />
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
  const alerts = useStore((s) => s.alerts);
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
            costSummary ? (
              <span>≈ {formatCny(costSummary.totalCostCny)}</span>
            ) : (
              "—"
            )
          }
          sub={
            <span>
              按官方 API 单价估算 · 非实际 Billing
              {unknownCount > 0 ? ` · ${unknownCount} 个模型价格未知` : ""}
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
        <SpeedCard speed={dash.speed} />
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

      {/* Codex 本地 Token(独立于 ZCode 指标与官方额度) */}
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

      {/* recent local alerts */}
      {alerts.length > 0 && (
        <Glass className="panel sample-glass" material="regular" renderer="css">
          <div className="panel-title">异常提醒(本地)</div>
          <AnimatePresence initial={false}>
            {alerts.slice(0, 3).map((a) => (
              <motion.div
                variants={listItemVariants}
                initial="initial"
                animate="enter"
                exit="exit"
                key={`${a.rule}-${a.tsMs}`}
                className={`alert-chip ${a.severity >= 2 ? "critical" : ""}`}
              >
                <span style={{ fontWeight: 650 }}>{a.title}</span>
                <span className="muted" style={{ flex: 1 }}>
                  {a.body}
                </span>
                <span className="muted" style={{ fontSize: 10.5 }}>
                  {formatRelative(a.tsMs)}
                </span>
              </motion.div>
            ))}
          </AnimatePresence>
        </Glass>
      )}

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

      {/* trend */}
      <Glass className="panel sample-glass" material="regular" renderer="css">
        <div className="panel-title">
          实时趋势 · {RANGE_LABELS[(trend?.rangeKey ?? rangeKey) as keyof typeof RANGE_LABELS] ?? rangeKey}
          <span className="right">
            <InfoDot text="点击模型名可单独显示/隐藏该模型的曲线。" />
          </span>
        </div>
        <TrendChart trend={trend} visibleModels={visibleModels} />
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

/** 响应速度卡:首字延迟均值 + 加权 tok/s;无样本时按惯例显示 unavailable。 */
function SpeedCard({ speed }: { speed: SpeedStats | undefined }) {
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
        available
          ? `首 token P95 ${formatLatency(speed!.ttftP95Ms)} · 样本 ${speed!.ttftSamples}/${speed!.completedRequests} 条请求`
          : undefined
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
        api.modelDetail(row.name).then((d) => store.set({ modelDetail: d })).catch(() => {});
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
