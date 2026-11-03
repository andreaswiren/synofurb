// Drives the Add-ons page of the demo like a person would (run by tests/test_ui.py; needs node + playwright + chromium).
// usage: node ui_addons.js <demo-url-without-query>  -> prints one JSON document; exit 77 when no browser can be started.
const { chromium } = require('playwright');
const LIB = require('./ui_lib.js');
const BASE = process.argv[2];
(async () => {
  let b;
  try { b = await chromium.launch(); } catch (e) { console.error(String(e)); process.exit(77); }
  const out = {errors: [], flows: {}, problems: []};
  const open = async (q, w = 1280, h = 900) => {
    const ctx = await b.newContext({viewport: {width: w, height: h}}); const p = await ctx.newPage();
    p.on('pageerror', e => out.errors.push('pageerror: ' + e.message));
    p.on('console', m => { if (['error', 'warning'].includes(m.type())) out.errors.push(m.type() + ': ' + m.text()); });
    p.on('request', r => { if (!/^(file|data|blob):/.test(r.url())) out.errors.push('external request ' + r.url()); });
    await p.goto(BASE + '?' + q); await p.waitForSelector('.shell');
    if (w < 700) { await p.click('.burger'); await p.click('.side .nav button:has-text("Add-ons")'); } else await p.click('.nav button:has-text("Add-ons")');
    await p.waitForSelector('h1:has-text("Add-ons")'); await p.waitForTimeout(350);
    return p;
  };
  const check = async (p, n) => { for (const x of await LIB.problems(p)) out.problems.push(n + ': ' + x); };
  const why = (p, sel) => p.evaluate(s => { const b = document.querySelector(s), d = b && document.getElementById(b.getAttribute('aria-describedby') || '-'); return b ? {off: b.getAttribute('aria-disabled') === 'true', native: b.disabled, text: d ? d.textContent.trim() : null, visible: !!d && d.checkVisibility()} : null; }, sel);
  // 1. install Plex: the privileges are not ticked, the button says why next to itself (the checkbox is below the fold), then the job
  let p = await open('');
  await p.click('.acard:has-text("Plex") button:has-text("Install")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(200);
  await check(p, 'install-dialog');
  out.flows.install = await p.evaluate(() => {
    const dlg = document.querySelector('dialog'), ok = [...dlg.querySelectorAll('.df button')].find(x => x.textContent.includes('Install')), d = document.getElementById(ok.getAttribute('aria-describedby') || '-'), r = ok.getBoundingClientRect();
    const helps = [...dlg.querySelectorAll('.popwrap > button')].map(x => x.getAttribute('aria-label'));
    const media = [...dlg.querySelectorAll('select')].map(s => [...s.options].map(o => o.textContent));
    return {off: ok.getAttribute('aria-disabled') === 'true', reason: d ? d.textContent.trim() : null, reasonVisible: !!d && d.checkVisibility(), buttonInView: r.bottom <= innerHeight, helps, media};
  });
  await p.locator('dialog .df button:has-text("Install")').focus();
  out.flows.installFocused = await p.evaluate(() => document.activeElement.textContent.includes('Install'));
  await p.locator('dialog .df button:has-text("Install")').dispatchEvent('click');
  out.flows.installClickIgnored = (await p.locator('dialog').count()) === 1 && (await p.locator('dialog[open] progress').count()) === 0;
  await p.check('dialog #ad-accept'); out.flows.installOnAfterAccept = (await why(p, 'dialog .df .btn.primary')).off === false;
  await p.fill('dialog #ad-plex_claim', 'claim-xyz');
  await p.click('dialog .df button:has-text("Install")'); await p.waitForSelector('dialog progress'); await p.waitForTimeout(300);
  await check(p, 'job');
  out.flows.jobRunning = {primary: await p.locator('dialog .df button.primary').innerText(), progressName: await p.locator('dialog progress').getAttribute('aria-label')};
  await p.waitForFunction(() => document.querySelector('dialog .df button.primary') && document.querySelector('dialog .df button.primary').textContent.trim() === 'Close', null, {timeout: 20000});
  out.flows.jobDone = await p.locator('dialog .df button.primary').innerText();
  await p.focus('dialog .df button.primary'); await p.keyboard.press('Enter'); await p.waitForTimeout(500);
  out.flows.focusAfterJob = await p.evaluate(() => { const m = document.getElementById('main'), cs = getComputedStyle(m); return {onMain: document.activeElement === m, outline: cs.outlineStyle, width: cs.outlineWidth}; });
  out.flows.plexNowInstalled = await p.locator('tbody tr:has-text("Plex")').count();
  await p.context().close();
  // 2. every status on one page; the add-on that needs attention is listed once (installed, with Retry), and its reason is on the row
  p = await open('addons=states');
  await check(p, 'states');
  out.flows.states = await p.evaluate(() => ({
    plexMentions: [...document.querySelectorAll('main b')].filter(x => x.textContent.includes('Plex')).length,
    plexCards: document.querySelectorAll('.acard').length,
    retry: [...document.querySelectorAll('tbody button')].filter(x => x.textContent.includes('Retry')).length,
    rowDetail: (document.querySelector('tbody tr:has(.badge.warn) .adet') || {}).textContent || null,
    tags: [...document.querySelectorAll('tbody .badge')].map(x => x.textContent.trim()),
  }));
  await p.context().close();
  // 3. the Problem badge keeps the width of its text
  p = await open('addons=problems');
  await check(p, 'problems');
  out.flows.problems = await p.evaluate(() => [...document.querySelectorAll('tbody .badge')].map(x => { const r = x.getBoundingClientRect(), cs = getComputedStyle(x); return {t: x.textContent.trim(), w: Math.round(r.width), mb: cs.marginBottom}; }));
  await p.context().close();
  // 4. Docker is missing: the cards say so in text that keyboard and touch users reach, at desktop and phone width
  for (const [w, h] of [[1280, 900], [390, 844]]) {
    p = await open('addons=nodocker', w, h);
    await check(p, 'nodocker-' + w);
    const r = await p.evaluate(() => [...document.querySelectorAll('.acard')].map(c => { const b = c.querySelector('button'), d = document.getElementById(b.getAttribute('aria-describedby') || '-'); return {name: c.querySelector('b').textContent, off: b.getAttribute('aria-disabled') === 'true', native: b.disabled, reason: d ? d.textContent.trim() : null, visible: !!d && d.checkVisibility(), focusable: b.tabIndex >= 0}; }));
    out.flows['nodocker' + w] = r;
    if (w === 1280) { await p.locator('.acard:has-text("Portainer") button').focus(); out.flows.nodockerFocus = await p.evaluate(() => document.activeElement.closest('.acard').textContent.includes('Portainer')); }
    await p.context().close();
  }
  // 5. logs, details at 320 px (chips stay inside the dialog), remove with the typed name
  p = await open('', 320, 700);
  await p.click('tbody tr:has-text("Portainer") button[aria-label="Details"]'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(200);
  await check(p, 'details-320');
  out.flows.details320 = await p.evaluate(() => { const d = document.querySelector('dialog'), r = d.getBoundingClientRect(); return {scrolls: d.scrollWidth > d.clientWidth + 1, chipsOutside: [...d.querySelectorAll('.chip')].filter(c => c.getBoundingClientRect().right > r.right + 1).length, chips: d.querySelectorAll('.chip').length, labelled: [...d.querySelectorAll('.chip')].map(c => c.textContent.trim())[0]}; });
  await p.context().close();
  p = await open('');
  await p.click('tbody tr:has-text("Portainer") button[aria-label="Logs"]'); await p.waitForSelector('dialog[open] pre'); await p.waitForTimeout(300);
  await check(p, 'logs');
  out.flows.logs = (await p.locator('dialog pre').innerText()).includes('ready');
  await p.keyboard.press('Escape');
  await p.click('tbody tr:has-text("Portainer") button[aria-label="Remove"]'); await p.waitForSelector('dialog[open]');
  await check(p, 'remove');
  await p.check('dialog input[type=checkbox]'); await p.waitForTimeout(100);
  await check(p, 'remove-typed');
  out.flows.removeOff = await why(p, 'dialog .btn.danger');
  await p.fill('dialog input.mono', 'portainer');
  out.flows.removeOn = (await why(p, 'dialog .btn.danger')).off === false;
  await p.context().close();
  // 6. phone width: the install dialog keeps its reason and button in view
  p = await open('', 390, 844);
  await p.click('.acard:has-text("Frigate") button:has-text("Install")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(200);
  await check(p, 'install-phone');
  out.flows.installPhone = await p.evaluate(() => { const ok = [...document.querySelectorAll('dialog .df button')].find(x => x.textContent.includes('Install')), r = ok.getBoundingClientRect(); return {inView: r.bottom <= innerHeight && r.top >= 0, overflow: document.documentElement.scrollWidth > innerWidth}; });
  await p.context().close();
  console.log(JSON.stringify(out)); await b.close();
})();
