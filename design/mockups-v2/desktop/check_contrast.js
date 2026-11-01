// Contrast gate on the prototype's *computed* colours (what the browser really paints), per screen state, light + dark.
//  text:     every visible text node vs the effective (alpha-composited) background of its element: >= 4.5:1 (>= 3:1 for large text: >=24px, or >=18.66px bold)
//  non-text: borders of inputs / selects / segmented controls / checkboxes and focus ring: >= 3:1 vs the surface they sit on
//  Elements with aria-disabled="true" are exempt (WCAG 1.4.3) but counted. Mock mark / dev menu are not product UI and skipped.
// usage: node check_contrast.js [regex] [-v]     exit 1 on any failure
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const path = require('path'), fs = require('fs');
const H = __dirname, list = JSON.parse(fs.readFileSync(path.join(H, 'screens.json'), 'utf8'));
const filt = process.argv[2] && !process.argv[2].startsWith('-') ? new RegExp(process.argv[2]) : null, V = process.argv.includes('-v');
const run = () => {
  const num = '(-?[\\d.]+(?:e-?\\d+)?)';
  function parse(s) {
    let m = s.match(/^rgba?\(([^)]+)\)$/); if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1] }
    m = s.match(/^color\(srgb ([^)]+)\)$/); if (m) { const p = m[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p.length > 3 ? p[3] : 1] }
    return null }
  const over = (f, b) => { const a = f[3]; return [f[0] * a + b[0] * (1 - a), f[1] * a + b[1] * (1 - a), f[2] * a + b[2] * (1 - a), 1] };
  const lin = c => { c /= 255; return c <= .04045 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4 };
  const lum = c => .2126 * lin(c[0]) + .7152 * lin(c[1]) + .0722 * lin(c[2]);
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + .05) / (Math.min(x, y) + .05) };
  const hex = c => '#' + c.slice(0, 3).map(v => Math.round(v).toString(16).padStart(2, '0')).join('');
  const vis = e => { for (let n = e; n && n !== document.documentElement; n = n.parentElement) { const c = getComputedStyle(n); if (c.display === 'none' || c.visibility === 'hidden') return false } const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 };
  function bgOf(e) { // composite background colours from e upward on white; opacity folded into the result
    const chain = []; for (let n = e; n; n = n.parentElement) chain.push(n);
    let bg = [255, 255, 255, 1];
    for (let i = chain.length - 1; i >= 0; i--) { const cs = getComputedStyle(chain[i]); const c = parse(cs.backgroundColor); if (c && c[3] > 0) bg = over(c, bg); const o = parseFloat(cs.opacity); if (i > 0 && o < 1) { /* ancestor opacity: handled by caller */ } }
    return bg }
  function opac(e) { let o = 1; for (let n = e; n; n = n.parentElement) o *= parseFloat(getComputedStyle(n).opacity); return o }
  const layer = document.getElementById('layer');
  const modal = layer.querySelector('.dlg,.sheet');
  const rootSel = modal ? [modal, ...layer.querySelectorAll('.pop,.tip')] : [document.querySelector('.win')];
  const inScope = e => rootSel.some(r => r.contains(e)) && !e.closest('.mockmark,.dev,.toasts' + (modal ? '' : ''));
  const out = { n: 0, exempt: 0, fails: [], min: 99, nt: 0, ntMin: 99 };
  const seen = new Set();
  const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let t = w.nextNode(); t; t = w.nextNode()) {
    if (!t.nodeValue.trim()) continue; const e = t.parentElement; if (!e || e.closest('script,style,svg') || !vis(e) || !inScope(e)) continue;
    if (e.closest('[aria-disabled=true]')) { out.exempt++; continue }
    const cs = getComputedStyle(e); let fg = parse(cs.color); if (!fg) continue;
    const bg = bgOf(e); const o = opac(e); if (o < 1) fg[3] *= o;
    const eff = over(fg, bg); const r = ratio(eff, bg); const px = parseFloat(cs.fontSize), bold = parseInt(cs.fontWeight) >= 700;
    const need = px >= 24 || (px >= 18.66 && bold) ? 3 : 4.5; out.n++; out.min = Math.min(out.min, r);
    if (r < need) { const key = e.tagName + e.className + hex(eff) + hex(bg); if (!seen.has(key)) { seen.add(key); out.fails.push(`text "${t.nodeValue.trim().slice(0, 28)}" <${e.tagName.toLowerCase()}.${String(e.className).slice(0, 20)}> ${hex(eff)} on ${hex(bg)} = ${r.toFixed(2)} < ${need}`) } }
  }
  document.querySelectorAll('input,select,textarea').forEach(e => { // values and placeholders are not text nodes
    if (!vis(e) || !inScope(e) || e.closest('[aria-disabled=true]') || e.type === 'checkbox' || e.type === 'radio' || e.type === 'range') return;
    const ph = e.tagName === 'INPUT' && !e.value && e.placeholder; const cs = ph ? getComputedStyle(e, '::placeholder') : getComputedStyle(e);
    const fg = parse(cs.color); if (!fg) return; const bg = bgOf(e), eff = over(fg, bg), r = ratio(eff, bg); out.n++; out.min = Math.min(out.min, r);
    if (r < 4.5) out.fails.push(`${ph ? 'placeholder' : 'value'} <${e.tagName.toLowerCase()}.${String(e.className).slice(0, 16)}> ${hex(eff)} on ${hex(bg)} = ${r.toFixed(2)} < 4.5`) });
  document.querySelectorAll('.inp:not([aria-disabled=true]),.seg,select,.chk .bx,.sw,input[type=range]').forEach(e => {
    if (!vis(e) || !inScope(e) || e.closest('[aria-disabled=true]')) return;
    const cs = getComputedStyle(e), bc = parse(cs.borderTopColor), bw = parseFloat(cs.borderTopWidth); if (!bc || bw < 1) return;
    const p = e.parentElement, bg = bgOf(p), eff = over(bc, bg), r = ratio(eff, bg); out.nt++; out.ntMin = Math.min(out.ntMin, r);
    if (r < 3) out.fails.push(`border <${e.tagName.toLowerCase()}.${String(e.className).slice(0, 16)}> ${hex(eff)} on ${hex(bg)} = ${r.toFixed(2)} < 3`) });
  // focus ring token vs the page surfaces
  const root = getComputedStyle(document.documentElement); const ring = parse((() => { const d = document.createElement('i'); d.style.color = root.getPropertyValue('--ring'); document.body.appendChild(d); const c = getComputedStyle(d).color; d.remove(); return c })());
  for (const v of ['--canvas', '--surface', '--surface-2']) { const d = document.createElement('i'); d.style.color = root.getPropertyValue(v); document.body.appendChild(d); const c = parse(getComputedStyle(d).color); d.remove(); const r = ratio(ring, c); out.nt++; out.ntMin = Math.min(out.ntMin, r); if (r < 3) out.fails.push(`focus ring vs ${v} = ${r.toFixed(2)}`) }
  return out };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome', args: ['--no-sandbox'] });
  let bad = 0, T = 0, N = 0, NT = 0, ex = 0, mn = 99, nmn = 99;
  for (const s of list) { if (filt && !filt.test(s.n)) continue;
    for (const th of ['light', 'dark']) {
      const p = await b.newPage({ viewport: { width: 1280, height: 800 } });
      await p.goto(`file://${process.env.INDEX||H+"/index.html"}?bare=1&theme=${th}&${s.q}#${s.h}`); await p.waitForTimeout(300);
      const r = await p.evaluate(run); await p.close(); T++; N += r.n; NT += r.nt; ex += r.exempt; mn = Math.min(mn, r.min); nmn = Math.min(nmn, r.ntMin);
      if (r.fails.length) { bad++; console.log(`FAIL ${s.n} ${th}`); r.fails.slice(0, 8).forEach(f => console.log('   ' + f)) } else if (V) console.log(`ok   ${s.n.padEnd(26)} ${th.padEnd(5)} text ${r.n} (min ${r.min.toFixed(2)}) non-text ${r.nt} (min ${r.ntMin.toFixed(2)})`);
    } }
  await b.close();
  console.log(`${T} state renders, ${N} text checks (min ${mn.toFixed(2)}:1), ${NT} non-text checks (min ${nmn.toFixed(2)}:1), ${ex} disabled-exempt; ${bad ? bad + ' renders FAIL' : 'all pass'}`);
  process.exit(bad ? 1 : 0);
})();
