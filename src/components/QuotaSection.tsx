import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { api } from "../lib/ipc";
import {
  backdropVariants,
  cardVariants,
  dialogVariants,
  progressSpring,
  softSpring,
  staggerContainer,
} from "../lib/motion";
import { useStore } from "../lib/store";
import type {
  HistoryPointDto,
  LauncherStatus,
  PackageInfo,
  ProviderSnapshot,
  QuotaWindow,
} from "../lib/types";
import {
  PROVIDER_LABELS,
  PROVIDER_STATUS_LABELS,
} from "../lib/types";
import {
  formatCountdown,
  formatDateTime,
  formatDuration,
  formatQuotaAmount,
  formatRelative,
  formatTokens,
} from "../lib/format";

/**
 * 服务额度:unified quota cards for ZCode / Codex / Antigravity / Volcengine.
 * Design rules:
 * - official plan quota and local token usage NEVER merge into one metric,
 * - missing data renders `unavailable` — nothing is invented,
 * - forecasts are always labelled 预测,
 * - units are not compared across providers.
 */

const STATUS_CLASS: Record<string, string> = {
  ok: "ok",
  not_configured: "idle",
  not_installed: "idle",
  disabled: "idle",
  stale: "warn",
  error: "warn",
};

export const QuotaSection = memo(function QuotaSection() {
  const providers = useStore((s) => s.providers);
  const [detail, setDetail] = useState<string | null>(null);

  const refreshAll = useCallback(() => {
    api.providersRefresh().catch(() => {});
  }, []);

  if (providers.length === 0) {
    return (
      <div className="panel">
        <div className="panel-title">
          服务额度
          <span className="right">
            <button className="link-btn" onClick={refreshAll}>
              刷新
            </button>
          </span>
        </div>
        <div className="empty-state">正在初始化 Provider…</div>
      </div>
    );
  }

  return (
    <div className="panel">
      <div className="panel-title">
        服务额度
        <span className="right" style={{ display: "flex", gap: 10, alignItems: "center" }}>
          <span className="muted" style={{ fontSize: 10.5 }}>
            官方额度与本地用量分开统计
          </span>
          <button className="link-btn" onClick={refreshAll}>
            全部刷新
          </button>
        </span>
      </div>
      <motion.div
        className="quota-grid"
        variants={staggerContainer}
        initial="initial"
        animate="enter"
      >
        {providers.map((p) => (
          <ProviderCard key={p.provider} snap={p} onDetail={() => setDetail(p.provider)} />
        ))}
      </motion.div>
      <AnimatePresence>
        {detail && (
          <ProviderDetailModal
            key={detail}
            provider={detail}
            onClose={() => setDetail(null)}
          />
        )}
      </AnimatePresence>
    </div>
  );
});

function ProviderCard({ snap, onDetail }: { snap: ProviderSnapshot; onDetail: () => void }) {
  const label = PROVIDER_LABELS[snap.provider] ?? snap.provider;
  const statusClass = STATUS_CLASS[snap.status] ?? "idle";
  const zcodeWin = snap.provider === "zcode" ? snap.windows.find((w) => w.key === "today_tokens") : null;

  const refresh = (e: React.MouseEvent) => {
    e.stopPropagation();
    api.providersRefresh(snap.provider).catch(() => {});
  };

  return (
    <motion.div
      layout
      variants={cardVariants}
      whileHover={{ y: -2 }}
      whileTap={{ scale: 0.992 }}
      transition={softSpring}
      className={`quota-card status-${statusClass}`}
      onClick={onDetail}
      title={`数据来源:${snap.source}\n点击查看详情`}
    >
      <div className="quota-card-head">
        <span className="name">{label}</span>
        <span className={`status-dot ${statusClass === "ok" ? "" : statusClass}`} />
        <span className="muted status-text">
          {PROVIDER_STATUS_LABELS[snap.status] ?? snap.status}
        </span>
        <span style={{ marginLeft: "auto", display: "flex", gap: 8 }}>
          <button
            className="link-btn"
            onClick={refresh}
            title={`刷新 ${label}`}
          >
            ⟳
          </button>
        </span>
      </div>

      {snap.planName && (
        <div className="quota-plan">
          {snap.planName}
          {snap.account ? <span className="muted"> · {snap.account}</span> : null}
        </div>
      )}

      {snap.error && <div className="quota-error" title={snap.error}>{snap.error}</div>}

      {/* ZCode: quick-launch button + today's tokens */}
      {snap.provider === "zcode" && <ZcodeLauncher launcher={snap.launcher} />}

      {/* quota windows (progress rows) */}
      {snap.windows
        .filter((w) => w.key !== "today_tokens" || snap.provider !== "zcode")
        .slice(0, 3)
        .map((w) => (
          <QuotaRow key={w.key} w={w} />
        ))}

      {/* ZCode today tokens line */}
      {zcodeWin && (
        <div className="quota-row">
          <span className="quota-row-label">{zcodeWin.label ?? "今日"}</span>
          <span className="quota-row-value">
            {formatQuotaAmount(zcodeWin.usedQuota, "tokens")} tokens
          </span>
        </div>
      )}

      {/* volcengine packages summary */}
      {snap.packages.length > 0 && (
        <div className="quota-row">
          <span className="quota-row-label">
            Token 包 {snap.packages.filter((p) => p.status === "Effective").length}/
            {snap.packages.length}
          </span>
          <span className="quota-row-value" title="有效包剩余 / 总量">
            {formatQuotaAmount(
              snap.packages
                .filter((p) => p.status === "Effective")
                .reduce((s, p) => s + p.availableAmount * p.unitMultiplier, 0),
              "tokens",
            )}
          </span>
        </div>
      )}

      {/* codex/antigravity local usage one-liner */}
      {snap.localUsage && snap.provider !== "zcode" && snap.localUsage.allTime.totalTokens > 0 && (
        <div className="quota-row local">
          <span className="quota-row-label">本地统计</span>
          <span className="quota-row-value" title="本地 session 统计,与官方额度相互独立">
            今日 {formatTokens(snap.localUsage.today.totalTokens)} · 累计{" "}
            {formatTokens(snap.localUsage.allTime.totalTokens)}
          </span>
        </div>
      )}

      <div className="quota-foot">
        <span title={snap.source}>{formatRelative(snap.updatedAtMs)}</span>
        <span className="muted">详情 ›</span>
      </div>
    </motion.div>
  );
}

function QuotaRow({ w }: { w: QuotaWindow }) {
  const pct = w.usedPercent;
  const remainingPct = pct === null ? null : Math.max(0, 100 - pct);
  const critical = remainingPct !== null && remainingPct < 20;
  return (
    <div className="quota-row with-bar">
      <div className="quota-row-top">
        <span className="quota-row-label">{w.label}</span>
        {pct === null ? (
          <span className="quota-row-value muted">unavailable</span>
        ) : (
          <span className={`quota-row-value ${critical ? "critical" : ""}`}>
            {pct.toFixed(0)}%
            {w.remainingQuota !== null && w.unit
              ? ` · ${formatQuotaAmount(w.remainingQuota, w.unit)}`
              : ""}
          </span>
        )}
      </div>
      {pct !== null && (
        <div className="share-track">
          <motion.div
            className={`share-fill motion-driven ${critical ? "critical" : ""}`}
            initial={{ width: 0 }}
            animate={{ width: `${Math.min(100, Math.max(0, pct)).toFixed(1)}%` }}
            transition={progressSpring}
          />
        </div>
      )}
      <div className="quota-row-sub">
        {w.resetAtMs !== null && <span>{formatCountdown(w.resetAtMs)}重置</span>}
        {w.forecast && (
          <span
            className="forecast"
            title={`预测(置信度 ${w.forecast.confidence},样本 ${w.forecast.samples})·按近期速度估算,非官方数据`}
          >
            预计可用 {formatDuration(w.forecast.etaMs)}
          </span>
        )}
      </div>
    </div>
  );
}

/** ZCode 一键启动/唤醒。 */
function ZcodeLauncher({ launcher }: { launcher: LauncherStatus | null }) {
  const [busy, setBusy] = useState(false);
  if (!launcher) return null;
  const running = launcher.state === "running";
  const notInstalled = launcher.state === "not_installed";
  const onClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (notInstalled || busy) return;
    setBusy(true);
    (running ? api.zcodeReveal() : api.zcodeLaunch())
      .catch(() => {})
      .finally(() => setTimeout(() => setBusy(false), 400));
  };
  return (
    <div className="zcode-launcher">
      <span className={`status-dot ${running ? "" : "paused"}`} />
      <span className="muted launcher-state">
        {running ? `运行中${launcher.version ? ` · ${launcher.version}` : ""}` : notInstalled ? "未安装" : busy ? "启动中…" : "未运行"}
      </span>
      {!notInstalled && (
        <button
          className={`link-btn launch ${running ? "" : "primary"}`}
          onClick={onClick}
          title={running ? "显示/聚焦 ZCode 窗口" : "启动 ZCode(已运行则激活)"}
        >
          {running ? "显示" : busy ? "启动中" : "启动"}
        </button>
      )}
    </div>
  );
}

/** Detail modal: trend + full data per provider. */
function ProviderDetailModal({ provider, onClose }: { provider: string; onClose: () => void }) {
  const snaps = useStore((s) => s.providers);
  const snap = snaps.find((p) => p.provider === provider);
  const [range, setRange] = useState<"today" | "7d" | "30d">("7d");
  const [trend, setTrend] = useState<HistoryPointDto[] | null>(null);
  const [consumption, setConsumption] = useState<[number, number][]>([]);

  const windowKey = snap?.windows[0]?.key ?? "5h";

  useEffect(() => {
    if (!snap || snap.windows.length === 0) {
      setTrend(null);
      setConsumption([]);
      return;
    }
    let alive = true;
    api
      .providersHistory(provider, windowKey, range)
      .then((pts) => alive && setTrend(pts))
      .catch(() => alive && setTrend([]));
    api
      .providersConsumption(provider, windowKey, 30)
      .then((c) => alive && setConsumption(c))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [provider, windowKey, range, snap?.windows.length, snap?.updatedAtMs]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  if (!snap) return null;
  const label = PROVIDER_LABELS[provider] ?? provider;

  return (
    <motion.div
      className="overlay-backdrop"
      variants={backdropVariants}
      initial="initial"
      animate="enter"
      exit="exit"
      onClick={onClose}
    >
      <motion.div
        className="panel overlay-card quota-detail"
        variants={dialogVariants}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="panel-title">
          <span>
            {label} · 额度详情
            {snap.planName ? <span className="muted"> · {snap.planName}</span> : null}
          </span>
          <span className="right">
            <button className="model-chip" onClick={onClose}>
              ✕
            </button>
          </span>
        </div>

        <div className="kv">
          <span className="k">状态</span>
          <span>{PROVIDER_STATUS_LABELS[snap.status] ?? snap.status}</span>
          <span className="k">账号</span>
          <span>{snap.account ?? "—"}</span>
          <span className="k">数据来源</span>
          <span title={snap.source}>{snap.source}</span>
          <span className="k">最近更新</span>
          <span>{formatDateTime(snap.updatedAtMs)}</span>
          {snap.sourceUrl && (
            <>
              <span className="k">官方文档</span>
              <span className="muted">{snap.sourceUrl}</span>
            </>
          )}
        </div>

        {snap.error && <div className="quota-error">{snap.error}</div>}
        {snap.notes.map((n, i) => (
          <div key={i} className="muted note-line">
            · {n}
          </div>
        ))}

        {snap.windows.length > 0 && (
          <>
            <div className="panel-title modal-section">
              官方额度
              <span className="right muted" style={{ fontSize: 10.5 }}>
                与本地用量独立
              </span>
            </div>
            {snap.windows.map((w) => (
              <QuotaRow key={w.key} w={w} />
            ))}
            <div className="trend-range">
              {(["today", "7d", "30d"] as const).map((r) => (
                <button
                  key={r}
                  className={`link-btn ${range === r ? "active" : ""}`}
                  onClick={() => setRange(r)}
                >
                  {r === "today" ? "今日" : r === "7d" ? "7 天" : "30 天"}
                </button>
              ))}
              <span className="muted" style={{ fontSize: 10.5 }}>
                变化趋势({windowKey})
              </span>
            </div>
            <Sparkline points={trend} />
          </>
        )}

        {snap.packages.length > 0 && (
          <>
            <div className="panel-title modal-section">Token 包({snap.packages.length})</div>
            <div className="pkg-list">
              {snap.packages.map((p) => (
                <PackageRow key={p.instanceNo} p={p} />
              ))}
            </div>
          </>
        )}

        {snap.localUsage && (snap.localUsage.allTime.totalTokens > 0 || snap.localUsage.today.totalTokens > 0) && (
          <>
            <div className="panel-title modal-section">
              本地 Harness 用量统计
              <span className="right muted" style={{ fontSize: 10.5 }}>
                来自本地 session 文件 ≠ 官方剩余额度
              </span>
            </div>
            <LocalUsageTable usage={snap.localUsage} />
          </>
        )}

        {consumption.length > 0 && (
          <>
            <div className="panel-title modal-section">每日消耗(近 30 天,估算)</div>
            <DailyConsumptionBars data={consumption} />
          </>
        )}
      </motion.div>
    </motion.div>
  );
}

function PackageRow({ p }: { p: PackageInfo }) {
  const pct = p.usagePercent;
  const daysLeft = p.expiryMs ? Math.floor((p.expiryMs - Date.now()) / 86_400_000) : null;
  const expiring = daysLeft !== null && daysLeft <= 7 && p.status === "Effective";
  return (
    <div className={`pkg-row ${p.status === "Effective" ? "" : "idle"}`}>
      <div style={{ minWidth: 0, flex: 1 }}>
        <div className="name" title={`${p.name} · ${p.configuration}`}>
          {p.name || p.configuration}
        </div>
        <div className="muted" style={{ fontSize: 10.5 }}>
          {p.status}
          {p.expiryMs ? ` · 到期 ${formatDateTime(p.expiryMs)}` : ""}
          {expiring ? ` · ${daysLeft} 天后到期` : ""}
        </div>
      </div>
      <div style={{ textAlign: "right", fontSize: 11 }}>
        <div>
          剩余 {formatQuotaAmount(p.availableAmount * p.unitMultiplier, "tokens")} /{" "}
          {formatQuotaAmount(p.totalAmount * p.unitMultiplier, "tokens")}
        </div>
        <div className="share-track" style={{ marginTop: 4 }}>
          <div
            className={`share-fill ${expiring ? "critical" : ""}`}
            style={{ width: `${pct === null ? 0 : Math.min(100, Math.max(0, pct)).toFixed(1)}%` }}
          />
        </div>
        <div className="muted">{pct === null ? "—" : `已用 ${pct.toFixed(0)}%`}</div>
      </div>
    </div>
  );
}

function LocalUsageTable({ usage }: { usage: NonNullable<ProviderSnapshot["localUsage"]> }) {
  const rows: [string, string, string][] = [
    ["Input", formatTokens(usage.today.inputTokens), formatTokens(usage.allTime.inputTokens)],
    [
      "Cached Input",
      formatTokens(usage.today.cachedInputTokens),
      formatTokens(usage.allTime.cachedInputTokens),
    ],
    [
      "Cache Write",
      formatTokens(usage.today.cacheWriteTokens),
      formatTokens(usage.allTime.cacheWriteTokens),
    ],
    ["Output", formatTokens(usage.today.outputTokens), formatTokens(usage.allTime.outputTokens)],
    [
      "Reasoning",
      formatTokens(usage.today.reasoningTokens),
      formatTokens(usage.allTime.reasoningTokens),
    ],
    ["Total", formatTokens(usage.today.totalTokens), formatTokens(usage.allTime.totalTokens)],
  ];
  return (
    <div className="local-usage">
      <div className="model-row model-head local-usage-head">
        <span>分项</span>
        <span style={{ textAlign: "right" }}>今日</span>
        <span style={{ textAlign: "right" }}>累计</span>
      </div>
      {rows.map(([k, today, all]) => (
        <div key={k} className="model-row local-usage-row">
          <span>{k}</span>
          <span className="num">{today}</span>
          <span className="num">{all}</span>
        </div>
      ))}
      <div className="model-row local-usage-row">
        <span className="muted">Sessions / 模型数</span>
        <span className="num">{usage.sessions}</span>
        <span className="num">{usage.models.length}</span>
      </div>
    </div>
  );
}

function Sparkline({ points }: { points: HistoryPointDto[] | null }) {
  const path = useMemo(() => {
    if (!points || points.length < 2) return null;
    const values = points.map((p) => p.usedPercent ?? p.remaining ?? null);
    const nums = values.filter((v): v is number => v !== null);
    if (nums.length < 2) return null;
    const min = Math.min(...nums);
    const max = Math.max(...nums);
    const span = max - min || 1;
    const W = 320;
    const H = 56;
    const step = W / (values.length - 1);
    let d = "";
    let x = 0;
    let started = false;
    for (const v of values) {
      if (v === null) {
        started = false;
        x += step;
        continue;
      }
      const y = H - ((v - min) / span) * (H - 6) - 3;
      d += `${started ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)} `;
      started = true;
      x += step;
    }
    return { d, min, max, H, W };
  }, [points]);

  if (!points || points.length < 2 || !path) {
    return <div className="muted" style={{ fontSize: 11, padding: "8px 0" }}>样本不足,暂无趋势(数据积累后自动出现)</div>;
  }
  return (
    <svg viewBox={`0 0 ${path.W} ${path.H}`} className="quota-spark" preserveAspectRatio="none">
      <path d={path.d} fill="none" stroke="var(--zup-blue-500, #4b8ac4)" strokeWidth="1.6" />
    </svg>
  );
}

function DailyConsumptionBars({ data }: { data: [number, number][] }) {
  const max = Math.max(...data.map(([, v]) => v), 0.0001);
  return (
    <div className="daily-bars">
      {data.map(([day, v]) => (
        <div
          key={day}
          className="bar"
          style={{ height: `${Math.max(2, (v / max) * 44)}px` }}
          title={`${formatDateTime(day * 86_400_000).slice(0, 10)}:${formatQuotaAmount(v)}`}
        />
      ))}
    </div>
  );
}
