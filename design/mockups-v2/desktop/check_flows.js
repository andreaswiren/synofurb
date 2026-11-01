// Interaction test of the prototype with real mouse / keyboard events (Playwright, scrollbars NOT hidden).
// usage: node check_flows.js [-v]    exit 1 on any failed expectation
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const H = __dirname; const URL = 'file://' + H + '/index.html';
process.on('unhandledRejection', e => { console.log('ERROR', String(e.message).split('\n').slice(0, 6).join(' | ')); process.exit(2) });
let pass = 0, fail = 0; const ok = (c, m) => { c ? pass++ : (fail++, console.log('FAIL ' + m)); if (c && process.argv.includes('-v')) console.log('ok   ' + m) };
let T = 0;
(async () => {
  const b = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome', args: ['--no-sandbox'], ignoreDefaultArgs: ['--hide-scrollbars'] });
  const mk = async (q = '', size = { width: 1280, height: 800 }) => { const p = await b.newPage({ viewport: size }); p.errs = []; p.on('pageerror', e => p.errs.push(e.message)); await p.goto(URL + '?t=' + (++T) + '&fast=1&nofirst=1&bare=1' + (q ? '&' + q : '')); await p.waitForTimeout(200); return p };
  const S = (p, f) => p.evaluate(f);
  const screen = p => p.evaluate(() => window.__S.screen);
  const dlgs = p => p.locator('#layer .dlg').count();
  const txt = (p, s) => p.locator(s).first().innerText();
  const tagText = p => p.locator('.statusbar .tag').first().innerText().catch(() => '');
  const pops = p => p.locator('#layer .pop:visible, #layer .lb:visible').count();

  // 1. main path with clicks; Connect failure states
  let p = await mk();
  await p.locator('[data-sel="ds918"]').click(); ok(await p.locator('tr.sel .rad').count() === 1, 'selected row shows the radio');
  await p.locator('#f-connect').click(); ok(await screen(p) === 'connect', 'Find -> Connect');
  ok(await p.evaluate(() => document.activeElement.tagName === 'H1'), 'focus moves to the page title after navigation');
  await p.locator('[data-act="dosignin"]').click(); ok(await p.locator('.ferr').count() === 1 && await dlgs(p) === 0, 'empty password: inline field error, no host-key dialog');
  await p.locator('#c-pw').fill('wrong'); ok(await p.locator('.ferr').count() === 0, 'typing clears the field error');
  await p.locator('[data-act="dosignin"]').click(); ok(await dlgs(p) === 1, 'unknown host key asks first'); await p.keyboard.press('Escape'); ok(await dlgs(p) === 0, 'Esc closes the dialog');
  await p.locator('[data-act="dosignin"]').click(); await p.locator('#layer .dlg button.primary').click(); await p.waitForTimeout(400);
  ok(await screen(p) === 'connect' && /Wrong password/.test(await txt(p, '.main')), 'wrong password shows an error and stays on Connect');
  await p.locator('#c-pw').fill('x'); await p.locator('[data-act="dosignin"]').click(); await p.waitForTimeout(400); ok(await screen(p) === 'inspect', 'Connect -> Inspect');
  await p.locator('.foot .btn.primary').click(); ok(await screen(p) === 'goal', 'Inspect -> Goal');
  await p.locator('.main .btn:has-text("Customize")').click(); ok(await screen(p) === 'plan', 'Goal -> Setup (Storage)');
  await p.locator('[data-setup-tab="system"]').click(); ok(await screen(p) === 'system' && await p.locator('#s-host').count() === 1, 'Setup > System tab');
  await p.locator('[data-setup-tab="storage"]').click(); await p.locator('#f-next').click(); ok(await screen(p) === 'confirm', 'Setup -> Confirm (6-step wizard)');
  ok(await p.locator('.stepper .item').count() === 6, 'stepper has 6 steps');
  ok(await p.locator('.stepper button').evaluateAll(l => l.every(x => x.tabIndex === -1)), 'stepper buttons are out of the Tab order');
  await p.keyboard.press('Alt+ArrowLeft'); ok(await screen(p) === 'plan', 'Alt+Left goes to the previous completed step'); await p.keyboard.press('Alt+ArrowRight'); ok(await screen(p) === 'confirm', 'Alt+Right goes forward again');
  // gate
  const erase = p.locator('#f-erase'); ok(await erase.getAttribute('aria-disabled') === 'true', 'Erase locked at start');
  ok(await p.locator('.foot .ib[aria-label="Why locked"]').count() === 1, 'locked Erase has a visible info icon next to it');
  await erase.click({ force: true }); ok(await screen(p) === 'confirm' && await pops(p) === 1 && /Missing/.test(await txt(p, '#layer .pop')), 'clicking locked Erase opens a popover listing what is missing');
  await p.keyboard.press('Escape');
  ok(/No backup found/.test(await txt(p, '.main')), 'Confirm leads with the backup fact for this unit');
  const boxes = p.locator('.gate .chk'); for (let i = 0; i < await boxes.count(); i++) await boxes.nth(i).click();
  const ph = p.locator('#g-phrase'); await ph.fill('ERASE ds918+'); ok(await erase.getAttribute('aria-disabled') === 'true', 'wrong-case phrase does not unlock');
  await ph.fill('  ERASE DS918+ '); ok(await erase.getAttribute('aria-disabled') !== 'true', 'surrounding whitespace is trimmed');
  await ph.press('Enter'); ok(await screen(p) === 'confirm', 'Enter in the phrase field does nothing');
  await erase.focus(); await p.keyboard.press('Enter'); await p.waitForTimeout(100); ok(await screen(p) === 'confirm', 'Enter on the focused Erase button does NOT erase');
  await p.keyboard.press('Space'); await p.waitForTimeout(150); ok(await screen(p) === 'install', 'Space on the focused Erase button starts the install');
  // live install: the point of no return and the current step stay in view; counter never exceeds 12
  let maxN = 0, flagSeen = 0, samples = 0; const t0 = Date.now();
  while (Date.now() - t0 < 12000 && await screen(p) === 'install') { const r = await p.evaluate(() => { const m = document.querySelector('.main').getBoundingClientRect(), f = document.querySelector('.pflag'), c = document.body.innerText.match(/Step (\d+) of 12/); if (!f) return null; const fr = f.getBoundingClientRect(); return { vis: fr.top >= m.top && fr.bottom <= m.bottom, n: c ? +c[1] : 0 } }); if (r) { samples++; flagSeen += r.vis ? 1 : 0; maxN = Math.max(maxN, r.n) } await p.waitForTimeout(100) }
  ok(samples > 5 && flagSeen === samples, `No-return marker visible in every sample (${flagSeen}/${samples})`); ok(maxN <= 12 && maxN >= 8, 'step counter never exceeds 12 (max ' + maxN + ')');
  await p.waitForFunction(() => window.__S.screen === 'done', null, { timeout: 8000 }).catch(() => { }); ok(await screen(p) === 'done', 'install simulation ends on Done');
  const fin = p.locator('#f-finish'); ok(await fin.getAttribute('aria-disabled') === 'true' && await p.locator('.foot .ib').count() === 1, 'Finish locked with a visible reason icon');
  await p.locator('[data-act="copy-pw2"]').click(); ok(await fin.getAttribute('aria-disabled') !== 'true', 'Copy password unlocks Finish'); await fin.click(); ok(await screen(p) === 'manage', 'Finish -> Manage');
  ok(/Healthy|Needs attention|Critical|Unreachable/.test(await txt(p, '.main')) && await tagText(p) === 'Debian running', 'Manage opens on Health; risk Tag derived from health');
  ok(p.errs.length === 0, 'no page errors on main path ' + p.errs.join('|')); await p.close();

  // 2. twin protection in both directions; command == plan
  for (const id of ['ds918', 'ds918b']) { p = await mk(`twin=1&unit=${id}&gate=ready#confirm`); const ph_ = await p.evaluate(() => PHRASE()); ok(/Same model nearby/.test(await txt(p, '.main')) && /ERASE DS918\+ \d{4}/.test(ph_), `twin warning and unit-specific phrase for ${id} (${ph_})`); ok(await p.locator('#f-erase').getAttribute('aria-disabled') !== 'true', 'prefilled gate unlocks for ' + id); await p.close(); }
  p = await mk('unit=ds1621&raid=hyb2&copies=2&gate=ready#confirm'); await p.locator('[data-act="cmd"]').click(); const cmd = await txt(p, '#layer .fp'); ok(/--raid hyb2/.test(cmd) && /--copies 2/.test(cmd) && /--mode wipe/.test(cmd) && /--confirm "ERASE DS1621\+"/.test(cmd), 'command carries Hybrid 2-disk, copies and the confirmation'); ok(await p.locator('#layer .tag.ok:has-text("Matches plan")').count() === 1, 'visible check: command matches plan');
  ok(await p.evaluate(() => cmdMatches(buildCmd(true).replace('--raid hyb2', '--raid auto'))) === false, 'a command that differs from the plan is detected'); await p.close();
  p = await mk('unit=ds918&keep=1&sys=8&gate=ready#confirm'); await p.locator('[data-act="cmd"]').click(); ok(/--mode keep/.test(await txt(p, '#layer .fp')) && /--system-size 8G/.test(await txt(p, '#layer .fp')), 'Keep mode is in the command'); await p.close();

  // 3. install: stop before step 3, outcome screens, clocks that end
  p = await mk('unit=ds918&step=2&dlg=cancelinst#install'); await p.locator('#layer .dlg [data-dbtn="1"]').click(); await p.waitForTimeout(200); ok(await screen(p) === 'failed' && /Stopped by you/.test(await txt(p, '.main')) && await tagText(p) === 'Flash changed', 'Stop install -> stopped screen, risk Tag Flash changed'); await p.close();
  p = await mk('unit=ds918&step=6#install'); ok(await p.locator('#f-cancel').getAttribute('aria-disabled') === 'true' && /Disks changed/.test(await tagText(p)), 'Cancel is locked after step 3 and the Tag says Disks changed'); await p.close();
  p = await mk('unit=ds918&fail=pre&clock=2&run=1#failed'); await p.waitForFunction(() => window.__S.fail.phase === 'back', null, { timeout: 9000 }).catch(() => { }); ok(await p.evaluate(() => window.__S.fail.phase) === 'back' && /DSM is back/.test(await txt(p, '.main')), 'pre-failure clock ends in "DSM is back"'); await p.close();
  p = await mk('unit=ds918&fail=pre&clock=2&run=1&out=notback#failed'); await p.waitForFunction(() => window.__S.fail.phase === 'notback', null, { timeout: 9000 }).catch(() => { }); ok(/did not return/.test(await txt(p, '.main')) && await p.locator('.foot .btn:has-text("Check again")').count() === 1, 'DSM not back: power-cycle advice and Check again'); await p.close();
  p = await mk('unit=ds918&fail=post&clock=2&run=1#failed'); ok(await p.locator('[aria-label="Rescue"] [data-copy^="ssh root@"]').count() === 1 && await p.locator('[aria-label="Rescue"] [data-copy="rescue-Tn4k-Qe7"]').count() === 1, 'rescue command and password are persistent and copyable');
  await p.locator('.foot [data-act="rescue"]').click(); ok(await p.locator('[aria-label="Rescue"]').count() === 1, 'Open rescue shell keeps the Rescue panel');
  await p.waitForFunction(() => window.__S.fail.phase === 'loader', null, { timeout: 12000 }).catch(() => { }); ok(/DSM loader/.test(await txt(p, '.main')) && await p.locator('[aria-label="Rescue"]').count() === 0, 'post-failure clock ends at the DSM loader and the shell panel is gone'); await p.close();
  p = await mk('unit=ds918&fail=post&dlg=rescuenotes#failed'); ok(/Debian is installed/.test(await txt(p, '#layer .dlg')), 'rescue notes say Debian is installed'); await p.close();
  p = await mk('unit=ds918&step=9&lost=1#install'); ok(/Connection lost/.test(await txt(p, '.main')), 'install connection-lost state'); await p.locator('[data-act="reconnect"]').click(); ok(await p.locator('.alert.err').count() === 0, 'Reconnect clears it'); await p.close();

  // 4. facts-derived risk Tag and Way back
  const tag = async q => { const x = await mk(q); const t = await tagText(x); await x.close(); return t.trim() };
  ok(await tag('unit=ds918#recovery') === 'Disks changed', 'Recovery after a failed install: Disks changed');
  ok(await tag('unit=ds918&nofacts=1#recovery') === 'Read-only', 'Recovery on an untouched unit: Read-only');
  ok(await tag('unit=ds918&health=unreachable#manage') === 'Unreachable', 'Manage unreachable: Unreachable (never green)');
  ok(await tag('unit=ds918&health=faildisk#manage') === 'Critical', 'Manage failed disk: Critical');
  ok(await tag('unit=ds918#settings') === '' && await tag('unit=ds918#about') === '', 'Settings and About show no risk Tag');
  ok(await tag('unit=ds918&boot=progress#boot') === 'Flash changed', 'Boot manager during install: Flash changed');
  p = await mk('unit=ds918&gate=ready#confirm'); await p.locator('[data-pop="wayback"]').click(); ok(/15-minute rescue shell/.test(await txt(p, '#layer .pop')), 'Way back on Confirm names the 15-minute shell'); await p.close();

  // 5. recovery rows
  p = await mk('unit=ds918#recovery'); ok(await p.locator('.rows .btn.primary').innerText() === 'Open guide', 'primary reflects the state (disks modified: Open guide)');
  for (const [k, re] of [['rec-bootdsm', /Boot DSM/], ['rec-agent', /agent/], ['rec-remove', /Remove our entries/], ['rec-restore', /Restore boot flash/], ['rec-guide', /Reinstall DSM/]]) { await p.locator(`[data-act="${k}"]`).click(); ok(await dlgs(p) === 1 && re.test(await txt(p, '#layer .dlg-h')), `${k} opens its confirmation`); await p.keyboard.press('Escape') }
  await p.locator('[data-act="rec-remove"]').click(); const rb = p.locator('#layer .dlg [data-dbtn="1"]'); ok(await rb.getAttribute('aria-disabled') === 'true', 'Remove entries needs the typed phrase'); await p.locator('#rp').fill('REMOVE ENTRIES'); await rb.click(); await p.waitForTimeout(100); ok(/removed/.test(await txt(p, '.main')), 'Remove entries shows a result in the page'); await p.close();
  p = await mk('unit=ds918&facts=flash,debian#recovery&nofacts=1'); ok(await p.locator('[data-act="rec-remove"]').getAttribute('aria-disabled') === 'true', 'Remove entries is unavailable while Debian boots through our entries'); await p.locator('[data-act="rec-remove"]').click({ force: true }); ok(/Boot DSM first/.test(await txt(p, '#layer .pop')), 'its reason is shown in a popover'); await p.close();

  // 6. Boot manager: confirm, progress, result
  p = await mk('unit=ds918#boot'); await p.locator('[data-act="bootconfirm"]').click(); ok(await dlgs(p) === 1, 'Install agent asks first'); await p.locator('#layer .dlg [data-dbtn="1"]').click();
  await p.waitForFunction(() => window.__S.bootm.phase === 'result', null, { timeout: 8000 }).catch(() => { }); ok(/ssh boot@/.test(await txt(p, '.main')) && await p.locator('.tag:has-text("30 min watchdog")').count() === 1, 'result shows the ssh command and the 30 minute watchdog'); await p.locator('[data-act="returndsm"]').click(); await p.locator('#layer .dlg [data-dbtn="1"]').click(); ok(/rebooting to DSM/.test(await txt(p, '.main')), 'Return to DSM has an outcome'); await p.close();

  // 7. Manage
  p = await mk('unit=ds918&health=unreachable#manage'); ok(await p.locator('[data-act="mgmtupdate"]').getAttribute('aria-disabled') === 'true' && await p.locator('.mgmt .ib[aria-label="Why unavailable"]').count() >= 1, 'unreachable: Update disabled with a visible reason icon'); ok(/Check power, cable and network/.test(await txt(p, '.main')) && await p.locator('.alrow .btn:has-text("Retry")').count() === 1, 'unreachable: next step and Retry'); await p.close();
  p = await mk('unit=ds918&health=svcdown#manage'); await p.locator('[data-svc]').first().click(); await p.locator('#layer .dlg [data-dbtn="1"]').click(); await p.waitForTimeout(100); ok(await p.locator('[data-svc]').count() === 0 && await tagText(p) === 'Debian running', 'Restart service has an outcome and clears the warning'); await p.close();
  p = await mk('unit=ds918&health=rebuild&dlg=mgmtupdate#manage'); ok(/degraded or rebuilding/.test(await txt(p, '#layer .dlg')), 'update while an array rebuilds shows a warning but is allowed'); await p.locator('#layer .dlg .btn.primary').click(); await p.waitForFunction(() => window.__S.manage.mgmt === 'updated', null, { timeout: 8000 }).catch(() => { }); ok(await p.evaluate(() => window.__S.manage.mgmt) === 'updated', 'update completes'); await p.close();
  p = await mk('unit=ds918&health=faildisk#manage'); ok(await p.locator('[data-act="mgmtupdate"]').evaluate(e => !e.classList.contains('primary')) && await p.locator('[data-act="openweb"]').evaluate(e => e.classList.contains('primary')), 'Critical: Update is not primary, Open web UI is'); ok(await p.locator('[data-dlg="oplan"]').count() === 0 && !/Replace/.test(await txt(p, '.main')), 'no desktop Replace dialog or button'); await p.close();
  p = await mk('unit=ds918&health=faildisk&tab=storage#manage'); ok(await p.evaluate(() => { const r = [...document.querySelectorAll('tr')].find(x => /^sdc\b/.test(x.innerText.trim())); return !!r && /md0/.test(r.innerText) && /md1/.test(r.innerText) }), 'Storage lists every array a disk belongs to (sdc in md0 and md1)'); ok(await p.locator('button:has-text("Create array")').count() === 0, 'no disabled Create array'); await p.close();
  p = await mk('unit=ds1621&layout=hyb&tab=storage#manage'); ok(await p.locator('.tag:has-text("Hybrid 1-disk")').count() === 1 && await p.locator('.tbl tbody tr').count() >= 8, 'Manage Storage shows a Hybrid layout with members'); await p.close();

  // 8. tooltips, popovers, dialogs, sheet, select, text size, F1
  p = await mk('unit=ds918#plan'); const ib = p.locator('.ib').first(); await ib.hover(); await p.waitForTimeout(300); ok(await p.locator('.tip:visible').count() === 1, 'hover shows a tooltip'); await p.mouse.move(5, 400); await p.waitForTimeout(200); ok(await p.locator('.tip:visible').count() === 0, 'tooltip hides on leave');
  await p.locator('#pl-raid').click(); ok(await p.locator('#layer .lb [role=option]').count() === 9 && await p.locator('#pl-raid').getAttribute('aria-expanded') === 'true', 'Select opens its popup list'); await p.keyboard.press('ArrowDown'); await p.keyboard.press('Enter'); await p.waitForTimeout(100); ok(await p.evaluate(() => window.__S.plan.raid) === 'none' && await pops(p) === 0, 'Select: arrow keys and Enter choose an option'); await p.locator('#pl-raid').click(); await p.keyboard.press('Escape'); ok(await pops(p) === 0, 'Esc closes the Select');
  await p.keyboard.press('Escape'); await p.locator('body').click({ position: { x: 600, y: 400 } }).catch(() => { }); await p.keyboard.press('F1'); await p.waitForTimeout(200); ok(await p.locator('#layer .sheet').count() === 1 && /RAID levels/.test(await txt(p, '#layer .sheet-b')), 'F1 on Setup opens the RAID levels topic');
  for (let i = 0; i < 25; i++) await p.keyboard.press('Tab'); ok(await p.evaluate(() => !!document.activeElement.closest('.sheet')), 'Tab stays inside the sheet'); await p.keyboard.press('Escape'); ok(await p.locator('#layer .sheet').count() === 0, 'Esc closes the sheet');
  await p.keyboard.press('F1'); await p.locator('[data-help-back]').click(); await p.locator('#hq').pressSequentially('raid'); ok(await p.evaluate(() => document.activeElement.id) === 'hq' && await p.locator('#hb .help-list li').count() < 14, 'Help search keeps focus and filters'); await p.close();
  p = await mk('unit=ds918#settings'); const h1 = () => p.locator('h1').evaluate(e => parseFloat(getComputedStyle(e).fontSize)); const a = await h1(); await p.locator('[data-set-size="Largest"]').click(); const bb = await h1(); ok(Math.abs(bb / a - 1.3) < 0.02, `Text size Largest scales the window (${a}px -> ${bb}px)`); await p.locator('[data-set-size="Default"]').click(); ok(Math.abs(await h1() - a) < 0.5, 'and Default restores it'); await p.close();
  p = await mk('unit=ds918#goal'); await p.locator('[data-act="quick"]').click(); ok(/Quick setup/.test(await txt(p, '#layer .dlg-h')), 'Quick setup shows the plan summary'); await p.locator('#layer .dlg .btn.primary').click(); await p.waitForTimeout(100); ok(await screen(p) === 'confirm', 'Quick setup jumps to Confirm'); await p.close();

  // 9. scrollbars are real, one scroll container per state, no horizontal scroll at 960x640 (and at Largest for five states)
  const list = require('./screens.json'); const BIG = new Set(['find', 'plan', 'confirm-ready', 'install', 'failed-post']);
  p = await mk('unit=ds918#confirm', { width: 960, height: 640 }); const gw = await p.evaluate(() => { const m = document.querySelector('.main'); return m.offsetWidth - m.clientWidth }); ok(gw >= 12 && gw <= 24, 'scrollbar gutter is reserved and drawn (' + gw + 'px)'); await p.close();
  p = await mk('', { width: 960, height: 640 }); let hs = 0, multi = 0, clipped = 0;
  for (const s of list) { if (s.n === 'dev-menu') continue; for (const extra of ['', ...(BIG.has(s.n) ? ['&size=largest'] : [])]) { await p.goto(`${URL}?t=${++T}&bare=1&${s.q}${extra}#${s.h}`); await p.waitForTimeout(/sheet=|dlg=/.test(s.q) ? 350 : 100);
      const r = await p.evaluate(() => { const win = document.querySelector('.win'), m = document.querySelector('.main'), sb = document.querySelector('.sheet-b'); const h = (m && m.scrollWidth > m.clientWidth) || document.documentElement.scrollWidth > document.documentElement.clientWidth || win.scrollWidth > win.clientWidth || (sb && sb.scrollWidth > sb.clientWidth);
        const sc = [...document.querySelectorAll('.win *')].filter(e => { if (e.closest('#layer,.mockmark,.dev,svg')) return false; const cs = getComputedStyle(e); return /(auto|scroll)/.test(cs.overflowY) && e.scrollHeight > e.clientHeight + 1 }).length;
        const foot = document.querySelector('.foot'), fr = foot && foot.getBoundingClientRect(), bad = [...document.querySelectorAll('.foot .btn')].some(e => { const r = e.getBoundingClientRect(); return r.right > window.innerWidth + 1 || r.left < -1 });
        return { h, sc, bad } });
      if (r.h) { hs++; console.log('HSCROLL ' + s.n + extra) } if (r.sc > 1) { multi++; console.log('MULTI-SCROLL ' + s.n + extra + ' ' + r.sc) } if (r.bad) { clipped++; console.log('FOOTER CLIPPED ' + s.n + extra) } } }
  ok(hs === 0, `no horizontal scroll on ${list.length} states at 960x640 (+130% on five)`); ok(multi === 0, 'at most one scroll container per state'); ok(clipped === 0, 'action bar buttons are never clipped'); await p.close();
  p = await mk('', { width: 960, height: 640 }); p.errs.length; await p.close();
  p = await mk('unit=ds918#confirm'); await p.keyboard.press('Control+Shift+D'); ok(await p.locator('.dev').count() === 1, 'Ctrl+Shift+D opens the dev menu'); await p.keyboard.press('Control+Shift+D'); await p.locator('.mockmark button').click(); ok(await p.locator('.mockmark:visible').count() === 0, 'mock mark can be hidden'); await p.close();
  await b.close(); console.log(`${pass} passed, ${fail} failed`); process.exit(fail ? 1 : 0);
})();
