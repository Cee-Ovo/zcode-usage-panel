import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import "open-glass-ui/styles.css";
import "../styles/theme.css";
import "../styles/global.css";
import "./popup.css";
import { api, onEvent } from "../lib/ipc";
import type { DashboardDto, ProviderSnapshot, UsageUpdateEvent } from "../lib/types";
import { cacheHitRate, totalTokens } from "../lib/types";
import { formatFull, formatPercent, formatQuotaAmount, formatRelative, formatTokens } from "../lib/format";

// Match the main window's resolved theme (written by index.html bootstrap).
try {
  const saved = localStorage.getItem("zup.theme");
  if (saved) document.documentElement.setAttribute("data-theme", saved);
} catch {
  /* light default */
}

/** One compact line per provider (quota only — never merges metrics). */
function ProviderLine({ snap }: { snap: ProviderSnapshot }) {
  const name =
    snap.provider === "zcode"
      ? "ZCode"
      : snap.provider === "codex"
        ? "Codex"
        : snap.provider === "antigravity"
          ? "Antigravity"
          : "火山";
  const degraded = snap.status === "error" || snap.status === "stale";

  let value: string;
  if (degraded) {
    value = `⚠ 数据未更新 · ${formatRelative(snap.updatedAtMs)}`;
  } else if (snap.provider === "zcode") {
    const w = snap.windows.find((x) => x.key === "today_tokens");
    value = w?.usedQuota != null ? `今日 ${formatTokens(w.usedQuota)}` : "—";
  } else if (snap.provider === "volcengine") {
    const avail = snap.packages
      .filter((p) => p.status === "Effective")
      .reduce((s, p) => s + p.availableAmount * p.unitMultiplier, 0);
    value =
      avail > 0
        ? `剩余 ${formatQuotaAmount(avail, "tokens")}`
        : snap.status === "ok"
          ? "无生效 Token 包"
          : "未配置";
  } else {
    // codex / antigravity: up to two windows, e.g. "5h 72% · 周 54%"
    const parts = snap.windows
      .slice(0, 2)
      .map((w) => {
        const label =
          w.key === "5h" ? "5h" : w.key === "weekly" ? "周" : w.label.slice(0, 6);
        return w.usedPercent !== null ? `${label} ${w.usedPercent.toFixed(0)}%` : `${label} —`;
      });
    value = parts.length > 0 ? parts.join(" · ") : snap.status === "ok" ? "无额度数据" : "未配置";
  }

  return (
    <div className="popup-provider" title={`${snap.source}${snap.error ? `\n${snap.error}` : ""}`}>
      <span className="p-name">{name}</span>
      <span className={`p-value ${degraded ? "degraded" : ""}`}>{value}</span>
    </div>
  );
}

/** Compact tray popup: today's total, provider quotas, footer health. */
function Popup() {
  const [dash, setDash] = useState<DashboardDto | null>(null);
  const [update, setUpdate] = useState<UsageUpdateEvent | null>(null);
  const [providers, setProviders] = useState<ProviderSnapshot[]>([]);

  useEffect(() => {
    const load = () => {
      api.dashboard("today").then(setDash).catch(() => {});
      api.providersOverview().then(setProviders).catch(() => {});
    };
    load();
    const unsubs: (() => void)[] = [];
    onEvent<UsageUpdateEvent>("usage-update", (e) => {
      setUpdate(e);
      load();
    }).then((u) => unsubs.push(u));
    onEvent<ProviderSnapshot[]>("provider-update", (snaps) => setProviders(snaps ?? [])).then((u) =>
      unsubs.push(u),
    );
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
  const degraded = providers.filter(
    (p) => p.status === "error" || p.status === "stale",
  );
  const newestUpdate = providers.reduce<number>(
    (m, p) => Math.max(m, p.updatedAtMs || 0),
    update?.lastRefreshMs ?? 0,
  );

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

      {providers.length > 0 && (
        <div className="popup-providers">
          <div className="popup-providers-title">服务额度</div>
          {providers.map((p) => (
            <ProviderLine key={p.provider} snap={p} />
          ))}
        </div>
      )}

      <div className="popup-foot">
        {degraded.length === 0 ? (
          <span className="healthy">● 各服务数据正常</span>
        ) : (
          <span className="degraded-text">⚠ {degraded.length} 项数据异常</span>
        )}
        {newestUpdate > 0 && <span>更新 {formatRelative(newestUpdate)}</span>}
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
