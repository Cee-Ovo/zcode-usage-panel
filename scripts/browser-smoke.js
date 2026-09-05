// Playwright CLI run-code input. Run against the DEV server only; fixtures are
// synthetic and are injected into the browser mock, never into user storage.
async (page) => {
  await page.goto("http://127.0.0.1:5173/");
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  if (await page.getByRole("button", { name: "显示详细指标", exact: true }).count()) {
    await page.getByRole("button", { name: "显示详细指标", exact: true }).click();
  }
  await page.getByRole("button", { name: "精简视图", exact: true }).click();
  check(await page.locator(".metrics-grid").first().getByText("Input Token", { exact: true }).count() === 0, "compact metrics");
  await page.getByRole("button", { name: "显示详细指标", exact: true }).click();

  await page.evaluate(async () => {
    const moduleUrl = performance.getEntriesByType("resource").find((r) => r.name.includes("/src/lib/devMock.ts")).name;
    const { mockState } = await import(moduleUrl);
    const base = mockState.sessions[0];
    mockState.sessions = Array.from({ length: 620 }, (_, i) => ({
      ...structuredClone(base), id: `smoke-session-${String(i).padStart(4, "0")}`,
      project: i === 619 ? "historic-only" : "synthetic-project",
      agg: { ...base.agg, input: i + 1, lastTsMs: Date.now() - i * 1000 },
    }));
  });
  await page.getByRole("button", { name: "Sessions", exact: true }).click();
  await page.getByText("620 sessions", { exact: true }).waitFor();
  const search = page.getByRole("searchbox", { name: "搜索 session / 项目 / 模型" });
  await search.fill("historic-only");
  await page.getByText("1 sessions", { exact: true }).waitFor();
  check(await page.getByRole("button", { name: "查看 Session smoke-session-0619", exact: true }).count() === 1, "search beyond 500");
  await search.fill("");
  await page.getByText("620 sessions", { exact: true }).waitFor();
  await page.getByRole("combobox", { name: "每页 Session 数" }).selectOption("25");
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await page.getByText(/第 2 \/ 25 页/).waitFor();
  await page.waitForFunction(() => document.querySelectorAll('.session-row[role="button"]').length === 25);
  check(await page.locator(".session-row[role=button]").count() === 25, "page size");
  const row = page.locator(".session-row[role=button]").first();
  await row.focus();
  await page.keyboard.press("Enter");
  await page.getByRole("dialog", { name: "Session 详情", exact: true }).waitFor();
  await page.keyboard.press("Shift+Tab");
  check(await page.evaluate(() => !!document.activeElement.closest('[role="dialog"]')), "focus trap");
  await page.keyboard.press("Escape");
  await page.getByRole("dialog").waitFor({ state: "detached" });
  check(await row.evaluate((el) => document.activeElement === el), "restore focus");

  await page.getByRole("button", { name: "模型", exact: true }).click();
  await page.locator(".model-row[role=button]").first().click();
  await page.getByRole("dialog").waitFor();
  await page.keyboard.press("Escape");
  await page.getByRole("dialog").waitFor({ state: "detached" });
  await page.getByRole("button", { name: "仪表盘", exact: true }).click();
  await page.locator('[aria-label$="成本明细"]').first().click();
  await page.getByRole("dialog").waitFor();
  await page.keyboard.press("Escape");
  await page.getByRole("dialog").waitFor({ state: "detached" });

  // Introduce deterministic latency/error in the browser-only IPC adapter.
  await page.evaluate(async () => {
    const { api } = await import(performance.getEntriesByType("resource").find((r) => r.name.includes("/src/lib/ipc.ts")).name);
    window.__smokeOriginal = api.usageView;
    window.__smokeFlight = 0;
    window.__smokeMaxFlight = 0;
    api.usageView = async (key, trend) => {
      window.__smokeMaxFlight = Math.max(window.__smokeMaxFlight, ++window.__smokeFlight);
      try {
        await new Promise((resolve) => setTimeout(resolve, key === "7d" ? 300 : 20));
        return await window.__smokeOriginal(key, trend);
      } finally { window.__smokeFlight--; }
    };
  });
  await page.getByRole("button", { name: "7 天", exact: true }).first().click();
  await page.getByRole("button", { name: "30 天", exact: true }).first().click();
  await page.waitForFunction(async () => {
    const { store } = await import(performance.getEntriesByType("resource").find((r) => r.name.includes("/src/lib/store.ts")).name);
    return store.get().dash?.rangeKey === "30d" && !store.get().refresh.loading;
  });
  check(await page.evaluate(() => window.__smokeMaxFlight) === 1, "single-flight range switch");
  await page.evaluate(async () => {
    const { api } = await import(performance.getEntriesByType("resource").find((r) => r.name.includes("/src/lib/ipc.ts")).name);
    api.usageView = async () => { throw new Error("synthetic-private-error"); };
  });
  await page.getByRole("button", { name: "全部", exact: true }).first().click();
  await page.getByRole("alert").first().waitFor();
  check(!(await page.locator("body").innerText()).includes("synthetic-private-error"), "redacted query error");
  await page.evaluate(async () => {
    const { api } = await import(performance.getEntriesByType("resource").find((r) => r.name.includes("/src/lib/ipc.ts")).name);
    api.usageView = window.__smokeOriginal;
  });
  await page.getByRole("button", { name: "重试", exact: true }).click();
  await page.waitForFunction(async () => {
    const { store } = await import(performance.getEntriesByType("resource").find((r) => r.name.includes("/src/lib/store.ts")).name);
    return store.get().dash?.rangeKey === "all" && !store.get().refresh.loading && !store.get().refresh.error;
  });
  check(errors.length === 0, `browser errors: ${errors.join("; ")}`);
  return "PASS: compact, full-history search, paging, keyboard dialogs, focus restore, range race, single-flight, redacted error and retry";
}
