// Drives the Certificates and Hostnames pages of the demo like a person would (run by tests/test_edge.py; needs node + playwright + chromium).
// usage: node ui_edge.js <demo-url-without-query>  -> prints one JSON document; exit 77 when no browser can be started.
const { chromium } = require('playwright');
const LIB = require('./ui_lib.js');
const BASE = process.argv[2];
(async () => {
  let b;
  try { b = await chromium.launch(); } catch (e) { console.error(String(e)); process.exit(77); }
  const out = {errors: [], flows: {}, problems: []};
  const open = async (q, page, w = 1280, h = 900) => {
    const ctx = await b.newContext({viewport: {width: w, height: h}}); const p = await ctx.newPage();
    p.on('pageerror', e => out.errors.push('pageerror: ' + e.message));
    p.on('console', m => { if (['error', 'warning'].includes(m.type())) out.errors.push(m.type() + ': ' + m.text()); });
    p.on('request', r => { if (!/^(file|data|blob):/.test(r.url())) out.errors.push('external request ' + r.url()); });
    await p.goto(BASE + '?' + q); await p.waitForSelector('.shell');
    if (page) {
      if (w < 700) { await p.click('.burger'); await p.click('.side .nav button:has-text("' + page + '")'); } else await p.click('.nav button:has-text("' + page + '")');
      await p.waitForSelector('h1:has-text("' + page + '")'); await p.waitForTimeout(350);
    }
    return p;
  };
  const check = async (p, n) => { for (const x of await LIB.problems(p)) out.problems.push(n + ': ' + x); };
  const why = (p, sel) => p.evaluate(s => { const b = document.querySelector(s), d = b && document.getElementById(b.getAttribute('aria-describedby') || '-');
    return b ? {off: b.getAttribute('aria-disabled') === 'true', native: b.disabled, text: d ? d.textContent.trim() : null, visible: !!d && d.checkVisibility(), focusable: b.tabIndex >= 0} : null; }, sel);
  const jobDone = p => p.waitForFunction(() => /Close/.test((document.querySelector('dialog .df button.primary') || {}).textContent || ''), null, {timeout: 25000});

  // 1. the list: tags, challenge, expiry; no horizontal scroll at phone width
  let p = await open('certs=states', 'Certificates');
  await check(p, 'list');
  out.flows.rows = await p.evaluate(() => [...document.querySelectorAll('tbody tr')].map(r => ({name: r.querySelector('b').textContent, tags: [...r.querySelectorAll('.badge')].map(x => x.textContent.trim()), challenge: r.querySelector('.chip') && r.querySelector('.chip').textContent.trim(),
    buttons: [...r.querySelectorAll('button')].map(x => x.getAttribute('aria-label')), detail: (r.querySelector('.adet') || {}).textContent || ''})));
  out.flows.autoRenewChip = await p.locator('.ph .chip').first().innerText();
  await p.context().close();
  p = await open('certs=states', 'Certificates', 390, 844);
  await check(p, 'list-phone');
  out.flows.listPhoneOverflow = await p.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1);
  await p.context().close();

  // 2. not set up: Issue is off with a visible reason, a banner says what to do
  p = await open('certs=unset', 'Certificates');
  await check(p, 'unset');
  out.flows.unsetBanner = await p.locator('.banner.warn').first().innerText();
  await p.click('main button:has-text("Issue certificate")'); await p.waitForSelector('dialog[open]');
  await p.fill('#ce-names', 'files.example.org');
  await p.check('dialog input[value="dns-01"]'); await p.waitForTimeout(100);
  out.flows.ownSecretField = await p.locator('dialog #ce-own2').count();
  await p.fill('dialog #ce-own2', 'short'); await p.click('dialog .dsec button:has-text("Save")'); await p.waitForTimeout(300);
  await p.fill('dialog #ce-own2', 'my-own-secret-0123456789-abcdef'); await p.click('dialog .dsec button:has-text("Save")'); await p.waitForSelector('dialog .cn');
  out.flows.ownSecretSaved = await p.locator('dialog .cn').count();
  out.flows.unsetReason = await why(p, 'dialog .df .btn.primary');
  await check(p, 'unset-dialog');
  await p.locator('dialog .df .btn.primary').focus();
  out.flows.unsetFocus = await p.evaluate(() => document.activeElement.textContent.includes('Issue'));
  await p.locator('dialog .df .btn.primary').dispatchEvent('click');
  out.flows.unsetClickIgnored = (await p.locator('dialog[open] progress').count()) === 0;
  await p.context().close();

  // 3. issue with DNS: names, reason lines, the CNAME with Copy, Check DNS (wrong, missing, ok), then the job
  p = await open('certs=', 'Certificates');
  await p.click('main button:has-text("Issue certificate")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(200);
  out.flows.emptyNamesReason = await why(p, 'dialog .df .btn.primary');
  out.flows.challengeHelps = await p.evaluate(() => [...document.querySelectorAll('dialog .popwrap > button')].map(x => x.getAttribute('aria-label')));
  await p.fill('#ce-names', 'new.example.org old.example.org'); await p.waitForTimeout(150);
  out.flows.cnames = await p.evaluate(() => [...document.querySelectorAll('dialog .cn')].map(c => ({record: c.querySelector('code').textContent, target: c.querySelectorAll('code')[1] && c.querySelectorAll('code')[1].textContent})));
  out.flows.copyLabels = await p.evaluate(() => [...document.querySelectorAll('dialog .cn button')].map(x => x.getAttribute('aria-label')));
  out.flows.issueOnWithName = (await why(p, 'dialog .df .btn.primary')).off === false;
  await p.click('dialog button:has-text("Check DNS")'); await p.waitForSelector('dialog .cn .badge'); await p.waitForTimeout(200);
  out.flows.dnsStates = await p.evaluate(() => [...document.querySelectorAll('dialog .cn')].map(c => (c.querySelector('.badge') || {}).textContent.trim()));
  await check(p, 'issue-dns');
  await p.fill('#ce-names', 'files.example.org'); await p.waitForTimeout(150);
  await p.click('dialog .df .btn.primary'); await p.waitForSelector('dialog progress');
  await jobDone(p);
  out.flows.jobTitle = await p.locator('dialog h3').innerText();
  await check(p, 'job');
  await p.keyboard.press('Enter'); await p.waitForTimeout(400);
  out.flows.issuedRows = await p.locator('tbody tr').count();
  await p.context().close();

  // 4. issue with HTTP: what must be reachable, the pre-check, the wildcard refusal
  p = await open('certs=', 'Certificates');
  await p.click('main button:has-text("Issue certificate")'); await p.waitForSelector('dialog[open]');
  await p.fill('#ce-names', 'files.example.org'); await p.check('dialog input[value="http-01"]'); await p.waitForTimeout(150);
  out.flows.httpHint = await p.locator('dialog .dsec p[data-d=prose]').first().innerText();
  await p.click('dialog button:has-text("Pre-check")'); await p.waitForSelector('dialog .pre-r .badge'); await p.waitForTimeout(150);
  out.flows.precheck = await p.evaluate(() => document.querySelector('dialog .pre-r').innerText);
  await check(p, 'issue-http');
  await p.fill('#ce-names', '*.example.org'); await p.click('dialog .df .btn.primary'); await p.waitForSelector('dialog .err:not([hidden])');
  out.flows.wildcardRefused = await p.locator('dialog .err').innerText();
  await p.context().close();

  // 5. settings: a generated secret is shown once with Copy; a new one asks first
  p = await open('certs=', 'Certificates');
  await p.click('main button:has-text("Settings")'); await p.waitForSelector('dialog[open]');
  out.flows.settingsTarget = await p.locator('dialog code[data-d]').first().innerText();
  await check(p, 'settings');
  await p.click('dialog button:has-text("Generate secret")'); await p.waitForSelector('dialog[open] + dialog[open], dialog[open] .df button:has-text("Generate secret")');
  out.flows.regenAsks = (await p.locator('dialog[open]').count()) === 2;
  await p.locator('dialog[open]').last().locator('.df button:has-text("Generate secret")').click(); await p.waitForTimeout(500);
  out.flows.secretShown = await p.evaluate(() => { const c = [...document.querySelectorAll('dialog .cn code')].map(x => x.textContent); return c.length === 1 && c[0].length >= 32; });
  await p.context().close();

  // 6. renew, details, remove with the typed name (and the reason beside the off button)
  p = await open('certs=states', 'Certificates');
  await p.click('tbody tr:has-text("old.example.org") button[aria-label="Details"]'); await p.waitForSelector('dialog[open]');
  out.flows.details = await p.locator('dialog').innerText();
  await check(p, 'details');
  await p.keyboard.press('Escape'); await p.waitForTimeout(150);
  await p.click('tbody tr:has-text("media.example.org") button[aria-label="Remove"]'); await p.waitForSelector('dialog[open]');
  out.flows.removeOff = await why(p, 'dialog .df .btn.danger');
  await check(p, 'remove-dialog');
  await p.fill('dialog input.mono', 'media.example.org'); await p.waitForTimeout(100);
  out.flows.removeOn = (await why(p, 'dialog .df .btn.danger')).off === false;
  await p.click('dialog .df .btn.danger'); await p.waitForTimeout(700);
  out.flows.removedRows = await p.locator('tbody tr:has-text("media.example.org")').count();
  await p.click('tbody tr:has-text("nas.example.org") button[aria-label="Renew now"]'); await p.waitForSelector('dialog progress'); await jobDone(p);
  out.flows.renewTitle = await p.locator('dialog h3').innerText();
  await p.context().close();

  // 7. hostnames: reasons beside Enable/Apply, the explanation dialog with the recovery command, the job, then Keep
  p = await open('certs=', 'Hostnames');
  await check(p, 'proxy');
  await p.fill('tbody tr:first-child input.mono', 'nas.example.org'); await p.waitForTimeout(150);
  out.flows.noCertReason = await why(p, 'main .row .btn.primary');
  await p.selectOption('tbody tr:first-child select', 'nas.example.org'); await p.waitForTimeout(150);
  out.flows.coveredOn = (await why(p, 'main .row .btn.primary')).off === false;
  await p.fill('tbody tr:nth-child(2) input.mono', 'portainer.other.net'); await p.selectOption('tbody tr:nth-child(2) select', 'nas.example.org'); await p.waitForTimeout(150);
  out.flows.coverReason = await why(p, 'main .row .btn.primary');
  await p.fill('tbody tr:nth-child(2) input.mono', 'portainer.example.org'); await p.waitForTimeout(150);
  out.flows.wildcardOk = (await why(p, 'main .row .btn.primary')).off === false;
  await p.fill('tbody tr:nth-child(2) input.mono', 'nas.example.org'); await p.waitForTimeout(150);
  out.flows.dupReason = (await why(p, 'main .row .btn.primary')).text;
  await p.fill('tbody tr:nth-child(2) input.mono', 'portainer.example.org'); await p.waitForTimeout(150);
  await p.click('main button:has-text("Check")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(150);
  out.flows.checkDialog = await p.locator('dialog .banner').innerText();
  await p.keyboard.press('Escape'); await p.waitForTimeout(150);
  await p.click('main button:has-text("Enable front")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(150);
  out.flows.enableText = await p.locator('dialog .db').innerText();
  await check(p, 'enable-dialog');
  await p.click('dialog .df .btn.primary'); await p.waitForSelector('dialog progress'); await jobDone(p);
  out.flows.addresses = await p.locator('dialog .chips .chip').allInnerTexts();
  await check(p, 'enable-job');
  await p.keyboard.press('Enter'); await p.waitForTimeout(700);
  out.flows.pending = await p.locator('#pend-edge .banner').innerText();
  out.flows.pendingButtons = await p.locator('#pend-edge button').allInnerTexts();
  await check(p, 'pending');
  await p.click('#pend-edge button:has-text("Keep")'); await p.waitForTimeout(600);
  out.flows.keptGone = (await p.locator('#pend-edge .banner').count()) === 0;
  out.flows.applyShown = await p.locator('main button:has-text("Apply")').count() === 1 && await p.locator('main button:has-text("Disable front")').count() === 1;
  await p.click('main button:has-text("Disable front")'); await p.waitForSelector('dialog[open]');
  out.flows.disableText = await p.locator('dialog .db').innerText();
  await p.click('dialog .df .btn.danger'); await p.waitForSelector('dialog progress'); await jobDone(p);
  await p.keyboard.press('Enter'); await p.waitForTimeout(700);
  out.flows.offAgain = await p.locator('main button:has-text("Enable front")').count();
  await p.context().close();

  // 8. an unconfirmed change on another page; Revert; phone width
  p = await open('proxy=pending', 'Certificates');
  out.flows.bannerOnOtherPage = await p.locator('#pend-edge .banner').innerText();
  await check(p, 'pending-banner');
  await p.click('#pend-edge button:has-text("Revert")'); await p.waitForTimeout(500);
  out.flows.revertedGone = (await p.locator('#pend-edge .banner').count()) === 0;
  await p.context().close();
  p = await open('proxy=pending', 'Hostnames', 390, 844);
  await check(p, 'proxy-phone');
  out.flows.proxyPhoneOverflow = await p.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1);
  await p.click('main button:has-text("Apply")'); await p.waitForSelector('dialog[open]'); await p.waitForTimeout(200);
  await check(p, 'apply-dialog-phone');
  out.flows.dialogPhoneOverflow = await p.evaluate(() => { const d = document.querySelector('dialog[open]'); const r = d.getBoundingClientRect(); return r.left < -1 || r.right > innerWidth + 1 || d.scrollWidth > d.clientWidth + 1; });
  await p.context().close();
  p = await open('certs=', 'Certificates', 390, 844);
  await p.click('main button:has-text("Issue certificate")'); await p.waitForSelector('dialog[open]'); await p.fill('#ce-names', '*.example.org nas.example.org'); await p.waitForTimeout(200);
  await check(p, 'issue-phone');
  out.flows.issuePhone = await p.evaluate(() => { const d = document.querySelector('dialog[open]'), r = d.getBoundingClientRect(); return {overflow: d.scrollWidth > d.clientWidth + 1 || r.right > innerWidth + 1, cnameOutside: [...d.querySelectorAll('.cn code')].filter(c => c.getBoundingClientRect().right > r.right + 1).length}; });
  await p.context().close();

  console.log(JSON.stringify(out)); await b.close();
})();
