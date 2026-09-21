// 临时验收脚本:纸感主题四页 × 明暗 截图(验收后删除)
const { chromium } = require('playwright-core');

(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const errors = [];
  const page = await browser.newPage({ viewport: { width: 980, height: 760 } });
  page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()); });

  await page.goto('http://localhost:5199/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(1200);

  // 背景诊断:确认点阵与纸面生效
  const probe = async () =>
    page.evaluate(() => {
      const shell = document.querySelector('.zup-shell');
      const card = document.querySelector('.metric-card');
      const nav = document.querySelector('.zup-nav-item.active');
      const s = shell ? getComputedStyle(shell) : null;
      const c = card ? getComputedStyle(card) : null;
      const n = nav ? getComputedStyle(nav) : null;
      return {
        shellBg: s?.backgroundColor,
        shellBefore: s?.content === 'none' ? 'no-before' : 'has-before',
        cardBg: c?.backgroundColor,
        cardBlur: c?.backdropFilter,
        navActiveBg: n?.backgroundColor,
        navActiveColor: n?.color,
      };
    });

  console.log('LIGHT probe:', JSON.stringify(await probe()));
  await page.screenshot({ path: 'output/playwright/paper-dashboard-light.png' });

  // 深色
  await page.evaluate(() => {
    document.documentElement.setAttribute('data-theme', 'dark');
    localStorage.setItem('zup.theme', 'dark');
  });
  await page.waitForTimeout(400);
  console.log('DARK probe:', JSON.stringify(await probe()));
  await page.screenshot({ path: 'output/playwright/paper-dashboard-dark.png' });

  // Sessions
  await page.getByRole('button', { name: 'Sessions', exact: true }).click();
  await page.waitForTimeout(800);
  await page.screenshot({ path: 'output/playwright/paper-sessions-dark.png' });

  // Models + 详情弹窗
  await page.getByRole('button', { name: '模型', exact: true }).click();
  await page.waitForTimeout(900);
  await page.screenshot({ path: 'output/playwright/paper-models-dark.png' });
  const hasCodex = await page.locator('.models-surface .model-row', { hasText: '（Codex）' }).count();
  if (hasCodex > 0) {
    await page.locator('.models-surface .model-row', { hasText: '（Codex）' }).first().click();
    await page.waitForTimeout(800);
    await page.screenshot({ path: 'output/playwright/paper-models-detail-dark.png' });
    await page.keyboard.press('Escape');
    await page.waitForTimeout(400);
  }

  // Settings
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.waitForTimeout(900);
  await page.screenshot({ path: 'output/playwright/paper-settings-dark.png' });

  // 回到浅色补一张设置页
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'));
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'output/playwright/paper-settings-light.png' });

  if (errors.length) console.log('PAGE ERRORS:', JSON.stringify(errors, null, 1));
  else console.log('no page errors');
  await browser.close();
})();
