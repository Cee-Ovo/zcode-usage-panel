// README 配图生成:连真机 WebView2 的 CDP 口截取仪表盘(见 output/HANDOFF-UI.md 3.2)。
//
// 只截仪表盘这一页 —— Sessions / 模型页会带上真实的会话名、项目路径和用量,
// 不适合放进公开仓库。
//
// 前置:
//   Stop-Process -Name zcode-usage-panel -Force
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222'
//   Start-Process 'D:\Program Files\ZCode Usage Panel\zcode-usage-panel.exe'
// 用法:node scripts/app-shot-readme.cjs [outDir]
const { chromium } = require('playwright-core');
const fs = require('fs');
const path = require('path');

const outDir = process.argv[2] || 'docs/screenshots';
const W = 996;
const H = 709;

(async () => {
  fs.mkdirSync(outDir, { recursive: true });
  const browser = await chromium.connectOverCDP('http://127.0.0.1:9222');
  const ctx = browser.contexts()[0];
  const page = ctx.pages().find((p) => !p.url().includes('popup'));
  if (!page) throw new Error('找不到主窗口页面');
  await page.bringToFront().catch(() => {});

  // 取景 = 默认窗口尺寸(996x709),不额外拉高:README 只要首屏这一屏,
  // 再往下露出的趋势图不是这张图要展示的东西。
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Emulation.setDeviceMetricsOverride', {
    width: W, height: H, deviceScaleFactor: 2, mobile: false,
  });

  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'));
  const dashNav = page.locator('.zup-nav-item', { hasText: '仪表盘' }).first();
  if (await dashNav.count()) await dashNav.click();
  await page.getByRole('heading', { name: '用量概览' }).waitFor({ timeout: 20000 });
  await page.waitForTimeout(1800);

  const file = path.join(outDir, 'dashboard.png');
  await page.screenshot({ path: file });
  console.log('saved', file);

  await cdp.send('Emulation.clearDeviceMetricsOverride');
  await browser.close();
})().catch((e) => {
  console.error('FAILED:', e.message);
  process.exit(1);
});