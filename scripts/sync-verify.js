// DEV-only visual sync check. Execute with Playwright CLI run-code --filename.
async (page) => {
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  await page.waitForTimeout(1200);
  await page.screenshot({ path: 'output/playwright/sync-1-dashboard.png' });

  const nav = page.getByRole('navigation');
  await nav.getByRole('button', { name: 'Sessions', exact: true }).click();
  await page.locator('.sessions-page').waitFor();
  await page.waitForTimeout(1000);
  await page.screenshot({ path: 'output/playwright/sync-2-sessions.png' });
  await page.locator('.session-row:not(.table-head)').first().click();
  await page.locator('.overlay-card').waitFor();
  await page.waitForTimeout(900);
  await page.screenshot({ path: 'output/playwright/sync-3-session-dialog.png' });
  await page.keyboard.press('Escape');
  await page.waitForTimeout(400);

  await nav.getByRole('button', { name: '模型', exact: true }).click();
  await page.locator('.models-page').waitFor();
  await page.waitForTimeout(1000);
  await page.screenshot({ path: 'output/playwright/sync-4-models.png' });
  await page.locator('.model-row').first().click();
  await page.locator('.overlay-card').waitFor();
  await page.waitForTimeout(900);
  await page.screenshot({ path: 'output/playwright/sync-5-model-dialog.png' });
  await page.keyboard.press('Escape');
  await page.waitForTimeout(400);

  await nav.getByRole('button', { name: '设置', exact: true }).click();
  await page.locator('.settings-page').waitFor();
  await page.waitForTimeout(800);
  await page.screenshot({ path: 'output/playwright/sync-6-settings-top.png' });
  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 700));
  await page.waitForTimeout(500);
  await page.screenshot({ path: 'output/playwright/sync-7-settings-scrolled.png' });

  await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 0));
  await nav.getByRole('button', { name: '仪表盘', exact: true }).click();
  await page.locator('.dashboard-page').waitFor();
  await page.getByRole('button', { name: '深色样板' }).click();
  await page.waitForTimeout(600);
  await page.screenshot({ path: 'output/playwright/sync-8-dashboard-dark.png' });
  await page.getByRole('button', { name: '浅色样板' }).click();

  if (errors.length) throw new Error('pageerrors: ' + errors.join('; '));
  return { result: 'DONE', errors };
}
