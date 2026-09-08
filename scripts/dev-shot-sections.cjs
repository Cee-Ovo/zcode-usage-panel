const { chromium } = require('playwright-core');
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', executablePath: '/usr/bin/google-chrome' });
  const page = await browser.newPage({ viewport: { width: 1180, height: 860 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(600);
  // ZCode section (default)
  await page.screenshot({ path: 'output/playwright/sections-zcode-light.png', fullPage: true });
  // switch to Codex
  await page.getByRole('button', { name: 'Codex', exact: true }).click();
  await page.waitForTimeout(1000);
  await page.screenshot({ path: 'output/playwright/sections-codex-light.png', fullPage: true });
  // switch to DSH
  await page.getByRole('button', { name: 'DSH', exact: true }).click();
  await page.waitForTimeout(1000);
  await page.screenshot({ path: 'output/playwright/sections-dsh-light.png', fullPage: true });
  // dark theme DSH
  await page.getByRole('button', { name: '深色样板' }).click();
  await page.waitForTimeout(800);
  await page.screenshot({ path: 'output/playwright/sections-dsh-dark.png', fullPage: true });
  // dark ZCode
  await page.getByRole('button', { name: 'ZCode', exact: true }).click();
  await page.waitForTimeout(800);
  await page.screenshot({ path: 'output/playwright/sections-zcode-dark.png', fullPage: true });
  console.log('errors:', JSON.stringify(errors));
  await browser.close();
})();
