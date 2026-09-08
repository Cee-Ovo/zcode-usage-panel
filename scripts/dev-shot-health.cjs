// Captures the anti-flicker status states: transient failure (stays green),
// persistent failure (red dot + top pill agree), recovery.
const { chromium } = require('playwright-core');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', executablePath: '/usr/bin/google-chrome' });
  const page = await browser.newPage({ viewport: { width: 1180, height: 860 } });
  const errors = [];
  const check = (ok, msg) => { if (!ok) throw new Error(msg); };
  page.on('pageerror', e => errors.push(e.message));
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(900);
  const inject = (fail, key) => page.evaluate(async ({ fail, key }) => {
    const { store } = await import(performance.getEntriesByType('resource').find(r => r.name.includes('/src/lib/store.ts')).name);
    const { api } = await import(performance.getEntriesByType('resource').find(r => r.name.includes('/src/lib/ipc.ts')).name);
    if (!window.__origUsage) window.__origUsage = api.usageView;
    api.usageView = fail ? async () => { throw new Error('synthetic'); } : window.__origUsage;
    store.set({ rangeKey: key });
  }, { fail, key });
  const dotClass = () => page.evaluate(() => document.querySelector('.sidebar-status .status-dot')?.className ?? '');
  const pillVisible = () => page.evaluate(() => [...document.querySelectorAll('.dashboard-toolbar .badge-note')].some(b => b.textContent?.includes('数据源异常')));

  await inject(true, '7d');
  await page.waitForTimeout(700);
  check((await dotClass()).includes('live'), 'transient failure must keep green dot');
  check(!(await pillVisible()), 'transient failure must not show pill');
  await page.screenshot({ path: 'output/playwright/health-transient-green.png' });

  await inject(true, '30d');
  await page.getByText('刷新异常', { exact: true }).waitFor();
  check((await dotClass()).includes('error'), 'persistent failure must turn dot red');
  check(await pillVisible(), 'persistent failure must show top pill');
  await page.screenshot({ path: 'output/playwright/health-persistent-red.png' });

  await inject(false, 'all');
  await page.getByText('监控中', { exact: true }).waitFor();
  check((await dotClass()).includes('live'), 'recovery must return green');
  console.log('health states verified; errors:', JSON.stringify(errors));
  await browser.close();
})();
