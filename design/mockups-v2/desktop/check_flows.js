// Interaction test of the prototype with real mouse / keyboard events (Playwright). Every D1-D18 screen must be reachable by clicking,
// tooltips / popovers / dialogs / sheet / accordions must work, the data-loss gate must gate, the install must run to Done.
// usage: node check_flows.js     exit 1 on any failed expectation
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const path = require('path'); const H = __dirname; const URL = 'file://' + H + '/index.html';
process.on("unhandledRejection", e => { console.log("ERROR", String(e.message).split("\n")[0]); process.exit(2) });
let pass = 0, fail = 0; const ok = (c, m) => { c ? pass++ : (fail++, console.log('FAIL ' + m)); if (c && process.argv.includes('-v')) console.log('ok   ' + m) };
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome', args: ['--no-sandbox'] });
  const mk = async (q = 'fast=1&nofirst=1&bare=1', size = { width: 1280, height: 800 }) => { const p = await b.newPage({ viewport: size }); const errs = []; p.on('pageerror', e => errs.push(e.message)); p.errs = errs; await p.goto(URL + '?' + q); await p.waitForTimeout(200); return p };
  const screen = p => p.evaluate(() => window.__S.screen);
  const btn = (p, t) => p.locator('.main button:visible, .foot button:visible, .foot a:visible').filter({ hasText: t }).first();
  const dlgOpen = p => p.locator('#layer .dlg').count();

  // 1. main path by clicking only: Find -> Connect -> Inspect -> Goal -> Plan -> System -> Confirm -> Install -> Done -> Manage
  let p = await mk();
  ok(await screen(p) === 'find', 'starts on Find');
  await p.locator('[data-sel="ds918"]').click(); await btn(p, 'Connect').click(); ok(await screen(p) === 'connect', 'Find -> Connect');
  await p.locator('#c-pw').fill('x'); await btn(p, 'Connect').click(); await p.waitForTimeout(300);
  ok(await dlgOpen(p) === 1, 'unknown host key asks first (dialog)'); await p.keyboard.press('Escape'); ok(await dlgOpen(p) === 0, 'Esc closes the dialog');
  await btn(p, 'Connect').click(); await p.locator('#layer .dlg [data-act="trust"], #layer .dlg button.primary').first().click(); await p.waitForTimeout(500);
  ok(await screen(p) === 'inspect', 'Connect -> Inspect after trusting host key');
  for (const [lbl, s] of [['Connect', 'goal'], ['Plan install', 'plan'], ['Next: System', 'system'], ['Next: Confirm', 'confirm']]) { const n = lbl === 'Plan install' ? p.locator('.main .btn:has-text("Plan install")') : p.locator('.foot .btn.primary:visible').first(); await n.click(); await p.waitForTimeout(150); ok(await screen(p) === s, `-> ${s}`) }
  // gate: erase must stay locked until all three boxes + phrase
  const erase = p.locator('[data-act="erase"]'); ok(await erase.getAttribute('aria-disabled') === 'true', 'Erase locked at start');
  await erase.click(); ok(await screen(p) === 'confirm', 'clicking locked Erase does nothing');
  await erase.hover(); await p.waitForTimeout(600); ok(await p.locator('.tip:visible').count() === 1 && /Tick|Type|Confirm|box/i.test(await p.locator('.tip:visible').innerText()), 'locked Erase explains why in a tooltip: ' + (await p.locator('.tip:visible').innerText().catch(() => '-')).replace(/\s+/g, ' '));
  const boxes = p.locator('.gate .chk'); for (let i = 0; i < await boxes.count(); i++) await boxes.nth(i).click();
  ok(await erase.getAttribute('aria-disabled') === 'true', 'three boxes alone do not unlock');
  const ph = p.locator('.gate input[type=text], .gate input:not([type])').first(); await ph.fill('ERASE ds918+'); ok(await erase.getAttribute('aria-disabled') === 'true', 'wrong-case phrase does not unlock');
  await ph.fill('ERASE DS918+'); ok(await erase.getAttribute('aria-disabled') !== 'true', 'phrase + boxes unlock');
  await ph.press('Enter'); ok(await screen(p) === 'confirm', 'Enter in the phrase field does not erase');
  await boxes.nth(1).click(); ok(await erase.getAttribute('aria-disabled') === 'true', 'unticking relocks');
  await boxes.nth(1).click(); await erase.click(); await p.waitForTimeout(150); ok(await screen(p) === 'install', 'Erase -> Install');
  await p.waitForFunction(() => window.__S.screen === 'done', null, { timeout: 15000 }).catch(() => { }); ok(await screen(p) === 'done', 'install simulation ends on Done');
  const fin = p.locator('[data-act="finish"]'); ok(await fin.getAttribute('aria-disabled') === 'true', 'Finish locked until the password is copied');
  await p.locator('[data-act="copy-pw"]').click(); ok(await fin.getAttribute('aria-disabled') !== 'true', 'Copy password unlocks Finish');
  await fin.click(); ok(await screen(p) === 'manage', 'Finish -> Manage');
  ok(await p.locator('.hcards, .toprow').count() > 0 && /Healthy|Needs attention|Critical|Unreachable/.test(await p.locator('.main').innerText()), 'Manage opens on the Health view with an overall status');
  ok(p.errs.length === 0, 'no page errors on main path ' + p.errs.join('|')); await p.close();

  // 2. tooltips: hover and keyboard focus; popover; accordion; stepper navigation
  p = await mk('fast=1&nofirst=1&bare=1&unit=ds918#plan');
  const ib = p.locator('.ib').first(); await ib.hover(); await p.waitForTimeout(600); ok(await p.locator('.tip:visible').count() === 1, 'hover shows tooltip after delay');
  await p.mouse.move(5, 400); await p.waitForTimeout(250); ok(await p.locator('.tip:visible').count() === 0, 'tooltip hides on leave');
  await p.locator('[data-tip]').first().focus(); await p.waitForTimeout(300); ok(await p.locator('.tip:visible').count() === 1, 'keyboard focus shows tooltip');
  await p.locator('[data-pop="raid"]').first().click(); ok(await p.locator('#layer .pop:visible').count() === 1, 'info popover opens'); await p.keyboard.press('Escape'); ok(await p.locator('#layer .pop:visible').count() === 0, 'Esc closes popover');
  await p.locator('[data-dlg="slices"], .btn:has-text("Slices"), .link:has-text("Slices")').first().click().catch(() => { });
  await p.locator('.stepper [data-go="inspect"], .stepper button:has-text("Inspect")').first().click(); ok(await screen(p) === 'inspect', 'stepper jumps back to Inspect'); await p.close();

  // 3. dialog focus trap + Esc returns focus
  p = await mk('fast=1&nofirst=1&bare=1&unit=ds918#connect');
  await p.locator('.main [data-help-open], .link:has-text("How to")').first().click().catch(() => { });
  await p.keyboard.press('Escape'); await p.keyboard.press('F1'); await p.waitForTimeout(200); ok(await p.locator('#layer .sheet').count() === 1, 'F1 opens the Help sheet');
  ok(await p.evaluate(() => !!document.activeElement.closest('.sheet')), 'focus moves into the sheet');
  for (let i = 0; i < 25; i++) await p.keyboard.press('Tab'); ok(await p.evaluate(() => !!document.activeElement.closest('.sheet')), 'Tab stays inside the sheet (focus trap)');
  await p.keyboard.type('raid'); await p.waitForTimeout(150); const n = await p.locator('#hb .help-list li').count(); ok(n >= 1 && n < 14, `Help search filters topics (${n} of 14)`);
  await p.keyboard.press('Escape'); ok(await p.locator('#layer .sheet').count() === 0, 'Esc closes the sheet'); await p.close();

  // 4. management: update -> progress -> updated -> roll back; failed state; unreachable disables actions
  p = await mk('fast=1&nofirst=1&bare=1&unit=ds918&mgmt=&health=ok#manage');
  await p.locator('[data-act="mgmtupdate"]').first().click(); ok(await dlgOpen(p) === 1, 'Update opens the dialog (what restarts, rollback)');
  const dt = await p.locator('#layer .dlg').innerText(); ok(/restart/i.test(dt) && /roll/i.test(dt), 'dialog explains restart and auto-rollback');
  await p.locator('#layer .dlg .btn.primary').click(); await p.waitForTimeout(400); ok(/Upload|Install|Restart|Health/.test(await p.locator('.main').innerText()), 'progress steps visible while updating');
  await p.waitForFunction(() => window.__S.manage.mgmt === 'updated', null, { timeout: 8000 }).catch(() => { }); ok(await p.evaluate(() => window.__S.manage.mgmt) === 'updated', 'update completes');
  await p.locator('[data-act="rollback"]').first().click(); ok(await dlgOpen(p) === 1, 'Roll back opens a confirm dialog'); await p.keyboard.press('Escape'); await p.close();
  p = await mk('fast=1&nofirst=1&bare=1&unit=ds918&health=unreachable#manage'); ok(/Unreachable/.test(await p.locator('.main').innerText()), 'unreachable state shows status'); await p.close();

  // 5. every screen reachable from the menu / header + about, settings, boot manager, recovery
  p = await mk('fast=1&nofirst=1&bare=1&unit=ds918');
  await p.locator('[aria-label="Settings"]').click(); ok(await screen(p) === 'settings', 'Settings from the title bar'); await p.locator('.main button:has-text("Close"), .foot button:has-text("Close")').first().click();
  await p.locator('[data-menu]').click(); await p.waitForTimeout(100); const items = await p.locator('#layer [role=menuitem]').allInnerTexts(); ok(items.length >= 3, 'app menu lists items: ' + items.join(' / '));
  await p.locator('#layer [role=menuitem]').filter({ hasText: 'About' }).click(); ok(await screen(p) === 'about', 'menu -> About'); await p.locator('.main .btn:has-text("Close"), .foot .btn:has-text("Close"), .foot .btn:has-text("Back")').first().click().catch(() => { });
  await p.goto(URL + '?fast=1&nofirst=1&bare=1&unit=ds918#goal'); await p.waitForTimeout(150);
  await p.locator('.main .btn:has-text("Set up")').click(); ok(await screen(p) === 'boot', 'Goal -> Boot manager');
  await p.goto(URL + '?fast=1&nofirst=1&bare=1&unit=ds918#goal'); await p.waitForTimeout(150);
  await p.locator('.main .btn:has-text("Open recovery")').click(); ok(await screen(p) === 'recovery', 'Goal -> Recovery');
  await p.locator('[data-rec="restore"]').first().click(); ok(await dlgOpen(p) === 1, 'Restore opens a dialog'); await p.keyboard.press('Escape');
  await p.goto(URL + '?fast=1&nofirst=1&bare=1&unit=ds918#install'); await p.waitForTimeout(100);
  await p.goto(URL + '?fast=1&nofirst=1&bare=1&unit=ds918&fail=post#failed'); await p.waitForTimeout(150); ok(await screen(p) === 'failed', 'Failed screen renders');
  await p.locator('[data-dlg="next"], .link:has-text("What happens next")').first().click(); ok(await dlgOpen(p) === 1, 'Failed -> What happens next dialog'); await p.keyboard.press('Escape');
  await p.keyboard.press('Escape'); await p.close();

  // 6. dev menu hidden; mock mark hideable; no horizontal scroll at 960x640 on every screen
  p = await mk('fast=1&nofirst=1&unit=ds918'); ok(await p.locator('.dev').count() === 0, 'dev menu absent by default'); await p.keyboard.press('Control+Shift+D'); ok(await p.locator('.dev').count() === 1, 'Ctrl+Shift+D opens dev menu');
  await p.keyboard.press('Control+Shift+D'); await p.locator('.mockmark button').click(); ok(await p.locator('.mockmark:visible').count() === 0, 'mock mark can be hidden'); await p.close();
  const list = require('./screens.json'); p = await mk('fast=1&nofirst=1&bare=1', { width: 960, height: 640 }); let hs = 0;
  for (const s of list) { await p.goto(`${URL}?bare=1&${s.q}#${s.h}`); await p.waitForTimeout(120); if (await p.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth || document.querySelector('.win').scrollWidth > document.querySelector('.win').clientWidth)) { hs++; console.log('HSCROLL ' + s.n) } }
  ok(hs === 0, `no horizontal scroll at 960x640 on ${list.length} states`); await p.close();
  await b.close(); console.log(`${pass} passed, ${fail} failed`); process.exit(fail ? 1 : 0);
})();
