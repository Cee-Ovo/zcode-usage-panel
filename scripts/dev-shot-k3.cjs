// 临时验收:K3 趋势图(按模型/按类型) + 模型排行渐变条(验收后可删)
const { chromium } = require('playwright-core');

(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const errors = [];
  const page = await browser.newPage({ viewport: { width: 980, height: 900 } });
  page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()); });

  await page.goto('http://localhost:5199/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(1200);

  const shot = async (name) => {
    const trend = page.locator('.panel-title', { hasText: 'Token 趋势' }).first();
    await page.locator('.panel', { has: trend }).first().screenshot({ path: `output/playwright/k3-trend-${name}.png` });
    const rank = page.locator('.panel-title', { hasText: '模型排行' }).first();
    await page.locator('.panel', { has: rank }).first().screenshot({ path: `output/playwright/k3-rank-${name}.png` });
  };

  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'));
  await page.waitForTimeout(300);
  await page.screenshot({ path: 'output/playwright/k3-dashboard-light.png' });
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  await page.waitForTimeout(300);
  await page.screenshot({ path: 'output/playwright/k3-dashboard-dark.png' });

  // 悬停柱子看 tooltip
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'));
  await page.waitForTimeout(300);
  const bars = page.locator('svg g[style*="crosshair"]').nth(6);
  if (await bars.count()) await bars.hover();
  await page.waitForTimeout(400);
  const trendTitle = page.locator('.panel-title', { hasText: 'Token 趋势' }).first();
  await page.locator('.panel', { has: trendTitle }).first().screenshot({ path: 'output/playwright/k3-trend-hover.png' });

  if (errors.length) console.log('PAGE ERRORS:', JSON.stringify(errors));
  else console.log('no page errors');
  await browser.close();
})();
