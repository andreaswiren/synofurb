/* ===== dialogs, actions, router, init ===== */
const DLG={
 firstlaunch:()=>({id:'first',title:'Before you start',icon:'shield-check',body:'<p>synofurb only reads until you confirm.</p><p>Make a tested backup first.</p><p>Screen readers are not supported in this window; the same steps work from the command line.</p>',buttons:[{label:'Open Help',fn:()=>{setTimeout(()=>openHelp('cli'),0)}},{label:'Got it',cls:'primary'}]}),
 addip:()=>({id:'addip',title:'Add NAS',icon:'plus',body:'<div class="row gap12"><div class="fld grow"><span class="lab">Address</span><input class="inp mono" placeholder="192.168.1.x" aria-label="Address"></div><div class="fld" style="width:80px"><span class="lab">Port</span><input class="inp mono" value="5000" aria-label="Port"></div></div>',buttons:[{label:'Cancel'},{label:'Add',cls:'primary',fn:()=>toast('Probing 192.168.1.x…','search')}]}),
 hostkey:()=>({id:'hostkey',title:'Trust this NAS?',icon:'fingerprint-pattern',body:`<p>First connection to ${mono(cur().ip)}. Compare this fingerprint with DSM (Control Panel, Terminal and SNMP). If you did not expect a new key, cancel.</p><p class="mono" style="word-break:break-all;background:var(--surface-2);border:1px solid var(--border);border-radius:8px;padding:8px">SHA256:q3Zb1c8mPvX0dT7nKfL2yHrW9sUeJ5aG4oB6iNxVwYc</p>`,buttons:[{label:'Cancel',fn:()=>{S.conn.state='idle';render()}},{label:'Trust',cls:'primary',fn:()=>{S.conn.trusted=true;startSignin(true)}}]}),
 keep:()=>({id:'keep',title:'Keep existing volumes',icon:'triangle-alert',body:'<p>Keep mode reuses only DSM’s system and swap area, about 10 GiB per disk; the system is capped near 9.5 GiB.</p><p>DSM, its settings, users and packages are erased. Your data volume is left alone on a best-effort basis, with no guarantee.</p><p>It is not mounted automatically; import it read-only later.</p>',buttons:[{label:'Cancel'},{label:'Use Keep',cls:'primary',fn:()=>{S.plan.mode='keep';S.plan.sys=16;render()}}]}),
 slices:()=>{const cap=capacity(S.plan.raid,cur().disks);return {id:'slices',title:'Hybrid slices',icon:'layers',wide:true,body:`<table class="tbl sm"><thead><tr><th>Slice</th><th>Disks</th><th>Level</th><th class="r">Usable</th></tr></thead><tbody>${cap.slices.map(s=>`<tr><td>${mono(s.id)}</td><td>${d(s.mem.map(i=>cur().disks[i][0]).join(' '))}</td><td>${s.L==='unused'?tag('Unused',{i:'circle-minus',t:'A slice on one disk has no partner and stays unused.'}):d(s.L)}${s.weak?' '+tag('Weaker',{v:'warn',i:'triangle-alert'}):''}</td><td class="r">${mono(s.u?TiB(s.u):'—')}</td></tr>`).join('')}</tbody></table><p class="sub">Each slice present on two or more disks becomes one array; LVM joins them into one volume. Built by synofurb with mdadm and LVM; DSM may not adopt it, and nothing here has run on a real Synology.</p>`,buttons:[{label:'Close',cls:'primary'}]}},
 recovery:()=>({id:'rec',title:'How recovery works',icon:'life-buoy',body:'<p>DSM stays the default boot entry. Before installer step 3 (the point of no return), a failure or cancel reboots the NAS into DSM within 60 seconds.</p><p>From step 3, a failure leaves a rescue shell open for 15 minutes, then the NAS reboots into the DSM loader, which shows Synology’s install page. DSM’s own system may be gone by then; reinstall it from Synology. We promise nothing about your data.</p>',buttons:[{label:'Close',cls:'primary'}]}),
 next:()=>{const f=S.fail||{kind:'post'};const t=f.kind==='pre'?'<p>No disk was changed. The NAS sets DSM as the default and reboots in 60 seconds, then DSM starts as before. If DSM is not back after 10 minutes, power-cycle the NAS.</p><p>Our entries stay on the boot flash until you remove them under Recovery.</p>':f.keep?'<p>The first disk was changed. Export diagnostics, then open the rescue shell. At zero the NAS reboots into the DSM loader and Synology’s install page.</p><p>Your data volume was not part of any write command and should be intact unless the failure was a hardware problem. We cannot guarantee it.</p>':'<p>The first disk was changed and DSM’s own system may be gone. Export diagnostics first, then open the rescue shell.</p><p>At zero DSM becomes the default and the NAS reboots into the DSM loader, which shows Synology’s install page. Reinstall DSM from Synology. A Wipe install leaves no data to keep.</p>';return {id:'next',title:'What happens next',icon:'clock',body:t,buttons:[{label:'Close',cls:'primary'}]}},
 reinstall:()=>({id:'reinst',title:'Reinstall DSM',icon:'life-buoy',body:'<p>The NAS boots Synology’s loader, which shows an install page. Open find.synology.com or Synology Assistant, find the unit and install DSM with the .pat file for your model.</p><p>Synology documents a reinstall that can keep data volumes; check its current documentation. This project cannot guarantee it.</p>',buttons:[{label:'Close'},{label:'Open find.synology.com',cls:'primary',icon:'external-link',fn:()=>toast('Opens find.synology.com','external-link')}]}),
 restore:()=>({id:'rest',title:'Restore boot flash?',icon:'hard-drive',danger:true,body:`<p>This writes the saved image ${mono('dom-backups/DS918plus-20261101-1402.img')} back to the boot flash. It needs a working Linux on the unit, such as the agent or Debian.</p><div class="fld"><span class="lab">Type <span class="code">RESTORE FLASH</span> to continue</span><input class="inp mono" id="rp" autocomplete="off" aria-label="Type the phrase"></div>`,buttons:[{label:'Cancel'},{label:'Restore',cls:'danger',off:true,tip:'Type the phrase first.',fn:()=>toast('Flash restored (mock)','circle-check')}],onopen:api=>{const i=$('#rp',api.el),b=api.btn(1);i.addEventListener('input',()=>{const ok=i.value==='RESTORE FLASH';b.setAttribute('aria-disabled',!ok);ok?b.removeAttribute('data-tip'):b.setAttribute('data-tip','Type the phrase first.')})},initial:'#rp'}),
 report:()=>({id:'rep',title:'Report a result',icon:'external-link',body:'<p>Opens a pre-filled GitHub issue with: model, DSM version, synofurb version, result, boot method and a log excerpt without secrets.</p><p>The serial number is left out. Nothing is sent without your click.</p>',buttons:[{label:'Cancel'},{label:'Open issue',cls:'primary',fn:()=>toast('Opens GitHub (mock)','external-link')}]}),
 update:()=>{const up=S.update;const st=up.state;return {id:'upd',title:'synofurb 0.9.3',icon:'download',body:`<div class="row">${tag('Stable',{})}${d('38 MiB · 29 Oct 2026')}</div><ul style="margin-left:18px"><li>DS923+ added as Community-reported</li><li>Faster upload; resumes after Wi-Fi drops</li><li>Typed phrase is now case-sensitive</li></ul>${st==='downloading'?`<div class="prog" role="progressbar"><i style="width:${up.pct||63}%"></i></div>`:''}<div class="row sub"${tip('Checked with the project key and SHA-256 before use.')}>${ic('shield-check','s')}Signature ${st==='ready'?'verified':'pending'}</div>`,buttons:[{label:'Skip version',cls:'ghost',fn:()=>{S.update.state='hidden';render()}},{label:st==='ready'?'Restart and update':'Download',cls:'primary',off:st==='downloading'?'Waiting for verification or for the install to finish.':'',keep:st!=='ready',fn:api=>{if(st==='ready'){toast('Restarting (mock)','refresh-cw');return}S.update.state='downloading';S.update.pct=63;api.close();setTimeout(()=>{S.update.state='ready';openDialogByName('update')},900);return false}}]}},
 cancelinst:()=>({id:'cinst',title:'Cancel install?',icon:'octagon-x',body:'<p>The NAS returns to DSM within 60 seconds.</p>',buttons:[{label:'Keep going',cls:'primary'},{label:'Stop install',fn:()=>{S.inst.step=-1;go('inspect')}}]}),
 mgmtupdate:()=>({id:'mu',title:'Update management app',icon:'download',body:`<div class="row">${mono('0.9.1')}${ic('arrow-right','s')}${mono('0.9.2')}</div><p>The web UI restarts for about 10 seconds; signed-in sessions sign in again.</p><p>If the health check fails, the previous version is restored automatically. You can also roll back afterwards.</p>`,buttons:[{label:'Cancel'},{label:'Update',cls:'primary',fn:()=>startMgmtUpdate()}]}),
 rollback:()=>({id:'rb',title:'Roll back to 0.9.1?',icon:'undo-2',body:'<p>Restores the previous build and restarts the web UI (about 10 seconds).</p>',buttons:[{label:'Cancel'},{label:'Roll back',cls:'primary',fn:()=>{S.manage.mgmt='idle';render();toast('Rolled back to 0.9.1','undo-2')}}]}),
 oplan:()=>({id:'op',title:'Operation plan',icon:'list-filter',wide:true,body:`<p class="sub">Review the steps and exact commands first.</p><ol style="margin-left:18px"><li>Mark sdc failed and remove it from md1</li><li>Add the new disk</li><li>Rebuild md1 (about 9 hours)</li></ol>${acc('cmds','Commands',`<div class="logbox"><div>mdadm /dev/md1 --fail /dev/sdc</div><div>mdadm /dev/md1 --remove /dev/sdc</div><div>mdadm /dev/md1 --add /dev/sdX</div></div>`)}<div class="fld"><span class="lab">Type <span class="code">REPLACE sdc</span></span><input class="inp mono" aria-label="Type the phrase"></div>`,buttons:[{label:'Cancel'},{label:'Run',cls:'primary',off:true,tip:'Type the phrase first.'}]}),
 licence:()=>({id:'lic',title:'Licence',icon:'book-open',body:'<p class="mono" style="font-size:12px">SIL Open Font License 1.1 — the full text is bundled with the application and opens here.</p>',buttons:[{label:'Close',cls:'primary'}]}),
 script:()=>({id:'scr',title:'Read-only script',icon:'terminal',wide:true,body:'<div class="logbox"><div>cat /etc/synoinfo.conf /etc.defaults/VERSION</div><div>lsblk -b -o NAME,SIZE,MODEL,SERIAL; smartctl -i -H /dev/sd?</div><div>cat /proc/mdstat; lvs; ip -o addr</div></div>',buttons:[{label:'Copy',icon:'copy',fn:()=>copy('read-only script')},{label:'Close',cls:'primary'}]})};
function openDialogByName(n){const c=DLG[n]();return openDialog(c)}

function startSignin(skipTrust){const c=S.conn;if(!c.trusted&&!skipTrust){openDialogByName('hostkey');return}c.state='busy';c.err=null;render();setTimeout(()=>{c.state='idle';go('inspect')},S.fast?100:900)}
function startMgmtUpdate(){S.manage.mgmt='updating';S.manage.mgmtPct=4;render();clearInterval(S._mu);S._mu=setInterval(()=>{S.manage.mgmtPct+=S.fast?30:9;if(S.manage.mgmtPct>=100){clearInterval(S._mu);S.manage.mgmt='updated';S.manage.mgmtPct=100;toast('Updated to 0.9.2','circle-check')}if(S.screen==='manage')render()},700)}
/* ---------- navigation and rendering ---------- */
const SCREENS={find,connect,inspect,goal,plan,system,confirm,install,failed,done,boot,recovery,manage,settings,about};
function go(screen,o={}){closePop(false);hideTip();if(['settings','about'].includes(screen)&&!['settings','about'].includes(S.screen))S.back=S.screen;
  if(screen==='find'&&S.screen==='manage'){}
  S.screen=screen;if(o.page)S.settings.page=o.page;if(screen==='install'){startInstall()}if(screen==='confirm'&&S.gate.phrase!==PHRASE()){}
  if(screen==='manage'&&!S.manage.tab)S.manage.tab='overview';
  render();const m=$('.main');m&&(m.scrollTop=0);const h=$('.main h1');h&&h.setAttribute('tabindex','-1')&&!document.activeElement.closest('.dlg')&&h.focus({preventScroll:true});location.hash=screen}
function sig(el){return el?(el.id||el.getAttribute('data-act')||el.getAttribute('data-go')||el.getAttribute('data-sel')||el.getAttribute('data-acc')||el.getAttribute('data-mtab')||el.getAttribute('aria-label')||el.outerHTML.slice(0,60)):''}
function render(){const a=document.activeElement,s0=a&&a.closest('.main,.foot')?sig(a):null,sel=a&&a.selectionStart!==undefined?[a.selectionStart,a.selectionEnd]:null,sc=$('.main')?$('.main').scrollTop:0;
  const R=SCREENS[S.screen](),st=!NOSTEPPER.includes(S.screen);
  const top=(S.update&&['find','connect','inspect','goal','plan','system','confirm'].includes(S.screen)?updBanner():'')+unitHdr();
  $('#shell').innerHTML=titlebar()+(st?stepperHTML():'<div></div>')+`<div>${top}</div><main class="main" id="main" aria-label="${S.screen}">${R.html}</main>${R.foot}`+statusbar();
  if(R.after)R.after();const m=$('.main');if(m)m.scrollTop=sc;
  if(s0){const n=$$('.main *,.foot *').find(e=>sig(e)===s0&&e.matches('button,input,select,[tabindex]'));if(n){n.focus({preventScroll:true});if(sel&&n.setSelectionRange)try{n.setSelectionRange(...sel)}catch(e){}}}
  document.documentElement.dataset.screen=S.screen}
/* install simulation (demo speed) */
function startInstall(){S.inst.step=S.inst.frozen?S.inst.step:0;S.fail=null;clearInterval(S._it);if(S.inst.frozen)return;S._it=setInterval(()=>{if(S.screen!=='install'){clearInterval(S._it);return}S.inst.step++;if(S.inst.step>=18){clearInterval(S._it);render();setTimeout(()=>S.screen==='install'&&go('done'),1200);return}render()},S.fast?250:1300)}
/* ---------- actions ---------- */
const ACT={
 rescan(){S.scan='loading';render();setTimeout(()=>{S.scan='done';render()},S.fast?150:1100)},
 connect(){const u=UNITS.concat(S.twin?[TWIN]:[]).find(x=>x.id===S.sel);if(!u)return;S.unit=u;S.conn.state='idle';S.conn.err=null;go('connect')},
 dosignin(){startSignin(S.conn.trusted)},peek(){const i=$('#c-pw');i.type=i.type==='password'?'text':'password'},peek2(){const i=$('#s-pw');i.type=i.type==='password'?'text':'password'},
 'copy-pw'(){copy(S.sys.pw,'Password copied')},regen(){S.sys.pw=['tundra','ember','Sage',Math.floor(1000+Math.random()*8999),'plume'].join('-');render();toast('New password generated','refresh-cw')},
 cmd(){const cmd='synofurb '+(S.screen==='confirm'&&gateOk()?'install --have-backup --accept-data-loss --confirm ':'plan ')+'--host '+cur().ip+' --user admin';copy(cmd,'Command copied')},
 erase(){if(!gateOk())return;go('install')},cancelinst(){openDialogByName('cancelinst')},openlog(){toast('Opens the log in your editor','file-text')},export(){toast('Diagnostics saved (secrets removed)','download')},
 rescue(){toast('ssh root@192.168.1.40 (mock)','terminal')},waitdsm(){toast('Waiting for DSM…','loader-circle')},showpw(){S.done.show=!S.done.show;render()},openweb(){toast('Opens https://'+cur().ip+':8443','external-link')},
 finish(){if(S.done.copied){S.manage.name=cur().model;go('manage')}},later(){S.update.state='hidden';render()},refreshlist(){toast('List refreshed','refresh-cw')},dlbundle(){S.bundle=true;render();toast('Bundle downloaded','download')},
 bootinstall(){S.boot.done=true;toast('Agent installed; DSM stays default','circle-check');render()},openfolder(){toast('Opens the folder','folder-open')},diag(){copy('synofurb 0.9.2 · Linux · no secrets','Diagnostics copied')},docs(){toast('Opens docs (needs Internet)','external-link')},
 set9(){S.plan.sys=9;render()},rawauto(){S.plan.raid='auto';render()},recheck(){toast('Checked just now','refresh-cw')},mgmtupdate(){openDialogByName('mgmtupdate')},rollback(){openDialogByName('rollback')},
 manage(){S.unit=UNITS[1];S.manage.name='attic';S.manage.from='find';go('manage')}};
document.addEventListener('click',e=>{
  const t=e.target.closest('[aria-disabled=true]');if(t&&!t.closest('.seg')&&!t.matches('.inp')){e.preventDefault();e.stopPropagation();if(t.hasAttribute('data-tip'))showTip(t);return}
  const x=e.target.closest('[data-copy]');if(x){copy(x.dataset.copy);return}
  const mn=e.target.closest('[data-menu]');if(mn){openMenu(mn);return}
  const po=e.target.closest('[data-pop]');if(po){e.preventDefault();openPop(po,po.dataset.pop);return}
  const ho=e.target.closest('[data-help-open]');if(ho){closePop(false);const tp=ho.dataset.helpOpen;openHelp(tp||null);return}
  const dl=e.target.closest('[data-dlg]');if(dl){openDialogByName(dl.dataset.dlg);return}
  const gs=e.target.closest('[data-go]');if(gs){closeSheet(false);go(gs.dataset.go,gs.dataset.settings?{page:gs.dataset.settings}:{});return}
  const ac=e.target.closest('[data-act]');if(ac&&ACT[ac.dataset.act]){ACT[ac.dataset.act](ac);return}
  const ad=e.target.closest('[data-acc]');if(ad){const k=ad.dataset.acc,cur_=ad.getAttribute('aria-expanded')==='true';S.acc[k]=!cur_;render();return}
  const sl=e.target.closest('[data-sel]');if(sl&&!e.target.closest('button,.info')){S.sel=sl.dataset.sel;render();return}
  const mt=e.target.closest('[data-mtab]');if(mt){if(mt.dataset.mtab==='recovery'){S.manage.from='manage';go('recovery');return}S.manage.tab=mt.dataset.mtab;render();return}
  const rc=e.target.closest('[data-rec]');if(rc){const k=rc.dataset.rec;if(k==='restore')openDialogByName('restore');else if(k==='reinstall')openDialogByName('reinstall');else toast('Run: '+['Boot DSM next','Open agent','','Remove our entries'][+k]+' (mock)','play');return}
  const ht=e.target.closest('[data-help-topic]');if(ht){S.help.topic=ht.dataset.helpTopic;paintHelp();$('[data-help-back]').focus();return}
  if(e.target.closest('[data-help-back]')){S.help.topic=null;paintHelp();$('#hq')&&$('#hq').focus();return}
  const htb=e.target.closest('[data-help-tab]');if(htb){S.help.tab=htb.dataset.helpTab;paintHelp();return}
  const sp=e.target.closest('[data-set-page]');if(sp){S.settings.page=sp.dataset.setPage;render();return}
  const sg=e.target.closest('[data-set-theme]');if(sg){S.themeMode=sg.dataset.setTheme;setTheme(sg.dataset.setTheme==='Dark'?'dark':'light');render();return}
  const ss=e.target.closest('[data-set-size]');if(ss){S.textSize=ss.dataset.setSize;document.documentElement.style.fontSize={Default:'',Large:'',Largest:''}[ss.dataset.setSize];render();return}
  const pm=e.target.closest('[data-plan-mode]');if(pm){S.plan.mode=pm.dataset.planMode;render();return}
  const pc=e.target.closest('[data-plan-copies]');if(pc){S.plan.copies=+pc.dataset.planCopies;render();return}
  const sk=e.target.closest('[data-sys-key]');if(sk){S.sys.key=sk.dataset.sysKey;render();return}
  const sn=e.target.closest('[data-sys-kernel]');if(sn){S.sys.kernel=sn.dataset.sysKernel;render();return}
  const sr=e.target.closest('[data-sys-source]');if(sr){S.sys.source=sr.dataset.sysSource;render();return}
  const ca=e.target.closest('[data-conn-auth]');if(ca){S.conn.auth=ca.dataset.connAuth;render();return}
  const bk=e.target.closest('[data-boot-key]');if(bk){S.boot.key=bk.dataset.bootKey;render();return}
  const dm=e.target.closest('[data-dev]');if(dm){devAction(dm.dataset.dev);return}
});
document.addEventListener('input',e=>{const t=e.target;
  if(t.id==='flt'){S.filter=t.value;render()}
  else if(t.id==='g-phrase'){S.gate.phrase=t.value;refreshGate()}
  else if(t.id==='pl-sys'){S.plan.sys=+t.value;$('#pl-sysv').textContent=t.value+' GiB';refreshPlan()}
  else if(t.id==='pl-free'){S.plan.free=+t.value;$('#pl-freev').textContent=t.value+'%';refreshPlan()}
  else if(t.id==='c-user')S.conn.user=t.value;else if(t.id==='c-port')S.conn.port=t.value;else if(t.id==='c-pw')S.conn.pw=t.value;
  else if(t.id==='s-host')S.sys.host=t.value;else if(t.id==='s-user')S.sys.user=t.value;else if(t.id==='s-pw')S.sys.pw=t.value;
  else if(t.id==='hq'){S.help.q=t.value;paintHelp()}});
document.addEventListener('change',e=>{const t=e.target;
  if(t.dataset.gate){S.gate[t.dataset.gate]=t.checked;refreshGate()}
  else if(t.id==='pl-raid'){S.plan.raid=t.value;render()}
  else if(t.dataset.conn){S.conn[t.dataset.conn]=t.checked}
  else if(t.hasAttribute('data-sys-web')){S.sys.web=t.checked}
  else if(t.hasAttribute('data-done')){S.done.copied=t.checked;const b=$('#f-finish');b.setAttribute('aria-disabled',!t.checked);t.checked?b.removeAttribute('data-tip'):b.setAttribute('data-tip','Copy the password first.')}});
document.addEventListener('keydown',e=>{
  if(e.key==='Escape'){if(popEl){closePop();e.stopPropagation()}else hideTip()}
  if(e.key==='F1'){e.preventDefault();const f=document.activeElement&&document.activeElement.closest('[data-help]');openHelp(f?f.dataset.help:null)}
  if(e.ctrlKey&&e.key===','){e.preventDefault();go('settings')}
  if(e.ctrlKey&&e.shiftKey&&(e.key==='C'||e.key==='c')){e.preventDefault();ACT.cmd()}
  if(e.ctrlKey&&e.shiftKey&&(e.key==='D'||e.key==='d')){e.preventDefault();toggleDev()}
  if(e.ctrlKey&&(e.key==='f'||e.key==='F')){const f=$('#flt,#hq');if(f){e.preventDefault();f.focus()}}
  if(e.altKey&&e.key==='ArrowLeft'){const i=STEP_OF[S.screen];if(i>0&&!['install','failed','done'].includes(S.screen)){e.preventDefault();go(SCREEN_OF[i-1])}}
  if(e.key==='Enter'&&e.target.matches('tr[data-sel]')){S.sel=e.target.dataset.sel;ACT.connect()}
  if(e.key==='Enter'&&e.target.matches('#g-phrase')){e.preventDefault()}   // Enter never triggers Erase
  if((e.key==='Enter'||e.key===' ')&&e.target.matches('tr[data-sel]')&&e.key===' '){e.preventDefault();S.sel=e.target.dataset.sel;render()}
  if(e.key==='ArrowDown'||e.key==='ArrowUp'){const r=e.target.closest('tr[data-sel]');if(r){const n=e.key==='ArrowDown'?r.nextElementSibling:r.previousElementSibling;if(n){e.preventDefault();n.focus()}}}
});
function openMenu(anchor){closePop(false);hideTip();const m=document.createElement('div');m.className='pop';m.style.width='220px';m.style.padding='4px';m.setAttribute('role','menu');
  m.innerHTML=[['about','info','About'],['update','download','Check for updates'],['help','circle-question-mark','Help'],['settings','settings','Settings']].map(([k,i,l])=>`<button class="btn ghost" style="width:100%;justify-content:flex-start" role="menuitem" data-m="${k}">${ic(i)}${l}</button>`).join('');
  L().appendChild(m);place(m,anchor,'below');popEl=m;popAnchor=anchor;$('button',m).focus();
  m.addEventListener('click',e=>{const b=e.target.closest('[data-m]');if(!b)return;closePop(false);const k=b.dataset.m;if(k==='about')go('about');else if(k==='settings')go('settings');else if(k==='help')openHelp();else{S.update={state:'available'};render();toast('Update 0.9.3 available','download')}});
  m.addEventListener('keydown',e=>{if(e.key==='ArrowDown'||e.key==='ArrowUp'){e.preventDefault();const b=$$('button',m),i=b.indexOf(document.activeElement);b[(i+(e.key==='ArrowDown'?1:-1)+b.length)%b.length].focus()}})}
function setTheme(t){S.theme=t;document.documentElement.dataset.theme=t}
/* ---------- hidden dev menu (not product chrome) ---------- */
function toggleDev(){const d_=$('.dev');if(d_){d_.remove();return}
  const items=[['Screens',[['find','Find'],['connect','Connect'],['inspect','Inspect'],['goal','Goal'],['plan','Plan'],['system','System'],['confirm','Confirm'],['install','Install'],['boot','Boot manager'],['recovery','Recovery'],['manage','Manage'],['settings','Settings'],['about','About']]],
   ['Find states',[['scan-loading','Loading'],['scan-empty','Empty'],['offline','Toggle offline'],['twin','Toggle twin DS918+'],['update','Update available']]],
   ['Install',[['fail-pre','Failed before step 3'],['fail-post','Failed after step 3 (wipe)'],['fail-keep','Failed after step 3 (keep)'],['inst-freeze','Freeze at step 9'],['fast','Fast demo speed']]],
   ['Manage health',[['h-ok','Healthy'],['h-rebuild','Degraded, rebuilding'],['h-faildisk','Failed disk'],['h-svcdown','Services down'],['h-unreachable','Unreachable'],['m-failed','Update failed, rolled back'],['m-updated','Updated (roll back)'],['m-reset','Update available']]],
   ['Misc',[['theme','Toggle light/dark'],['mark','Show/hide mockup mark'],['unit-ds923','Unit: DS923+'],['unit-ds1621','Unit: DS1621+'],['unit-ds918','Unit: DS918+'],['gate-ready','Fill the gate'],['firstlaunch','First-launch dialog'],['help','Help centre']]]];
  const el=document.createElement('div');el.className='dev';el.setAttribute('role','menu');el.innerHTML='<b>Mock states</b> <span class="sub">(Ctrl+Shift+D)</span>'+items.map(([h,l])=>`<h4>${h}</h4>`+l.map(([k,t])=>`<button data-dev="${k}">${t}</button>`).join('')).join('');
  $('.win').appendChild(el)}
function devAction(k){
  const U=id=>{S.unit=UNITS.find(u=>u.id===id);S.sel=id};
  if(SCREENS[k]){if(!S.unit)S.unit=UNITS[1];if(k==='manage'){S.manage.name=cur().model}go(k);return}
  if(k==='scan-loading'){S.scan='loading';go('find')}else if(k==='scan-empty'){S.filter='zzz';go('find')}else if(k==='offline'){S.offline=!S.offline;render()}else if(k==='twin'){S.twin=!S.twin;render()}
  else if(k==='update'){S.update={state:'available'};render()}
  else if(k==='fail-pre'){S.unit=S.unit||UNITS[1];S.fail={kind:'pre'};go('failed')}else if(k==='fail-post'){S.unit=S.unit||UNITS[1];S.fail={kind:'post'};go('failed')}else if(k==='fail-keep'){S.unit=S.unit||UNITS[1];S.fail={kind:'post',keep:true};go('failed')}
  else if(k==='inst-freeze'){S.unit=S.unit||UNITS[1];S.inst.frozen=true;S.inst.step=12;go('install')}else if(k==='fast'){S.fast=true;toast('Fast demo speed','play')}
  else if(k.startsWith('h-')){S.unit=S.unit||UNITS[1];S.manage.health=k.slice(2);S.manage.name=cur().model;S.manage.tab='overview';go('manage')}
  else if(k==='m-failed'||k==='m-updated'||k==='m-reset'){S.unit=S.unit||UNITS[1];S.manage.mgmt={'m-failed':'failed','m-updated':'updated','m-reset':'idle'}[k];S.manage.name=cur().model;go('manage')}
  else if(k==='theme')setTheme(S.theme==='dark'?'light':'dark');else if(k==='mark'){S.mark=!S.mark;$('.mockmark').hidden=!S.mark}
  else if(k.startsWith('unit-')){U(k.slice(5));render()}
  else if(k==='gate-ready'){Object.assign(S.gate,{b1:true,b2:true,b3:true,phrase:PHRASE()});S.screen==='confirm'?render():go('confirm')}
  else if(k==='firstlaunch')openDialogByName('firstlaunch');else if(k==='help')openHelp();
  $('.dev')&&$('.dev').remove()}
/* ---------- timers for countdowns ---------- */
setInterval(()=>{if(S.screen!=='failed'||S.inst.frozen&&S.freezeCd)return;const c=$('#cd');if(!c)return;let [m,s]=c.textContent.split(':').map(Number);let t=m*60+s;if(t>0)t--;c.textContent=Math.floor(t/60)+':'+String(t%60).padStart(2,'0')},1000);
/* ---------- init and URL parameters (for screenshots; the product has none) ---------- */
function init(){
  const h=location.hash.slice(1).split('?')[0].split('&')[0],q=new URLSearchParams(location.search||(location.hash.includes('?')?'?'+location.hash.split('?')[1]:''));
  const p=k=>q.get(k);
  document.documentElement.dataset.theme=p('theme')||(matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light');S.theme=document.documentElement.dataset.theme;
  if(p('bare')==='1')$('#stage').classList.add('bare');if(p('fast')){S.fast=true;window.__fast=true}
  S.update=p('update')?{state:'available'}:null;if(p('offline'))S.offline=true;if(p('twin'))S.twin=true;
  const u=p('unit');if(u){S.unit=UNITS.find(x=>x.id===u);S.sel=u}
  if(p('raid'))S.plan.raid=p('raid');if(p('copies'))S.plan.copies=+p('copies');if(p('keep')){S.plan.mode='keep';S.plan.sys=+(p('sys')||16)}if(p('sys')&&!p('keep'))S.plan.sys=+p('sys');
  if(p('gate')==='ready')Object.assign(S.gate,{b1:true,b2:true,b3:true,phrase:'ERASE '+(S.unit||UNITS[1]).model});if(p('gate')==='part')Object.assign(S.gate,{b1:true,b2:true,phrase:'ERASE DS9'});
  if(p('step')){S.inst.step=+p('step');S.inst.frozen=true}
  if(p('fail')){S.fail={kind:p('fail').startsWith('pre')?'pre':'post',keep:p('fail')==='postkeep'}}
  if(p('health'))S.manage.health=p('health');if(p('mgmt')){S.manage.mgmt=p('mgmt');S.manage.mgmtPct=p('mgmt')==='updating'?58:100}
  if(p('tab'))S.manage.tab=p('tab');if(p('page'))S.settings.page=p('page');
  if(p('scan'))S.scan=p('scan');if(p('filter'))S.filter=p('filter');if(p('acc'))p('acc').split(',').forEach(k=>S.acc[k]=true);
  if(['confirm','install','failed','done','boot','recovery','manage','plan','system','inspect','goal','connect'].includes(h)&&!S.unit){S.unit=UNITS[1]}
  if(h==='manage')S.manage.name=p('name')||cur().model;if(h==='done'&&p('copied')){S.done.copied=true;S.done.show=p('show')==='1'}
  S.screen=SCREENS[h]?h:'find';
  if(p('mark')==='0')S.mark=false;
  $('.mockmark').hidden=!S.mark;
  if(S.screen==='install'&&!S.inst.frozen)S.inst.step=0;
  render();
  if(S.screen==='install'&&!S.inst.frozen)startInstall();
  if(p('dlg')){if(p('dlg')==='update'){S.update={state:p('upd')||'available'}}openDialogByName(p('dlg'))}
  if(p('sheet')==='help'){S.help.tab=p('htab')||'Topics';S.help.q=p('hq')||'';openHelp(p('topic')||null)}
  if(p('pop')){const el=$(`[data-pop="${p('pop')}"]`);el&&openPop(el,p('pop'))}
  if(p('dev'))toggleDev();
  if(p('tip')){const el=$(p('tip'));if(el)showTip(el)}
  if(!p('nofirst')&&S.screen==='find'&&!p('dlg')&&!p('sheet')&&!p('scan')&&!p('dev')&&!p('filter')&&!p('update')&&!p('offline')&&!p('twin')&&!p('pop')){}
  window.__S=S;
  if(!location.search&&S.screen==='find'){setTimeout(()=>{if(!$('.scrim'))openDialogByName('firstlaunch')},250);setTimeout(()=>{if(!S.update){S.update={state:'available'};render()}},10000)}
}
document.querySelector('.mockmark button').addEventListener('click',()=>{S.mark=false;$('.mockmark').hidden=true;toast('Hidden. Ctrl+Shift+D to show it again.','info')});
setTimeout(()=>{if(!S.update&&S.screen==='find'&&!window.__fast&&!location.search.includes('update')&&!location.search.includes('nofirst')){}},0);
init();
