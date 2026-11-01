// Type scale, weights and radii audit (review B8): every visible text node must map to a design-system style, every box to a DS radius.
// DS: proportional 12/14/16/24 px, mono 12/13 px, weights 400/600/700, radii 4/8/12/pill. Measured at Default text size (rem = 16).
// usage: node check_scale.js [regex]   exit 1 on any violation
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const fs = require('fs'), path = require('path'); const H = __dirname; const list = JSON.parse(fs.readFileSync(path.join(H, 'screens.json'), 'utf8'));
const filt = process.argv[2] ? new RegExp(process.argv[2]) : null;
const run = () => {
  const out = new Map(); const add = (k, e) => { if (!out.has(k)) out.set(k, { n: 0, ex: (e.className && e.className.baseVal === undefined ? String(e.className) : e.tagName).slice(0, 40) + ' "' + (e.textContent || '').trim().slice(0, 24) + '"' }); out.get(k).n++ };
  const vis = e => { for (let n = e; n && n !== document.documentElement; n = n.parentElement) { const c = getComputedStyle(n); if (c.display === 'none' || c.visibility === 'hidden') return false } const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 };
  const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let t = w.nextNode(); t; t = w.nextNode()) { if (!t.nodeValue.trim()) continue; const e = t.parentElement; if (!e || e.closest('script,style,svg,.mockmark,.dev') || !vis(e)) continue;
    const cs = getComputedStyle(e), px = Math.round(parseFloat(cs.fontSize) * 100) / 100, mono = /JetBrains|mono/i.test(cs.fontFamily), wt = parseInt(cs.fontWeight);
    const okSize = mono ? [12, 13].includes(px) : [12, 14, 16, 24].includes(px); if (!okSize) add(`size ${px}px${mono ? ' mono' : ''}`, e); if (![400, 600, 700].includes(wt)) add(`weight ${wt}`, e);
    if (px === 12 && wt === 700) add('12px bold', e) }
  for (const e of document.querySelectorAll('.win *')) { if (e.closest('svg,.mockmark,.dev') || !vis(e)) continue; const cs = getComputedStyle(e); const hasBox = parseFloat(cs.borderTopWidth) > 0 || (cs.backgroundColor && !/rgba\(0, 0, 0, 0\)|transparent/.test(cs.backgroundColor)); if (!hasBox) continue;
    for (const v of [cs.borderTopLeftRadius, cs.borderTopRightRadius, cs.borderBottomLeftRadius, cs.borderBottomRightRadius]) { if (v === '0px') continue; const px = parseFloat(v); const pill = v.includes('%') || px >= 999 || Math.abs(px - Math.min(e.offsetWidth, e.offsetHeight) / 2) < 1.5; if (![4, 8, 12].includes(px) && !pill) add('radius ' + v, e) } }
  return [...out.entries()] };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome', args: ['--no-sandbox'], ignoreDefaultArgs: ['--hide-scrollbars'] });
  const agg = new Map(); let states = 0;
  for (const s of list) { if (filt && !filt.test(s.n) || /size=/.test(s.q)) continue; const p = await b.newPage({ viewport: { width: 1280, height: 800 } }); await p.goto(`file://${H}/index.html?bare=1&${s.q}#${s.h}`); await p.waitForTimeout(250); states++;
    for (const [k, v] of await p.evaluate(run)) { const a = agg.get(k) || { n: 0, ex: v.ex, st: new Set() }; a.n += v.n; a.st.add(s.n); agg.set(k, a) } await p.close() }
  await b.close();
  for (const [k, v] of [...agg.entries()].sort()) console.log(`${k.padEnd(22)} ${String(v.n).padStart(5)} nodes in ${v.st.size} states  e.g. ${v.ex}`);
  console.log(agg.size ? `${agg.size} off-scale value(s) in ${states} states` : `type scale, weights and radii all on the design system (${states} states)`); process.exit(agg.size ? 1 : 0);
})();
