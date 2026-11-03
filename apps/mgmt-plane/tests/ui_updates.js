// Drives the Updates page of the demo like a person would (run by tests/test_ui.py; needs node + playwright + chromium).
// usage: node ui_updates.js <demo-url-without-query> [screenshot-dir]  -> prints one JSON document; exit 77 when no browser can be started.
const { chromium } = require('playwright');
const LIB = require('./ui_lib.js');
const BASE = process.argv[2], SHOTS = process.argv[3] || null;
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
    if (w < 700) await p.click('.burger');
    await p.click('.nav button:has-text("Updates")'); await p.waitForSelector('h1:has-text("Updates")');
    return p;
  };
  // every state is checked for stray text ("null", "undefined", "[object ..."), reasons that only a mouse can see, and overflow
  const shot = async (p, n) => { for (const x of await LIB.problems(p)) out.problems.push(n + ': ' + x); if (SHOTS) await p.screenshot({path: SHOTS + '/' + n + '.png', fullPage: true}); };
  // 1. check -> update -> progress -> reconnect -> result
  let p = await open('update=available');
  await shot(p, '1-initial');
  out.flows.initial = {updateDisabled: await p.locator('button:has-text("Update now")').first().isDisabled(), rollbackShown: await p.locator('button:has-text("Roll back")').count()};
  // the reason for the disabled button is visible text and the button is reachable with the keyboard (a disabled button shows no tooltip)
  const go0 = p.locator('main button:has-text("Update now")');
  await go0.focus();
  out.flows.reason = await p.evaluate(() => { const b = document.activeElement, d = document.getElementById(b.getAttribute('aria-describedby') || '-'); return {focused: b.textContent.includes('Update now'), text: d ? d.textContent.trim() : null, visible: !!d && d.checkVisibility(), native: b.disabled}; });
  await go0.dispatchEvent('click'); out.flows.reasonClickIgnored = (await p.locator('dialog[open]').count()) === 0;
  // the failed update of the seeded history says why in the row itself, not in a tooltip
  out.flows.historyReason = (await p.locator('tbody tr:has(.badge:has-text("Rolled back"))').first().innerText()).replace(/\s+/g, ' ');
  await p.click('button:has-text("Check")'); await p.waitForSelector('.badge:has-text("New")');
  await shot(p, '2-checked');
  out.flows.checked = {available: await p.locator('dd.mono').nth(1).innerText(), unsigned: await p.locator('.badge:has-text("Not signed")').count()};
  await p.click('button:has-text("Update now")'); await p.waitForSelector('dialog[open]');
  await shot(p, '3-dialog');
  const ok = p.locator('dialog button.primary:has-text("Update now")');
  out.flows.dialogDisabledUntilConsent = await ok.isDisabled();
  out.flows.dialogReason = await p.evaluate(() => { const b = document.querySelector('dialog button.primary'), d = document.getElementById(b.getAttribute('aria-describedby') || '-'), r = b.getBoundingClientRect(); return {text: d ? d.textContent.trim() : null, visible: !!d && d.checkVisibility(), inView: r.bottom <= innerHeight}; });
  out.flows.warningBeforeConsent = await p.evaluate(() => { const w = document.querySelector('dialog .banner.warn'), c = document.querySelector('dialog input[type=checkbox]'); return !!w && !!c && !!(w.compareDocumentPosition(c) & Node.DOCUMENT_POSITION_FOLLOWING) && /SHA-256/.test(w.textContent); });
  await p.check('dialog input[type=checkbox]'); out.flows.dialogEnabledAfterConsent = !(await ok.isDisabled());
  await ok.click();
  await p.waitForSelector('.us'); await shot(p, '4-running');
  await p.waitForSelector('text=Restarting', {timeout: 15000}); out.flows.sawRestart = true; await shot(p, '5-restarting');
  await p.waitForSelector('.banner.ok', {timeout: 30000}); await shot(p, '6-done');
  out.flows.done = {banner: (await p.locator('.banner.ok').innerText()).replace(/\s+/g, ' '), button: await p.locator('.banner.ok button').innerText(), rows: await p.locator('tbody tr').count()};
  await p.context().close();
  // 2. main channel needs the typed commit
  p = await open('update=available');
  await p.click('.seg button:has-text("Main")'); await p.click('button:has-text("Check")'); await p.waitForSelector('.badge:has-text("New")');
  await p.click('button:has-text("Update now")'); await p.waitForSelector('dialog[open]');
  const ok2 = p.locator('dialog button.primary:has-text("Update now")');
  await p.check('dialog input[type=checkbox]');
  out.flows.mainNeedsTyping = await ok2.isDisabled();
  await p.fill('dialog input.mono', '3f9c2a7'); out.flows.mainEnabledAfterTyping = !(await ok2.isDisabled());
  await shot(p, '7-main-dialog');
  await p.context().close();
  // 3. failing health check -> automatic rollback is reported
  p = await open('update=rollback');
  await p.click('button:has-text("Check")'); await p.waitForSelector('.badge:has-text("New")');
  await p.click('button:has-text("Update now")'); await p.check('dialog input[type=checkbox]'); await p.click('dialog button.primary:has-text("Update now")');
  await p.waitForSelector('main .banner.warn', {timeout: 40000}); await shot(p, '8-rolled-back');
  out.flows.rolledBack = (await p.locator('main .banner.warn').innerText()).replace(/\s+/g, ' ');
  await p.context().close();
  // 4. offline check and busy lock, phone width
  p = await open('update=offline', 390, 844);
  await p.click('button:has-text("Check")'); await p.waitForSelector('.toast.bad');
  out.flows.offline = await p.locator('.toast.bad').innerText();
  await shot(p, '9-offline-phone');
  out.flows.phoneOverflow = await p.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1);
  await p.context().close();
  p = await open('update=busy');
  await p.click('button:has-text("Check")'); await p.waitForSelector('.badge:has-text("New")');
  out.flows.busyDisabled = await p.locator('button:has-text("Update now")').first().isDisabled();
  out.flows.busyReason = await p.evaluate(() => { const b = [...document.querySelectorAll('main button')].find(x => x.textContent.includes('Update now')), d = document.getElementById(b.getAttribute('aria-describedby') || '-'); return d ? d.textContent.trim() : null; });
  await shot(p, '9b-busy');
  // 4b. up to date: nothing to install, and the page says so next to the button
  await p.context().close();
  p = await open('update=current');
  await p.click('button:has-text("Check")'); await p.waitForSelector('.badge:has-text("Up to date")');
  out.flows.currentReason = await p.evaluate(() => { const b = [...document.querySelectorAll('main button')].find(x => x.textContent.includes('Update now')), d = document.getElementById(b.getAttribute('aria-describedby') || '-'); return d ? d.textContent.trim() : null; });
  await shot(p, '9c-current');
  // 5. refused because the unit is unhealthy: the banner offers 'Update anyway'
  await p.context().close();
  p = await open('update=unhealthy');
  await p.click('button:has-text("Check")'); await p.waitForSelector('.badge:has-text("New")');
  await p.click('button:has-text("Update now")'); await p.check('dialog input[type=checkbox]'); await p.click('dialog button.primary:has-text("Update now")');
  await p.waitForSelector('.banner.warn button:has-text("Update anyway")', {timeout: 20000}); await shot(p, '10-refused');
  const card = (await p.locator('main section.card').nth(1).innerText()).replace(/\s+/g, ' ');
  out.flows.refused = card; out.flows.refusedMessageOnce = (card.match(/critical state/g) || []).length === 1;
  await p.click('button:has-text("Update anyway")'); await p.waitForSelector('dialog[open] .banner.warn');
  await p.check('dialog input[type=checkbox]'); await p.click('dialog button.primary:has-text("Update now")');
  await p.waitForSelector('.banner.ok', {timeout: 30000});
  out.flows.forced = (await p.locator('.banner.ok').innerText()).replace(/\s+/g, ' ');
  // 6. rollback button
  await p.context().close();
  p = await open('update=available');
  await p.click('button:has-text("Roll back")'); await p.waitForSelector('dialog[open]');
  await p.click('dialog button.primary:has-text("Roll back")');
  await p.waitForSelector('.banner.ok', {timeout: 30000});
  out.flows.rollbackDone = (await p.locator('.banner.ok').innerText()).replace(/\s+/g, ' ');
  console.log(JSON.stringify(out)); await b.close();
})();
