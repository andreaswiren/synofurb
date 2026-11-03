// Shared checks for the browser tests and tools/shots.js (needs playwright for the self-test only).
//   STRAY    stray text a person must never read: the words null, undefined, NaN, Infinity, "[object ...]" and unresolved "{name}" placeholders,
//            in visible text and in aria-label / title / placeholder. Log and code blocks (pre) carry system output and are skipped.
//   REASONS  every control that is off (aria-disabled or disabled) must give its reason as visible text that is reachable without a mouse:
//            an aria-describedby target with text that is visible, or (for a native disabled control) it is reported as a failure, because
//            a disabled button cannot be focused and shows no tooltip to keyboard or touch users.
//   OVERFLOW the page and every open dialog fit the viewport width.
// usage as a script: node ui_lib.js selftest   -> prints JSON, exit 0
const STRAY = `(() => {
  const BAD = /(^|[^A-Za-z0-9_.\\/-])(null|undefined|NaN|Infinity|\\[object [A-Za-z]*\\]|\\{[a-z_]+\\})([^A-Za-z0-9_-]|$)/;
  const out = [];
  const w = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  let n;
  while (n = w.nextNode()) {
    const el = n.parentElement;
    if (!el || el.closest('script,style,pre,#tip,.sprite,textarea')) continue;
    if (!el.checkVisibility({checkVisibilityCSS: true})) continue;
    const t = n.nodeValue.trim(); if (t && BAD.test(t)) out.push('text: ' + t.slice(0, 80));
  }
  for (const el of document.querySelectorAll('[aria-label],[title],[placeholder]')) {
    if (el.closest('pre,.sprite')) continue;
    for (const a of ['aria-label', 'title', 'placeholder']) { const v = el.getAttribute(a); if (v && BAD.test(v)) out.push(a + ': ' + v.slice(0, 80)); }
  }
  return out;
})()`;
const REASONS = `(() => {
  const out = [];
  for (const b of document.querySelectorAll('button,a.btn,input,select,textarea')) {
    if (!b.checkVisibility({checkVisibilityCSS: true}) || b.closest('.sprite,#demo-ribbon')) continue;
    const aria = b.getAttribute('aria-disabled') === 'true', nat = b.disabled === true;
    if (!aria && !nat) continue;
    const name = (b.getAttribute('aria-label') || b.textContent || b.id || b.tagName).trim().slice(0, 40);
    if (nat) { out.push('native disabled (not focusable, no reason): ' + name); continue; }
    const ids = (b.getAttribute('aria-describedby') || '').split(/\\s+/).filter(Boolean);
    const d = ids.map(i => document.getElementById(i)).find(e => e && e.id !== 'tip');
    if (!d) { out.push('no aria-describedby reason: ' + name); continue; }
    const r = d.getBoundingClientRect();
    if (!d.checkVisibility({checkVisibilityCSS: true}) || !d.textContent.trim() || r.width < 4) out.push('reason not visible: ' + name);
    else if (r.right > innerWidth + 1 || r.left < -1) out.push('reason outside the viewport: ' + name);
    if (b.tabIndex < 0) out.push('not focusable: ' + name);
  }
  return out;
})()`;
const OVERFLOW = `(() => {
  const out = [];
  if (document.documentElement.scrollWidth > innerWidth + 1) out.push('page ' + document.documentElement.scrollWidth + ' > ' + innerWidth);
  for (const d of document.querySelectorAll('dialog[open]')) {
    const r = d.getBoundingClientRect();
    if (r.left < -1 || r.right > innerWidth + 1) out.push('dialog outside the viewport');
    if (d.scrollWidth > d.clientWidth + 1) out.push('dialog scrolls sideways');
    for (const e of d.querySelectorAll('.chip,.badge,.btn,.banner,.chips')) { const q = e.getBoundingClientRect(); if (q.width && q.right > r.right + 1) out.push('overflows the dialog: ' + (e.textContent || '').trim().slice(0, 40)); }
  }
  return out;
})()`;
async function problems(p) {
  return [].concat(await p.evaluate(STRAY), await p.evaluate(REASONS), await p.evaluate(OVERFLOW));
}
module.exports = {STRAY, REASONS, OVERFLOW, problems};
if (require.main === module && process.argv[2] === 'selftest') {
  const { chromium } = require('playwright');
  (async () => {
    let b;
    try { b = await chromium.launch(); } catch (e) { console.error(String(e)); process.exit(77); }
    const p = await b.newPage();
    const run = async html => { await p.setContent(html); return {stray: await p.evaluate(STRAY), reasons: await p.evaluate(REASONS)}; };
    const res = {
      null: await run('<ul><li>Health</li></ul>null'),
      undef: await run('<p>Updated to undefined.</p>'),
      obj: await run('<p>[object Object]</p>'),
      tpl: await run('<p>Install {name} first.</p>'),
      attr: await run('<button aria-label="Help: undefined">?</button>'),
      native: await run('<button disabled>Go</button>'),
      noreason: await run('<button aria-disabled="true">Go</button>'),
      clean: await run('<p id="r">Check first.</p><button aria-disabled="true" aria-describedby="r">Go</button><p>Nullable and non-null are words, a null-terminated string too.</p>'),
    };
    console.log(JSON.stringify(res)); await b.close();
  })();
}
