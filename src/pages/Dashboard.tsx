import { useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { AnimatedNumber } from "../components/AnimatedNumber";
import { CodexUsagePanel } from "../components/CodexUsagePanel";
import { CostDetailModal } from "../components/CostDetailModal";
import { MetricCard, InfoDot } from "../components/MetricCard";
import { QuotaSection } from "../components/QuotaSection";
import { TrendChart } from "../components/TrendChart";
import { FxButton, useAction } from "../components/fx";
import { LiquidSegmentedControl } from "../components/LiquidSegmentedControl";
import { api } from "../lib/ipc";
import { listItemVariants, rowGestures, softSpring, staggerContainer } from "../lib/motion";
import { store, useStore } from "../lib/store";
import type { ModelCost, ModelRow } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { RANGE_KEYS, RANGE_LABELS } from "../lib/types";
import {
  formatCny,
  formatFull,
  formatPercent,
  formatRate,
  formatRelative,
  formatTokens,
} from "../lib/format";

const HIT_HINT =
  "Cache Hit Rate = cached input ÷ total input(逐条记录自动判定口径:inclusive schema 用 cached/input;exclusive schema 用 cache_read ÷ (input+cache_read+cache_write))。无 cache 字段的数据不计入,显示 unavailable。";
const TOTAL_HINT =
  "ZCode 总 Token = Input + Output + Reasoning + Cache(读+写),仅统计 ZCode 本地 usage 记录;\n与下方 Codex 本地 Token、服务额度区的官方套餐额度分开统计,互不计入。";

export function DashboardPage({ onRangeChange }: { onRangeChange: (key: string) => void }) {
  const dash = useStore((s) => s.dash);
  const rangeKey = useStore((s) => s.rangeKey);
  const trend = useStore((s) => s.trend);
  const visibleModels = useStore((s) => s.trendVisibleModels);
  const costSummary = useStore((s) => s.costSummary);
  const alerts = useStore((s) => s.alerts);
  const [expanded, setExpanded] = useState(false);
  const [compact, setCompact] = useState(() => {
    try { return localStorage.getItem("zup.compact") === "true"; } catch { return false; }
  });
  const [costModalModel, setCostModalModel] = useState<string | null>(null);

  const refreshAction = useAction(
    async () => {
      await api.refreshNow();
    },
    { okText: "已刷新" },
  );

  if (!dash) {
    return <div className="empty-state">正在加载 ZCode 用量数据…</div>;
  }

  const agg = dash.agg;
  const hit = cacheHitRate(agg);
  const models = expanded ? dash.models : dash.models.slice(0, 3);
  const costByModel = new Map<string, ModelCost>(
    (costSummary?.models ?? []).map((m) => [m.name, m]),
  );
  const unknownCount = costSummary?.unknownModels.length ?? 0;

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
      {/* range selector */}
      <div className="dashboard-toolbar">
        <LiquidSegmentedControl
          aria-label="时间范围"
          value={rangeKey}
          onValueChange={(v) => onRangeChange(v)}
          items={RANGE_KEYS.map((k) => ({ value: k, label: RANGE_LABELS[k] }))}
        />
        {dash.restored && <span className="badge-note">缓存快照 · 同步中</span>}
        {dash.rangeKey !== rangeKey && (
          <span role="status" className="badge-note">
            待更新 · 仍显示{RANGE_LABELS[dash.rangeKey as keyof typeof RANGE_LABELS] ?? dash.rangeKey}数据
          </span>
        )}
        <FxButton variant="quiet" size="small" aria-pressed={compact} onClick={() => {
          const next = !compact;
          setCompact(next);
          try { localStorage.setItem("zup.compact", String(next)); } catch { /* optional preference */ }
        }}>
          {compact ? "显示详细指标" : "精简视图"}
        </FxButton>
        {dash.dataError && (
          <span className="badge-note" title={dash.dataError}>
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

      {/* core metrics (ZCode only — Codex local tokens get their own panel
          below; official plan quotas live in the quota section) */}
      <motion.div
        key={rangeKey}
        className={`zup-grid metrics-grid dashboard-metrics${compact ? " is-compact" : ""}`}
        variants={staggerContainer}
        initial="initial"
        animate="enter"
      >
        <MetricCard
          className="metric-card--primary"
          label="ZCode 总 Token"
          value={<AnimatedNumber value={totalTokens(agg)} format={formatTokens} />}
          sub={`${formatFull(totalTokens(agg))} tokens`}
          hint={TOTAL_HINT}
        />
        <MetricCard
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
          label="Input Token"
          value={<AnimatedNumber value={agg.input} format={formatTokens} />}
        />
        <MetricCard
          label="Output Token"
          value={<AnimatedNumber value={agg.output} format={formatTokens} />}
        />
        <MetricCard
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
          label="请求次数"
          value={<AnimatedNumber value={agg.requests} format={formatFull} />}
        />
        <MetricCard
          label="Cache Hit Rate"
          value={hit === null ? "unavailable" : formatPercent(hit)}
          unavailable={hit === null}
          hint={HIT_HINT}
        />
        <MetricCard
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
      </motion.div>

      {/* Codex 本地 Token(独立于 ZCode 指标与官方额度) */}
      <CodexUsagePanel />

      {/* AI service quotas (Codex / Antigravity / Volcengine + ZCode card) */}
      <QuotaSection />

      {/* top models */}
      <div className="panel">
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
      </div>

      {/* recent local alerts */}
      {alerts.length > 0 && (
        <div className="panel">
          <div className="panel-title">异常提醒(本地)</div>
          <AnimatePresence initial={false}>
            {alerts.slice(0, 3).map((a) => (
              <motion.div
                layout="position"
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
        </div>
      )}

      {/* live session strip */}
      <div className="panel">
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
      </div>

      {/* trend */}
      <div className="panel">
        <div className="panel-title">
          实时趋势 · {RANGE_LABELS[(trend?.rangeKey ?? rangeKey) as keyof typeof RANGE_LABELS] ?? rangeKey}
          <span className="right">
            <InfoDot text="点击模型名可单独显示/隐藏该模型的曲线。" />
          </span>
        </div>
        <TrendChart trend={trend} visibleModels={visibleModels} />
      </div>

      <AnimatePresence>
        {costModalModel && (
          <CostDetailModal
            key={costModalModel}
            model={costModalModel}
            rangeKey={rangeKey}
            fx={costSummary?.fx}
            priceUpdatedAt={costSummary?.priceUpdatedAt}
            onClose={() => setCostModalModel(null)}
          />
        )}
      </AnimatePresence>
    </motion.div>
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
      layout="position"
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
