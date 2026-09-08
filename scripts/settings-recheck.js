// Re-check sticky settings heading scrim. Execute with Playwright CLI run-code --filename.
async (page) => {
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  const nav = page.getByRole('navigation');
  await nav.getByRole('button', { name: '设置', exact: true }).click();
  await page.locator('.settings-page').waitFor();
  await page.waitForTimeout(800);
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 700));
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'output/playwright/recheck-settings-scrolled-light.png' });
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 2200));
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'output/playwright/recheck-settings-scrolled-deep-light.png' });
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 0));
  await page.getByRole('button', { name: '深色样板' }).click();
  await page.waitForTimeout(500);
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 700));
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'output/playwright/recheck-settings-scrolled-dark.png' });
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 0));
  await nav.getByRole('button', { name: '仪表盘', exact: true }).click();
  await page.locator('.dashboard-page').waitFor();
  await page.getByRole('button', { name: '浅色样板' }).click();
  if (errors.length) throw new Error('pageerrors: ' + errors.join('; '));
  return { result: 'DONE', errors };
}
