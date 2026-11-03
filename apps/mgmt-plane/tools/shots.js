// Screenshots of the new web states (Add-ons, Updates) from the self-contained demo: light and dark, 1280 and 390 wide (phone).
//
//   NODE_PATH="$(npm root -g)" node apps/mgmt-plane/tools/shots.js apps/mgmt-plane/demo/index.html design/screenshots
//   ... shots.js <demo.html or file:// url> <out-dir> [--only name[,name...]] [--list]
//
// Needs node, playwright and a chromium build (exit 77 when none can be started). Files are named mgmt-demo-<state>-<light|dark>[-phone].png.
// The demo is rebuilt with `python3 apps/mgmt-plane/tools/build-demo.py`. Every state is also checked with tests/ui_lib.js (stray text such as "null",
// reasons only a mouse can see, overflow): a problem is printed and the exit code is 1. Each PNG stays small (the tool fails above 120 KB).
const path = require('path');
const fs = require('fs');
const { chromium } = require('playwright');
const LIB = require(path.join(__dirname, '..', 'tests', 'ui_lib.js'));
const args = process.argv.slice(2);
const flag = n => { const i = args.indexOf(n); return i < 0 ? null : args.splice(i, 2)[1]; };
const only = (flag('--only') || '').split(',').filter(Boolean);
const list = args.includes('--list');
const [SRC, OUT] = args.filter(a => !a.startsWith('--'));
const MAX = 120 * 1024;
const BASE = SRC && (/^(file|https?):/.test(SRC) ? SRC : 'file://' + path.resolve(SRC));

const go = async (p, page, phone) => {
  await p.waitForSelector('.shell');
  if (phone) { await p.click('.burger'); await p.click('.side .nav button:has-text("' + page + '")'); } else await p.click('.nav button:has-text("' + page + '")');
  await p.waitForSelector('h1:has-text("' + page + '")'); await p.waitForTimeout(350);
};
const installDialog = async (p, name) => { await p.click('.acard:has-text("' + name + '") button:has-text("Install")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(250); };
const check = async p => { await p.click('button:has-text("Check")'); await p.waitForSelector('.badge:has-text("New"), .badge:has-text("Up to date"), .toast.bad'); await p.waitForTimeout(250); };
const startUpdate = async p => { await p.click('main button:has-text("Update now")'); await p.waitForSelector('dialog[open]'); await p.check('dialog input[type=checkbox]'); await p.click('dialog button.primary:has-text("Update now")'); };

// name, demo query, page, how to reach the state, what to capture ('page' = whole page, 'view' = the window, a selector = that element)
const S = [
  {n: 'addons-catalog', q: 'addons=', page: 'Add-ons', cap: 'page'},
  {n: 'addons-install', q: 'addons=', page: 'Add-ons', cap: 'view', run: async p => { await installDialog(p, 'Plex'); }},
  {n: 'addons-install-consent', q: 'addons=', page: 'Add-ons', cap: 'view', run: async p => { await installDialog(p, 'Plex'); await p.check('dialog #ad-accept'); await p.evaluate(() => { const b = document.querySelector('dialog .db'); b.scrollTop = b.scrollHeight; }); await p.waitForTimeout(150); }},
  {n: 'addons-job', q: 'addons=', page: 'Add-ons', cap: 'view', run: async p => { await installDialog(p, 'Plex'); await p.check('dialog #ad-accept'); await p.click('dialog .df button:has-text("Install")'); await p.waitForSelector('dialog progress'); await p.waitForTimeout(1500); }},
  {n: 'addons-job-done', q: 'addons=', page: 'Add-ons', cap: 'view', run: async p => { await installDialog(p, 'Plex'); await p.check('dialog #ad-accept'); await p.click('dialog .df button:has-text("Install")'); await p.waitForFunction(() => /Close/.test((document.querySelector('dialog .df button.primary') || {}).textContent || ''), null, {timeout: 20000}); }},
  {n: 'addons-states', q: 'addons=states', page: 'Add-ons', cap: 'page'},
  {n: 'addons-problems', q: 'addons=problems', page: 'Add-ons', cap: 'page'},
  {n: 'addons-nodocker', q: 'addons=nodocker', page: 'Add-ons', cap: 'page'},
  {n: 'addons-logs', q: 'addons=', page: 'Add-ons', cap: 'view', run: async p => { await p.click('tbody tr:has-text("Portainer") button[aria-label="Logs"]'); await p.waitForSelector('dialog[open] pre'); await p.waitForTimeout(300); }},
  {n: 'addons-details', q: 'addons=states', page: 'Add-ons', cap: 'dialog[open]', run: async p => { await p.click('tbody tr:has-text("Plex") button[aria-label="Details"]'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(250); }},
  {n: 'addons-remove', q: 'addons=', page: 'Add-ons', cap: 'view', run: async p => { await p.click('tbody tr:has-text("Portainer") button[aria-label="Remove"]'); await p.waitForSelector('dialog[open]'); await p.check('dialog input[type=checkbox]'); await p.fill('dialog input.mono', 'portai'); await p.waitForTimeout(150); }},
  {n: 'updates-initial', q: 'update=available', page: 'Updates', cap: 'page'},
  {n: 'updates-available', q: 'update=available', page: 'Updates', cap: 'page', run: async p => { await check(p); }},
  {n: 'updates-current', q: 'update=current', page: 'Updates', cap: 'page', run: async p => { await check(p); }},
  {n: 'updates-offline', q: 'update=offline', page: 'Updates', cap: 'view', run: async p => { await check(p); }},
  {n: 'updates-busy', q: 'update=busy', page: 'Updates', cap: 'page', run: async p => { await check(p); }},
  {n: 'updates-consent', q: 'update=available', page: 'Updates', cap: 'view', run: async p => { await check(p); await p.click('main button:has-text("Update now")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(200); }},
  {n: 'updates-progress', q: 'update=available', page: 'Updates', cap: 'page', run: async p => { await check(p); await startUpdate(p); await p.waitForSelector('.us li.run'); await p.waitForTimeout(400); }},
  {n: 'updates-result', q: 'update=available', page: 'Updates', cap: 'page', run: async p => { await check(p); await startUpdate(p); await p.waitForSelector('.banner.ok', {timeout: 30000}); await p.waitForTimeout(250); }},
  {n: 'updates-rollback', q: 'update=rollback', page: 'Updates', cap: 'page', run: async p => { await check(p); await startUpdate(p); await p.waitForSelector('main .banner.warn', {timeout: 40000}); await p.waitForTimeout(250); }},
  {n: 'updates-rollback-dialog', q: 'update=available', page: 'Updates', cap: 'view', run: async p => { await p.click('button:has-text("Roll back")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(200); }},
  {n: 'updates-unhealthy', q: 'update=unhealthy', page: 'Updates', cap: 'page', run: async p => { await check(p); await startUpdate(p); await p.waitForSelector('.banner.warn button:has-text("Update anyway")', {timeout: 20000}); await p.waitForTimeout(250); }},
  {n: 'updates-history', q: 'update=available', page: 'Updates', cap: 'section.card:has(h2:has-text("History"))'},
];

(async () => {
  if (list) { console.log(S.map(s => s.n).join('\n')); return; }
  if (!BASE || !OUT) { console.error('usage: shots.js <demo.html> <out-dir> [--only a,b] [--list]'); process.exit(2); }
  fs.mkdirSync(OUT, {recursive: true});
  let b;
  try { b = await chromium.launch(); } catch (e) { console.error(String(e)); process.exit(77); }
  let bad = 0, n = 0;
  for (const s of S) {
    if (only.length && !only.includes(s.n)) continue;
    for (const [phone, w, h] of [[false, 1280, 800], [true, 390, 844]]) {
      const ctx = await b.newContext({viewport: {width: w, height: h}, colorScheme: 'light'}); const p = await ctx.newPage();
      const errs = [];
      p.on('pageerror', e => errs.push('pageerror: ' + e.message));
      await p.goto(BASE + '?' + s.q); await go(p, s.page, phone);
      if (s.run) await s.run(p);
      if (phone) await p.addStyleTag({content: '#demo-ribbon{display:none!important}'});          // on a phone the demo pill floats over the content
      await p.mouse.move(1, 1); await p.waitForTimeout(100);          // no leftover hover tooltip
      if (s.cap === 'page') {            // a tall window instead of a full-page capture, so the fixed tab bar and the demo ribbon stay at the bottom edge
        const full = await p.evaluate(() => Math.max(document.documentElement.scrollHeight, document.body.scrollHeight));
        await p.setViewportSize({width: w, height: Math.min(Math.max(h, full), 3000)}); await p.waitForTimeout(200);
      }
      for (const theme of ['light', 'dark']) {
        await p.evaluate(t => document.documentElement.setAttribute('data-theme', t), theme); await p.waitForTimeout(120);
        const file = path.join(OUT, 'mgmt-demo-' + s.n + '-' + theme + (phone ? '-phone' : '') + '.png');
        const shot = s.cap === 'page' || s.cap === 'view' ? await p.screenshot() : await p.locator(s.cap).screenshot();
        fs.writeFileSync(file, shot); n++;
        const probs = (theme === 'light' ? await LIB.problems(p) : []).concat(errs.splice(0));
        for (const x of probs) { console.log(path.basename(file) + ': ' + x); bad++; }
        if (shot.length > MAX) { console.log(path.basename(file) + ': ' + Math.round(shot.length / 1024) + ' KB is above the 120 KB limit'); bad++; }
      }
      await ctx.close();
    }
  }
  console.log(n + ' screenshots written to ' + OUT + (bad ? ', ' + bad + ' problem(s)' : ', no problems'));
  await b.close();
  process.exit(bad ? 1 : 0);
})();
