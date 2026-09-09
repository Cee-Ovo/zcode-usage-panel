/** Number & time formatting helpers. */

import type { SpeedStats } from "./types";

export function formatTokens(n: number): string {
  if (!isFinite(n)) return "—";
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 1 : 2)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e5 ? 0 : 1)}K`;
  return String(Math.round(n));
}

export function formatFull(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

export function formatPercent(v: number | null, digits = 1): string {
  if (v === null) return "unavailable";
  return `${(v * 100).toFixed(digits)}%`;
}

export function formatRate(v: number): string {
  return `${formatTokens(v)}/min`;
}

/** Latency in ms → "2.8 秒" / "870 毫秒"; null → "—". */
export function formatLatency(ms: number | null): string {
  if (ms === null || !isFinite(ms) || ms < 0) return "—";
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)} 秒`;
  return `${Math.round(ms)} 毫秒`;
}

/** Tokens per second → "87 tok/s"; null → "—". */
export function formatTps(v: number | null): string {
  if (v === null || !isFinite(v) || v <= 0) return "—";
  if (v >= 10) return `${Math.round(v)} tok/s`;
  return `${v.toFixed(1)} tok/s`;
}

/** 模型页响应速度单元格:"2.8 秒 · 87 tok/s"(与仪表盘速度卡同格式);
 *  任一子项缺样本就省略,全缺为 "—"。 */
export function formatModelSpeed(speed: SpeedStats | null | undefined): string {
  if (!speed) return "—";
  const ttft = speed.ttftAvgMs !== null ? formatLatency(speed.ttftAvgMs) : null;
  const tps = speed.speedTps !== null ? formatTps(speed.speedTps) : null;
  const parts = [ttft, tps].filter((p): p is string => p !== null);
  return parts.length > 0 ? parts.join(" · ") : "—";
}

/** 响应速度单元格的悬停补充:P95 首字 / P50 TPS / 样本覆盖。 */
export function formatModelSpeedHint(speed: SpeedStats | null | undefined): string {
  if (!speed) return "暂无速度样本";
  const bits: string[] = [];
  if (speed.ttftP95Ms !== null) bits.push(`首字 P95 ${formatLatency(speed.ttftP95Ms)}`);
  if (speed.speedP50Tps !== null) bits.push(`TPS P50 ${formatTps(speed.speedP50Tps)}`);
  bits.push(`样本 ${speed.speedSamples}/${speed.completedRequests} 条请求`);
  return bits.join(" · ");
}

export function formatClock(ms: number | null): string {
  if (ms === null) return "—";
  const d = new Date(ms);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function formatDateTime(ms: number | null): string {
  if (ms === null) return "—";
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function formatRelative(ms: number | null, now = Date.now()): string {
  if (ms === null) return "—";
  const diff = Math.max(0, now - ms);
  if (diff < 10_000) return "刚刚";
  if (diff < 60_000) return `${Math.floor(diff / 1000)} 秒前`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  return `${Math.floor(diff / 86_400_000)} 天前`;
}

export function formatBucketLabel(startMs: number, rangeKey: string): string {
  const d = new Date(startMs);
  if (rangeKey === "60m") return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
  if (rangeKey === "7d")
    return `${pad(d.getMonth() + 1)}/${pad(d.getDate())} ${pad(d.getHours())}时`;
  if (rangeKey === "30d" || rangeKey === "all")
    return `${pad(d.getMonth() + 1)}/${pad(d.getDate())}`;
  return `${pad(d.getHours())}:00`;
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

/** Shortened session id: keeps head+tail recognizable. */
export function shortSessionId(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 8)}…${id.slice(-4)}`;
}

/** CNY cost: ¥ + thousand separators + 2 decimals; 0 → ¥0.00; tiny (>0, <0.01) → <¥0.01. */
export function formatCny(n: number): string {
  if (!isFinite(n)) return "¥0.00";
  if (n > 0 && n < 0.01) return "<¥0.01";
  return `¥${n.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

/** Per-million unit price for the pricing table: 3 significant digits; null → —; 0 → 免费. */
export function formatPerM(v: number | null | undefined): string {
  if (v === null || v === undefined) return "—";
  if (v === 0) return "免费";
  return String(parseFloat(v.toPrecision(3)));
}

/** Per-million unit price in a cost-detail line, e.g. $1.40/M or ¥3.00/M. */
export function formatUnitPerM(v: number | null, currency: string): string {
  if (v === null) return "—";
  const sym = currency === "USD" ? "$" : "¥";
  return `${sym}${v.toFixed(2)}/M`;
}

/** Human duration: "3 天 7 小时" / "5 小时" / "42 分钟". */
export function formatDuration(ms: number): string {
  if (!isFinite(ms) || ms <= 0) return "—";
  const mins = Math.round(ms / 60_000);
  if (mins < 60) return `${Math.max(1, mins)} 分钟`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) {
    const m = mins % 60;
    return m > 0 ? `${hours} 小时 ${m} 分钟` : `${hours} 小时`;
  }
  const days = Math.floor(hours / 24);
  const h = hours % 24;
  return h > 0 ? `${days} 天 ${h} 小时` : `${days} 天`;
}

/** Quota amount: tokens get the compact K/M/B form, others keep precision. */
export function formatQuotaAmount(v: number | null, unit?: string | null): string {
  if (v === null || !isFinite(v)) return "—";
  if (unit && unit.toLowerCase().includes("token")) return formatTokens(v);
  if (Math.abs(v) >= 1000) return formatFull(v);
  return String(parseFloat(v.toPrecision(4)));
}

/** Countdown to a future timestamp, e.g. "还剩 2 小时 05 分". */
export function formatCountdown(targetMs: number | null, now = Date.now()): string {
  if (targetMs === null) return "—";
  const diff = targetMs - now;
  if (diff <= 0) return "已到期/已重置";
  const h = Math.floor(diff / 3_600_000);
  const m = Math.floor((diff % 3_600_000) / 60_000);
  if (h >= 24) {
    const d = Math.floor(h / 24);
    return `还剩 ${d} 天 ${h % 24} 时`;
  }
  return `还剩 ${h}:${String(m).padStart(2, "0")}`;
}
