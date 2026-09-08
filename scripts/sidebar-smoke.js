// DEV-only fixtures. Execute with Playwright CLI run-code --filename.
async (page) => {
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  const check = (ok, msg) => { if (!ok) throw new Error(msg); };
  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  const layouts = [];
  for (const theme of ['light', 'dark']) {
    await page.getByRole('button', { name: theme === 'light' ? '浅色样板' : '深色样板' }).click();
    for (const [width,height] of [[1000,760],[1280,900],[400,760],[700,460]]) {
      await page.setViewportSize({width,height});
      const m = await page.evaluate(() => {
        const nav = document.querySelector('.zup-nav');
        const group = document.querySelector('.sidebar-navigation');
        const status = document.querySelector('.sidebar-status');
        return { body:document.body.scrollWidth, width:nav.clientWidth, scroll:nav.scrollWidth,
          background:getComputedStyle(nav).backgroundColor, border:getComputedStyle(nav).borderRightWidth,
          glass:getComputedStyle(group).backdropFilter, statusWidth:status.clientWidth, statusScroll:status.scrollWidth };
      });
      check(m.body <= width+2 && m.scroll <= m.width+2 && m.statusScroll <= m.statusWidth+2, 'overflow: '+JSON.stringify(m));
      check(m.background === 'rgba(0, 0, 0, 0)' && m.border === '0px' && m.glass.includes('blur'), 'sidebar material');
      layouts.push({theme,viewport:[width,height],...m});
    }
    await page.setViewportSize({width:1000,height:760});
    await page.evaluate(() => document.querySelector('.zup-content').scrollTo(0,0));
    await page.screenshot({path:`output/playwright/sidebar-${theme}.png`});
  }
  const summary = page.locator('.sidebar-diagnostics summary');
  await summary.focus(); await page.keyboard.press('Enter');
  check(await page.locator('.sidebar-diagnostics').evaluate(e => e.open), 'keyboard details');
  await page.keyboard.press('Enter');
  const nav = page.getByRole('navigation');
  await nav.getByRole('button', {name:'Sessions',exact:true}).focus();
  await page.keyboard.press('Enter');
  await page.locator('.dashboard-page').waitFor({state:'detached'});
  check(await nav.getByRole('button', {name:'Sessions',exact:true}).getAttribute('aria-current') === 'page', 'navigation state');
  check(await page.locator('.sidebar-navigation').count() === 1, 'floating sidebar persists on all pages');
  await nav.getByRole('button',{name:'仪表盘',exact:true}).click();
  await page.locator('.sidebar-navigation').waitFor();
  await page.evaluate(async () => {
    const {store} = await import(performance.getEntriesByType('resource').find(r=>r.name.includes('/src/lib/store.ts')).name);
    const {api} = await import(performance.getEntriesByType('resource').find(r=>r.name.includes('/src/lib/ipc.ts')).name);
    window.__sidebarOriginalSave = api.saveSettings;
    api.saveSettings = async settings => { store.set({settings}); };
  });
  const toggle = nav.getByRole('switch',{name:'实时监控开关'});
  await toggle.focus(); await page.keyboard.press('Space');
  await nav.getByText('已暂停',{exact:true}).waitFor();
  await page.keyboard.press('Space');
  await nav.getByText('监控中',{exact:true}).waitFor();
  await page.evaluate(async () => {
    const {store} = await import(performance.getEntriesByType('resource').find(r=>r.name.includes('/src/lib/store.ts')).name);
    const {api} = await import(performance.getEntriesByType('resource').find(r=>r.name.includes('/src/lib/ipc.ts')).name);
    api.saveSettings = window.__sidebarOriginalSave;
    window.__sidebarOriginalUsage = api.usageView;
    api.usageView = async () => { throw new Error('synthetic-sidebar-error'); };
    store.set({rangeKey:'7d'});
  });
  await nav.getByRole('alert').waitFor();
  await nav.getByText('刷新异常',{exact:true}).waitFor();
  check(await nav.getByRole('button',{name:'重试',exact:true}).isVisible(), 'retry hidden');
  check(!await page.locator('.sidebar-diagnostics').evaluate(e=>e.open), 'error should not need details');
  await page.evaluate(async () => {
    const {api} = await import(performance.getEntriesByType('resource').find(r=>r.name.includes('/src/lib/ipc.ts')).name);
    api.usageView = window.__sidebarOriginalUsage;
  });
  await nav.getByRole('button',{name:'重试',exact:true}).click();
  await nav.getByRole('alert').waitFor({state:'detached'});
  await page.getByRole('button',{name:'浅色样板'}).click();
  check(!errors.length, errors.join('; '));
  return {result:'PASS',layouts,errors};
}
