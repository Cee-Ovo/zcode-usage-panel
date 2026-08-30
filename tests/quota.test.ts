/** Tests for the quota-dashboard format helpers and derived display logic. */

import { describe, expect, it } from "vitest";
import {
  formatCountdown,
  formatDuration,
  formatQuotaAmount,
} from "../src/lib/format";
import {
  PROVIDER_LABELS,
  PROVIDER_STATUS_LABELS,
  type ProviderSnapshot,
  type QuotaWindow,
} from "../src/lib/types";

describe("formatDuration", () => {
  it("renders minutes / hours / days", () => {
    expect(formatDuration(30_000)).toBe("1 分钟");
    expect(formatDuration(5 * 60_000)).toBe("5 分钟");
    expect(formatDuration(3 * 3_600_000)).toBe("3 小时");
    expect(formatDuration(3 * 3_600_000 + 30 * 60_000)).toBe("3 小时 30 分钟");
    expect(formatDuration(3 * 86_400_000 + 7 * 3_600_000)).toBe("3 天 7 小时");
    expect(formatDuration(2 * 86_400_000)).toBe("2 天");
  });

  it("degrades on invalid input", () => {
    expect(formatDuration(0)).toBe("—");
    expect(formatDuration(-5)).toBe("—");
    expect(formatDuration(NaN)).toBe("—");
  });
});

describe("formatQuotaAmount", () => {
  it("uses compact tokens form for token units", () => {
    expect(formatQuotaAmount(72_300_000, "tokens")).toBe("72.3M");
    expect(formatQuotaAmount(1500, "tokens")).toBe("1.5K");
  });

  it("keeps precision for non-token units and null", () => {
    expect(formatQuotaAmount(null)).toBe("—");
    expect(formatQuotaAmount(360.5)).toBe("360.5");
    expect(formatQuotaAmount(12_345)).toBe("12,345");
  });
});

describe("formatCountdown", () => {
  it("counts down to a future reset", () => {
    const now = Date.now();
    expect(formatCountdown(now + 2 * 3_600_000, now)).toBe("还剩 2:00");
    expect(formatCountdown(now + 3 * 86_400_000, now)).toBe("还剩 3 天 0 时");
    expect(formatCountdown(now - 1000, now)).toBe("已到期/已重置");
    expect(formatCountdown(null, now)).toBe("—");
  });
});

describe("quota display model", () => {
  it("labels every known provider and status", () => {
    for (const id of ["zcode", "codex", "antigravity", "volcengine"]) {
      expect(PROVIDER_LABELS[id]).toBeTruthy();
    }
    const statuses = ["ok", "not_configured", "not_installed", "disabled", "stale", "error"] as const;
    for (const s of statuses) {
      expect(PROVIDER_STATUS_LABELS[s]).toBeTruthy();
    }
  });

  it("official quota and local usage stay separable in the DTO shape", () => {
    const w: QuotaWindow = {
      key: "5h",
      label: "5 小时窗口",
      usedPercent: 72,
      totalQuota: null,
      usedQuota: null,
      remainingQuota: null,
      unit: "% 套餐额度",
      resetAtMs: Date.now() + 3 * 3_600_000,
      windowMinutes: 300,
      forecast: null,
    };
    const snap: ProviderSnapshot = {
      provider: "codex",
      status: "ok",
      account: null,
      planName: "ChatGPT plus",
      windows: [w],
      packages: [],
      localUsage: {
        today: {
          requests: 10,
          inputTokens: 1000,
          cachedInputTokens: 800,
          cacheWriteTokens: 0,
          outputTokens: 200,
          reasoningTokens: 50,
          totalTokens: 1250,
        },
        last7d: {
          requests: 0,
          inputTokens: 0,
          cachedInputTokens: 0,
          cacheWriteTokens: 0,
          outputTokens: 0,
          reasoningTokens: 0,
          totalTokens: 0,
        },
        allTime: {
          requests: 10,
          inputTokens: 1000,
          cachedInputTokens: 800,
          cacheWriteTokens: 0,
          outputTokens: 200,
          reasoningTokens: 50,
          totalTokens: 1250,
        },
        sessions: 1,
        models: [],
      },
      launcher: null,
      source: "test",
      sourceUrl: null,
      notes: [],
      error: null,
      updatedAtMs: Date.now(),
      nextPollMs: 0,
    };
    expect(snap.windows[0].usedPercent).toBe(72);
    expect(snap.localUsage?.allTime.totalTokens).not.toBe(snap.windows[0].usedPercent);
    // the two numbers describe different things and must never be derived
    // from one another
    expect(typeof snap.localUsage?.today.inputTokens).toBe("number");
    expect(snap.windows[0].totalQuota).toBeNull();
  });
});
