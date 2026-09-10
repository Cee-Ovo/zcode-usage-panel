import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Glass } from "open-glass-ui";
import { AnimatedNumber } from "../components/AnimatedNumber";
import { MetricCard } from "../components/MetricCard";
import { TrendChart } from "../components/TrendChart";
import { api, onEvent } from "../lib/ipc";
import { store, useStore } from "../lib/store";
import type { ModelCost, ModelDetailDto, UsageViewDto } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { displayModelName, displayModelParts } from "../lib/modelDisplay";
import {
  enabledModelProviders,
  LOCAL_MODEL_PROVIDERS,
  mergeModelRows,
  type LocalModelProvider,
} from "../lib/modelRows";
import { formatCny, formatFull, formatModelSpeed, formatModelSpeedHint, formatPercent, formatRelative, formatTokens, shortSessionId } from "../lib/format";
import { listItemVariants, rowGestures, softSpring } from "../lib/motion";
import { FxCloseChip } from "../components/fx";
import { AccessibleDialog } from "../components/AccessibleDialog";

export function ModelsPage() {
  const dash = useStore((s) => s.dash);
  const detail = useStore((s) => s.modelDetail);
  const costSummary = useStore((s) => s.costSummary);
  const rangeKey = useStore((s) => s.rangeKey);
  const providers = useStore((s) => s.settings?.providers);
  const [localViews, setLocalViews] = useState<
    Partial<Record<LocalModelProvider, UsageViewDto | null>>
  >({});
  const [refreshTick, setRefreshTick] = useState(0);

  const enabled = useMemo(() => enabledModelProviders(providers), [providers]);
  const enabledKey = enabled.join(",");

  // Each enabled local source contributes its own model rows; the ZCode rows
  // arrive through the shared dashboard slice instead.
  useEffect(() => {
    let disposed = false;
    const wanted = enabledKey ? (enabledKey.split(",") as LocalModelProvider[]) : [];
    Promise.all(
      wanted.map((p) =>
        api
          .localUsageView(p, rangeKey, false)
          .then((view) => [p, view] as const)
          .catch(() => [p, null] as const),
      ),
    ).then((entries) => {
      if (disposed) return;
      setLocalViews(Object.fromEntries(entries) as Partial<Record<LocalModelProvider, UsageViewDto | null>>);
    });
    return () => {
      disposed = true;
    };
  }, [enabledKey, rangeKey, refreshTick]);

  useEffect(() => {
    const bump = () => setRefreshTick((t) => t + 1);
    let alive = true;
    const unsubs: Array<() => void> = [];
    onEvent<import("../lib/types").ProviderSnapshot[]>("provider-update", (snaps) => {
      if (snaps?.some((p) => LOCAL_MODEL_PROVIDERS.includes(p.provider as LocalModelProvider))) bump();
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
  }, []);

  const rows = useMemo(() => mergeModelRows(dash, localViews), [dash, localViews]);
  const costBySource = useMemo(() => {
    const index = new Map<string, Map<string, ModelCost>>();
    index.set("zcode", new Map((costSummary?.models ?? []).map((m) => [m.name, m])));
    for (const p of LOCAL_MODEL_PROVIDERS) {
      const view = localViews[p];
      if (view) {
        index.set(p, new Map((view.costSummary?.models ?? []).map((m) => [m.name, m])));
      }
    }
    return index;
  }, [costSummary, localViews]);

  if (!dash) return <div className="empty-state">加载中…</div>;

  return (
    <div className="models-page" style={{ paddingTop: 6 }}>
      <header className="page-heading page-heading--compact">
        <div>
          <span className="page-eyebrow">MODEL BREAKDOWN</span>
          <h1>模型</h1>
          <p>按模型查看 Token 构成、占比与估算花费,括号标注数据来源。</p>
        </div>
      </header>
      <Glass className="panel sample-glass page-surface models-surface" material="regular" renderer="css" interactive={false}>
        <div className="panel-title">全部模型(四源合并 · 当前时间范围)</div>
        {rows.length === 0 && <div className="empty-state">该范围内没有模型调用</div>}
        <AnimatePresence initial={false}>
          {rows.map(({ source, row }, i) => {
            const parts = displayModelParts(row.name, source);
            const tagged = displayModelName(row.name, source);
            const cost = costBySource.get(source)?.get(row.name);
            const hit = cacheHitRate(row.agg);
            return (
            <motion.div
              key={`${source}:${row.name}`}
              className="model-row"
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.currentTarget.click(); }
              }}
              layout="position"
              variants={listItemVariants}
              initial="initial"
              animate="enter"
              exit="exit"
              {...rowGestures}
              transition={softSpring}
              onClick={() =>
                api
                  .modelDetail(row.name, source)
                  .then((d) => store.set({ modelDetail: d }))
                  .catch(() => {})
              }
              title="点击查看模型详情"
            >
              <div>
                <div className="name" title={tagged}>
                  <span className="muted model-rank">{i + 1}.</span>
                  <span className="model-name-text">{parts.name}</span>
                  {parts.badge && <span className="model-source-badge">{parts.badge}</span>}
                </div>
              </div>
              <span className="num">{formatTokens(totalTokens(row.agg))}</span>
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
              <span className="num" style={{ color: "var(--zup-blue-600)" }}>
                {cost?.priced ? `≈ ${formatCny(cost.costCny)}` : "价格未知"}
              </span>
              <span className="num" title={formatModelSpeedHint(row.speed)}>
                {formatModelSpeed(row.speed)}
              </span>
            </motion.div>
            );
          })}
        </AnimatePresence>
      </Glass>

      <AnimatePresence>
        {detail && <ModelDetailCard key={`${detail.source ?? "zcode"}:${detail.name}`} detail={detail} />}
      </AnimatePresence>
    </div>
  );
}

function ModelDetailCard({ detail }: { detail: ModelDetailDto }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((x) => x + 1), 30_000);
    return () => clearInterval(t);
  }, []);

  const ratioTotal =
    detail.allTime.input + detail.allTime.output + detail.allTime.reasoning.sum || 1;

  return (
    <AccessibleDialog
      label={`模型详情 · ${displayModelName(detail.name, detail.source ?? null)}`}
      onClose={() => store.set({ modelDetail: null })}
      glass
    >
        <div className="panel-title">
          模型详情 · {displayModelName(detail.name, detail.source ?? null)}
          <span className="right">
            <FxCloseChip onClick={() => store.set({ modelDetail: null })} />
          </span>
        </div>
        <div className="zup-grid metrics-grid" style={{ marginBottom: 12 }}>
          <MetricCard
            glass
            label="今天"
            value={<AnimatedNumber value={totalTokens(detail.today)} format={formatTokens} />}
          />
          <MetricCard
            glass
            label="7 天"
            value={<AnimatedNumber value={totalTokens(detail.last7d)} format={formatTokens} />}
          />
          <MetricCard
            glass
            label="30 天"
            value={<AnimatedNumber value={totalTokens(detail.last30d)} format={formatTokens} />}
          />
          <MetricCard
            glass
            label="请求次数"
            value={<AnimatedNumber value={detail.allTime.requests} format={formatFull} />}
            sub={`平均每请求 ${formatTokens(detail.avgTokensPerRequest)} tokens`}
          />
        </div>
        <div className="kv" style={{ marginBottom: 12 }}>
          <span className="k">Input / Output / Reasoning 比例</span>
          <span>
            {((detail.allTime.input / ratioTotal) * 100).toFixed(1)}% /{" "}
            {((detail.allTime.output / ratioTotal) * 100).toFixed(1)}% /{" "}
            {((detail.allTime.reasoning.sum / ratioTotal) * 100).toFixed(1)}%
            {detail.allTime.reasoning.present === 0 && (
              <span className="muted">(reasoning unavailable)</span>
            )}
          </span>
          <span className="k">Cache Hit Rate</span>
          <span>
            {detail.hitRate === null ? "unavailable" : formatPercent(detail.hitRate)}
          </span>
          <span className="k">最近使用</span>
          <span>{formatRelative(detail.lastUsedMs)}</span>
        </div>
        <div className="panel-title">Token 时间趋势(近 30 天)</div>
        <TrendChart
          trend={{
            rangeKey: "30d",
            fromMs: Date.now() - 30 * 86400_000,
            toMs: Date.now(),
            buckets: detail.trend30d,
            restored: false,
          }}
          visibleModels={null}
          height={150}
        />
        <div className="panel-title" style={{ marginTop: 10 }}>
          Session 分布(近 30 天 Top 10)
        </div>
        {detail.topSessions.length === 0 && <div className="muted">无 session 关联数据</div>}
        {detail.topSessions.map(([sid, tokens]) => (
          <div
            key={sid}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              padding: "4px 0",
              fontSize: 12,
            }}
          >
            <span style={{ width: 130 }} className="muted" title={sid}>
              {shortSessionId(sid)}
            </span>
            <div className="share-track" style={{ flex: 1, marginTop: 0 }}>
              <div
                className="share-fill"
                style={{
                  width: `${Math.max(
                    2,
                    (tokens / (detail.topSessions[0]?.[1] || 1)) * 100,
                  )}%`,
                }}
              />
            </div>
            <span style={{ width: 70, textAlign: "right" }}>{formatTokens(tokens)}</span>
          </div>
        ))}
    </AccessibleDialog>
  );
}
