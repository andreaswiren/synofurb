/* ===== dialogs, actions, router, init ===== */
const REPL_PHRASE='RESTORE FLASH',REMOVE_PHRASE='REMOVE ENTRIES';
const typed=(id,phrase,go)=>({id,onopen:api=>{const i=$('#rp',api.el),b=api.btn(1);i.addEventListener('input',()=>{const ok=i.value.trim()===phrase;b.setAttribute('aria-disabled',!ok);ok?b.removeAttribute('data-reason'):b.setAttribute('data-reason','Type the phrase first.')})},initial:'#rp'});
const mixedDisks=u=>{const m=u.disks.map(x=>x[2]);return Math.max(...m)/Math.min(...m)>=1.1};
const defaultsPlan=(u=cur())=>({mode:'wipe',sys:32,copies:3,raid:mixedDisks(u)&&capacity('hyb1',u.disks).usable>capacity('auto',u.disks).usable*1.2?'hyb1':'auto',free:20});
const DLG={
 firstlaunch:()=>({id:'first',title:'Before you start',icon:'shield-check',body:'<p>synofurb only reads until you confirm.</p><p>Make a tested backup first.</p><p>Screen readers are not supported in this window; the same steps work from the command line.</p>',buttons:[{label:'Open Help',fn:()=>{setTimeout(()=>openHelp('cli'),0)}},{label:'Got it',cls:'primary'}]}),
 addip:()=>({id:'addip',title:'Add NAS',icon:'plus',body:'<div class="row gap12"><div class="fld grow"><span class="lab">Address</span><input class="inp mono" placeholder="192.168.1.x" aria-label="Address"></div><div class="fld" style="width:5rem"><span class="lab">Port</span><input class="inp mono" value="5000" aria-label="Port"></div></div>',buttons:[{label:'Cancel'},{label:'Add',cls:'primary',fn:()=>toast('Probing 192.168.1.x…','search')}]}),
 hostkey:()=>{const fp='SHA256:q3Zb1c8mPvX0dT7nKfL2yHrW9sUeJ5aG4oB6iNxVwYc';return {id:'hostkey',title:'Trust this NAS?',icon:'fingerprint-pattern',body:`<p>First connection to ${mono(cur().ip)}. Compare this fingerprint with DSM (Control Panel, Terminal and SNMP). If you did not expect a new key, cancel.</p><div class="fp"><span>SHA256:<br>${fp.slice(7,29)}<br>${fp.slice(29)}</span><button class="btn ghost icon sm" data-copy="${fp}" aria-label="Copy fingerprint"${tip('Copy')}>${ic('copy','s')}</button></div>`,buttons:[{label:'Cancel',fn:()=>{S.conn.state='idle';render()}},{label:'Trust',cls:'primary',fn:()=>{S.conn.trusted=true;startSignin(true)}}]}},
 hostchanged:()=>({id:'hostchg',title:'Host key changed',icon:'octagon-x',danger:true,body:`<p>${mono(cur().ip)} now answers with a different key. That can mean DSM was reinstalled, or that another device took this address.</p><p>Do not sign in until you have checked the unit. synofurb will not continue.</p>`,buttons:[{label:'Cancel',cls:'primary',fn:()=>{S.conn.err='hostchanged';render()}}]}),
 keep:()=>({id:'keep',title:'Keep existing volumes',icon:'triangle-alert',body:'<p>Keep mode reuses only DSM’s system and swap area, about 10 GiB per disk; the system is capped near 9.5 GiB.</p><p>DSM, its settings, users and packages are erased. Your data volume is left alone on a best-effort basis, with no guarantee.</p><p>It is not mounted automatically; import it read-only later.</p>',buttons:[{label:'Cancel'},{label:'Use Keep',cls:'primary',fn:()=>{S.plan.mode='keep';S.plan.sys=8;render()}}]}),
 slices:()=>{const cap=capacity(S.plan.raid,cur().disks);return {id:'slices',title:'Hybrid slices',icon:'layers',wide:true,body:`<table class="tbl sm"><thead><tr><th>Slice</th><th>Disks</th><th>Level</th><th class="r">Usable</th></tr></thead><tbody>${cap.slices.map(s=>`<tr><td>${mono(s.id)}</td><td>${d(s.mem.map(i=>cur().disks[i][0]).join(' '))}</td><td>${s.L==='unused'?tag('Unused',{i:'circle-minus',t:'A slice on one disk has no partner and stays unused.'}):d(s.L)}${s.weak?' '+tag('Weaker',{v:'warn',i:'triangle-alert'}):''}</td><td class="r">${mono(s.u?TiB(s.u):'—')}</td></tr>`).join('')}</tbody></table><p class="sub">Each slice present on two or more disks becomes one array; LVM joins them into one volume. Built by synofurb with mdadm and LVM; DSM may not adopt it, and nothing here has run on a real Synology.</p>`,buttons:[{label:'Close',cls:'primary'}]}},
 recovery:()=>({id:'rec',title:'How recovery works',icon:'life-buoy',body:'<p>DSM stays the default boot entry. Before installer step 3 (the point of no return), a failure or cancel reboots the NAS into DSM within 60 seconds.</p><p>From step 3, a failure leaves a rescue shell open for 15 minutes, then the NAS reboots into the DSM loader, which shows Synology’s install page. DSM’s own system may be gone by then; reinstall it from Synology. We promise nothing about your data.</p>',buttons:[{label:'Close',cls:'primary'}]}),
 next:()=>{const f=S.fail||{kind:'post'};const t=f.kind==='backup'?'<p>Nothing was written: the boot flash and the disks are as they were, and DSM is still running.</p><p>Go back to Confirm to try again, or open Recovery to check the unit.</p>':f.kind==='agent'?'<p>The app waited 10 minutes and cannot tell what happened. If the agent started, its watchdog reboots the NAS into DSM, 20 minutes or more from now.</p><p>If it never started, the NAS may keep booting the agent after every power cycle. Then choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or restore the boot flash from another computer.</p><p>DSM is not confirmed. Your disks are untouched. See Help, Recovery.</p>':f.kind!=='post'?'<p>No disk was changed. The NAS sets DSM as the default and reboots in 60 seconds, then DSM starts as before. If DSM is not back after 10 minutes, power-cycle the NAS.</p><p>Our entries stay on the boot flash until you remove them under Recovery.</p>':f.keep?'<p>The first disk was changed. Export diagnostics, then open the rescue shell. At zero the NAS reboots into the DSM loader and Synology’s install page.</p><p>Your data volume was not part of any write command and should be intact unless the failure was a hardware problem. We cannot guarantee it.</p>':'<p>The first disk was changed and DSM’s own system may be gone. Export diagnostics first, then open the rescue shell.</p><p>At zero DSM becomes the default and the NAS reboots into the DSM loader, which shows Synology’s install page. Reinstall DSM from Synology. A Wipe install leaves no data to keep.</p>';return {id:'next',title:'What happens next',icon:'clock',body:t,buttons:f.kind==='agent'?[{label:'Close'},{label:'Recovery options',fn:()=>{go('recovery')}},{label:'Manual recovery',cls:'primary',fn:()=>{setTimeout(()=>openDialogByName('manualrec'),0)}}]:[{label:'Close',cls:'primary'}]}},
 manualrec:()=>{const u=cur(),bf=backupPath(u),cmds='sudo synofurb-dom default dsm --yes\nsudo SYNOFURB_DOM=/dev/sdX synofurb-dom restore --yes '+bf;return {id:'mrec',title:'Manual recovery',icon:'life-buoy',wide:true,body:`<p>This needs hands on the NAS. Your data is untouched.</p><ol style="margin-left:1.25rem"><li>With a serial or VGA console, choose SYNOLOGY_2 in the boot menu, then run the first command on DSM.</li><li>Or connect the boot flash to another Linux computer and run the second command. Set SYNOFURB_DOM to the 128 MB flash (check it twice).</li></ol><div class="logbox d">${cmds.split('\n').map(c=>`<div><span class="m">${esc(c)}</span></div>`).join('')}</div>`,buttons:[{label:'Close'},{label:'Copy commands',icon:'copy',fn:()=>copy(cmds,'Commands copied'),keep:true},{label:'Open guide',cls:'primary',icon:'external-link',fn:()=>toast('Opens Help, Recovery','external-link')}]}},
 rescuenotes:()=>{const f=S.fail||{step:11},late=f.step>=11,cmds='journalctl -b -u synofurb-agent\ncat /var/log/synofurb-install.log\nlsblk -f\nmdadm --detail --scan';return {id:'rnotes',title:'Rescue notes',icon:'file-text',wide:true,body:`<p>${late?'Debian is installed under /target; only the boot entry failed. Export diagnostics first, then look at the log.':'The disks are partly changed. Export diagnostics first, then look at the log.'}</p><div class="logbox d">${cmds.split('\n').map(c=>`<div><span class="m">${esc(c)}</span></div>`).join('')}</div><p class="sub">The shell closes when the clock ends.</p>`,buttons:[{label:'Copy commands',icon:'copy',fn:()=>copy(cmds,'Commands copied'),keep:true},{label:'Close',cls:'primary'}]}},
 reinstall:()=>({id:'reinst',title:'Reinstall DSM',icon:'life-buoy',body:'<p>The NAS boots Synology’s loader, which shows an install page. Open find.synology.com or Synology Assistant, find the unit and install DSM with the .pat file for your model.</p><p>Synology documents a reinstall that can keep data volumes; check its current documentation. This project cannot guarantee it.</p>',buttons:[{label:'Close'},{label:'Open find.synology.com',cls:'primary',icon:'external-link',fn:()=>toast('Opens find.synology.com in your browser','external-link')}]}),
 restore:()=>({...typed('rest',REPL_PHRASE),title:'Restore boot flash?',icon:'hard-drive',danger:true,body:`<p>This writes the saved image ${mono(backupPath(cur()))} back to the boot flash. It needs a working Linux on the unit, such as the agent or Debian.</p><div class="fld"><span class="lab">Type <span class="code">${REPL_PHRASE}</span> to continue</span><input class="inp mono" id="rp" autocomplete="off" aria-label="Type the phrase"></div>`,buttons:[{label:'Cancel'},{label:'Restore flash',cls:'danger',off:true,fn:()=>{S.F.flash=false;S.rec.result={v:'ok',i:'circle-check',t:'Boot flash restored.',s:'Verified against the saved image.'};render()}}]}),
 remove:()=>({...typed('rem',REMOVE_PHRASE),title:'Remove our entries?',icon:'trash',danger:true,body:`<p>Debian may no longer start. This leaves only Synology’s own files on the boot flash.</p><div class="fld"><span class="lab">Type <span class="code">${REMOVE_PHRASE}</span> to continue</span><input class="inp mono" id="rp" autocomplete="off" aria-label="Type the phrase"></div>`,buttons:[{label:'Cancel'},{label:'Remove entries',cls:'danger',off:true,fn:()=>{S.F.flash=false;S.rec.result={v:'ok',i:'circle-check',t:'Our entries are removed.',s:'The flash holds only Synology’s files.'};render()}}]}),
 recbootdsm:()=>({id:'rbd',title:'Boot DSM?',icon:'rotate-ccw',body:S.F.disks?'<p>The NAS reboots into the DSM loader. DSM’s own system may be gone, so Synology’s install page can appear.</p>':'<p>The NAS reboots and starts DSM. It takes about a minute.</p>',buttons:[{label:'Cancel'},{label:'Boot DSM',cls:'primary',fn:()=>{if(S.F.agentUnknown&&!S.rec.tried){S.rec.tried=true;S.rec.result={v:'err',i:'wifi-off',t:'The NAS did not answer.',s:'Try again.'};render();return}S.F.dflt='dsm';S.F.agentUnknown=false;S.rec.tried=false;S.rec.result={v:'ok',i:'rotate-ccw',t:'Rebooting.',s:S.F.disks?'Expect the DSM loader in about 60 seconds.':'DSM returns in about 60 seconds.'};render()}}]}),
 recagent:()=>({id:'rag',title:'Open the agent?',icon:'terminal',body:'<p>The NAS reboots into the SSH agent. It returns to DSM by itself after 60 minutes without a login.</p>',buttons:[{label:'Cancel'},{label:'Open agent',cls:'primary',fn:()=>{S.rec.result={v:'ok',i:'terminal',t:'Agent running.',s:'ssh boot@'+cur().ip};render()}}]}),
 report:()=>({id:'rep',title:'Report a result',icon:'external-link',body:'<p>Opens a pre-filled GitHub issue with: model, DSM version, synofurb version, result, boot method and a log excerpt without secrets.</p><p>The serial number is left out. Nothing is sent without your click.</p>',buttons:[{label:'Cancel'},{label:'Open issue',cls:'primary',fn:()=>toast('Opens GitHub in your browser','external-link')}]}),
 update:()=>{const up=S.update;const st=up.state;return {id:'upd',title:'synofurb 0.9.3',icon:'download',body:`<div class="row">${tag('Stable',{})}${d('38 MiB · 29 Oct 2026')}</div><ul style="margin-left:1.25rem"><li>DS923+ added as Community-reported</li><li>Faster upload; resumes after Wi-Fi drops</li><li>Typed phrase is now case-sensitive</li></ul>${st==='downloading'?`<div class="prog" role="progressbar"><i style="width:${up.pct||63}%"></i></div>`:''}<div class="row sub">${ic('shield-check','s')}Signature ${st==='ready'?'verified':'pending'} ${infoTxt('Checked with the project key and SHA-256 before use.','About the signature')}</div>`,buttons:[{label:'Skip version',cls:'ghost',fn:()=>{S.update.state='hidden';render()}},{label:st==='ready'?'Restart and update':'Download',cls:'primary',off:st==='downloading'?'Waiting for verification or for the install to finish.':'',keep:st!=='ready',fn:api=>{if(st==='ready'){toast('Restarting to apply the update','refresh-cw');return}S.update.state='downloading';S.update.pct=63;api.close();setTimeout(()=>{S.update.state='ready';openDialogByName('update')},900);return false}}]}},
 cancelinst:()=>instN()>=3?({id:'cinst',title:'Too late to cancel',icon:'octagon-x',body:'<p>The first disk is being changed. The install can no longer be stopped from here.</p><p>If it fails, a rescue shell opens for 15 minutes.</p>',buttons:[{label:'Close',cls:'primary'}]}):({id:'cinst',title:'Cancel install?',icon:'octagon-x',body:'<p>Before step 3 nothing on the disks has changed. The NAS returns to DSM within 60 seconds.</p><p>Our entries stay on the boot flash until you remove them under Recovery.</p>',buttons:[{label:'Keep going',cls:'primary'},{label:'Stop install',fn:api=>{if(instN()>=3){tooLate(api);return false}clearInterval(S._it);S.F.flash=true;S.fail=mkFail('stopped');go('failed')}}]}),
 mgmtupdate:()=>{const H=healthData(S.manage.health),deg=H.arrays.some(a=>a.v==='err'||a.v==='warn'||a.v==='info');return {id:'mu',title:'Update management app',icon:'download',body:`<div class="row">${mono('0.9.1')}${ic('arrow-right','s')}${mono('0.9.2')}</div>${deg?alertH('warn','triangle-alert','<b>An array is degraded or rebuilding.</b> Updating is allowed: the rebuild runs in the kernel and keeps running; only the web service restarts for about 10 seconds.'):''}<p>The web UI restarts for about 10 seconds; signed-in sessions sign in again.</p><p>If the health check fails, the previous version is restored automatically. You can also roll back afterwards.</p>`,buttons:[{label:'Cancel'},{label:'Update',cls:'primary',fn:()=>startMgmtUpdate()}]}},
 rollback:()=>({id:'rb',title:'Roll back to 0.9.1?',icon:'undo-2',body:'<p>Restores the previous build and restarts the web UI (about 10 seconds).</p>',buttons:[{label:'Cancel'},{label:'Roll back',cls:'primary',fn:()=>{S.manage.mgmt='idle';render();toast('Rolled back to 0.9.1','undo-2')}}]}),
 restartsvc:()=>({id:'rs',title:'Restart service?',icon:'rotate-ccw',body:`<p>Restarting ${mono(S.pendSvc||'synofurb-mgmt')} ${S.pendSvc==='synofurb-mgmt'?'ends web sessions for a few seconds.':'takes a few seconds.'}</p>`,buttons:[{label:'Cancel'},{label:'Restart',cls:'primary',fn:()=>{S.manage.svcFixed=true;render();toast((S.pendSvc||'Service')+' is running again','circle-check')}}]}),
 bootconfirm:()=>({id:'bc',title:'Install boot agent?',icon:'download',body:`<p>The whole boot flash is saved on this computer first, then our entries and the agent are added. DSM stays the default entry.</p><p>If the agent does not answer, power-cycle the NAS: it boots DSM.</p>`,buttons:[{label:'Cancel'},{label:'Install',cls:'primary',fn:()=>startBootInstall()}]}),
 returndsm:()=>({id:'rd',title:'Return to DSM?',icon:'rotate-ccw',body:'<p>The NAS reboots into DSM. Our entries stay on the boot flash until you remove them under Recovery.</p>',buttons:[{label:'Cancel'},{label:'Return to DSM',cls:'primary',fn:()=>{S.bootm.returned=true;S.F.dflt='dsm';render()}}]}),
 quick:()=>{const u=cur(),y=S.sys,mixed=mixedDisks(u),hyb=capacity('hyb1',u.disks),alt=capacity('auto',u.disks),p0=defaultsPlan(u),cap=capacity(mixed?'hyb1':p0.raid,u.disks);
  /* mixed disks: Hybrid 1-disk (survives 1) against the auto level (survives 2 on six disks) is an explicit choice: nothing is preselected, Continue is locked with a visible reason until one is picked.
     Reason: the two options differ in how many failed disks they survive, which is the one thing a "recommended default" must not decide silently. */
  const opt=(k,l,c)=>`<button type="button" class="rrow" role="radio" aria-checked="false" tabindex="${k==='hyb1'?0:-1}" data-q="${k}"><span class="rad"><i></i></span><span class="grow"><b>${l}</b><br><span class="d">${TiB(c.usable)} usable · survives ${c.survives}</span></span></button>`;
  const store=mixed?`<div role="radiogroup" aria-label="Data layout" class="col" style="gap:.5rem">${opt('hyb1','Hybrid 1-disk',hyb)}${opt('auto',LEVEL_LABEL[alt.level],alt)}</div><p class="sub" id="qpick">Choose a layout.</p><p class="sub">Hybrid is built by synofurb; DSM may not adopt it.</p>`:`<dl class="dl" style="grid-template-columns:7rem 1fr"><dt>Storage</dt><dd>${d('Wipe all '+u.disks.length+' disks · '+LEVEL_LABEL[cap.level]+' · '+TiB(cap.usable)+' usable · survives '+cap.survives)}</dd></dl>`;
  return {id:'qk',initial:mixed?'[data-q="hyb1"]':'[data-dbtn="0"]',title:'Quick setup',icon:'zap',body:`<p>Recommended defaults for this unit:</p>${store}<dl class="dl" style="grid-template-columns:7rem 1fr"><dt>System</dt><dd>${d('32 GiB × 3 copies · '+hostOf(u)+' · '+y.user)}</dd><dt>Boot</dt><dd>${d('Stable kernel · Online · boot flash')}</dd></dl><p>The password is generated and shown once on Done. Confirm shows what is erased before anything changes.</p>`,
   onopen:api=>{if(!mixed)return;S._qk=null;const b=api.btn(2);const pick=r=>{S._qk=r.dataset.q;$$('[data-q]',api.el).forEach(x=>{x.setAttribute('aria-checked',x===r);x.setAttribute('tabindex',x===r?0:-1)});const pk=$('#qpick',api.el);pk&&(pk.hidden=true);b.setAttribute('aria-disabled','false');b.removeAttribute('data-reason');b.removeAttribute('tabindex')};
    api.el.addEventListener('click',e=>{const r=e.target.closest('[data-q]');if(r)pick(r)});
    api.el.addEventListener('keydown',e=>{const r=e.target.closest('[data-q]');if(!r||!/Arrow(Down|Up|Left|Right)/.test(e.key))return;e.preventDefault();const rs=$$('[data-q]',api.el),i=rs.indexOf(r),n=rs[(i+(/Down|Right/.test(e.key)?1:-1)+rs.length)%rs.length];pick(n);n.focus()})},
   buttons:[{label:'Cancel'},{label:'Customize',fn:()=>{go('plan')}},{label:'Continue to Confirm',cls:'primary',off:mixed?'Choose a layout first.':false,fn:()=>{S.plan=defaultsPlan(u);if(mixed)S.plan.raid=S._qk||'hyb1';S.sys.host=hostOf(u);S.sys.hostEdited=false;S.gate={b1:false,b2:false,b3:false,phrase:'',touched:false};go('confirm')}}]}},
 cmd:()=>{const ok=gateOk(),c=buildCmd(ok),m=cmdMatches(c);return {id:'cmd',title:'Command',icon:'terminal',wide:true,body:`<p>${ok?'Runs this plan, including the confirmations you gave.':'Runs this plan. The confirmation flags are added once the gate is complete.'}</p><div class="fp"><span>${esc(c)}</span></div><div class="row">${m?tag('Matches plan',{v:'ok',i:'circle-check'}):tag('Differs from plan',{v:'err',i:'circle-x'})}<span class="sub">Same choices as the screens.</span></div>`,buttons:[{label:'Close'},{label:'Copy command',cls:'primary',icon:'copy',fn:()=>{copy(c,'Command copied')}}]}},
 licence:()=>({id:'lic',title:'Licence',icon:'book-open',body:'<p class="mono">SIL Open Font License 1.1: the full text is bundled with the application and opens here.</p>',buttons:[{label:'Close',cls:'primary'}]}),
 script:()=>({id:'scr',title:'Read-only script',icon:'terminal',wide:true,body:'<div class="logbox"><div><span class="m">cat /etc/synoinfo.conf /etc.defaults/VERSION</span></div><div><span class="m">lsblk -b -o NAME,SIZE,MODEL,SERIAL; smartctl -i -H /dev/sd?</span></div><div><span class="m">cat /proc/mdstat; lvs; ip -o addr</span></div></div>',buttons:[{label:'Copy',icon:'copy',fn:()=>copy('read-only script'),keep:true},{label:'Close',cls:'primary'}]})};
Object.assign(POP,{
 authblock:()=>({title:'Wrong password',body:'<p>DSM may block this address after several failed logins. Wait a few minutes before you try again.</p>',help:'troubleshooting'}),
 sshoff:()=>({title:'Turn SSH on',body:'<p>In DSM: Control Panel, Terminal and SNMP, tick Enable SSH service, Apply. Then retry.</p>',help:'troubleshooting'}),
 unreach:()=>({title:'No answer',body:'<p>Check that the NAS is on and cabled, and that this computer is on the same network. Then retry.</p>',help:'troubleshooting'}),
 keychg:()=>({title:'Host key changed',body:'<p>The NAS answers with a different key than before. Check the unit before you go on.</p>',help:'troubleshooting'}),
 hybsug:()=>{const u=cur(),c=capacity(S.plan.raid,u.disks),h=capacity('hyb1',u.disks);return {title:'Hybrid RAID',body:`<p>${esc(LEVEL_LABEL[c.level]||'Current')}: ${TiB(c.usable)}, survives ${c.survives}. Hybrid 1-disk: ${TiB(h.usable)}, survives ${h.survives}. More space, less fault tolerance.</p><div><button class="btn sm primary" data-act="usehyb">Use Hybrid 1-disk</button></div>`,help:'hybrid-raid'}},
 slices2:()=>({title:'Weaker slice',body:'<p>A slice on only two or three disks tolerates fewer failures than the level you chose. See Slices.</p>',help:'hybrid-raid'}),
 legend:()=>({title:'Colour key',body:'<p>Green: system mirror. Other colours: data slices, named in the chips below. Dashed: unused or kept.</p>',help:'raid-levels'}),
 cause:()=>{const f=S.fail||{kind:'post'};return {title:'Engine message',body:`<p class="mono">${{pre:'disk sdc changed after you confirmed; the installer refused to continue',agent:'the boot agent did not appear within 10 minutes of the reboot; nothing was reset after the wait (watchdog: 30 minutes after the agent booted, if it ran)',backup:'sha256 of the boot flash on the NAS differs from the copy on this computer; nothing was written',post:'boot entry failed read-back; the original flash was restored and verified'}[f.kind]||''}</p>`,help:'recovery'}},
 rescueinfo:()=>({title:'Rescue shell',body:'<p>The installer keeps an SSH shell open for 15 minutes. Then the NAS reboots into the DSM loader.</p>',help:'recovery'}),
 recov:()=>({title:'How recovery works',body:'<p>Cancel is free until step 3. After that, a failure opens a 15-minute rescue shell.</p><button class="link" data-dlg="recovery">Read more</button>',help:'recovery'}),
 webraid:()=>({title:'Why the web UI?',body:'<p>This app reads health and updates the management app. Array changes run in the web UI, where the NAS can apply them safely.</p>',help:'raid-levels'}),
 hybman:()=>({title:'Hybrid layout',body:'<p>Slices at the sizes of the different disks; each slice on two or more disks is one array. LVM joins them.</p>',help:'hybrid-raid'})});
/* the Cancel dialog must re-evaluate when step 3 starts while it is open */
function tooLate(api){const el=api?api.el:$('.scrim[data-dname="cinst"]');if(!el)return;$('.dlg-h span',el).textContent='Too late to cancel';$('.dlg-b',el).innerHTML='<p>The first disk is being changed. The install can no longer be stopped from here.</p><p>If it fails, a rescue shell opens for 15 minutes.</p>';const b=$('[data-dbtn="1"]',el);if(b){b.setAttribute('aria-disabled','true');b.setAttribute('tabindex','-1');b.dataset.reason='Not possible after step 3.'}}
function openDialogByName(n){const c=DLG[n]();return openDialog(c)}

function startSignin(skipTrust){const c=S.conn;c.field=null;
  if(c.auth==='Password'&&!c.pw){c.err=null;c.field='pw';render();const i=$('#c-pw');i&&i.focus();return}
  if(c.sim==='hostchanged'){c.err='hostchanged';render();openDialogByName('hostchanged');return}
  if(!c.trusted&&!skipTrust){c.err=null;openDialogByName('hostkey');return}
  c.err=null;c.state='busy';render();
  setTimeout(()=>{c.state='idle';const e=c.sim||(c.pw==='wrong'?'login':null);if(e){c.err=e;render();return}go('inspect')},S.fast?100:900)}
function startMgmtUpdate(){S.manage.mgmt='updating';S.manage.mgmtPct=4;render();clearInterval(S._mu);S._mu=setInterval(()=>{S.manage.mgmtPct+=S.fast?30:9;if(S.manage.mgmtPct>=100){clearInterval(S._mu);S.manage.mgmt='updated';S.manage.mgmtPct=100;toast('Updated to 0.9.2','circle-check')}if(S.screen==='manage')render()},700)}
function startBootInstall(){S.F.flash=true;S.boot.unit=cur().id;S.bootm={phase:'progress',pct:0,returned:false};render();clearInterval(S._bt);S._bt=setInterval(()=>{S.bootm.pct++;if(S.bootm.pct>=4){clearInterval(S._bt);S.bootm.phase='result';S.boot.done=true}if(S.screen==='boot')render()},S.fast?250:1100)}
/* ---------- navigation and rendering ---------- */
const SCREENS={find,connect,inspect,goal,setup,plan:setup,system:setup,confirm,install,failed,done,boot,recovery,manage,settings,about};
function go(screen,o={}){closePop(false);hideTip();if(['settings','about'].includes(screen)&&!['settings','about'].includes(S.screen))S.back=S.screen;
  if(screen==='plan')S.setup.tab='storage';if(screen==='system'){S.setup.tab='system';S.setup.seenSys=true}
  if(screen==='recovery'&&S.screen!=='manage')S.manage.from=S.screen==='goal'||S.screen==='failed'?S.screen:S.manage.from;
  S.screen=screen;if(o.page)S.settings.page=o.page;if(screen==='install')startInstall();
  if(screen==='failed'&&!S.fail)S.fail=mkFail('post');
  if(screen==='manage'&&!S.manage.tab)S.manage.tab='overview';
  const si=STEP_OF[screen];if(si!==undefined)S.reached=Math.max(S.reached||0,si);
  render();const m=$('.main');m&&(m.scrollTop=0);const h=$('.main h1');if(h){h.setAttribute('tabindex','-1');if(!(document.activeElement&&document.activeElement.closest('.dlg')))h.focus({preventScroll:true})}location.hash=screen}
function sig(el){return el?(el.id||el.getAttribute('data-act')||el.getAttribute('data-go')||el.getAttribute('data-sel')||el.getAttribute('data-acc')||el.getAttribute('data-mtab')||el.getAttribute('data-select')||el.getAttribute('aria-label')||el.outerHTML.slice(0,60)):''}
function render(){const a=document.activeElement,s0=a&&a.closest('.main,.foot')?sig(a):null,sel=a&&a.selectionStart!==undefined&&a.type!=='range'?[a.selectionStart,a.selectionEnd]:null,sc=$('.main')?$('.main').scrollTop:0;
  syncFacts();if(!S.sys.hostEdited&&S.unit)S.sys.host=hostOf(S.unit);
  if(S.screen==='install'&&instN()>=3)tooLate();
  document.documentElement.dataset.sheet=S.help.open?'open':'';
  const R=SCREENS[S.screen](),st=!NOSTEPPER.includes(S.screen);
  const top=(S.update&&['find','connect','inspect','goal','plan','system','setup','confirm'].includes(S.screen)?updBanner():'')+unitHdr();
  $('#shell').innerHTML=titlebar()+`<div class="top">${top}</div><main class="main" id="main" aria-label="${S.screen}">${R.html}</main>${R.foot}`+(st?stepperHTML():'')+statusbar();
  if(R.after)R.after();const m=$('.main');if(m)m.scrollTop=sc;
  if(s0){const n=$$('.main *,.foot *').find(e=>sig(e)===s0&&e.matches('button,input,[tabindex]'));if(n){n.focus({preventScroll:true});if(sel&&n.setSelectionRange)try{n.setSelectionRange(...sel)}catch(e){}}}
  document.documentElement.dataset.screen=S.screen}
/* install simulation (demo speed) */
function startInstall(){S.inst.step=S.inst.frozen?S.inst.step:0;S.fail=null;clearInterval(S._it);if(S.inst.frozen)return;S._it=setInterval(()=>{if(S.screen!=='install'){clearInterval(S._it);return}if(S.inst.lost)return;S.inst.step++;if(S.inst.step>=18){clearInterval(S._it);render();setTimeout(()=>S.screen==='install'&&go('done'),1200);return}render()},S.fast?250:1300)}
function setSize(v){S.textSize=v;const k={Default:'',Large:'large',Largest:'largest'}[v];if(k)document.documentElement.dataset.size=k;else delete document.documentElement.dataset.size}
/* ---------- actions ---------- */
const rescueCopy=()=>{const u=cur();copy('ssh root@'+u.ip,'Rescue command copied');const p=$('[aria-label="Rescue"]');p&&p.scrollIntoView({block:'nearest'});const b=p&&$('button',p);b&&b.focus()};
const ACT={
 rescan(){S.scan='loading';render();setTimeout(()=>{S.scan='done';render()},S.fast?150:1100)},
 connect(){const u=allUnits().find(x=>x.id===S.sel);if(!u)return;S.unit=u;S.conn.state='idle';S.conn.err=null;S.conn.field=null;go('connect')},
 dosignin(){startSignin(S.conn.trusted)},peek(){const i=$('#c-pw');i.type=i.type==='password'?'text':'password'},peek2(){const i=$('#s-pw');i.type=i.type==='password'?'text':'password'},
 'copy-pw'(){copy(S.sys.pw,'Password copied')},'copy-pw2'(){copy(S.sys.pw,'Password copied');S.done.copied=true;render()},regen(){S.sys.pw=['tundra','ember','Sage',Math.floor(1000+Math.random()*8999),'plume'].join('-');render();toast('New password generated','refresh-cw')},
 cmd(){if(['plan','system','setup','confirm'].includes(S.screen))openDialogByName('cmd')},
 erase(){if(!gateOk())return;go('install')},cancelinst(){openDialogByName('cancelinst')},openlog(){toast('Opens the log in your editor','file-text')},copylog(){copy(LOGS.map(l=>l.join(' ')).join('\n'),'Log copied')},export(){toast('Diagnostics saved (secrets removed)','download')},
 rescue(){rescueCopy()},waitdsm(){const f=S.fail;if(!f)return;f.phase='waiting';f.el=0;render()},reconnect(){if(S.screen==='install'){S.inst.lost=false;render();toast('Reconnected','wifi')}else if(S.fail){S.fail.phase=S.fail.t>0?'clock':'waiting';render();toast('Reconnected','wifi')}},reinstall(){openDialogByName('reinstall')},
 showpw(){S.done.show=!S.done.show;render()},showrpw(){S.fail.rpw=!S.fail.rpw;render()},openweb(){toast('Opens https://'+cur().ip+':8443 in your browser','external-link')},'openweb-raid'(){toast('Opens https://'+cur().ip+':8443/raid in your browser','external-link')},
 finish(){if(S.done.copied){S.manage.name=cur().model;go('manage')}},later(){S.update.state='hidden';render()},refreshlist(){toast('List refreshed','refresh-cw')},dlbundle(){S.bundle=true;render();toast('Bundle downloaded','download')},
 bootconfirm(){openDialogByName('bootconfirm')},bootcancel(){clearInterval(S._bt);S.bootm={phase:'idle',pct:0};render()},returndsm(){openDialogByName('returndsm')},openfolder(){toast('Opens the folder in your file manager','folder-open')},diag(){copy('synofurb 0.9.2 · Linux · no secrets','Diagnostics copied')},docs(){toast('Opens the documentation in your browser','external-link')},
 nextsys(){S.setup.seenSys=true;S.setup.tab='system';S.screen='system';render();const h=$('.main h1');h&&h.focus({preventScroll:true})},setsys(){S.plan.sys=S.plan.sys>9?9:8;render()},rawauto(){S.plan.raid='auto';render()},usehyb(){closePop(false);S.plan.raid='hyb1';render()},recheck(){S.manage.checked=true;render();toast('Still no answer','wifi-off')},mgmtupdate(){openDialogByName('mgmtupdate')},rollback(){openDialogByName('rollback')},
 manage(){S.unit=UNITS[1];S.manage.name='attic';S.manage.from='find';S.F.debian=true;go('manage')},quick(){openDialogByName('quick')},
 manualrec(){openDialogByName('manualrec')},'rec-bootdsm'(){openDialogByName('recbootdsm')},'rec-agent'(){openDialogByName('recagent')},'rec-restore'(){openDialogByName('restore')},'rec-remove'(){openDialogByName('remove')},'rec-guide'(){openDialogByName('reinstall')}};
document.addEventListener('click',e=>{
  const t=e.target.closest('[aria-disabled=true]');
  if(t&&t.dataset.reason){e.preventDefault();e.stopPropagation();const missing_=t.id==='f-erase';openText(t,missing_?'Locked':'Unavailable',t.dataset.reason);return}
  const ir=e.target.closest('.ib[data-reason]');if(ir){openText(ir,ir.getAttribute('aria-label')==='Why locked'?'Locked':'Unavailable',ir.dataset.reason);return}
  if(t&&!t.closest('.seg')&&!t.matches('.inp')){e.preventDefault();e.stopPropagation();return}
  const x=e.target.closest('[data-copy]');if(x){copy(x.dataset.copy);return}
  const mn=e.target.closest('[data-menu]');if(mn){openMenu(mn);return}
  const sx=e.target.closest('[data-select]');if(sx){openSelect(sx);return}
  const pt=e.target.closest('[data-ptext]');if(pt){e.preventDefault();openText(pt,pt.getAttribute('aria-label')||pt.textContent.trim().slice(0,40)||'Info',pt.dataset.ptext);return}
  const po=e.target.closest('[data-pop]');if(po){e.preventDefault();openPop(po,po.dataset.pop);return}
  const ho=e.target.closest('[data-help-open]');if(ho){closePop(false);const tp=ho.dataset.helpOpen;openHelp(tp||null);return}
  const dl=e.target.closest('[data-dlg]');if(dl){closePop(false);openDialogByName(dl.dataset.dlg);return}
  const gs=e.target.closest('[data-go]');if(gs){closeSheet(false);go(gs.dataset.go,gs.dataset.settings?{page:gs.dataset.settings}:{});return}
  const ac=e.target.closest('[data-act]');if(ac&&ACT[ac.dataset.act]){closePop(false);ACT[ac.dataset.act](ac);return}
  const sv=e.target.closest('[data-svc]');if(sv){S.pendSvc=sv.dataset.svc;openDialogByName('restartsvc');return}
  const ad=e.target.closest('[data-acc]');if(ad){const k=ad.dataset.acc,cur_=ad.getAttribute('aria-expanded')==='true';S.acc[k]=!cur_;render();return}
  const sl=e.target.closest('[data-sel]');if(sl&&!e.target.closest('button,.ib')){S.sel=sl.dataset.sel;render();return}
  const mt=e.target.closest('[data-mtab]');if(mt){S.manage.tab=mt.dataset.mtab;if(mt.dataset.mtab==='recovery')S.rec.result=null;render();return}
  const st=e.target.closest('[data-setup-tab]');if(st){S.setup.tab=st.dataset.setupTab;if(S.setup.tab==='system')S.setup.seenSys=true;S.screen=S.setup.tab==='system'?'system':'plan';render();return}
  const ht=e.target.closest('[data-help-topic]');if(ht){S.help.topic=ht.dataset.helpTopic;paintHelp();$('[data-help-back]').focus();return}
  if(e.target.closest('[data-help-back]')){S.help.topic=null;paintHelp();$('#hq')&&$('#hq').focus();return}
  const htb=e.target.closest('[data-help-tab]');if(htb){S.help.tab=htb.dataset.helpTab;paintHelp();return}
  const sp=e.target.closest('[data-set-page]');if(sp){S.settings.page=sp.dataset.setPage;render();return}
  const sg=e.target.closest('[data-set-theme]');if(sg){S.themeMode=sg.dataset.setTheme;setTheme(sg.dataset.setTheme==='Dark'?'dark':'light');render();return}
  const ss=e.target.closest('[data-set-size]');if(ss){setSize(ss.dataset.setSize);render();return}
  const pm=e.target.closest('[data-plan-mode]');if(pm){S.plan.mode=pm.dataset.planMode;render();return}
  const pc=e.target.closest('[data-plan-copies]');if(pc){S.plan.copies=+pc.dataset.planCopies;render();return}
  const sk=e.target.closest('[data-sys-key]');if(sk){S.sys.key=sk.dataset.sysKey;render();return}
  const sn=e.target.closest('[data-sys-kernel]');if(sn){S.sys.kernel=sn.dataset.sysKernel;render();return}
  const sr=e.target.closest('[data-sys-source]');if(sr){S.sys.source=sr.dataset.sysSource;render();return}
  const ca=e.target.closest('[data-conn-auth]');if(ca){S.conn.auth=ca.dataset.connAuth;S.conn.field=null;render();return}
  const bk=e.target.closest('[data-boot-key]');if(bk){S.boot.key=bk.dataset.bootKey;render();return}
  const dm=e.target.closest('[data-dev]');if(dm){devAction(dm.dataset.dev);return}
});
document.addEventListener('input',e=>{const t=e.target;
  if(t.id==='flt'){S.filter=t.value;render()}
  else if(t.id==='g-phrase'){S.gate.phrase=t.value;refreshGate()}
  else if(t.id==='pl-sys'){S.plan.sys=+t.value;$('#pl-sysv').textContent=t.value+' GiB';refreshPlan()}
  else if(t.id==='pl-free'){S.plan.free=+t.value;$('#pl-freev').textContent=t.value+'%';refreshPlan()}
  else if(t.id==='c-user')S.conn.user=t.value;else if(t.id==='c-port')S.conn.port=t.value;else if(t.id==='c-pw'){S.conn.pw=t.value;if(S.conn.field){S.conn.field=null;const e2=$('#c-pw-e');e2&&e2.remove();t.closest('.inp').classList.remove('err')}}
  else if(t.id==='s-host'){S.sys.host=t.value;S.sys.hostEdited=true}else if(t.id==='s-user')S.sys.user=t.value;else if(t.id==='s-pw')S.sys.pw=t.value;else if(t.id==='s-pub')S.sys.pubkey=t.value;else if(t.id==='b-pub')S.boot.pub=t.value;
  else if(t.id==='hq'){S.help.q=t.value;paintHelp()}});
document.addEventListener('focusout',e=>{if(e.target.id==='g-phrase'){S.gate.touched=true;refreshGate()}});
document.addEventListener('change',e=>{const t=e.target;
  if(t.dataset.gate){S.gate[t.dataset.gate]=t.checked;refreshGate()}
  else if(t.dataset.conn){S.conn[t.dataset.conn]=t.checked}
  else if(t.hasAttribute('data-sys-web')){S.sys.web=t.checked}});
document.addEventListener('keydown',e=>{
  if(e.key==='Escape'){if(popEl){closePop();e.stopPropagation()}else hideTip()}
  if(e.key==='F1'){e.preventDefault();const f=document.activeElement&&document.activeElement.closest('[data-help]');openHelp(f?f.dataset.help:(SCREEN_HELP[S.screen]||null))}
  if(e.ctrlKey&&e.key===','){e.preventDefault();go('settings')}
  if(e.ctrlKey&&e.shiftKey&&(e.key==='C'||e.key==='c')){e.preventDefault();ACT.cmd()}
  if(e.ctrlKey&&e.shiftKey&&(e.key==='D'||e.key==='d')){e.preventDefault();toggleDev()}
  if(e.ctrlKey&&(e.key==='f'||e.key==='F')){const f=$('#flt,#hq');if(f){e.preventDefault();f.focus()}}
  if(e.altKey&&e.key==='ArrowLeft'){const i=STEP_OF[S.screen];if(i>0&&!['install','failed','done'].includes(S.screen)){e.preventDefault();go(SCREEN_OF[i-1])}}
  if(e.altKey&&e.key==='ArrowRight'){const i=STEP_OF[S.screen];if(i!==undefined&&i<(S.reached||0)&&!['install','failed','done'].includes(S.screen)){e.preventDefault();go(SCREEN_OF[i+1])}}
  if(e.key==='Enter'&&e.target.matches('#f-erase')){e.preventDefault();e.stopPropagation()}   // Enter never triggers Erase; Space and click do
  if(e.key==='Enter'&&e.target.matches('tr[data-sel]')){S.sel=e.target.dataset.sel;ACT.connect()}
  if(e.key==='Enter'&&e.target.matches('#g-phrase')){e.preventDefault()}
  if(e.key===' '&&e.target.matches('tr[data-sel]')){e.preventDefault();S.sel=e.target.dataset.sel;render()}
  if(e.key==='ArrowDown'||e.key==='ArrowUp'){const r=e.target.closest('tr[data-sel]');if(r){const n=e.key==='ArrowDown'?r.nextElementSibling:r.previousElementSibling;if(n){e.preventDefault();n.focus()}}}
},true);
function openMenu(anchor){closePop(false);hideTip();const m=document.createElement('div');m.className='pop';m.style.width='13.75rem';m.style.padding='.25rem';m.setAttribute('role','menu');
  m.innerHTML=[['about','info','About'],['update','download','Check for updates'],['help','circle-question-mark','Help'],['settings','settings','Settings']].map(([k,i,l])=>`<button class="btn ghost" style="width:100%;justify-content:flex-start" role="menuitem" data-m="${k}">${ic(i)}${l}</button>`).join('');
  L().appendChild(m);place(m,anchor,'below');popEl=m;popAnchor=anchor;$('button',m).focus();
  m.addEventListener('click',e=>{const b=e.target.closest('[data-m]');if(!b)return;closePop(false);const k=b.dataset.m;if(k==='about')go('about');else if(k==='settings')go('settings');else if(k==='help')openHelp();else{S.update={state:'available'};render();toast('Update 0.9.3 available','download')}});
  m.addEventListener('keydown',e=>{if(e.key==='ArrowDown'||e.key==='ArrowUp'){e.preventDefault();const b=$$('button',m),i=b.indexOf(document.activeElement);b[(i+(e.key==='ArrowDown'?1:-1)+b.length)%b.length].focus()}})}
function setTheme(t){S.theme=t;document.documentElement.dataset.theme=t}
/* ---------- hidden dev menu (not product chrome) ---------- */
function toggleDev(){const d_=$('.dev');if(d_){d_.remove();return}
  const items=[['Screens',[['find','Find'],['connect','Connect'],['inspect','Inspect'],['goal','Goal'],['plan','Setup: Storage'],['system','Setup: System'],['confirm','Confirm'],['install','Install'],['boot','Boot manager'],['recovery','Recovery'],['manage','Manage'],['settings','Settings'],['about','About'],['specimens','Components']]],
   ['Find states',[['scan-loading','Loading'],['scan-empty','Empty'],['offline','Toggle offline'],['twin','Toggle twin DS918+'],['update','Update available']]],
   ['Connect',[['c-login','Wrong password'],['c-off','SSH off'],['c-unreach','No answer'],['c-hostchg','Host key changed'],['c-ok','Reset']]],
   ['Install',[['inst-freeze','Freeze at step 9'],['inst-lost','Connection lost'],['fail-stopped','Stopped by you'],['fail-agent','Staging: agent did not answer'],['fail-backup','Staging: flash backup mismatch'],['fail-pre','Failed before step 3'],['fail-post','Failed after step 3 (wipe)'],['fail-keep','Failed after step 3 (keep)'],['fast','Fast demo speed']]],
   ['After the clock',[['ph-waiting','Waiting for DSM'],['ph-back','DSM is back'],['ph-notback','DSM did not return'],['ph-loader','DSM loader reached'],['ph-lost','Lost contact']]],
   ['Manage health',[['h-ok','Healthy'],['h-rebuild','Degraded, rebuilding'],['h-faildisk','Failed disk'],['h-svcdown','Services down'],['h-unreachable','Unreachable'],['h-hybrid','Hybrid layout'],['m-failed','Update failed, rolled back'],['m-updated','Updated (roll back)'],['m-reset','Update available']]],
   ['Misc',[['theme','Toggle light/dark'],['mark','Show/hide mockup mark'],['unit-ds923','Unit: DS923+'],['unit-ds1621','Unit: DS1621+'],['unit-ds918','Unit: DS918+'],['gate-ready','Fill the gate'],['firstlaunch','First-launch dialog'],['help','Help centre']]]];
  const el=document.createElement('div');el.className='dev';el.setAttribute('role','menu');el.innerHTML='<b>Mock states</b> <span class="sub">(Ctrl+Shift+D)</span>'+items.map(([h,l])=>`<h4>${h}</h4>`+l.map(([k,t])=>`<button data-dev="${k}">${t}</button>`).join('')).join('');
  $('.win').appendChild(el)}
function devAction(k){
  const U=id=>{S.unit=UNITS.find(u=>u.id===id);S.sel=id};
  if(k==='specimens'){go('specimens');$('.dev')&&$('.dev').remove();return}
  if(SCREENS[k]){if(!S.unit)S.unit=UNITS[1];if(k==='manage'){S.manage.name=cur().model}go(k);$('.dev')&&$('.dev').remove();return}
  if(k==='scan-loading'){S.scan='loading';go('find')}else if(k==='scan-empty'){S.filter='zzz';go('find')}else if(k==='offline'){S.offline=!S.offline;render()}else if(k==='twin'){S.twin=!S.twin;render()}
  else if(k==='update'){S.update={state:'available'};render()}
  else if(k.startsWith('c-')){S.unit=S.unit||UNITS[1];S.conn.sim={'c-login':null,'c-off':'off','c-unreach':'unreach','c-hostchg':'hostchanged','c-ok':null}[k];S.conn.err=k==='c-login'?'login':S.conn.sim;if(k==='c-login')S.conn.pw='wrong';go('connect')}
  else if(k==='fail-stopped'){S.unit=S.unit||UNITS[1];S.fail=mkFail('stopped');go('failed')}
  else if(k==='fail-agent'){S.unit=S.unit||UNITS[1];S.fail=mkFail('agent');go('failed')}else if(k==='fail-backup'){S.unit=S.unit||UNITS[1];S.fail=mkFail('backup');go('failed')}
  else if(k==='fail-pre'){S.unit=S.unit||UNITS[1];S.fail=mkFail('pre');go('failed')}else if(k==='fail-post'){S.unit=S.unit||UNITS[1];S.fail=mkFail('post');go('failed')}else if(k==='fail-keep'){S.unit=S.unit||UNITS[1];S.fail=mkFail('post',{keep:true});go('failed')}
  else if(k.startsWith('ph-')){S.unit=S.unit||UNITS[1];S.fail=S.fail||mkFail('post');S.fail.phase=k.slice(3);go('failed')}
  else if(k==='inst-freeze'){S.unit=S.unit||UNITS[1];S.inst.frozen=true;S.inst.step=12;go('install')}else if(k==='inst-lost'){S.unit=S.unit||UNITS[1];S.inst.frozen=true;S.inst.step=12;S.inst.lost=true;go('install')}else if(k==='fast'){S.fast=true;window.__fast=true;toast('Fast demo speed','play')}
  else if(k==='h-hybrid'){S.unit=UNITS[2];S.manage.layout='hyb';S.manage.health='ok';S.manage.tab='storage';S.manage.name=cur().model;S.F.debian=true;go('manage')}
  else if(k.startsWith('h-')){S.unit=S.unit||UNITS[1];S.manage.health=k.slice(2);S.manage.name=cur().model;S.manage.tab='overview';S.F.debian=true;go('manage')}
  else if(k==='m-failed'||k==='m-updated'||k==='m-reset'){S.unit=S.unit||UNITS[1];S.manage.mgmt={'m-failed':'failed','m-updated':'updated','m-reset':'idle'}[k];S.manage.name=cur().model;S.F.debian=true;go('manage')}
  else if(k==='theme')setTheme(S.theme==='dark'?'light':'dark');else if(k==='mark'){S.mark=!S.mark;$('.mockmark').hidden=!S.mark}
  else if(k.startsWith('unit-')){U(k.slice(5));render()}
  else if(k==='gate-ready'){Object.assign(S.gate,{b1:true,b2:true,b3:true,phrase:PHRASE()});S.screen==='confirm'?render():go('confirm')}
  else if(k==='firstlaunch')openDialogByName('firstlaunch');else if(k==='help')openHelp();
  $('.dev')&&$('.dev').remove()}
/* ---------- countdowns (one second each; "fast" runs at 4x) ---------- */
setInterval(()=>{const f=S.fail;if(S.screen!=='failed'||!f||S.clockFrozen||f.phase==='done')return;f.acc=(f.acc||0)+250*(window.__fast?4:1);
  while(f.acc>=1000){f.acc-=1000;
    if(f.phase==='clock'){if(f.t>0){f.t--;const c=$('#cd');if(c)c.textContent=fmt(f.t)}if(f.t<=0){f.phase=f.kind==='agent'?'notback':'waiting';f.el=0;render()}}
    else if(f.phase==='waiting'){f.el++;const c=$('#cd');if(c)c.textContent=fmt(f.el);if(f.el>=4){f.phase=f.out||(f.kind==='post'?'loader':'back');render()}}}},250);
/* ---------- init and URL parameters (for screenshots; the product has none) ---------- */
function init(){
  const h=location.hash.slice(1).split('?')[0].split('&')[0],q=new URLSearchParams(location.search||(location.hash.includes('?')?'?'+location.hash.split('?')[1]:''));
  const p=k=>q.get(k);
  document.documentElement.dataset.theme=p('theme')||(matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light');S.theme=document.documentElement.dataset.theme;
  if(p('size'))setSize({large:'Large',largest:'Largest'}[p('size')]||'Default');
  if(p('bare')==='1')$('#stage').classList.add('bare');if(p('fast')){S.fast=true;window.__fast=true}
  S.update=p('update')?{state:'available'}:null;if(p('offline'))S.offline=true;S.twin=p('twin')!=='0';
  const u=p('unit');if(u){S.unit=allUnits().concat([TWIN]).find(x=>x.id===u);S.sel=u}
  if(p('raid'))S.plan.raid=p('raid');if(p('copies'))S.plan.copies=+p('copies');if(p('keep')){S.plan.mode='keep';S.plan.sys=+(p('sys')||8)}if(p('sys')&&!p('keep'))S.plan.sys=+p('sys');
  if(p('gate')==='ready')Object.assign(S.gate,{b1:true,b2:true,b3:true,phrase:'ERASE '+(S.unit||UNITS[1]).model+(twinOf(S.unit||UNITS[1])?' '+(S.unit||UNITS[1]).sn.slice(-4):'')});if(p('gate')==='part')Object.assign(S.gate,{b1:true,b2:true,phrase:'ERASE DS9',touched:true});
  if(p('step')){S.inst.step=+p('step');S.inst.frozen=true}if(p('lost')){S.inst.lost=true}
  if(p('fail')){const v=p('fail');S.fail=mkFail(v.startsWith('pre')?'pre':v==='stopped'?'stopped':v==='agent'?'agent':v==='backup'?'backup':'post',{keep:v==='postkeep'});if(p('phase'))S.fail.phase=p('phase');if(p('out'))S.fail.out=p('out');if(p('clock'))S.fail.t=+p('clock');if(!p('run'))S.clockFrozen=true;if(p('el'))S.fail.el=+p('el')}
  if(p('health'))S.manage.health=p('health');if(p('mgmt')){S.manage.mgmt=p('mgmt');S.manage.mgmtPct=p('mgmt')==='updating'?58:100}
  if(p('layout'))S.manage.layout=p('layout');
  if(p('tab'))S.manage.tab=p('tab');if(p('page'))S.settings.page=p('page');if(p('stab'))S.setup.tab=p('stab');
  if(p('conn')){const v=p('conn');if(v==='empty'){S.conn.field='pw'}else if(v==='login'){S.conn.err='login';S.conn.pw='wrong'}else{S.conn.err=v;S.conn.sim=v}}
  if(p('facts')){const f=p('facts');if(f.includes('flash'))S.F.flash=true;if(f.includes('disks'))S.F.disks=true;if(f.includes('debian')){S.F.debian=true;S.F.dflt='debian';S.F.flash=true;S.F.disks=true}if(f.includes('fail'))S.F.fail=true;if(f.includes('agent')){S.F.flash=true;S.F.fail=true;S.F.agentUnknown=true}}
  if(p('rec')==='result')S.rec.result={v:'ok',i:'rotate-ccw',t:'Rebooting.',s:'DSM returns in about 60 seconds.'};if(p('rec')==='fail'){S.rec.result={v:'err',i:'wifi-off',t:'The NAS did not answer.',s:'Try again.'};S.rec.tried=true}
  if(p('boot')){S.bootm={phase:p('boot'),pct:p('boot')==='progress'?2:4,returned:p('returned')==='1'};if(p('boot')==='result'){S.boot.done=true;S.F.flash=true;S.boot.unit=(S.unit||UNITS[1]).id}if(p('boot')==='progress')S.F.flash=true}
  if(p('scan'))S.scan=p('scan');if(p('filter'))S.filter=p('filter');if(p('acc'))p('acc').split(',').forEach(k=>S.acc[k]=true);
  if(['confirm','install','failed','done','boot','recovery','manage','plan','system','setup','inspect','goal','connect'].includes(h)&&!S.unit){S.unit=UNITS[1]}
  if(h==='manage'){S.manage.name=p('name')||cur().model;if(!p('facts'))S.F.debian=true}
  if(h==='recovery'&&!p('facts')&&!p('nofacts')){S.F.flash=true;S.F.disks=true;S.F.fail=true}
  if(h==='done'&&p('copied')){S.done.copied=true;S.done.show=p('show')==='1'}
  if(h==='plan')S.setup.tab='storage';if(h==='system')S.setup.tab='system';
  S.screen=SCREENS[h]||h==='specimens'?h:'find';
  if(S.screen==='failed'&&!S.fail)S.fail=mkFail('post');
  if(p('mark')==='0')S.mark=false;
  $('.mockmark').hidden=!S.mark;
  if(S.screen==='install'&&!S.inst.frozen)S.inst.step=0;
  const si=STEP_OF[S.screen];if(si!==undefined)S.reached=si;
  render();
  if(S.screen==='install'&&!S.inst.frozen)startInstall();
  if(p('dlg')){if(p('dlg')==='update'){S.update={state:p('upd')||'available'}}openDialogByName(p('dlg'))}
  if(p('sheet')==='help'){S.help.tab=p('htab')||'Topics';S.help.q=p('hq')||'';openHelp(p('topic')||null)}
  if(p('pop')){const el=$(`[data-pop="${p('pop')}"]`);el&&openPop(el,p('pop'))}
  if(p('select')){const el=$('#'+p('select'));el&&openSelect(el)}
  if(p('reason')){const el=$(p('reason'));el&&openText(el,'Unavailable',el.dataset.reason)}
  if(p('ptext')){const el=$(p('ptext'));el&&el.click()}
  if(p('dev'))toggleDev();
  if(p('tip')){const el=$(p('tip'));if(el)showTip(el)}
  window.__S=S;
  if(!location.search&&S.screen==='find'){setTimeout(()=>{if(!$('.scrim'))openDialogByName('firstlaunch')},250);setTimeout(()=>{if(!S.update){S.update={state:'available'};render()}},10000)}
}
document.querySelector('.mockmark button').addEventListener('click',()=>{S.mark=false;$('.mockmark').hidden=true;toast('Hidden. Ctrl+Shift+D to show it again.','info')});
