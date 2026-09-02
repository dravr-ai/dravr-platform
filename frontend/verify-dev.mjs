import { chromium } from '@playwright/test';

const BASE = 'https://dravr-mcp-server-frontend-ojda26xiwa-nn.a.run.app';
const OUT = process.argv[2];
const theme = process.argv[3] ?? 'light';

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, locale: 'fr-FR' });
const page = await ctx.newPage();

await page.addInitScript((t) => {
  localStorage.setItem('dravr.theme', t);
  localStorage.setItem('pierre_app_language', 'fr');
}, theme);

await page.goto(`${BASE}/`, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(1500);

const email = page.locator('input[type="email"], input[name="email"]').first();
if (await email.count()) {
  await email.fill('alice@acme.com');
  await page.locator('input[type="password"]').first().fill('DemoUser123!');
  await page.locator('button[type="submit"]').first().click();
  await page.waitForTimeout(5000);
}

const shots = [
  ['chat', '#chat'],
  ['discover', '#discover'],
  ['settings-connections', '#settings/connections'],
];
for (const [name, hash] of shots) {
  await page.goto(`${BASE}/${hash}`, { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(3500);
  await page.screenshot({ path: `${OUT}/${theme}-${name}.png` });
  console.log(name, 'ok');
}
console.log('title:', await page.title());
await browser.close();
