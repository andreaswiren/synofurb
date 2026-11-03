// Runtime audit of the web UI, run by tests/test_ui.py against the self-contained demo (needs node + playwright + a chromium build).
// usage: node ui_audit.js <demo-url-without-query>   -> prints one JSON document; exit 77 when no browser can be started.
const { chromium } = require('playwright');
const BASE = process.argv[2];
const LIB = require('./ui_lib.js');
const PAGES = ['dashboard', 'storage', 'raid', 'mounts', 'addons', 'certs', 'proxy', 'network', 'users', 'boot', 'system', 'updates', 'help', 'about'];
// words a person reads by default: headings, labels, column headers, buttons, prose. Data (values from the system) is not counted; row buttons count once per label.
// Manifest prose (add-on summaries, data-d="prose") is not exempt: it is counted separately in `man` and capped on its own (see MANIFEST_BUDGET in test_ui.py).
const COUNT = `(() => {
  const DATA = '.mono,.val,.sub,.chip,.badge,dd,.avatar,.ver,progress,.pr,.num,#crumb,.brand small,.me b,td,.log,pre,[data-d]';
  const SKIP = '#toasts,.sprite,#tip,script,style,.vh,.skip,#live,#demo-ribbon,#demo-ov';
  const out = [], rowBtn = new Set(), man = [];
  const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  let n;
  while (n = w.nextNode()) {
    const t = n.nodeValue.trim(); if (!t) continue;
    const el = n.parentElement;
    if (!el.checkVisibility({checkVisibilityCSS: true}) || el.closest(SKIP)) continue;
    if (el.closest('[popover]:not(:popover-open)') || el.closest('dialog:not([open])') || el.closest('details:not([open]) > :not(summary)')) continue;
    if (el.closest('[data-d=prose]') && !el.closest('tbody')) { man.push(t); continue; }
    if (el.closest('tbody')) { if (el.closest('button,a')) rowBtn.add(t.toLowerCase()); continue; }
    if (el.closest(DATA) && !el.closest('button,a,th,label,h1,h2,h3,summary')) continue;
    if (el.closest('.arr,.stack>.card,.acard')) { rowBtn.add(t.toLowerCase()); continue; }       // repeated cards: each label counts once
    out.push(t);
  }
  const ws = s => s.split(/\\s+/).filter(x => /[A-Za-z]/.test(x));
  return {n: out.flatMap(ws).concat([...rowBtn].flatMap(ws)).length, text: out.concat([...rowBtn]), man: man.flatMap(ws).length, manText: man};
})()`;
(async () => {
  let b;
  try { b = await chromium.launch(); } catch (e) { console.error(String(e)); process.exit(77); }
  const res = {pages: {}, errors: [], unresolved: [], overflow: [], fonts: [], login: null, stray: [], addonStates: {}, problems: []};
  for (const unit of ['ds918', 'ds1621']) for (const [w, h] of [[1280, 800], [390, 844]]) {
    const ctx = await b.newContext({viewport: {width: w, height: h}}); const p = await ctx.newPage();
    p.on('pageerror', e => res.errors.push('pageerror: ' + e.message));
    p.on('console', m => { if (['error', 'warning'].includes(m.type())) res.errors.push(m.type() + ': ' + m.text()); });
    p.on('requestfailed', r => res.errors.push('requestfailed ' + r.url()));
    p.on('request', r => { if (!/^(file|data|blob):/.test(r.url())) res.errors.push('external request ' + r.url()); });
    if (unit === 'ds918' && w > 700) {
      await p.goto(BASE + '?login=1&unit=' + unit); await p.waitForSelector('input[name=username]'); await p.waitForTimeout(250);
      res.login = await p.evaluate(COUNT);
      res.fonts = await p.evaluate(async () => { await document.fonts.ready; return [...document.fonts].filter(f => f.status === 'loaded').map(f => f.family + ' ' + f.weight); });
    }
    await p.goto(BASE + '?unit=' + unit); await p.waitForSelector('.shell');
    for (let i = 0; i < PAGES.length; i++) {
      if (w > 700) await p.click('.nav button:nth-child(' + (i + 1) + ')'); else { await p.click('.burger'); await p.click('.side .nav button:nth-child(' + (i + 1) + ')'); }
      await p.waitForTimeout(450);
      const r = await p.evaluate(COUNT); const key = PAGES[i];
      for (const x of await p.evaluate(LIB.STRAY)) res.stray.push(unit + ' ' + w + ' ' + key + ' ' + x);
      if (w > 700 && unit === 'ds918') res.pages[key] = r;
      else if (r.n > (res.pages[key] || {n: 0}).n) res.pages[key] = r;
      const bad = await p.evaluate(() => [...document.querySelectorAll('use')].map(u => u.getAttribute('href')).filter(h => !document.querySelector(h)));
      if (bad.length) res.unresolved.push(key + ': ' + [...new Set(bad)]);
      if (await p.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1)) res.overflow.push(unit + ' ' + w + ' ' + key);
    }
    await ctx.close();
  }
  // the other states of the Add-ons page (Docker missing, every status, problems): words, manifest prose, stray text, reasons, overflow
  for (const state of ['nodocker', 'states', 'problems']) for (const [w, h] of [[1280, 800], [390, 844]]) {
    const ctx = await b.newContext({viewport: {width: w, height: h}}); const p = await ctx.newPage();
    p.on('pageerror', e => res.errors.push('pageerror: ' + e.message));
    await p.goto(BASE + '?unit=ds918&addons=' + state); await p.waitForSelector('.shell');
    if (w > 700) await p.click('.nav button:has-text("Add-ons")'); else { await p.click('.burger'); await p.click('.side .nav button:has-text("Add-ons")'); }
    await p.waitForSelector('h1:has-text("Add-ons")'); await p.waitForTimeout(450);
    const r = await p.evaluate(COUNT);
    if (w > 700) res.addonStates[state] = r;
    for (const x of await LIB.problems(p)) res.problems.push('addons=' + state + ' ' + w + ': ' + x);
    await ctx.close();
  }
  console.log(JSON.stringify(res)); await b.close();
})();
