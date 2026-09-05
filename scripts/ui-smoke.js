// Visual-layout checks against the DEV synthetic data only.
async (page) => {
  const results = [];
  const failures = [];
  await page.goto("http://127.0.0.1:5173/");
  await page.getByRole("heading", { name: "用量概览" }).waitFor();
  for (const theme of ["light", "dark"]) {
    await page.evaluate(async (theme) => {
      const url = performance.getEntriesByType("resource").find((r) => r.name.includes("/src/lib/store.ts")).name;
      const { store } = await import(url);
      store.set({ settings: { ...store.get().settings, theme } });
    }, theme);
    for (const width of [1280, 980, 600, 400]) {
      await page.setViewportSize({ width, height: 800 });
      for (const name of ["仪表盘", "Sessions", "模型", "设置"]) {
        await page.getByRole("navigation").getByRole("button", { name, exact: true }).click();
        await page.waitForFunction((label) => {
          const marker = { "仪表盘": ".dashboard-page", "Sessions": "[aria-label='Session 排序']", "模型": ".model-row", "设置": "#sec-general" }[label];
          return !!document.querySelector(marker);
        }, name);
        // Wait until page entrance transitions settle before measuring.
        await page.waitForFunction(() => {
          const el = document.querySelector(".motion-page");
          return el && Number(getComputedStyle(el).opacity) > .99;
        });
        const measurement = await page.evaluate(() => {
          const el = document.querySelector(".zup-content");
          return { width: el.clientWidth, scroll: el.scrollWidth, body: document.body.scrollWidth };
        });
        const key = `${theme}/${width}/${name}`;
        results.push({ key, ...measurement });
        if (measurement.scroll > measurement.width + 2 || measurement.body > width + 2) failures.push(key);
      }
    }
  }
  if (failures.length) throw new Error(`Horizontal page overflow: ${failures.join(", ")} / ${JSON.stringify(results)}`);
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.getByRole("button", { name: "仪表盘", exact: true }).click();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.getByRole("button", { name: "精简视图", exact: true }).click();
  await page.getByRole("button", { name: "显示详细指标", exact: true }).waitFor();
  await page.getByRole("button", { name: "显示详细指标", exact: true }).click();
  await page.waitForFunction(() => [...document.querySelectorAll('.dashboard-metrics .metric-card')].every((el) => Number(getComputedStyle(el).opacity) > .99));
  await page.screenshot({ path: "output/playwright/refined-dark.png" });
  await page.evaluate(async () => {
    const url = performance.getEntriesByType("resource").find((r) => r.name.includes("/src/lib/store.ts")).name;
    const { store } = await import(url);
    store.set({ settings: { ...store.get().settings, theme: "light" } });
  });
  await page.waitForFunction(() => document.documentElement.dataset.theme === "light");
  await page.screenshot({ path: "output/playwright/refined-light.png" });
  return { result: "PASS: light/dark, 4 viewport sizes, all pages, reduced motion", results };
}
