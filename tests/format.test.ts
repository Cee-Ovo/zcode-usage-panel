import { describe, expect, it } from "vitest";
import {
  formatCny,
  formatPerM,
  formatPercent,
  formatRelative,
  formatTokens,
  formatUnitPerM,
  shortSessionId,
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

describe("formatCny", () => {
  it("renders zero, tiny and normal amounts", () => {
    expect(formatCny(0)).toBe("¥0.00");
    expect(formatCny(0.001)).toBe("<¥0.01");
    expect(formatCny(12.345)).toBe("¥12.35");
    expect(formatCny(1234.5)).toBe("¥1,234.50");
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
