// 模型页截图验证:四源合并 + 每行来源徽标不被截断;默认窗口尺寸下无横向滚动。
// 用法: node scripts/dev-shot-models.cjs  (需 dev server 已在 localhost:5173)
const { chromium } = require('playwright-core');

(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const page = await browser.newPage({ viewport: { width: 980, height: 700 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });

  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.getByRole('button', { name: '模型', exact: true }).click();
  await page.getByRole('heading', { name: '模型', exact: true }).waitFor();
  await page.waitForTimeout(900);

  const overflow = await page.evaluate(() => {
    const doc = document.documentElement;
    const panel = document.querySelector('.models-surface');
    return {
      docHOverflow: doc.scrollWidth > doc.clientWidth,
      panelHOverflow: panel ? panel.scrollWidth > panel.clientWidth : null,
    };
  });
  console.log('overflow:', JSON.stringify(overflow));

  // 每行的来源徽标:必须渲染出来且没有被裁掉(宽度 > 0 且未被 ellipsis 吃掉)
  const rows = await page.evaluate(() => {
    const out = [];
    document.querySelectorAll('.models-surface .model-row').forEach((row) => {
      const badge = row.querySelector('.model-source-badge');
      const text = row.querySelector('.model-name-text');
      const clippedNums = [...row.querySelectorAll('.num')]
        .filter((n) => n.scrollWidth > n.clientWidth + 1)
        .map((n) => n.textContent);
      out.push({
        name: text ? text.textContent : null,
        nameClipped: text ? text.scrollWidth > text.clientWidth + 1 : null,
        badge: badge ? badge.textContent : null,
        badgeWidth: badge ? Math.round(badge.getBoundingClientRect().width) : 0,
        clippedNums,
      });
    });
    return out;
  });
  console.log('rows:', JSON.stringify(rows, null, 1));

  await page.screenshot({ path: 'output/playwright/models-980-light.png' });

  // 详情弹窗必须按来源取数:点 Codex 行,标题带（Codex）徽标。
  await page.locator('.models-surface .model-row', { hasText: '（Codex）' }).first().click();
  await page.waitForTimeout(800);
  const dialogTitle = await page.locator('.panel-title', { hasText: '模型详情' }).first().textContent();
  console.log('dialogTitle:', JSON.stringify(dialogTitle));
  await page.screenshot({ path: 'output/playwright/models-detail-codex.png' });

  if (errors.length) console.log('page errors:', JSON.stringify(errors));
  await browser.close();
})();
