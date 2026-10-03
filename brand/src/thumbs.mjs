import { createRequire } from 'module';
const require = createRequire(process.env.PLAYWRIGHT_PKG ?? new URL('../../../maison/e2e/package.json', import.meta.url));
const { chromium } = require('playwright');
const base = new URL('../', import.meta.url).pathname;
const pages = process.argv.slice(2);
const b = await chromium.launch();
for (const name of pages) {
  for (const [tag, w, h, scheme, dpr] of [['desk', 1440, 900, 'light', 1], ['phone', 390, 844, 'dark', 2]]) {
    const p = await b.newPage({ viewport: { width: w, height: h }, colorScheme: scheme, deviceScaleFactor: dpr });
    await p.goto('file://' + base + name + '.html');
    await p.evaluate(() => document.fonts.ready); await p.waitForTimeout(120);
    const sw = await p.evaluate(() => document.documentElement.scrollWidth);
    if (sw > w) console.log('HSCROLL', name, tag, sw);
    await p.screenshot({ path: base + 'shots/thumb-' + name + '-' + tag + '.jpg', type: 'jpeg', quality: 82 });
    await p.close();
  }
}
await b.close();
