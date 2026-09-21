import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Glass } from "open-glass-ui";
import { AnimatedNumber } from "./AnimatedNumber";
import { CostDetailModal } from "./CostDetailModal";
import { MetricCard, InfoDot, SpeedTrendLine } from "./MetricCard";
import { TrendChart } from "./TrendChart";
import { FxButton } from "./fx";
import { api, onEvent } from "../lib/ipc";
import { listItemVariants, rowGestures, softSpring } from "../lib/motion";
import { useStore } from "../lib/store";
import type { ModelCost, ModelRow, SpeedStats, SpeedWindowStats, UsageViewDto } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { RANGE_LABELS } from "../lib/types";
import { displayModelName, type ModelSource } from "../lib/modelDisplay";
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

/**
 * 「本地源分区」——Codex / DSH / Claude Code 共用的 ZCode 密度分区。
 *
 * 与 ZCodeSection 同一呈现结构(指标卡网格 / 模型排行 / 当前 Session /
 * 趋势图),数据来自各源本地 session 日志经统一 UsageRecord 口径聚合的
 * `get_local_usage_view`:
 * - 指标能算就显示,源里没有的字段(如 TTFT / 响应速度)诚实 unavailable;
 * - 费用为官方 API 单价估算(复用项目价格表),非实际 Billing;
 * - 本地 Token 与 ZCode 总 Token 互不计入。
 *
 * ZCode 分区本身不由此组件渲染(其内容/数值/布局保持冻结);本组件是
 * 三个本地分区的共享实现,分区之间除数据源外完全一致。
 */

const SPEED_UNAVAILABLE_HINT =
  "该数据源的本地日志不包含首 token 延迟 / 请求时长字段,响应速度无法计算,如实显示 unavailable(绝不编造)。\n" +
  "可与 ZCode 分区的速度卡对照:ZCode 的 model_usage 记录了原始时间字段。";

const ACTIVE_SESSION_FALLBACK =
  "暂无 Session 记录(在该客户端里发起一次对话后自动出现)";

export interface LocalSourceSectionProps {
  /** Provider snapshot id. */
  provider: "codex" | "dsh" | "claude-code";
  /** 分区主指标卡标题,如「Codex 总 Token」。 */
  totalLabel: string;
  /** ⓘ 完整口径说明。 */
  explain: string;
  /** 模型名来源徽标。 */
  modelSource: ModelSource;
  /** 无数据 / 未启用时的提示。 */
  emptyHint: string;
  /** 精简视图(与 ZCode 分区同一开关)。 */
  compact: boolean;
}

export function LocalSourceSection({
  provider,
  totalLabel,
  explain,
  modelSource,
  emptyHint,
  compact,
}: LocalSourceSectionProps) {
  const rangeKey = useStore((s) => s.rangeKey);
  const snap = useStore((s) => s.providers.find((p) => p.provider === provider) ?? null);
  const [view, setView] = useState<UsageViewDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [visibleModels, setVisibleModels] = useState<string[] | null>(null);
  const [costModalModel, setCostModalModel] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);

  // Refetch on range change, on refresh nudges (provider-update fires after
  // each poll; the page-refresh event covers manual refreshes), and once on
  // mount. Stale responses are dropped via the tick check.
  useEffect(() => {
    let disposed = false;
    setLoading(true);
    api
      .localUsageView(provider, rangeKey, true)
      .then((next) => {
        if (!disposed) {
          setView(next);
          setFailed(false);
          setLoading(false);
        }
      })
      .catch(() => {
        if (!disposed) {
          setFailed(true);
          setLoading(false);
        }
      });
    return () => {
      disposed = true;
    };
  }, [provider, rangeKey, refreshTick]);

  useEffect(() => {
    const bump = () => setRefreshTick((t) => t + 1);
    const unsubs: Array<() => void> = [];
    let alive = true;
    // provider-update fires after each hub poll; a refresh for THIS source
    // means the view may have changed. (Manual "立即刷新" also lands here via
    // refresh_now → hub kick.)
    onEvent<import("../lib/types").ProviderSnapshot[]>("provider-update", (snaps) => {
      if (snaps?.some((p) => p.provider === provider)) bump();
    })
      .then((u) => {
        if (alive) unsubs.push(u);
        else u();
      })
      .catch(() => {});
    return () => {
      alive = false;
      unsubs.forEach((u) => u());
    };
  }, [provider]);

  // TrendChart model toggles dispatch a window event; only one section is
  // mounted at a time so owning it locally is safe.
  useEffect(() => {
    const onToggle = (e: Event) => {
      setVisibleModels((e as CustomEvent).detail as string[] | null);
    };
    window.addEventListener("zup-toggle-model", onToggle);
    return () => window.removeEventListener("zup-toggle-model", onToggle);
  }, []);

  if (!snap || snap.status === "disabled") {
    return (
      <Glass className="panel sample-glass local-section-empty" material="regular" renderer="css" interactive={false}>
        <div className="empty-state">{emptyHint}</div>
      </Glass>
    );
  }

  const dash = view?.dash ?? null;
  const trend = view?.trend ?? null;
  const costSummary = view?.costSummary ?? null;
  const agg = dash?.agg;
  const hit = agg ? cacheHitRate(agg) : null;
  const models = dash ? (expanded ? dash.models : dash.models.slice(0, 3)) : [];
  const costByModel = new Map<string, ModelCost>(
    (costSummary?.models ?? []).map((m) => [m.name, m]),
  );
  const unknownCount = costSummary?.unknownModels.length ?? 0;

  return (
    <>
      <div
        key={`${provider}-${rangeKey}`}
        className={`zup-grid metrics-grid dashboard-metrics local-source-metrics${compact ? " is-compact" : ""}`}
      >
        <MetricCard
          glass
          layoutEnabled={false}
          className="metric-card--primary"
          label={totalLabel}
          value={
            agg ? <AnimatedNumber value={totalTokens(agg)} format={formatTokens} /> : "—"
          }
          sub={agg ? `${formatFull(totalTokens(agg))} tokens` : undefined}
          hint={explain}
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
          value={agg ? <AnimatedNumber value={agg.input} format={formatTokens} /> : "—"}
        />
        <MetricCard
          glass
          layoutEnabled={false}
          label="Output Token"
          value={agg ? <AnimatedNumber value={agg.output} format={formatTokens} /> : "—"}
        />
        <MetricCard
          glass
          layoutEnabled={false}
          label="Reasoning Token"
          value={
            agg && agg.reasoning.present > 0 ? (
              <AnimatedNumber value={agg.reasoning.sum} format={formatTokens} />
            ) : (
              "unavailable"
            )
          }
          unavailable={!agg || agg.reasoning.present === 0}
          sub={
            agg && agg.reasoning.present > 0 && agg.reasoning.present < agg.requests
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
            agg && agg.cacheRead.present > 0 ? (
              <AnimatedNumber
                value={agg.cacheRead.sum + agg.cacheWrite.sum}
                format={formatTokens}
              />
            ) : (
              "unavailable"
            )
          }
          unavailable={!agg || agg.cacheRead.present === 0}
          sub={
            agg && agg.cacheRead.present > 0
              ? `读 ${formatTokens(agg.cacheRead.sum)} · 写 ${formatTokens(agg.cacheWrite.sum)}`
              : undefined
          }
        />
        </>}
        <MetricCard
          glass
          layoutEnabled={false}
          label="请求次数"
          value={agg ? <AnimatedNumber value={agg.requests} format={formatFull} /> : "—"}
        />
        <LocalSpeedCard speed={dash?.speed} windows={dash?.speedWindows} />
        <MetricCard
          glass
          layoutEnabled={false}
          label="Cache Hit Rate"
          value={hit === null ? "unavailable" : formatPercent(hit)}
          unavailable={hit === null}
          hint="Cache Hit Rate = cached input ÷ total input(按各源记录口径:exclusive 源为 cache_read ÷ (input+cache_read+cache_write);inclusive 源为 cached ÷ input)。无 cache 字段的数据不计入,显示 unavailable。"
        />
        <MetricCard
          glass
          layoutEnabled={false}
          className="metric-card--model"
          label="活跃模型"
          value={
            <span style={{ fontSize: 14.5 }}>
              {dash?.activeSession?.activeModel ?? dash?.models[0]?.name ?? "—"}
            </span>
          }
          sub={
            dash && dash.models.length > 1 ? `共 ${dash.models.length} 个模型` : undefined
          }
        />
      </div>

      {/* top models */}
      <Glass className="panel sample-glass" material="regular" renderer="css">
        <div className="panel-title">
          模型排行
          <span className="right muted">
            {dash && dash.models.length > 3 && (
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
        {loading && !dash && <div className="empty-state">正在加载本地用量…</div>}
        {failed && !dash && (
          <div className="empty-state" role="alert">
            本地用量加载失败,稍后自动重试。
          </div>
        )}
        {dash && models.length === 0 && (
          <div className="empty-state">该时间范围内没有模型调用</div>
        )}
        <AnimatePresence initial={false}>
          {models.map((m) => (
            <LocalModelLine
              key={m.name}
              row={m}
              cost={costByModel.get(m.name)}
              modelSource={modelSource}
              onCostClick={() => setCostModalModel(m.name)}
            />
          ))}
        </AnimatePresence>
      </Glass>

      {/* latest session strip */}
      <Glass className="panel sample-glass" material="regular" renderer="css">
        <div className="panel-title">最近 Session</div>
        {dash?.activeSession ? (
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
          <div className="muted">{ACTIVE_SESSION_FALLBACK}</div>
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
            key={`${provider}-${costModalModel}`}
            model={costModalModel}
            rangeKey={rangeKey}
            provider={provider}
            fx={costSummary?.fx}
            priceUpdatedAt={costSummary?.priceUpdatedAt}
            onClose={() => setCostModalModel(null)}
          />
        )}
      </AnimatePresence>
    </>
  );
}

/** 响应速度卡:TTFT 如实不可用(本地日志不记录首 token 时刻);Codex 的
 * tps 由事件时间戳近似(窗口含首字等待),speedApproximate 时明确标注。
 * 第二行为近 24h / 近 7 天固定窗口对比(样本不足自动隐藏)。 */
function LocalSpeedCard({
  speed,
  windows,
}: {
  speed: SpeedStats | undefined;
  windows: SpeedWindowStats[] | undefined;
}) {
  const hasSamples = !!speed && speed.ttftSamples > 0;
  const hasSpeed = !!speed && (speed.speedSamples > 0 || speed.speedTps !== null);
  const available = hasSamples || hasSpeed;
  const approx = available && !hasSamples && !!speed?.speedApproximate;
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
            {approx && <span className="muted" style={{ fontWeight: 400, fontSize: "0.62em" }}>（近似）</span>}
          </span>
        ) : (
          "unavailable"
        )
      }
      unavailable={!available}
      sub={
        approx ? (
          <>
            <div>{`tps 按事件时间戳近似(含首字等待) · 样本 ${speed!.speedSamples}/${speed!.completedRequests} 条请求`}</div>
            <SpeedTrendLine windows={windows} />
          </>
        ) : available ? (
          <>
            <div>{`首 token P95 ${formatLatency(speed!.ttftP95Ms)} · 样本 ${speed!.ttftSamples}/${speed!.completedRequests} 条请求`}</div>
            <SpeedTrendLine windows={windows} />
          </>
        ) : undefined
      }
      hint={
        approx
          ? "该数据源的日志不记录首 token 时刻,TTFT 如实显示不可用。\ntps 按请求起止事件的时间戳近似推导(窗口含首字等待,数值略偏低),已标注「近似」。"
          : SPEED_UNAVAILABLE_HINT
      }
    />
  );
}

/** 模型排行行:与 ZCode 的 ModelLine 同布局;费用明细按本源数据查询。 */
function LocalModelLine({
  row,
  cost,
  modelSource,
  onCostClick,
}: {
  row: ModelRow;
  cost: ModelCost | undefined;
  modelSource: ModelSource;
  onCostClick: () => void;
}) {
  const hit = cacheHitRate(row.agg);
  const displayName = displayModelName(row.name, modelSource);
  return (
    <motion.div
      variants={listItemVariants}
      initial="initial"
      animate="enter"
      exit="exit"
      {...rowGestures}
      transition={softSpring}
      className="model-row"
      title={`${displayName}\n点击右侧金额查看成本明细`}
    >
      <div>
        <div className="name" title={displayName}>{displayName}</div>
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
        aria-label={`${displayName} 成本明细`}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault(); onCostClick();
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
