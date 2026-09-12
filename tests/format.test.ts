import { describe, expect, it } from "vitest";
import {
  formatCny,
  formatPerM,
  formatPercent,
  formatRelative,
  formatTokens,
  formatUnitPerM,
  shortSessionId,
  truncateHead,
  formatLatency,
  formatTps,
} from "../src/lib/format";

describe("formatTokens", () => {
  it("uses compact units", () => {
    expect(formatTokens(950)).toBe("950");
    expect(formatTokens(1234)).toBe("1.2K");
    expect(formatTokens(4_200_000)).toBe("4.20M");
    expect(formatTokens(128_400_000)).toBe("128.4M");
    expect(formatTokens(1_020_000_000)).toBe("1.02B");
  });
  it("handles non-finite", () => {
    expect(formatTokens(NaN)).toBe("—");
  });
});

describe("formatPercent", () => {
  it("renders null as unavailable", () => {
    expect(formatPercent(null)).toBe("unavailable");
  });
  it("renders ratio as percent", () => {
    expect(formatPercent(0.8234)).toBe("82.3%");
    expect(formatPercent(1, 0)).toBe("100%");
  });
});

describe("formatRelative", () => {
  it("buckets elapsed time", () => {
    const now = 1_756_300_000_000;
    expect(formatRelative(now - 3_000, now)).toBe("刚刚");
    expect(formatRelative(now - 42_000, now)).toBe("42 秒前");
    expect(formatRelative(now - 5 * 60_000, now)).toBe("5 分钟前");
    expect(formatRelative(now - 3 * 3_600_000, now)).toBe("3 小时前");
    expect(formatRelative(now - 2 * 86_400_000, now)).toBe("2 天前");
    expect(formatRelative(null)).toBe("—");
  });
});

describe("shortSessionId", () => {
  it("keeps head and tail", () => {
    const id = "a4b3c2d1e0f4a5b6c7d8e9f0a1b2c3d4";
    const short = shortSessionId(id);
    expect(short.startsWith("a4b3c2d1")).toBe(true);
    expect(short.endsWith("c3d4")).toBe(true);
  });
  it("returns short ids unchanged", () => {
    expect(shortSessionId("abc")).toBe("abc");
  });
});

describe("truncateHead", () => {
  it("keeps short strings untouched", () => {
    expect(truncateHead("abc", 10)).toBe("abc");
    expect(truncateHead("", 10)).toBe("");
  });
  it("cuts to max-1 chars plus ellipsis", () => {
    expect(truncateHead("sess_6db49f3e-4cac-4298", 10)).toBe("sess_6db4…");
    expect(truncateHead("a".repeat(14), 14)).toBe("a".repeat(14));
    expect(truncateHead("a".repeat(15), 14)).toBe(`${"a".repeat(13)}…`);
  });
  it("counts unicode code points, not UTF-16 units", () => {
    // 中文:12 个码点内不截断;超过时保留 max-1=11 个码点 + "…"。
    expect(truncateHead("节点小宝远程屏幕控制连接失败排查", 12)).toBe("节点小宝远程屏幕控制连…");
    expect(truncateHead("优化登录性能", 12)).toBe("优化登录性能");
    // emoji(代理对)不会被切成乱码。
    expect(truncateHead("😀😃😄😁😆", 3)).toBe("😀😃…");
  });
  it("always keeps at least one visible character before the ellipsis", () => {
    expect(truncateHead("ab", 1)).toBe("a…");
    expect(truncateHead("ab", 2)).toBe("ab");
  });
});

describe("formatCny", () => {
  it("renders zero, tiny and normal amounts", () => {
    expect(formatCny(0)).toBe("¥0.00");
    expect(formatCny(0.001)).toBe("<¥0.01");
    expect(formatCny(12.345)).toBe("¥12.35");
    expect(formatCny(1234.5)).toBe("¥1,234.50");
  });

  it("never renders negative zero", () => {
    // JSON round-trip of a Rust f64 -0.0 arrives as JS -0.
    expect(formatCny(JSON.parse("-0.0"))).toBe("¥0.00");
    expect(formatCny(-0)).toBe("¥0.00");
  });
});

describe("formatPerM", () => {
  it("renders unit prices with 3 significant digits", () => {
    expect(formatPerM(null)).toBe("—");
    expect(formatPerM(0)).toBe("免费");
    expect(formatPerM(1.4)).toBe("1.4");
    expect(formatPerM(0.0032)).toBe("0.0032");
  });
});

describe("formatUnitPerM", () => {
  it("renders per-million unit with currency symbol", () => {
    expect(formatUnitPerM(null, "CNY")).toBe("—");
    expect(formatUnitPerM(3, "CNY")).toBe("¥3.00/M");
    expect(formatUnitPerM(1.4, "USD")).toBe("$1.40/M");
  });
});

describe("formatLatency", () => {
  it("renders sub-second latency in milliseconds", () => {
    expect(formatLatency(870)).toBe("870 毫秒");
    expect(formatLatency(0)).toBe("0 毫秒");
  });
  it("renders seconds with one decimal", () => {
    expect(formatLatency(1873)).toBe("1.9 秒");
    expect(formatLatency(2830)).toBe("2.8 秒");
  });
  it("degrades null and invalid values", () => {
    expect(formatLatency(null)).toBe("—");
    expect(formatLatency(Number.NaN)).toBe("—");
    expect(formatLatency(-5)).toBe("—");
  });
});

describe("formatTps", () => {
  it("rounds to integers from 10 tps up", () => {
    expect(formatTps(87)).toBe("87 tps");
    expect(formatTps(126.7)).toBe("127 tps");
    expect(formatTps(74.62)).toBe("75 tps");
  });
  it("keeps one decimal for single-digit speeds", () => {
    expect(formatTps(8.44)).toBe("8.4 tps");
  });
  it("degrades null and non-positive values", () => {
    expect(formatTps(null)).toBe("—");
    expect(formatTps(0)).toBe("—");
    expect(formatTps(Number.POSITIVE_INFINITY)).toBe("—");
  });
});
