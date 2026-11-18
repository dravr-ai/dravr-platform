import { chromium } from '@playwright/test';

const BASE = 'https://app.dravr.ai';
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

// This page is the deployed app, so the account comes from the environment:
// the repository is public and holds no deployed password (carnet#1597).
const email = page.locator('input[type="email"], input[name="email"]').first();
if (await email.count()) {
  const account = process.env.VERIFY_EMAIL;
  const password = process.env.VERIFY_PASSWORD;
  if (!account || !password) {
    console.error('The app asks for a sign-in: set VERIFY_EMAIL and VERIFY_PASSWORD.');
    await browser.close();
    process.exit(1);
  }
  await email.fill(account);
  await page.locator('input[type="password"]').first().fill(password);
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
