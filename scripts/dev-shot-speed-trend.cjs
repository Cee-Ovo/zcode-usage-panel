const { chromium } = require('playwright-core');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome' });
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

  // ZCode 分区速度卡:tps + 趋势行(近24h/近7天)
  const speedCard = page.locator('.metric-card', { hasText: '响应速度' }).first();
  const speedText = await speedCard.innerText();
  check(speedText.includes('tps'), 'speed card tps: ' + speedText);
  check(speedText.includes('近24h') && speedText.includes('近7天'), 'trend line missing: ' + speedText);
  console.log('zcode speed card:', JSON.stringify(speedText.replaceAll('\n', ' | ')));
  await speedCard.screenshot({ path: 'output/playwright/trend-card-zcode.png' });
  await page.screenshot({ path: 'output/playwright/trend-dashboard-light.png' });

  // Codex 分区(近似口径)速度卡
  const codexCard = page.locator('.metric-card', { hasText: '响应速度' }).nth(1);
  if (await codexCard.count()) {
    const t = await codexCard.innerText();
    console.log('codex speed card:', JSON.stringify(t.replaceAll('\n', ' | ')));
    check(t.includes('近24h') && t.includes('近7天'), 'codex trend line missing: ' + t);
    await codexCard.screenshot({ path: 'output/playwright/trend-card-codex.png' });
  }

  // dark
  const darkBtn = page.getByRole('button', { name: '深色样板' });
  if (await darkBtn.count()) {
    await darkBtn.click();
    await page.waitForTimeout(700);
    await page.screenshot({ path: 'output/playwright/trend-dashboard-dark.png' });
  }
  console.log('errors:', JSON.stringify(errors));
  await browser.close();
})().catch(e => { console.error('FAILED:', e.message); process.exit(1); });
