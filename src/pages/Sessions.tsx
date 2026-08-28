import { useState } from "react";
import { SearchField } from "open-glass-ui";
import { TrendChart } from "../components/TrendChart";
import { api } from "../lib/ipc";
import { store, useStore } from "../lib/store";
import type { SessionSummary } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { formatDateTime, formatRelative, formatTokens, shortSessionId } from "../lib/format";

export function SessionsPage() {
  const sessions = useStore((s) => s.sessions);
  const detail = useStore((s) => s.sessionDetail);
  const [query, setQuery] = useState("");

  const filtered = sessions.filter(
    (s) =>
      !query ||
      s.id.toLowerCase().includes(query.toLowerCase()) ||
      (s.project ?? "").toLowerCase().includes(query.toLowerCase()) ||
      s.models.some((m) => m.toLowerCase().includes(query.toLowerCase())),
  );

  return (
    <div style={{ paddingTop: 6 }}>
      <div style={{ display: "flex", gap: 10, alignItems: "center", marginBottom: 12 }}>
        <SearchField
          label="搜索 session / 项目 / 模型"
          value={query}
          onValueChange={setQuery}
          placeholder="搜索 session / 项目 / 模型…"
        />
        <span className="muted" style={{ fontSize: 11 }}>
          {filtered.length} / {sessions.length} sessions
        </span>
      </div>
      <div className="panel">
        <div className="session-row table-head" style={{ cursor: "default" }}>
          <span>Session</span>
          <span>项目</span>
          <span>模型</span>
          <span style={{ textAlign: "right" }}>Input</span>
          <span style={{ textAlign: "right" }}>Output</span>
          <span style={{ textAlign: "right" }}>Reasoning</span>
          <span style={{ textAlign: "right" }}>Cache</span>
          <span style={{ textAlign: "right" }}>总 Token</span>
          <span style={{ textAlign: "right" }}>命中率 / 最近活动</span>
        </div>
        {filtered.length === 0 && (
          <div className="empty-state">
            没有匹配的 Session。
            <br />
            <span style={{ fontSize: 11 }}>
              数据来源为 ZCode 本地记录;若列表为空,请到「设置 → 数据源详情」检查目录。
            </span>
          </div>
        )}
        {filtered.slice(0, 200).map((s) => (
          <SessionLine key={s.id} s={s} />
        ))}
        {filtered.length > 200 && (
          <div className="muted" style={{ padding: 10, fontSize: 11 }}>
            仅显示前 200 条(共 {filtered.length}),可导出全部数据。
          </div>
        )}
      </div>

      {detail && (
        <div
          className="overlay-backdrop"
          onClick={(e) => {
            if (e.target === e.currentTarget) store.set({ sessionDetail: null });
          }}
        >
          <div className="panel overlay-card rise">
            <div className="panel-title">
              Session 详情 · {shortSessionId(detail.summary.id)}
              <span className="right">
                <button
                  className="model-chip"
                  onClick={() => store.set({ sessionDetail: null })}
                >
                  关闭
                </button>
              </span>
            </div>
            <div className="kv" style={{ marginBottom: 12 }}>
              <span className="k">项目</span>
              <span>{detail.summary.project ?? "—"}</span>
              <span className="k">模型</span>
              <span>{detail.summary.models.join(", ")}</span>
              <span className="k">开始 → 最后活动</span>
              <span>
                {formatDateTime(detail.summary.agg.firstTsMs)} →{" "}
                {formatDateTime(detail.summary.agg.lastTsMs)}
              </span>
              <span className="k">总 Token</span>
              <span>{formatTokens(totalTokens(detail.summary.agg))}</span>
            </div>
            <TrendChart
              trend={{
                rangeKey: "session",
                fromMs: detail.summary.agg.firstTsMs ?? 0,
                toMs: detail.summary.agg.lastTsMs ?? 0,
                buckets: detail.buckets,
                restored: false,
              }}
              visibleModels={null}
            />
          </div>
        </div>
      )}
    </div>
  );
}

function SessionLine({ s }: { s: SessionSummary }) {
  const hit = cacheHitRate(s.agg);
  return (
    <div
      className="session-row"
      onClick={() => {
        api
          .sessionDetail(s.id)
          .then((d) => store.set({ sessionDetail: d }))
          .catch(() => {});
      }}
      title="点击查看 Session 趋势"
    >
      <span title={s.id}>{shortSessionId(s.id)}</span>
      <span
        className="muted"
        style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
      >
        {s.project ?? "—"}
      </span>
      <span
        className="muted"
        style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
      >
        {s.models.join(", ")}
      </span>
      <span className="num">{formatTokens(s.agg.input)}</span>
      <span className="num">{formatTokens(s.agg.output)}</span>
      <span className="num">
        {s.agg.reasoning.present > 0 ? formatTokens(s.agg.reasoning.sum) : "—"}
      </span>
      <span className="num">
        {s.agg.cacheRead.present > 0 ? formatTokens(s.agg.cacheRead.sum) : "—"}
      </span>
      <span className="num" style={{ fontWeight: 600 }}>
        {formatTokens(totalTokens(s.agg))}
      </span>
      <span className="num">
        {hit === null ? "—" : `${(hit * 100).toFixed(0)}%`}
        <div className="muted" style={{ fontSize: 10 }}>
          {formatRelative(s.agg.lastTsMs)}
        </div>
      </span>
    </div>
  );
}
