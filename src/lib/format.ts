/** Number & time formatting helpers. */

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
