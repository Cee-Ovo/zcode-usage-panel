const { chromium } = require('playwright-core');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', executablePath: '/usr/bin/google-chrome' });
  const errors = [];
  const check = (ok, msg) => { if (!ok) throw new Error(msg); };
  const page = await browser.newPage({ viewport: { width: 1180, height: 860 } });
  page.on('pageerror', e => errors.push(e.message));
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(300);
  await page.evaluate(() => localStorage.removeItem('zup.compact'));
  await page.reload();
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(900);
  const speedText = await page.locator('.metric-card', { hasText: '响应速度' }).innerText();
  check(speedText.includes('秒') && speedText.includes('tok/s') && speedText.includes('P95'), 'speed card content: ' + speedText);
  console.log('speed card text:', JSON.stringify(speedText.replaceAll('\n', ' | ')));
  // grid balance: cards in last rows align
  const grid = await page.evaluate(() => {
    const cards = [...document.querySelectorAll('.dashboard-metrics > .metric-card')];
    return cards.map(c => ({ label: c.querySelector('.label')?.childNodes[0]?.textContent?.trim(), rect: JSON.parse(JSON.stringify(c.getBoundingClientRect())) }));
  });
  console.log('grid rows:', grid.map(g => `${g.label}@y=${Math.round(g.rect.y)},h=${Math.round(g.rect.height)}`).join('  '));
  await page.screenshot({ path: 'output/playwright/speed-zcode-light.png' });
  // compact
  await page.getByRole('button', { name: '精简视图' }).click();
  await page.waitForTimeout(700);
  const compactText = await page.locator('.dashboard-metrics.is-compact', { hasText: '响应速度' }).count();
  check(compactText === 1, 'speed card missing in compact');
  await page.screenshot({ path: 'output/playwright/speed-zcode-compact.png' });
  await page.getByRole('button', { name: '显示详细指标' }).click();
  // dark
  await page.getByRole('button', { name: '深色样板' }).click();
  await page.waitForTimeout(700);
  await page.screenshot({ path: 'output/playwright/speed-zcode-dark.png' });
  console.log('errors:', JSON.stringify(errors));
  await browser.close();
})();
