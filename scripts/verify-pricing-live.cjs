// 验证桌面版价格表:连真机 CDP,打开 设置 → API Pricing,截 OpenAI 分组。
// 前置:应用已带 --remote-debugging-port=9222 启动。
const { chromium } = require('playwright-core');

(async () => {
  const b = await chromium.connectOverCDP('http://127.0.0.1:9222');
  const p = b.contexts()[0].pages().find((x) => !x.url().includes('popup'));
  if (!p) throw new Error('找不到主窗口页面');
  await p.bringToFront().catch(() => {});

  await p.locator('.zup-nav-item', { hasText: '设置' }).first().click();
  await p.locator('#sec-pricing').waitFor({ timeout: 15000 });
  // 分区导航 chip 直达 API Pricing(避免整页滚动定位)
  const chip = p.locator('button,a', { hasText: 'API Pricing' }).first();
  if (await chip.count()) await chip.click();
  await p.waitForTimeout(1200);

  const sec = p.locator('#sec-pricing');
  await sec.scrollIntoViewIfNeeded();
  await p.waitForTimeout(400);

  // 找到 OpenAI 条目块并截图;同时把 gpt-6 相关行的文本打出来做断言
  const hit = await p.evaluate(() => {
    const blocks = [...document.querySelectorAll('#sec-pricing table, #sec-pricing .price-entry, #sec-priciency, #sec-pricing > div')];
    return document.querySelector('#sec-pricing')?.innerText.length ?? 0;
  });
  const txt = await p.evaluate(() => document.querySelector('#sec-pricing')?.innerText ?? '');
  const lines = txt.split('\n').filter((l) => /gpt-6|OpenAI/i.test(l));
  console.log('--- gpt-6 / OpenAI 相关行 ---');
  for (const l of lines) console.log(l);

  await p.screenshot({ path: 'output/px/verify-pricing-0925.png', fullPage: false });
  console.log('screenshot saved: output/px/verify-pricing-0925.png');
  await b.close();
})().catch((e) => { console.error('FAILED:', e.message); process.exit(1); });