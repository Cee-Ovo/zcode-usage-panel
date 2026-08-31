import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { AnimatedNumber } from "../components/AnimatedNumber";
import { MetricCard } from "../components/MetricCard";
import { TrendChart } from "../components/TrendChart";
import { api } from "../lib/ipc";
import { store, useStore } from "../lib/store";
import type { ModelDetailDto } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { formatCny, formatFull, formatPercent, formatRelative, formatTokens, shortSessionId } from "../lib/format";
import { backdropVariants, dialogVariants, listItemVariants } from "../lib/motion";

export function ModelsPage() {
  const dash = useStore((s) => s.dash);
  const detail = useStore((s) => s.modelDetail);
  const costSummary = useStore((s) => s.costSummary);
  const costByModel = new Map(
    (costSummary?.models ?? []).map((m) => [m.name, m]),
  );

  if (!dash) return <div className="empty-state">加载中…</div>;

  return (
    <div style={{ paddingTop: 6 }}>
      <div className="panel">
        <div className="panel-title">全部模型(当前时间范围)</div>
        {dash.models.length === 0 && <div className="empty-state">该范围内没有模型调用</div>}
        <AnimatePresence initial={false}>
          {dash.models.map((m, i) => (
            <motion.div
              key={m.name}
              className="model-row"
              layout="position"
              variants={listItemVariants}
              initial="initial"
              animate="enter"
              exit="exit"
              onClick={() =>
                api
                  .modelDetail(m.name)
                  .then((d) => store.set({ modelDetail: d }))
                  .catch(() => {})
              }
              title="点击查看模型详情"
            >
              <div>
                <div className="name">
                  <span className="muted" style={{ marginRight: 6 }}>
                    {i + 1}.
                  </span>
                  {m.name}
                </div>
                <div className="share-track">
                  <div className="share-fill" style={{ width: `${Math.round(m.share * 100)}%` }} />
                </div>
              </div>
              <span className="num">{formatTokens(totalTokens(m.agg))}</span>
              <span className="num">{(m.share * 100).toFixed(1)}%</span>
              <span className="num">{formatTokens(m.agg.input)}</span>
              <span className="num">{formatTokens(m.agg.output)}</span>
              <span className="num">
                {m.agg.reasoning.present > 0 ? formatTokens(m.agg.reasoning.sum) : "—"}
              </span>
              <span className="num">
                {m.agg.cacheRead.present > 0 ? formatTokens(m.agg.cacheRead.sum) : "—"}
              </span>
              <span className="num">
                {cacheHitRate(m.agg) === null
                  ? "—"
                  : `${(cacheHitRate(m.agg)! * 100).toFixed(0)}% · ${formatFull(m.agg.requests)}`}
              </span>
              <span className="num" style={{ color: "var(--zup-blue-600)" }}>
                {(() => {
                  const c = costByModel.get(m.name);
                  return c?.priced ? `≈ ${formatCny(c.costCny)}` : "价格未知";
                })()}
              </span>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>

      <AnimatePresence>
        {detail && <ModelDetailCard key={detail.name} detail={detail} />}
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
    <motion.div
      key={detail.name}
      className="overlay-backdrop"
      variants={backdropVariants}
      initial="initial"
      animate="enter"
      exit="exit"
      onClick={(e) => {
        if (e.target === e.currentTarget) store.set({ modelDetail: null });
      }}
    >
      <motion.div className="panel overlay-card" variants={dialogVariants}>
        <div className="panel-title">
          模型详情 · {detail.name}
          <span className="right">
            <button className="model-chip" onClick={() => store.set({ modelDetail: null })}>
              关闭
            </button>
          </span>
        </div>
        <div className="zup-grid metrics-grid" style={{ marginBottom: 12 }}>
          <MetricCard
            label="今天"
            value={<AnimatedNumber value={totalTokens(detail.today)} format={formatTokens} />}
          />
          <MetricCard
            label="7 天"
            value={<AnimatedNumber value={totalTokens(detail.last7d)} format={formatTokens} />}
          />
          <MetricCard
            label="30 天"
            value={<AnimatedNumber value={totalTokens(detail.last30d)} format={formatTokens} />}
          />
          <MetricCard
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
      </motion.div>
    </motion.div>
  );
}
