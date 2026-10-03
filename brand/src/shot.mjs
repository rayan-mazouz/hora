// usage: node shot.mjs <url-or-file> <out.png> [width] [height] [scheme] [fullPage]
import { createRequire } from 'module';
const require = createRequire(process.env.PLAYWRIGHT_PKG ?? new URL('../../../maison/e2e/package.json', import.meta.url));
const { chromium } = require('playwright');
const [,, src, out, w='1440', h='900', scheme='light', full='1'] = process.argv;
const b = await chromium.launch();
const p = await b.newPage({ viewport: { width: +w, height: +h }, colorScheme: scheme, deviceScaleFactor: 2 });
await p.goto(src.startsWith('http')||src.startsWith('file:') ? src : 'file://' + src);
await p.evaluate(() => document.fonts.ready);
await p.waitForTimeout(150);
const sw = await p.evaluate(() => document.documentElement.scrollWidth);
if (sw > +w) console.log('HSCROLL', sw, '>', w);
await p.screenshot({ path: out, fullPage: full === '1' });
await b.close();
