// Follow-up shots: lower halves (inner scroll container) + detail modal + searches.
const { chromium } = require('playwright-core');
const fs = require('fs');

const OUT = 'output/playwright/multisource';
(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch({ channel: 'chrome', executablePath: '/usr/bin/google-chrome' });
  const page = await browser.newPage({ viewport: { width: 1180, height: 860 } });
  const errors = [];
  page.on('pageerror', e => errors.push('pageerror: ' + e.message));
  page.on('console', m => { if (m.type() === 'error') errors.push('console: ' + m.text()); });

  const scroller = () => page.locator('.zup-content').first();
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(900);

  // CC section lower half (model table + recent session + trend)
  await page.getByRole('button', { name: 'CC', exact: true }).click();
  await page.waitForTimeout(1000);
  await scroller().evaluate(el => (el.scrollTop = 620));
  await page.waitForTimeout(500);
  await page.screenshot({ path: `${OUT}/dash-cc-mid.png` });
  await scroller().evaluate(el => (el.scrollTop = 1240));
  await page.waitForTimeout(500);
  await page.screenshot({ path: `${OUT}/dash-cc-lower.png` });
  await scroller().evaluate(el => (el.scrollTop = 1860));
  await page.waitForTimeout(500);
  await page.screenshot({ path: `${OUT}/dash-cc-quota.png` });

  // horizontal scroll check at each stop
  for (const stop of [0, 620, 1240, 1860]) {
    await scroller().evaluate(el => (el.scrollTop = stop));
    await page.waitForTimeout(200);
    const overflow = await page.evaluate(() => {
      const el = document.querySelector('.zup-content');
      return el ? el.scrollWidth - el.clientWidth : 0;
    });
    if (overflow > 1) errors.push(`H-SCROLL in dash-cc @${stop}: +${overflow}px`);
  }

  // Sessions detail modal (cc- row)
  await page.getByRole('button', { name: 'Sessions' }).click();
  await page.waitForTimeout(1000);
  const ccRow = page.locator('.session-row:not(.table-head)', { hasText: 'cc-9f3a2b' }).first();
  await ccRow.click();
  await page.waitForTimeout(900);
  await page.screenshot({ path: `${OUT}/sessions-detail-cc.png` });
  await page.keyboard.press('Escape');
  await page.waitForTimeout(400);

  // search result counts per source
  const counts = {};
  for (const q of ['cc-', 'cx-', 'dsh-', 'zcode-usage-panel', 'claude-opus', '不存在的词']) {
    await page.getByPlaceholder(/搜索 session/).fill(q);
    await page.waitForTimeout(700);
    const label = await page.locator('.sessions-toolbar .muted', { hasText: 'sessions' }).first().textContent().catch(() => null);
    counts[q] = label?.trim() ?? '?';
  }
  console.log('search counts:', JSON.stringify(counts));
  await page.getByPlaceholder(/搜索 session/).fill('');
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${OUT}/sessions-final.png` });

  console.log('errors:', JSON.stringify(errors, null, 2));
  await browser.close();
})();
