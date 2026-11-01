// Counts the visible static words per screen state (data spans .d, the mock mark, dev menu and toasts are excluded) and fails over the budget.
// usage: node check_words.js [--size=960x640]   exit 1 on any violation
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const path = require('path'), fs = require('fs');
const H = __dirname, list = JSON.parse(fs.readFileSync(path.join(H, 'screens.json'), 'utf8'));
const size = (process.argv.find(a => a.startsWith('--size=')) || '--size=1280x800').slice(7).split('x').map(Number);
const BUDGET = 60, LIM = { title: 4, sub: 12, btn: 3, lab: 4, dlg: 90, pop: 60, sheetTopic: 150 };
const countJS = () => {
  const PROSE = /\b(is|are|was|were|has|have|will|can|may|stay|stays|return|returns|erased|changed|failed|refused|installed|needs|keeps|runs|closes|opens|timed|rolled|answers|expects)\b/i;
  const LIM = { title: 4, sub: 12, btn: 3, lab: 4 };
  const W = s => (s.replace(/[·—–×+\/|•]/g, ' ').match(/[^\s]*[A-Za-z0-9][^\s]*/g) || []).length;
  const vis = e => { for (let n = e; n && n !== document.body; n = n.parentElement) { const c = getComputedStyle(n); if (c.display === 'none' || c.visibility === 'hidden') return false } const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 };
  function words(root, skip) {
    let n = 0; const w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let t = w.nextNode(); t; t = w.nextNode()) { const p = t.parentElement; if (p && p.closest('.d') && !p.closest('.mockmark,.dev,.toasts,.tip,.pop,script,style,svg,[data-skip]') && (!skip || !p.closest(skip)) && vis(p)) { const dd = p.closest('.d'); if (!dd.__pw) { const tx = dd.textContent.trim(); dd.__pw = (W(tx) > 3 && PROSE.test(tx)) ? 1 : 2; } if (dd.__pw === 1 && !dd.__c) { dd.__c = 1; n += W(dd.textContent); } continue; }
    if (!p || p.closest('.d,.mockmark,.dev,.toasts,.tip,.pop,script,style,svg,[data-skip]') || (skip && p.closest(skip))) continue; if (!vis(p)) continue; n += W(t.nodeValue) }
    root.querySelectorAll('input[placeholder]').forEach(i => { if (!i.value && vis(i) && !i.closest(skip || '.none')) n += W(i.placeholder) });
    root.querySelectorAll('select').forEach(s => { if (vis(s) && !s.closest(skip || '.none')) n += W(s.options[s.selectedIndex].text) });
    return n }
  const win = document.querySelector('.win'), layer = document.getElementById('layer');
  const out = { screen: words(win, '#layer'), issues: [] };
  const clip = (sel, lim, what) => document.querySelectorAll(sel).forEach(e => { if (!e.closest('#layer') && vis(e)) { const k = words(e); if (k > lim) out.issues.push(`${what} "${e.textContent.trim().slice(0, 40)}" has ${k} words (max ${lim})`) } });
  clip('.t-title', LIM.title, 'title'); clip('.t-sub', LIM.sub, 'subtitle'); clip('.main .btn:not(.icon),.foot .btn:not(.icon)', LIM.btn, 'button'); clip('.fld>.lab', LIM.lab, 'label');
  out.dialogs = [...layer.querySelectorAll('.dlg')].map(d => ({ t: d.querySelector('.dlg-h').textContent.trim(), n: words(d) }));
  out.sheet = layer.querySelector('.sheet') ? words(layer.querySelector('.sheet .sheet-b')) : 0;
  return out };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome', args: ['--no-sandbox'] });
  let bad = 0; const rows = [];
  for (const s of list) {
    if (s.n === 'dev-menu' || s.n === 'specimens') continue;
    const p = await b.newPage({ viewport: { width: size[0], height: size[1] } });
    await p.goto(`file://${H}/index.html?bare=1&theme=light&${s.q}#${s.h}`); await p.waitForTimeout(250);
    const r = await p.evaluate(countJS); await p.close();
    const probs = [...r.issues]; if (r.screen > BUDGET) probs.push(`screen ${r.screen} > ${BUDGET}`);
    r.dialogs.forEach(d => { if (d.n > LIM.dlg) probs.push(`dialog "${d.t}" ${d.n} > ${LIM.dlg}`) });
    if (r.sheet > 400) probs.push(`sheet ${r.sheet}`);
    rows.push([s.n, r.screen, r.dialogs.map(d => d.n).join(','), r.sheet || '', probs.length ? 'FAIL ' + probs.join('; ') : 'ok']); if (probs.length) bad++;
  }
  { const p = await b.newPage(); await p.goto(`file://${H}/index.html?bare=1&nofirst=1#find`); const tw = await p.evaluate(() => HELP.map(t => [t.id, t.text.trim().split(/\s+/).length])); await p.close();
    const over = tw.filter(([, n]) => n > 150); over.forEach(([id, n]) => { bad++; rows.push([`help:${id}`, n, '', '', `FAIL help topic ${n} > 150`]) }); console.log(`Help topics: ${tw.length}, longest ${Math.max(...tw.map(x => x[1]))} words (limit 150)`); }
  await b.close();
  console.log(`Visible words per screen state at ${size.join('x')} (budget ${BUDGET}; shell included; data excluded)`);
  console.log('state'.padEnd(28) + 'words'.padEnd(7) + 'dialog'.padEnd(8) + 'sheet'.padEnd(7) + 'result');
  rows.forEach(r => console.log(String(r[0]).padEnd(28) + String(r[1]).padEnd(7) + String(r[2]).padEnd(8) + String(r[3]).padEnd(7) + r[4]));
  console.log(bad ? `${bad} state(s) over budget` : `all ${rows.length} states within budget; max ${Math.max(...rows.map(r => r[1]))} words`); process.exit(bad ? 1 : 0);
})();
