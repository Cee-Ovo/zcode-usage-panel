// Playwright CLI run-code input; browser DEV synthetic data only.
async (page) => {
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const check = (ok, why) => { if (!ok) throw new Error(why); };
  await page.goto('http://127.0.0.1:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  if (await page.getByRole('button', { name: '显示详细指标', exact: true }).count()) {
    await page.getByRole('button', { name: '显示详细指标', exact: true }).click();
  }
  const layouts = [];
  for (const theme of ['light', 'dark']) {
    await page.getByRole('button', { name: theme === 'light' ? '浅色样板' : '深色样板', exact: true }).click();
    await page.waitForFunction((t) => document.documentElement.dataset.theme === t, theme);
    for (const width of [1280, 980, 600, 400]) {
      await page.setViewportSize({ width, height: 960 });
      await page.waitForFunction(() => [...document.querySelectorAll('.dashboard-metrics .metric-card')].every((e) => Number(getComputedStyle(e).opacity) > .99));
      const result = await page.evaluate(() => {
        const content = document.querySelector('.zup-content');
        const card = document.querySelector('.metric-card');
        const style = getComputedStyle(card);
        return { width: content.clientWidth, scroll: content.scrollWidth, blur: style.backdropFilter,
          background: style.backgroundColor, renderer: card.dataset.oguiRenderer,
          filter: style.filter, material: card.dataset.oguiMaterial };
      });
      check(result.scroll <= result.width + 2, `page overflow ${theme}/${width}: ${JSON.stringify(result)}`);
      check(result.blur.includes('blur(22px)') && result.renderer === 'css' && result.material === 'regular', 'not library regular CSS material');
      check(result.background.startsWith('rgba(') && result.filter === 'none', 'opaque or custom element filter');
      layouts.push({ theme, width, ...result });
    }
    await page.setViewportSize({ width: 1280, height: 960 });
    await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0, 0));
    await page.screenshot({ path: `output/playwright/frosted-${theme}.png` });
  }
  const trigger = page.locator('.codex-heading').getByRole('button', { name: /详情/ });
  await trigger.click();
  const dialog = page.getByRole('dialog', { name: /Codex.*额度详情/ });
  await dialog.waitFor();
  await page.waitForFunction(() => getComputedStyle(document.querySelector('[role="dialog"]')).opacity === '1');
  const overlay = await page.locator('.overlay-backdrop').boundingBox();
  check(overlay && overlay.x <= 1 && overlay.y <= 1 && overlay.width >= 1279 && overlay.height >= 959, 'overlay trapped inside glass card');
  check(await dialog.evaluate((e) => getComputedStyle(e).backdropFilter.includes('blur(34px)')), 'dialog is not library frosted preset');
  await page.keyboard.press('Shift+Tab');
  check(await page.evaluate(() => !!document.activeElement.closest('[role="dialog"]')), 'dialog focus escaped');
  await page.screenshot({ path: 'output/playwright/frosted-dialog.png' });
  await page.keyboard.press('Escape');
  await dialog.waitFor({ state: 'detached' });
  check(await trigger.evaluate((e) => document.activeElement === e), 'focus not restored');

  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-transparency', value: 'reduce' }] });
  await page.waitForFunction(() => getComputedStyle(document.querySelector('.metric-card')).backdropFilter === 'none');
  await cdp.send('Emulation.setEmulatedMedia', { features: [{ name: 'forced-colors', value: 'active' }] });
  await page.waitForFunction(() => getComputedStyle(document.querySelector('.metric-card')).backdropFilter === 'none');
  await cdp.send('Emulation.setEmulatedMedia', { features: [] });
  await cdp.detach();
  await page.getByRole('button', { name: '浅色样板', exact: true }).click();
  await page.getByRole('navigation').getByRole('button', { name: '模型', exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('.frosted-sample'));
  await page.locator('.dashboard-page').waitFor({ state: 'detached' });
  check(await page.locator('.metric-card[data-ogui-glass]').count() === 0, 'sample leaked into models');
  await page.getByRole('navigation').getByRole('button', { name: '仪表盘', exact: true }).click();
  check(errors.length === 0, `browser errors: ${errors.join('; ')}`);
  return { result: 'PASS', layouts, checks: ['library material', '8 layouts', 'portal bounds', 'focus trap and restore', 'reduced transparency', 'forced colors', 'sample scope'], errors };
}
