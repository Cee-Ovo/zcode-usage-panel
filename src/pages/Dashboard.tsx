import { useState } from "react";
import { Button, SegmentedControl } from "open-glass-ui";
import { AnimatedNumber } from "../components/AnimatedNumber";
import { CostDetailModal } from "../components/CostDetailModal";
import { MetricCard, InfoDot } from "../components/MetricCard";
import { TrendChart } from "../components/TrendChart";
import { api } from "../lib/ipc";
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
  "总 Token = Input + Output + Reasoning + Cache(读+写),仅累加数据源真实提供的字段;未提供的字段不计入、不推算。";

export function DashboardPage({ onRangeChange }: { onRangeChange: (key: string) => void }) {
  const dash = useStore((s) => s.dash);
  const rangeKey = useStore((s) => s.rangeKey);
  const trend = useStore((s) => s.trend);
  const visibleModels = useStore((s) => s.trendVisibleModels);
  const costSummary = useStore((s) => s.costSummary);
  const alerts = useStore((s) => s.alerts);
  const [expanded, setExpanded] = useState(false);
  const [costModalModel, setCostModalModel] = useState<string | null>(null);

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
    <div className="zup-grid" style={{ paddingTop: 6 }}>
      {/* range selector */}
      <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
        <SegmentedControl
          aria-label="时间范围"
          value={rangeKey}
          onValueChange={(v) => onRangeChange(v)}
          items={RANGE_KEYS.map((k) => ({ value: k, label: RANGE_LABELS[k] }))}
        />
        {dash.restored && <span className="badge-note">缓存快照 · 同步中</span>}
        {dash.dataError && (
          <span className="badge-note" title={dash.dataError}>
            数据源异常
          </span>
        )}
        <span style={{ marginLeft: "auto" }}>
          <Button
            variant="quiet"
            onClick={() => {
              api.refreshNow();
            }}
          >
            立即刷新
          </Button>
        </span>
      </div>

      {/* core metrics */}
      <div className="zup-grid metrics-grid">
        <MetricCard
          label="总 Token"
          value={<AnimatedNumber value={totalTokens(agg)} format={formatTokens} />}
          sub={`${formatFull(totalTokens(agg))} tokens`}
          hint={TOTAL_HINT}
        />
        <MetricCard
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

      {/* top models */}
      <div className="panel">
        <div className="panel-title">
          模型排行
          <span className="right muted">
            {dash.models.length > 3 && (
              <Button variant="quiet" onClick={() => setExpanded(!expanded)}>
                {expanded ? "收起" : `展开全部 (${dash.models.length})`}
              </Button>
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
        {models.map((m) => (
          <ModelLine
            key={m.name}
            m={m}
            cost={costByModel.get(m.name)}
            onCostClick={() => setCostModalModel(m.name)}
          />
        ))}
      </div>

      {/* recent local alerts */}
      {alerts.length > 0 && (
        <div className="panel">
          <div className="panel-title">异常提醒(本地)</div>
          {alerts.slice(0, 3).map((a) => (
            <div key={`${a.rule}-${a.tsMs}`} className={`alert-chip ${a.severity >= 2 ? "critical" : ""}`}>
              <span style={{ fontWeight: 650 }}>{a.title}</span>
              <span className="muted" style={{ flex: 1 }}>
                {a.body}
              </span>
              <span className="muted" style={{ fontSize: 10.5 }}>
                {formatRelative(a.tsMs)}
              </span>
            </div>
          ))}
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
          实时趋势 · {RANGE_LABELS[rangeKey as keyof typeof RANGE_LABELS] ?? rangeKey}
          <span className="right">
            <InfoDot text="点击模型名可单独显示/隐藏该模型的曲线。" />
          </span>
        </div>
        <TrendChart trend={trend} visibleModels={visibleModels} />
      </div>

      {costModalModel && (
        <CostDetailModal
          model={costModalModel}
          rangeKey={rangeKey}
          fx={costSummary?.fx}
          priceUpdatedAt={costSummary?.priceUpdatedAt}
          onClose={() => setCostModalModel(null)}
        />
      )}
    </div>
  );
}

function ModelLine({
  m,
  cost,
  onCostClick,
}: {
  m: ModelRow;
  cost: ModelCost | undefined;
  onCostClick: () => void;
}) {
  const hit = cacheHitRate(m.agg);
  return (
    <div
      className="model-row"
      onClick={() => {
        store.set({ page: "models" });
        api.modelDetail(m.name).then((d) => store.set({ modelDetail: d })).catch(() => {});
      }}
      title="点击查看模型详情"
    >
      <div>
        <div className="name">{m.name}</div>
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
        {hit === null ? "—" : `${(hit * 100).toFixed(0)}% · ${formatFull(m.agg.requests)}`}
      </span>
      <span
        className="num"
        title="点击查看成本明细"
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
    </div>
  );
}
