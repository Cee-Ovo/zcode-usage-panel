import { describe, expect, it } from "vitest";
import { cacheHitRate, totalTokens, type Agg } from "../src/lib/types";
import { store } from "../src/lib/store";

function agg(partial: Partial<Agg>): Agg {
  return {
    requests: 0,
    input: 0,
    output: 0,
    reasoning: { sum: 0, present: 0 },
    cacheRead: { sum: 0, present: 0 },
    cacheWrite: { sum: 0, present: 0 },
    hitCached: 0,
    hitInputTotal: 0,
    firstTsMs: null,
    lastTsMs: null,
    ...partial,
  };
}

describe("totalTokens", () => {
  it("sums only provided fields", () => {
    const a = agg({ input: 100, output: 50 });
    expect(totalTokens(a)).toBe(150);
    const b = agg({
      input: 100,
      output: 50,
      reasoning: { sum: 25, present: 10 },
      cacheRead: { sum: 1_000, present: 10 },
      cacheWrite: { sum: 200, present: 10 },
    });
    expect(totalTokens(b)).toBe(1375);
  });
});

describe("cacheHitRate (统计口径)", () => {
  it("returns null when no cache data (unavailable)", () => {
    expect(cacheHitRate(agg({}))).toBeNull();
    expect(cacheHitRate(agg({ hitInputTotal: 0, hitCached: 0 }))).toBeNull();
  });
  it("computes cached/total from Rust-side classified sums", () => {
    // exclusive schema: 39000 / (1000+39000+5000)
    const a = agg({ hitCached: 39_000, hitInputTotal: 45_000 });
    expect(cacheHitRate(a)!).toBeCloseTo(39_000 / 45_000, 9);
    // inclusive schema: 800 / 900
    const b = agg({ hitCached: 800, hitInputTotal: 900 });
    expect(cacheHitRate(b)!).toBeCloseTo(800 / 900, 9);
  });
});

describe("store", () => {
  it("notifies subscribers and merges patches immutably", () => {
    const before = store.get();
    let calls = 0;
    const un = store.subscribe(() => calls++);
    store.set({ version: "1.2.3" });
    const after = store.get();
    expect(after.version).toBe("1.2.3");
    expect(after).not.toBe(before); // new top-level object…
    expect(after.page).toBe(before.page); // …with stable slice references
    expect(calls).toBe(1);
    un();
    store.set({ version: "1.0.0" });
    expect(calls).toBe(1); // unsubscribed
  });
});
