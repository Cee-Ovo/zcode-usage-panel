/** dev-mock multi-source coverage: the browser mock must serve the same
 *  four-source Sessions page and per-source dashboard views the Tauri
 *  backend serves, so visual verification works without Tauri. */

import { beforeAll, describe, expect, it } from "vitest";

describe("devMock multi-source data", () => {
  let mockInvoke: typeof import("../src/lib/devMock").mockInvoke;
  let mockSessionsPage: typeof import("../src/lib/devMock").mockSessionsPage;
  let mockState: typeof import("../src/lib/devMock").mockState;

  beforeAll(async () => {
    // devMock touches window.location at module scope — jsdom provides it.
    const mod = await import("../src/lib/devMock");
    mockInvoke = mod.mockInvoke;
    mockSessionsPage = mod.mockSessionsPage;
    mockState = mod.mockState;
  });

  it("sessions list mixes four sources with the documented prefixes", () => {
    const all = mockState.sessions ?? [];
    expect(all.some((s) => s.id.startsWith("cc-"))).toBe(true);
    expect(all.some((s) => s.id.startsWith("cx-"))).toBe(true);
    expect(all.some((s) => s.id.startsWith("dsh-"))).toBe(true);
    expect(all.some((s) => !/^(cc|cx|dsh)-/.test(s.id))).toBe(true);
    // prefixed sessions carry real metadata (or honest nulls, never fake text)
    for (const s of all.filter((x) => /^(cc|cx|dsh)-/.test(x.id))) {
      expect(s.models.length).toBeGreaterThan(0);
      if (s.title !== null) expect(s.title.length).toBeGreaterThan(0);
    }
  });

  it("search hits every source by prefix, project and model", () => {
    expect(mockSessionsPage("cc-").total).toBe(2);
    expect(mockSessionsPage("cx-").total).toBe(2);
    expect(mockSessionsPage("dsh-").total).toBe(1);
    // project path spans sources
    expect(mockSessionsPage("zcode-usage-panel").total).toBeGreaterThanOrEqual(4);
    // model search matches a Claude Code model
    expect(mockSessionsPage("claude-opus").items[0].id.startsWith("cc-")).toBe(true);
    expect(mockSessionsPage("不存在的词").total).toBe(0);
  });

  it("get_sessions_page IPC serves the same merged list", async () => {
    const page = await mockInvoke("get_sessions_page", { query: "cc-", sort: "recent", page: 0, pageSize: 50 });
    expect(page.total).toBe(2);
    expect(page.items.every((s: { id: string }) => s.id.startsWith("cc-"))).toBe(true);
  });

  it("get_local_usage_view serves a ZCode-density view per local source", async () => {
    for (const provider of ["codex", "dsh", "claude-code"]) {
      const view = await mockInvoke("get_local_usage_view", { provider, rangeKey: "7d", includeTrend: true });
      // speed honestly unavailable
      expect(view.dash.speed.ttftAvgMs).toBeNull();
      expect(view.dash.speed.speedTps).toBeNull();
      expect(view.dash.models.length).toBeGreaterThan(0);
      expect(view.trend).not.toBeNull();
      expect(view.trend!.buckets.length).toBeGreaterThan(0);
      expect(view.costSummary.disclaimer).toContain("非实际 Billing");
      expect(view.dash.activeSession).not.toBeNull();
    }
    const noTrend = await mockInvoke("get_local_usage_view", { provider: "codex", rangeKey: "today", includeTrend: false });
    expect(noTrend.trend).toBeNull();
  });

  it("provider-scoped cost detail returns priced line items", async () => {
    const detail = await mockInvoke("cost_detail", {
      range: "7d",
      model: "claude-sonnet-5",
      provider: "claude-code",
    });
    expect(detail.priced).toBe(true);
    expect(detail.lines.length).toBeGreaterThan(0);
    expect(detail.notes[0]).toContain("估算");
  });

  it("session detail for a prefixed id returns buckets and models", async () => {
    const detail = await mockInvoke("get_session_detail", { sessionId: "cc-9f3a2b71-1111-2222-3333-444455556666" });
    expect(detail.summary.id).toBe("cc-9f3a2b71-1111-2222-3333-444455556666");
    expect(detail.summary.title).toContain("summary");
    expect(detail.buckets.length).toBe(8);
    expect(detail.models.length).toBeGreaterThan(0);
    expect(await mockInvoke("get_session_detail", { sessionId: "cc-missing" })).toBeNull();
  });

  it("providers overview includes the Claude Code card without fabricated quota", () => {
    const cc = (mockState.providers ?? []).find((p) => p.provider === "claude-code");
    expect(cc).toBeDefined();
    expect(cc!.windows).toHaveLength(0);
    expect(cc!.packages).toHaveLength(0);
    expect(cc!.localUsage).not.toBeNull();
  });
});
