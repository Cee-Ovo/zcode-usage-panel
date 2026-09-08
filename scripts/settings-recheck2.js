// Verify settings heading: transparent at rest, frosted scrim when stuck.
async (page) => {
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(800);
  const nav = page.getByRole('navigation');
  await nav.getByRole('button', { name: '设置', exact: true }).click();
  await page.locator('.settings-page').waitFor();
  await page.waitForTimeout(800);
  const stuckAtTop = await page.locator('.settings-toolbar.is-stuck').count();
  await page.screenshot({ path: 'output/playwright/r2-settings-top-light.png' });
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 700));
  await page.waitForTimeout(600);
  const stuckScrolled = await page.locator('.settings-toolbar.is-stuck').count();
  await page.screenshot({ path: 'output/playwright/r2-settings-scrolled-light.png' });
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 0));
  await nav.getByRole('button', { name: '仪表盘', exact: true }).click();
  await page.locator('.dashboard-page').waitFor();
  await page.getByRole('button', { name: '深色样板' }).click();
  await page.waitForTimeout(400);
  await nav.getByRole('button', { name: '设置', exact: true }).click();
  await page.locator('.settings-page').waitFor();
  await page.waitForTimeout(600);
  await page.screenshot({ path: 'output/playwright/r2-settings-top-dark.png' });
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 700));
  await page.waitForTimeout(600);
  await page.screenshot({ path: 'output/playwright/r2-settings-scrolled-dark.png' });
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 0));
  await nav.getByRole('button', { name: '仪表盘', exact: true }).click();
  await page.locator('.dashboard-page').waitFor();
  await page.getByRole('button', { name: '浅色样板' }).click();
  if (errors.length) throw new Error('pageerrors: ' + errors.join('; '));
  return { result: 'DONE', stuckAtTop, stuckScrolled, errors };
}
