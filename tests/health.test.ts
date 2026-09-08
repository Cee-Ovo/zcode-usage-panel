import { describe, expect, it } from "vitest";
import {
  HealthTracker,
  QUERY_ERROR_VISIBILITY,
  STALE_ERROR_AFTER_MS,
} from "../src/lib/health";
import type { UsageUpdateEvent } from "../src/lib/types";

const T0 = 1_800_000_000_000;

function engineEvent(patch: Partial<UsageUpdateEvent> = {}): UsageUpdateEvent {
  return {
    recordCount: 10,
    lastRefreshMs: T0,
    lastRecordMs: T0,
    error: null,
    errorStreak: 0,
    paused: false,
    suspended: false,
    restoredFromCache: false,
    ...patch,
  };
}

function queryState(patch: { error?: string | null; lastSuccessMs?: number | null } = {}) {
  return { loading: false, error: patch.error ?? null, lastSuccessMs: patch.lastSuccessMs ?? null };
}

/** One failed request as the coordinator actually publishes it:
 * start clears the error, the catch sets it, the finally republishes it. */
function failRequest(t: HealthTracker, lastSuccessMs: number | null = T0) {
  t.onQueryState(queryState({ lastSuccessMs }));
  t.onQueryState(queryState({ error: "刷新失败，请稍后重试", lastSuccessMs }));
  t.onQueryState(queryState({ error: "刷新失败，请稍后重试", lastSuccessMs }));
}

/** One successful request (new success timestamp resets the error count). */
function succeedRequest(t: HealthTracker, at: number) {
  t.onQueryState(queryState({ lastSuccessMs: at }));
}

describe("HealthTracker hysteresis", () => {
  it("stays green through a single transient query failure after a recent success", () => {
    const t = new HealthTracker();
    succeedRequest(t, T0);
    expect(t.derive(T0 + 1_000).level).toBe("ok");
    failRequest(t);
    const view = t.derive(T0 + 2_000);
    expect(view.level).toBe("ok");
    expect(view.statusText).toBe("监控中");
    // The blip is still recorded honestly for 运行详情.
    expect(t.transientDetail()).toContain("1 次查询失败");
  });

  it("escalates to error after repeated query failures and recovers on success", () => {
    const t = new HealthTracker();
    succeedRequest(t, T0);
    for (let i = 0; i < QUERY_ERROR_VISIBILITY; i++) {
      failRequest(t);
    }
    const bad = t.derive(T0 + 5_000);
    expect(bad.level).toBe("error");
    expect(bad.statusText).toBe("刷新异常");
    expect(bad.detail).toContain(`${QUERY_ERROR_VISIBILITY} 次`);
    // Any success recovers immediately.
    succeedRequest(t, T0 + 6_000);
    expect(t.derive(T0 + 6_001).level).toBe("ok");
    expect(t.transientDetail()).toBeNull();
  });

  it("treats a lone query error as persistent once refreshes have been stale", () => {
    const t = new HealthTracker();
    succeedRequest(t, T0);
    failRequest(t);
    expect(t.derive(T0 + STALE_ERROR_AFTER_MS - 1).level).toBe("ok");
    expect(t.derive(T0 + STALE_ERROR_AFTER_MS + 1).level).toBe("error");
  });

  it("surfaces backend streak-gated engine errors with cycle detail", () => {
    const t = new HealthTracker();
    t.onEngineEvent(engineEvent({ error: "data directory not found", errorStreak: 2 }));
    const view = t.derive(T0);
    expect(view.level).toBe("error");
    expect(view.detail).toContain("连续 2 个刷新周期失败");
  });

  it("ignores engine raw errors that the backend has not escalated", () => {
    const t = new HealthTracker();
    // error null because the backend gates streak-1 failures away; the old
    // event shape (no errorStreak) also degrades gracefully.
    t.onEngineEvent(engineEvent({ error: null, errorStreak: 1 }));
    expect(t.derive(T0).level).toBe("ok");
  });

  it("paused and suspended win over errors for the visible label", () => {
    const t = new HealthTracker();
    t.onEngineEvent(engineEvent({ paused: true, error: "x", errorStreak: 3 }));
    expect(t.derive(T0).level).toBe("paused");
    expect(t.derive(T0).statusText).toBe("已暂停");

    const s = new HealthTracker();
    s.onEngineEvent(engineEvent({ suspended: true }));
    expect(s.derive(T0).level).toBe("suspended");
    expect(s.derive(T0).statusText).toBe("已挂起");
  });

  it("initialization failure is deterministic and shows immediately", () => {
    const t = new HealthTracker();
    t.onInitialization("初始化失败，请重试");
    const view = t.derive(T0);
    expect(view.level).toBe("error");
    expect(view.statusText).toBe("初始化失败");
    t.onInitialization(null);
    expect(t.derive(T0).level).toBe("ok");
  });

  it("engine success clears a previous engine error", () => {
    const t = new HealthTracker();
    t.onEngineEvent(engineEvent({ error: "boom", errorStreak: 2 }));
    expect(t.derive(T0).level).toBe("error");
    t.onEngineEvent(engineEvent({ errorStreak: 0 }));
    expect(t.derive(T0 + 1).level).toBe("ok");
  });
});
