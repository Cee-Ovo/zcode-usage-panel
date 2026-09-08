const { chromium } = require('playwright-core');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', executablePath: '/usr/bin/google-chrome' });
  const errors = [];
  const check = (ok, msg) => { if (!ok) throw new Error(msg); };

  // 1. DSH missing empty state
  let page = await browser.newPage({ viewport: { width: 1180, height: 860 } });
  page.on('pageerror', e => errors.push(e.message));
  await page.goto('http://localhost:5173/?dsh=missing');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(800);
  await page.getByRole('button', { name: 'DSH', exact: true }).click();
  await page.waitForTimeout(900);
  const emptyText = await page.locator('.local-usage-panel').innerText();
  check(emptyText.includes('unavailable') && emptyText.includes('未检测到'), 'DSH empty state text missing: ' + emptyText);
  await page.screenshot({ path: 'output/playwright/sections-dsh-missing-light.png' });
  // other sections unaffected by dsh=missing
  await page.getByRole('button', { name: 'ZCode', exact: true }).click();
  await page.waitForTimeout(800);
  const zcodeText = await page.locator('.dashboard-section').first().innerText();
  check(zcodeText.includes('ZCode 总 Token'), 'ZCode metrics broken under dsh=missing');
  await page.close();

  // 2. DSH detail modal
  page = await browser.newPage({ viewport: { width: 1180, height: 860 } });
  page.on('pageerror', e => errors.push(e.message));
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(600);
  await page.getByRole('button', { name: 'DSH', exact: true }).click();
  await page.waitForTimeout(900);
  await page.locator('.local-usage-panel').getByRole('button', { name: /详情/ }).click();
  await page.waitForTimeout(500);
  const modalText = await page.locator('.quota-detail').innerText();
  check(modalText.includes('DSH') && modalText.includes('本地'), 'DSH detail modal content unexpected');
  await page.screenshot({ path: 'output/playwright/sections-dsh-modal.png' });
  await page.keyboard.press('Escape');
  await page.close();

  // 3. compact + narrow responsive overflow check on all sections
  page = await browser.newPage({ viewport: { width: 700, height: 500 } });
  page.on('pageerror', e => errors.push(e.message));
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  for (const section of ['ZCode', 'Codex', 'DSH']) {
    await page.getByRole('button', { name: section, exact: true }).click();
    await page.waitForTimeout(800);
    const m = await page.evaluate(() => ({
      body: document.body.scrollWidth,
      inner: window.innerWidth,
      content: document.querySelector('.zup-content').scrollWidth,
      client: document.querySelector('.zup-content').clientWidth,
    }));
    check(m.body <= m.inner + 2 && m.content <= m.client + 2, `overflow in ${section}: ` + JSON.stringify(m));
  }
  // compact view toggle on ZCode
  await page.getByRole('button', { name: 'ZCode', exact: true }).click();
  await page.getByRole('button', { name: '精简视图' }).click();
  await page.waitForTimeout(600);
  await page.screenshot({ path: 'output/playwright/sections-zcode-compact-narrow.png' });
  const cards = await page.locator('.dashboard-metrics .metric-card').count();
  check(cards >= 4, 'compact cards missing');
  console.log('all checks passed; errors:', JSON.stringify(errors));
  await browser.close();
})();
