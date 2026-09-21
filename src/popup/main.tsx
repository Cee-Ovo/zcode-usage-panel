import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "open-glass-ui/styles.css";
import "../styles/theme.css";
import "../styles/global.css";
import "./popup.css";
import { api, onEvent } from "../lib/ipc";
import type { DashboardDto, UsageUpdateEvent } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { formatFull, formatPercent, formatRelative, formatTokens } from "../lib/format";

// Match the main window's resolved theme (written by index.html bootstrap).
try {
  const saved = localStorage.getItem("zup.theme");
  if (saved) document.documentElement.setAttribute("data-theme", saved);
} catch {
  /* light default */
}

/** Compact tray popup: today's total, top models, footer health. */
function Popup() {
  const [dash, setDash] = useState<DashboardDto | null>(null);
  const [update, setUpdate] = useState<UsageUpdateEvent | null>(null);

  useEffect(() => {
    const load = () => {
      api.dashboard("today").then(setDash).catch(() => {});
    };
    load();
    const unsubs: (() => void)[] = [];
    onEvent<UsageUpdateEvent>("usage-update", (e) => {
      setUpdate(e);
      load();
    }).then((u) => unsubs.push(u));
    // Backend asks for a refresh when the popup is shown.
    onEvent<unknown>("popup-refresh", load).then((u) => unsubs.push(u));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") api.popupClose().catch(() => {});
    };
    document.addEventListener("keydown", onKey);
    return () => {
      unsubs.forEach((u) => u());
      document.removeEventListener("keydown", onKey);
    };
  }, []);

  const hit = dash ? cacheHitRate(dash.agg) : null;
  const top = dash?.models.slice(0, 2) ?? [];
  const degraded = update?.errorStreak ? update.errorStreak > 0 : false;

  return (
    <div className="popup-card">
      <div className="popup-head">
        <span className="popup-title">ZCode 今日用量</span>
        <span className="muted" style={{ fontSize: 10.5 }}>
          {update?.restoredFromCache ? "缓存" : "实时"}
        </span>
      </div>
      {dash ? (
        <>
          <div className="popup-total" title="ZCode 本地 usage 记录统计">
            {formatTokens(totalTokens(dash.agg))} <small>tokens</small>
          </div>
          <div className="popup-sub">{formatFull(totalTokens(dash.agg))} tokens</div>
        </>
      ) : (
        <div className="muted" style={{ padding: "8px 0", fontSize: 11.5 }}>
          加载今日用量…
        </div>
      )}

      {top.map((m, i) => {
        const mh = cacheHitRate(m.agg);
        return (
          <div className="popup-model" key={m.name}>
            <div className="rank">{i + 1}</div>
            <div style={{ flex: 1, minWidth: 0 }}>
              <div className="m-name">{m.name}</div>
              <div className="share-track">
                <div className="share-fill" style={{ width: `${m.share * 100}%` }} />
              </div>
            </div>
            <div style={{ textAlign: "right", fontSize: 11, lineHeight: 1.5 }}>
              <div style={{ fontWeight: 650 }}>{formatTokens(totalTokens(m.agg))}</div>
              <div className="muted">{(m.share * 100).toFixed(1)}%</div>
              <div className="muted">
                Cache {mh === null ? "—" : formatPercent(mh, 0)}
              </div>
            </div>
          </div>
        );
      })}

      <div className="popup-foot">
        {degraded ? (
          <span className="degraded-text">⚠ 数据源连续失败 {update?.errorStreak} 次</span>
        ) : (
          <span className="healthy">● 数据正常</span>
        )}
        {update?.lastRefreshMs ? <span>更新 {formatRelative(update.lastRefreshMs)}</span> : null}
      </div>
      {dash && (
        <div className="popup-foot muted">
          <span>今日 {formatFull(dash.agg.requests)} 次请求</span>
          <span>{dash.activeSession?.activeModel ?? dash.models[0]?.name ?? "—"}</span>
        </div>
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Popup />
  </StrictMode>,
);