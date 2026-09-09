// Sessions 页截图验证:默认窗口尺寸 980×700,无横向滚动条 + 悬停完整内容。
// 用法: node scripts/dev-shot-sessions.cjs  (需 dev server 已在 localhost:5173)
const { chromium } = require('playwright-core');

(async () => {
  const browser = await chromium.launch({ channel: 'chrome', executablePath: '/usr/bin/google-chrome' });
  const page = await browser.newPage({ viewport: { width: 980, height: 700 } });
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });

  await page.goto('http://localhost:5173/');
  await page.getByRole('heading', { name: '用量概览' }).waitFor();
  // 导航到 Sessions 页
  await page.getByRole('button', { name: 'Sessions' }).click();
  await page.getByRole('heading', { name: 'Sessions', exact: true }).waitFor();
  await page.waitForTimeout(900);

  // 横向滚动检查:文档与 sessions 面板都不应出现横向溢出
  const overflow = await page.evaluate(() => {
    const doc = document.documentElement;
    const panel = document.querySelector('.sessions-surface');
    return {
      docScrollWidth: doc.scrollWidth,
      docClientWidth: doc.clientWidth,
      docHOverflow: doc.scrollWidth > doc.clientWidth,
      panelScrollWidth: panel ? panel.scrollWidth : null,
      panelClientWidth: panel ? panel.clientWidth : null,
      panelHOverflow: panel ? panel.scrollWidth > panel.clientWidth : null,
    };
  });
  console.log('overflow:', JSON.stringify(overflow));

  await page.screenshot({ path: 'output/playwright/sessions-980-light.png' });

  // 悬停会话名列 → 原生 title 提示存在性由 DOM 断言;截一张悬停高亮图
  const firstRow = page.locator('.session-row:not(.table-head)').first();
  await firstRow.hover();
  await page.waitForTimeout(400);
  await page.screenshot({ path: 'output/playwright/sessions-980-hover.png' });

  // 表头可见性:所有表头单元格都在视口内
  const headers = await page.evaluate(() =>
    Array.from(document.querySelectorAll('.session-row.table-head > span')).map((el) => {
      const r = el.getBoundingClientRect();
      return { text: el.textContent, visible: r.left >= 0 && r.right <= window.innerWidth };
    }),
  );
  console.log('headers:', JSON.stringify(headers));

  // title 属性(悬停完整内容)断言
  const titles = await page.evaluate(() =>
    Array.from(document.querySelectorAll('.session-row:not(.table-head)')).slice(0, 5).map((row) => {
      const spans = row.querySelectorAll('span');
      return {
        sessionTitleAttr: spans[0].getAttribute('title'),
        nameText: spans[1].textContent,
        nameTitleAttr: spans[1].getAttribute('title'),
        projectText: spans[2].textContent,
        projectTitleAttr: spans[2].getAttribute('title'),
        modelText: spans[3].textContent,
        modelTitleAttr: spans[3].getAttribute('title'),
      };
    }),
  );
  console.log('cells:', JSON.stringify(titles, null, 1));

  // 详情弹窗(含会话名 + 完整项目路径)
  await firstRow.click();
  await page.waitForTimeout(700);
  await page.screenshot({ path: 'output/playwright/sessions-980-detail.png' });

  console.log('errors:', JSON.stringify(errors));
  await browser.close();
})();
