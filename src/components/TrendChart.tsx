import { useMemo, useState } from "react";
import type { Agg, Bucket, TrendDto } from "../lib/types";
import { totalTokens } from "../lib/types";
import { formatBucketLabel, formatFull, formatTokens } from "../lib/format";

/**
 * Hand-rolled SVG trend chart (stacked bars / model lines).
 * Zero chart dependencies → tiny bundle, no layout thrash, fully memoized.
 */

const STACK_SERIES: {
  key:
    | keyof Pick<Agg, "input" | "output" | "reasoning" | "cacheRead" | "cacheWrite">
    | "total"
    | "cache";
  label: string;
  color: string;
  extract: (a: Agg) => number;
}[] = [
  { key: "output", label: "Output", color: "var(--zup-series-output)", extract: (a) => a.output },
  {
    key: "reasoning",
    label: "Reasoning",
    color: "var(--zup-series-reasoning)",
    extract: (a) => a.reasoning.sum,
  },
  {
    key: "cache",
    label: "Cache",
    color: "var(--zup-series-cache)",
    extract: (a) => a.cacheRead.sum + a.cacheWrite.sum,
  },
  {
    key: "input",
    label: "Input",
    color: "var(--zup-series-input)",
    extract: (a) => a.input,
  },
];

const MODEL_COLORS = [
  "#2f7bf6",
  "#35c5e8",
  "#8f7bf5",
  "#29a383",
  "#f5a524",
  "#e5484d",
  "#0ea5e9",
  "#be185d",
];

export function TrendChart({
  trend,
  visibleModels,
  height = 190,
}: {
  trend: TrendDto | null;
  visibleModels: string[] | null;
  height?: number;
}) {
  const [mode, setMode] = useState<"stack" | "models">("stack");
  const [hover, setHover] = useState<number | null>(null);
  const buckets = trend?.buckets ?? [];
  const modelNames = useMemo(() => {
    const set = new Set<string>();
    for (const b of buckets) {
      for (const m of Object.keys(b.byModel)) set.add(m);
    }
    return Array.from(set);
  }, [buckets]);

  if (!buckets.length) {
    return (
      <div className="empty-state" style={{ height }}>
        {trend?.restored
          ? "趋势数据将在首次同步完成后显示"
          : "当前时间范围内没有 usage 记录"}
      </div>
    );
  }

  const W = 1000;
  const H = height;
  const padL = 6;
  const padB = 20;
  const plotW = W - padL * 2;
  const plotH = H - padB;
  const bw = plotW / buckets.length;

  const visible = visibleModels?.length
    ? modelNames.filter((m) => visibleModels.includes(m))
    : modelNames;

  const max = Math.max(
    1,
    ...(mode === "stack"
      ? buckets.map((b) => STACK_SERIES.reduce((s, d) => s + d.extract(b.agg), 0))
      : buckets.map((b) =>
          visible.reduce((s, m) => s + (b.byModel[m] ? totalTokens(b.byModel[m]) : 0), 0),
        )),
  );

  const yScale = (v: number) => plotH - (v / max) * (plotH - 8);

  return (
    <div>
      <div style={{ display: "flex", gap: 6, marginBottom: 8, flexWrap: "wrap" }}>
        <button
          className={`model-chip ${mode === "stack" ? "on" : ""}`}
          onClick={() => setMode("stack")}
        >
          字段堆叠
        </button>
        {modelNames.map((m, i) => {
          const on = !visibleModels || visibleModels.includes(m);
          return (
            <button
              key={m}
              className={`model-chip ${on ? "on" : ""}`}
              onClick={() => {
                setMode("models");
                const current = visibleModels ?? modelNames;
                const next = on
                  ? current.filter((x) => x !== m)
                  : [...current, m];
                window.dispatchEvent(
                  new CustomEvent("zup-toggle-model", {
                    detail: next.length ? next : null,
                  }),
                );
              }}
              title={`${m} — 点击显示/隐藏`}
            >
              <span className="dot" style={{ background: MODEL_COLORS[i % MODEL_COLORS.length] }} />
              {m}
            </button>
          );
        })}
      </div>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        style={{ width: "100%", height: "auto", display: "block" }}
        onPointerLeave={() => setHover(null)}
      >
        {/* gridlines */}
        {[0.25, 0.5, 0.75, 1].map((f) => (
          <line
            key={f}
            x1={padL}
            x2={W - padL}
            y1={yScale(max * f)}
            y2={yScale(max * f)}
            stroke="currentColor"
            strokeOpacity="0.08"
            strokeDasharray="3 5"
          />
        ))}
        {mode === "stack"
          ? buckets.map((b, i) => {
              let acc = 0;
              return (
                <g
                  key={b.startMs}
                  onPointerEnter={() => setHover(i)}
                  style={{ cursor: "crosshair" }}
                >
                  <rect
                    x={padL + i * bw}
                    y={0}
                    width={bw}
                    height={H}
                    fill="transparent"
                  />
                  {STACK_SERIES.map((s) => {
                    const v = s.extract(b.agg);
                    if (v <= 0) return null;
                    const y1 = yScale(acc + v);
                    const y2 = yScale(acc);
                    acc += v;
                    return (
                      <rect
                        key={s.key}
                        x={padL + i * bw + bw * 0.14}
                        y={y1}
                        width={bw * 0.72}
                        height={Math.max(0, y2 - y1)}
                        fill={s.color}
                        opacity={hover === null || hover === i ? 0.88 : 0.45}
                        rx={Math.min(2, bw * 0.2)}
                        style={{ transition: "opacity 160ms ease" }}
                      />
                    );
                  })}
                </g>
              );
            })
          : visible.map((m, mi) => {
              const pts = buckets
                .map((b, i) => {
                  const v = b.byModel[m] ? totalTokens(b.byModel[m]) : 0;
                  return `${padL + i * bw + bw / 2},${yScale(v)}`;
                })
                .join(" ");
              return (
                <polyline
                  key={m}
                  points={pts}
                  fill="none"
                  stroke={MODEL_COLORS[modelNames.indexOf(m) % MODEL_COLORS.length]}
                  strokeWidth={mi === 0 ? 2 : 1.7}
                  strokeLinejoin="round"
                  opacity={0.9}
                />
              );
            })}
        {/* x labels (sparse) */}
        {buckets.map((b, i) => {
          const every = Math.ceil(buckets.length / 8);
          if (i % every !== 0) return null;
          return (
            <text
              key={b.startMs}
              x={padL + i * bw + bw / 2}
              y={H - 5}
              fontSize="10"
              textAnchor="middle"
              fill="var(--zup-text-3)"
            >
              {formatBucketLabel(b.startMs, trend?.rangeKey ?? "24h")}
            </text>
          );
        })}
        {hover !== null && (
          <line
            x1={padL + hover * bw + bw / 2}
            x2={padL + hover * bw + bw / 2}
            y1={0}
            y2={plotH}
            stroke="var(--zup-blue-500)"
            strokeOpacity="0.35"
          />
        )}
      </svg>
      {hover !== null && <ChartTooltip bucket={buckets[hover]} rangeKey={trend?.rangeKey ?? ""} />}
    </div>
  );
}

function ChartTooltip({ bucket, rangeKey }: { bucket: Bucket; rangeKey: string }) {
  const total = totalTokens(bucket.agg);
  const models = Object.entries(bucket.byModel)
    .map(([name, agg]) => ({ name, total: totalTokens(agg) }))
    .sort((a, b) => b.total - a.total)
    .slice(0, 3);
  return (
    <div
      className="panel"
      style={{
        padding: "8px 12px",
        fontSize: 11.5,
        pointerEvents: "none",
        margin: "4px 2px",
      }}
    >
      <div style={{ fontWeight: 650, marginBottom: 2 }}>
        {formatBucketLabel(bucket.startMs, rangeKey)} · {formatTokens(total)} tokens ·{" "}
        {formatFull(bucket.agg.requests)} 次
      </div>
      <div className="muted">
        In {formatTokens(bucket.agg.input)} · Out {formatTokens(bucket.agg.output)} · Reason{" "}
        {formatTokens(bucket.agg.reasoning.sum)} · Cache{" "}
        {formatTokens(bucket.agg.cacheRead.sum + bucket.agg.cacheWrite.sum)}
      </div>
      {models.length > 1 && (
        <div className="muted">
          {models.map((m) => `${m.name} ${formatTokens(m.total)}`).join(" · ")}
        </div>
      )}
    </div>
  );
}
