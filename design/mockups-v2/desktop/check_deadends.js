// "Every button either does something visible or is absent": clicks every enabled button of every distinct screen state and fails when
// nothing visible changed (screen, page markup, dialog / popover / toast layer).  usage: node check_deadends.js [regex]
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const fs = require('fs'), path = require('path'); const H = __dirname; const list = JSON.parse(fs.readFileSync(path.join(H, 'screens.json'), 'utf8'));
const filt = process.argv[2] ? new RegExp(process.argv[2]) : null;
const SEL = '.main button, .foot button, .titlebar button, .unithdr button, .statusbar button, .top button';
const SKIP = 'aria-disabled="true"';
const sigf = () => { const h = x => { let a = 5381; for (let i = 0; i < x.length; i++) a = ((a << 5) + a + x.charCodeAt(i)) | 0; return a }; const q = s => { const e = document.querySelector(s); return e ? h(e.innerHTML) : 0 }; return JSON.stringify([window.__S.screen, q('.main'), q('.foot'), q('#layer'), q('.statusbar'), document.querySelectorAll('.toast').length, document.documentElement.dataset.theme, document.documentElement.dataset.size || '', document.activeElement && (document.activeElement.id || document.activeElement.tagName)]) };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome', args: ['--no-sandbox'], ignoreDefaultArgs: ['--hide-scrollbars'] });
  const seen = new Set(), dead = []; let n = 0;
  for (const s of list) { if (/dlg=|sheet=|pop=|select=|reason=|tip=|dev=|ptext=/.test(s.q) || s.n === 'specimens' || (filt && !filt.test(s.n))) continue; const key = s.h + '?' + s.q.replace(/nofirst=1&?/, ''); if (seen.has(key)) continue; seen.add(key);
    const p = await b.newPage({ viewport: { width: 1280, height: 800 } }); let rr = 0; const U = () => `file://${H}/index.html?r=${++rr}&fast=1&bare=1&${s.q}#${s.h}`; const url = U();
    await p.goto(U()); await p.waitForTimeout(250);
    const labels = await p.evaluate(({ SEL, SKIP }) => [...document.querySelectorAll(SEL)].map((e, i) => ({ i, t: (e.getAttribute('aria-label') || e.textContent.trim()).slice(0, 30), ok: e.offsetParent !== null && !e.hasAttribute('aria-disabled') && !e.closest('.stepper,.dev,[hidden]') && !e.disabled && e.getAttribute('aria-pressed') !== 'true' && e.getAttribute('aria-selected') !== 'true' && e.getAttribute('aria-current') !== 'true' })).filter(x => x.ok), { SEL, SKIP });
    for (const L of labels) { await p.goto(U()); await p.waitForTimeout(200); const before = await p.evaluate(sigf);
      const hit = await p.evaluate(({ SEL, i }) => { const e = document.querySelectorAll(SEL)[i]; if (!e) return false; e.click(); return true }, { SEL, i: L.i }); if (!hit) continue; await p.waitForTimeout(40); const mid = await p.evaluate(sigf); await p.waitForTimeout(110); const after = await p.evaluate(sigf); n++;
      if (before === after && before === mid) dead.push(`${s.n}: "${L.t}"`) }
    await p.close() }
  await b.close(); dead.forEach(d => console.log('DEAD ' + d)); console.log(`${n} button clicks on ${seen.size} states; ${dead.length} dead end(s)`); process.exit(dead.length ? 1 : 0);
})();
