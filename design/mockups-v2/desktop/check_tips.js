// Tooltips are hover-only in gpui-component, so they may sit only on controls that are already focusable (as non-essential extras).
// Every other sentence must be visible text or sit on a focusable info button / tag. usage: node check_tips.js ; exit 1 on any offender.
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const fs = require('fs'), path = require('path'); const H = __dirname; const list = JSON.parse(fs.readFileSync(path.join(H, 'screens.json'), 'utf8'));
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome', args: ['--no-sandbox'] });
  const bad = new Map(); let n = 0;
  for (const s of list) { const p = await b.newPage({ viewport: { width: 1280, height: 800 } }); await p.goto(`file://${H}/index.html?bare=1&${s.q}#${s.h}`); await p.waitForTimeout(250);
    const r = await p.evaluate(() => { const foc = e => e.matches('button,a[href],input,select,textarea,[tabindex]:not([tabindex="-1"])') || !!e.closest('button,a[href]'); const out = [];
      document.querySelectorAll('[data-tip]').forEach(e => { if (e.closest('.mockmark,.dev')) return; if (!foc(e)) out.push((e.className || e.tagName) + ': ' + e.dataset.tip.slice(0, 60)) });
      return { n: document.querySelectorAll('[data-tip]').length, out, focusTip: !!document.querySelector('#tooltip') } });
    n += r.n; r.out.forEach(o => { if (!bad.has(o)) bad.set(o, []); bad.get(o).push(s.n) });
    // keyboard focus must not raise a tooltip (the widget has no focus path)
    const f = await p.evaluate(() => { const e = document.querySelector('.titlebar button'); if (e) e.focus(); return null }); await p.keyboard.press('Tab'); await p.waitForTimeout(550);
    if (await p.locator('.tip:visible').count()) bad.set('focus shows a tooltip', [s.n]);
    await p.close() }
  await b.close(); for (const [k, v] of bad) console.log('NON-FOCUSABLE TOOLTIP ' + k + '  (' + v.length + ' states, e.g. ' + v[0] + ')');
  console.log(`${n} tooltips in ${list.length} states; ${bad.size} offender(s)`); process.exit(bad.size ? 1 : 0);
})();
