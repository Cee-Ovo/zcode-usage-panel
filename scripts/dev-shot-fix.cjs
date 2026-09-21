// 临时验收:当前 Session 卡(最近模型速度行) + Token 趋势图加深色(验收后可删)
const { chromium } = require('playwright-core');

(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const errors = [];
  const page = await browser.newPage({ viewport: { width: 980, height: 860 } });
  page.on('pageerror', (e) => errors.push('pageerror: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()); });

  await page.goto('http://localhost:5199/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(1200);

  // 会话卡文本内容(验证新行存在)
  const kvText = await page.evaluate(() => {
    const panels = [...document.querySelectorAll('.panel-title')];
    const p = panels.find((el) => el.textContent.includes('当前 Session'));
    if (!p) return null;
    const kv = p.parentElement.querySelector('.kv');
    return kv ? kv.innerText.replace(/\n/g, ' | ') : null;
  });
  console.log('SESSION KV:', kvText);

  // 当前 Session 卡特写
  const sessionPanel = page.locator('.panel-title', { hasText: '当前 Session' }).first();
  await page.locator('.panel', { has: sessionPanel }).first().screenshot({ path: 'output/playwright/fix-session-card.png' });

  // Token 趋势卡特写(悬停到柱子上更接近用户截图?不悬停保持默认)
  const trendPanel = page.locator('.panel-title', { hasText: 'Token 趋势' }).first();
  await page.locator('.panel', { has: trendPanel }).first().screenshot({ path: 'output/playwright/fix-trend-light.png' });

  // 深色趋势图
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'dark'));
  await page.waitForTimeout(400);
  await page.locator('.panel', { has: trendPanel }).first().screenshot({ path: 'output/playwright/fix-trend-dark.png' });

  // 整页仪表盘(浅色,回切)
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'));
  await page.waitForTimeout(400);
  await page.screenshot({ path: 'output/playwright/fix-dashboard-light.png' });

  if (errors.length) console.log('PAGE ERRORS:', JSON.stringify(errors));
  else console.log('no page errors');
  await browser.close();
})();
