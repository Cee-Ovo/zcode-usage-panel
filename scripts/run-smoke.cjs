// Local runner for the repo's playwright-cli style scripts (async (page) => {...}).
// Usage: node scripts/run-smoke.cjs scripts/sidebar-smoke.js
const { chromium } = require('playwright-core');
const fs = require('fs');

(async () => {
  const file = process.argv[2];
  if (!file) throw new Error('usage: node scripts/run-smoke.cjs <script.js>');
  const src = fs.readFileSync(file, 'utf8');
  const body = `return (${src});`;
  const factory = new Function(body)();
  const browser = await chromium.launch({ channel: 'chrome', executablePath: '/usr/bin/google-chrome' });
  const page = await browser.newPage({ viewport: { width: 1000, height: 760 } });
  try {
    const result = await factory(page);
    console.log(JSON.stringify(result ?? { result: 'PASS' }, null, 2));
  } finally {
    await browser.close();
  }
})().catch((e) => { console.error(e); process.exit(1); });
