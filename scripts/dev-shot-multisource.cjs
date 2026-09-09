// Multi-source visual verification: dashboard 4 sections + Sessions 4-source
// table + search + detail dialog. Writes to output/playwright/multisource/.
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

  const noHorizScroll = async (label) => {
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    if (overflow > 1) errors.push(`HORIZONTAL SCROLL (${label}): +${overflow}px`);
  };

  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(900);

  // ---- Dashboard sections ----
  await page.screenshot({ path: `${OUT}/dash-zcode.png`, fullPage: true });
  await noHorizScroll('dash-zcode');

  for (const [name, label] of [['codex', 'Codex'], ['dsh', 'DSH'], ['claude', 'CC']]) {
    await page.getByRole('button', { name: label, exact: true }).click();
    await page.waitForTimeout(1100);
    await page.screenshot({ path: `${OUT}/dash-${name}.png`, fullPage: true });
    await noHorizScroll(`dash-${name}`);
  }

  // cost detail modal in CC section
  await page.getByRole('button', { name: 'CC', exact: true }).click();
  await page.waitForTimeout(600);
  const costCell = page.locator('.local-source-metrics ~ .panel .model-row:not(.model-head) .num[title="点击查看成本明细"]').first();
  if (await costCell.count()) {
    await costCell.click();
    await page.waitForTimeout(700);
    await page.screenshot({ path: `${OUT}/dash-cc-cost-modal.png` });
    await page.keyboard.press('Escape');
    await page.waitForTimeout(300);
  }

  // ---- Sessions page ----
  await page.getByRole('button', { name: 'Sessions' }).click();
  await page.waitForTimeout(1100);
  await page.screenshot({ path: `${OUT}/sessions-all.png`, fullPage: true });
  await noHorizScroll('sessions-all');

  // search by source / project / model
  for (const q of ['cc-', 'cx-', 'dsh-', 'crawler', 'claude-opus']) {
    const box = page.getByPlaceholder(/搜索 session/);
    await box.fill(q);
    await page.waitForTimeout(700);
    await page.screenshot({ path: `${OUT}/sessions-search-${q.replace(/[^a-z-]/gi, '_')}.png`, fullPage: true });
  }
  await page.getByPlaceholder(/搜索 session/).fill('');
  await page.waitForTimeout(500);

  // detail dialog of a prefixed session
  const ccRow = page.locator('.session-row:not(.table-head)', { hasText: 'cc-9f3a2b71' }).first();
  if (await ccRow.count()) {
    await ccRow.click();
    await page.waitForTimeout(900);
    await page.screenshot({ path: `${OUT}/sessions-detail-cc.png`, fullPage: true });
    await page.keyboard.press('Escape');
  } else {
    errors.push('cc- session row not found on Sessions page');
  }

  // ---- dark theme spot check ----
  await page.getByRole('button', { name: '深色样板' }).click().catch(() => {});
  await page.waitForTimeout(700);
  await page.getByRole('button', { name: 'CC', exact: true }).click();
  await page.waitForTimeout(900);
  await page.screenshot({ path: `${OUT}/dash-cc-dark.png`, fullPage: true });

  console.log('errors:', JSON.stringify(errors, null, 2));
  await browser.close();
})();
