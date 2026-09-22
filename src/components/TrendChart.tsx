import { useMemo, useState } from "react";
import type { Agg, Bucket, TrendDto } from "../lib/types";
import { totalTokens } from "../lib/types";
import { formatBucketLabel, formatFull, formatTokens } from "../lib/format";

/**
 * Hand-rolled SVG trend chart (zero chart dependencies).
 * K3 布局(参考 Usage panel web 版):按模型/按类型堆叠柱、y 轴刻度 +
 * 水平网格线、右上角图例(按模型下点击图例切换模型可见性)、
 * hover 十字线 + 悬浮明细卡。
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

/* K3 模型色板（Usage panel web 版 MODEL_PALETTE） */
const MODEL_COLORS = [
  "#3a5bec",
  "#6fc3d8",
  "#8e7cf0",
  "#3ec9a7",
  "#e8b04b",
  "#e37f94",
  "#98a8b8",
  "#b9bec7",
];
const OTHER_COLOR = "#b9bec7";

/** 排行图中折叠进「其他」的具名模型数(其余聚合为灰色)。 */
const NAMED_MODELS = 4;

interface Seg {
  key: string;
  color: string;
  value: number;
}

/** 上取整到 1/2/2.5/5×10^n,让 y 轴刻度是整数档(如 3.00M 步进)。 */
function niceCeil(v: number): number {
  if (!isFinite(v) || v <= 0) return 1;
  const exp = Math.floor(Math.log10(v));
  const base = Math.pow(10, exp);
  const n = v / base;
  const nice = n <= 1 ? 1 : n <= 2 ? 2 : n <= 2.5 ? 2.5 : n <= 5 ? 5 : 10;
  return nice * base;
}

export function TrendChart({
  trend,
  visibleModels,
  height = 220,
}: {
  trend: TrendDto | null;
  visibleModels: string[] | null;
  height?: number;
}) {
  const [mode, setMode] = useState<"models" | "stack">("models");
  const [hover, setHover] = useState<number | null>(null);
  const buckets = trend?.buckets ?? [];
  const modelNames = useMemo(() => {
    const set = new Set<string>();
    for (const b of buckets) {
      for (const m of Object.keys(b.byModel)) set.add(m);
    }
    return Array.from(set);
  }, [buckets]);
  // 按总量取 Top-N 具名模型,其余折进「其他」(与 Web 版排行图同思路)。
  const topModels = useMemo(() => {
    const totals = new Map<string, number>();
    for (const b of buckets) {
      for (const [m, agg] of Object.entries(b.byModel)) {
        totals.set(m, (totals.get(m) ?? 0) + totalTokens(agg));
      }
    }
    return [...totals.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, NAMED_MODELS)
      .map(([m]) => m);
  }, [buckets]);
  const visible =
    visibleModels?.length
      ? modelNames.filter((m) => visibleModels.includes(m))
      : modelNames;

  const bucketMin =
    buckets.length > 1
      ? Math.round((buckets[1].startMs - buckets[0].startMs) / 60_000)
      : null;

  if (!buckets.length) {
    return (
      <div className="empty-state" style={{ height }}>
        {trend?.restored
          ? "趋势数据将在首次同步完成后显示"
          : "当前时间范围内没有 usage 记录"}
      </div>
    );
  }

  const segsFor = (b: Bucket): Seg[] => {
    if (mode === "stack") {
      return STACK_SERIES.map((s) => ({
        key: s.key,
        color: s.color,
        value: s.extract(b.agg),
      })).filter((s) => s.value > 0);
    }
    const segs: Seg[] = [];
    let namedSum = 0;
    topModels.forEach((m, i) => {
      if (!visible.includes(m)) return;
      const agg = b.byModel[m];
      const v = agg ? totalTokens(agg) : 0;
      if (v > 0) {
        segs.push({ key: m, color: MODEL_COLORS[i % MODEL_COLORS.length], value: v });
        namedSum += v;
      }
    });
    // 桶总量减去具名可见部分 = 其他(含被隐藏的模型),保持总量诚实。
    const other = totalTokens(b.agg) - namedSum;
    if (other > 0.5) segs.push({ key: "__other__", color: OTHER_COLOR, value: other });
    return segs;
  };

  const W = 1000;
  const H = height;
  const padL = 56;
  const padR = 8;
  const padB = 22;
  const padT = 8;
  const plotW = W - padL - padR;
  const plotH = H - padB - padT;
  const bw = plotW / buckets.length;

  const max = niceCeil(
    Math.max(1, ...buckets.map((b) => segsFor(b).reduce((s, x) => s + x.value, 0))),
  );
  const yScale = (v: number) => padT + plotH - (v / max) * plotH;
  const yTicks = [0, 0.25, 0.5, 0.75, 1];

  const toggleModel = (m: string) => {
    const current = visibleModels ?? modelNames;
    const on = visible.includes(m);
    const next = on ? current.filter((x) => x !== m) : [...current, m];
    window.dispatchEvent(
      new CustomEvent("zup-toggle-model", { detail: next.length ? next : null }),
    );
  };

  return (
    <div>
      <div className="trend-head">
        <div className="trend-mode" role="group" aria-label="趋势维度">
          <button
            className={mode === "models" ? "on" : ""}
            onClick={() => setMode("models")}
            aria-pressed={mode === "models"}
          >
            按模型
          </button>
          <button
            className={mode === "stack" ? "on" : ""}
            onClick={() => setMode("stack")}
          >
            按类型
          </button>
        </div>
        {bucketMin !== null && (
          <span className="muted" style={{ fontSize: 11 }}>
            每桶 {bucketMin} 分钟
          </span>
        )}
        <div className="trend-legend">
          {mode === "models" ? (
            <>
              {topModels.map((m, i) => {
                const on = visible.includes(m);
                return (
                  <button
                    key={m}
                    className="model-chip"
                    onClick={() => toggleModel(m)}
                    title={`${m} — 点击显示/隐藏`}
                  >
                    <span
                      className="dot"
                      style={{ background: MODEL_COLORS[i % MODEL_COLORS.length] }}
                    />
                    {m}
                  </button>
                );
              })}
              <span className="model-chip" title="其余模型合计">
                <span className="dot" style={{ background: OTHER_COLOR }} />
                其他
              </span>
            </>
          ) : (
            STACK_SERIES.map((s) => (
              <span key={s.key} className="model-chip">
                <span className="dot" style={{ background: s.color }} />
                {s.label}
              </span>
            ))
          )}
        </div>
      </div>
      <svg
        viewBox={`0 0 ${W} ${H}`}
        style={{ width: "100%", height: "auto", display: "block" }}
        onPointerLeave={() => setHover(null)}
      >
        {/* y gridlines + labels */}
        {yTicks.map((f) => (
          <g key={f}>
            <line
              x1={padL}
              x2={W - padR}
              y1={yScale(max * f)}
              y2={yScale(max * f)}
              stroke="currentColor"
              strokeOpacity={f === 0 ? 0.16 : 0.08}
            />
            <text
              x={padL - 7}
              y={yScale(max * f) + 3}
              fontSize="9.5"
              textAnchor="end"
              fill="var(--zup-text-3)"
            >
              {formatTokens(Math.round(max * f))}
            </text>
          </g>
        ))}
        {buckets.map((b, i) => {
          let acc = 0;
          const segs = segsFor(b);
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
              {segs.map((s) => {
                const y1 = yScale(acc + s.value);
                const y2 = yScale(acc);
                acc += s.value;
                return (
                  <rect
                    key={s.key}
                    x={padL + i * bw + bw * 0.14}
                    y={y1}
                    width={bw * 0.72}
                    height={Math.max(0, y2 - y1)}
                    fill={s.color}
                    opacity={hover === null || hover === i ? 0.94 : 0.45}
                    rx={Math.min(2, bw * 0.2)}
                    style={{ transition: "opacity 160ms ease" }}
                  />
                );
              })}
            </g>
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
            y2={plotH + padT}
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
