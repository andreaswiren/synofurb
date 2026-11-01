/* Boot manager: the agent is armed with timeout_min: 60 (crates/synofurb-core/src/session.rs ~422); the install flow arms it with 30 (~330), see js4.js AGENT_*. */
/* ===== Boot manager, Recovery, Manage, Help, About ===== */
/* ---- D13 Boot manager: confirm, progress, result ---- */
const BOOTSTEPS=['Boot flash backup','Agent added','Reboot into the agent','Agent answers'];
function boot(){const b=S.boot,m=S.bootm,u=cur(),ph=m.phase;
 const steps=BOOTSTEPS.map((t,i)=>{const c=m.pct>i?'done':m.pct===i&&ph==='progress'?'run':'';return `<li class="d ${c}"><span class="si">${c==='done'?ic('check'):''}</span>${t}</li>`}).join('');
 let body;
 if(ph==='idle')body=`<div class="card"><div class="card-b col gap16"><div class="fld"><span class="lab">${lab('SSH key','The key pair is created on this computer.')}</span><div class="seg" role="group" style="align-self:flex-start">${['Generate','Paste'].map(k=>`<button aria-pressed="${b.key===k}" data-boot-key="${k}">${k}</button>`).join('')}</div>${b.key==='Paste'?`<input class="inp mono" id="b-pub" placeholder="ssh-ed25519 AAAA…" value="${esc(b.pub)}" aria-label="Public key">`:''}</div>
  <div class="fld"><span class="lab">${lab('Login timeout','The agent reboots to DSM after 60 minutes without a login.')}</span><div class="mono" style="padding:.25rem 0">${d('60 min')}</div></div>
  ${alertH('warn','triangle-alert',`<b>After another OS starts, only a power cycle returns.</b> ${infoBtn('oneshot','One-shot agent')}`)}</div></div>`;
 else if(ph==='progress')body=`<div class="card"><div class="card-b col"><div class="row"><b>Boot flash</b>${tag('Working',{v:'info',i:'loader-circle'})}<span class="sp"></span>${mono(Math.min(100,m.pct*25)+'%')}</div><div class="prog" role="progressbar" aria-valuenow="${m.pct*25}"><i style="width:${m.pct*25}%"></i></div><ol class="stp">${steps}</ol></div></div>`;
 else if(m.returned)body=`<div class="card"><div class="card-b col gap16"><div class="row gap12 wrap">${tag('Agent stopped',{i:'circle-check'})}</div>${alertH('ok','circle-check','<b>Back in DSM.</b> Our entries remain on the flash.')}</div></div>`;
 else body=`<div class="card"><div class="card-b col gap16"><div class="row gap12 wrap">${tag('Agent ready',{v:'ok',i:'circle-check',lg:true})}${tag('60 min watchdog',{i:'clock',t:'The agent reboots to DSM after 60 minutes without a login.'})}</div>
  <div class="copyv">${mono('ssh boot@'+u.ip)}<button class="btn ghost icon sm" data-copy="ssh boot@${u.ip}" aria-label="Copy command"${tip('Copy')}>${ic('copy','s')}</button></div>
  ${m.returned?alertH('ok','circle-check','<b>Back in DSM.</b> Our entries remain on the flash.'):''}</div></div>`;
 const foot=ph==='idle'?F(back('goal'),btn('Install agent',{cls:'primary',act:'bootconfirm',icon:'download'}))
  :ph==='progress'?F('',btn('Cancel',{act:'bootcancel',off:m.pct>=1?'Not possible after the first flash write.':''}))
  :F('',(m.returned?btn('Recovery options',{go:'recovery'}):btn('Return to DSM',{act:'returndsm',icon:'rotate-ccw'}))+btn('Done',{cls:'primary',go:'find',icon:'check'}));
 return {html:`<div class="page narrow"><div class="ph"><div><h1 class="t-title">Boot manager ${infoBtn('oneshot','One-shot agent')}</h1><div class="sub t-sub">SSH agent on the boot flash.</div></div></div>
 <div class="row wrap">${tag(ph==='idle'?'Will change flash':'Flash changed',{v:'warn',i:'triangle-alert',t:'Entries and files are added; DSM’s entry stays. The whole flash is saved here first.'})}${tag('Disks untouched',{v:'ok',i:'shield-check',t:'No partition table is written.'})}</div>${body}</div>`,foot}}
/* ---- D14 Recovery: every row reads the unit's facts and has a real outcome ---- */
function recRows(){const F=S.F,dm=F.disks,dsmDefault=F.dflt==='dsm';
 return [
  {k:'bootdsm',label:'Boot DSM',btn:'Boot',icon:'rotate-ccw',desc:dm?'Reboots into the DSM loader; it may show Synology’s install page.':'Makes the unit boot DSM next time.',off:dsmDefault?'DSM is already the default.':''},
  {k:'agent',label:'Open agent',btn:'Start',icon:'terminal',desc:'Boots the SSH agent; returns to DSM by itself.',off:F.flash?'':'No agent on the flash. Set it up under Boot manager.'},
  {k:'restore',label:'Restore flash',btn:'Restore',icon:'hard-drive',desc:'Writes the saved image back to the boot flash.',off:F.flash?'':'Nothing of ours is on the flash.',danger:true},
  {k:'remove',label:'Remove entries',btn:'Remove',icon:'trash',desc:'Leaves only Synology’s own files on the flash.',off:!F.flash?'Nothing of ours is on the flash.':F.dflt==='debian'?'Debian starts through these entries. Boot DSM first.':'',danger:true},
  {k:'guide',label:'Reinstall DSM',btn:'Open guide',icon:'life-buoy',desc:'Use Synology’s page; no data promise.',off:''}]}
/* The primary follows the facts: a unit whose default is already DSM but whose disks were written boots into Synology's loader, so the one useful first step is the reinstall guide (Boot DSM is disabled: nothing to switch).
   If the default is not DSM (Debian or the agent boots), switching back to DSM is the safe first step. A unit with nothing changed has no primary. */
function recPrimary(){const F=S.F;if(F.dflt!=='dsm'&&(F.flash||F.disks))return 'bootdsm';if(F.disks)return 'guide';return ''}
function recoveryBody(){const F=S.F,rows=recRows(),pr=recPrimary(),r=S.rec.result;
 const chips=`<div class="row wrap">${tag(d('Flash: '+(F.flash?'ours present':'clean')),{i:'hard-drive',v:F.flash?'warn':''})}${tag(d('Default: '+(F.dflt==='dsm'?'DSM':'Debian')),{i:'server'})}${tag(d(F.disks?'Disks: modified 14:25':'Disks: untouched'),{v:F.disks?'warn':'ok',i:F.disks?'triangle-alert':'shield-check'})}</div>`;
 const none=!F.flash&&!F.disks?alertH('info','info','<b>Nothing to recover.</b> The unit is as it was found.'):'';
 const res=r?alertH(r.v,r.i,'<b>'+r.t+'</b> '+d(r.s||'')):'';
 return `${chips}${none}${res}<div class="card cardtbl"><ul class="rows">${rows.map((x,i)=>`<li><div class="row rw" style="border-bottom:${i<rows.length-1?'1px solid var(--border)':'0'}"><span class="ib" style="background:var(--surface-2)">${ic(x.icon,'s')}</span><div class="grow col" style="gap:0"><span class="row" style="gap:.25rem"><b>${x.label}</b><button class="ib" data-ptext="${esc(x.desc)}" aria-label="About ${x.label}">${ic('info','s')}</button></span>${x.k==='bootdsm'&&S.F.disks?'<span class="sub">Loader may show Synology’s page.</span>':''}</div>${btn(x.btn,{cls:'sm '+(pr===x.k?'primary':''),act:'rec-'+x.k,off:x.off})}</div></li>`).join('')}</ul></div>
 ${acc('saved','Saved here',`<div class="col"><div class="copyv">${mono('dom-backups/DS918plus-20261101-1402.img')}<button class="btn ghost icon sm" data-copy="dom-backups/DS918plus-20261101-1402.img" aria-label="Copy path">${ic('copy','s')}</button></div><div><button class="btn sm" data-act="openfolder">${ic('folder-open')}Open folder</button></div></div>`,false,'')}`}
function recovery(){return {html:`<div class="page narrow" style="max-width:48rem"><div class="ph"><div><h1 class="t-title">Recovery</h1><div class="sub t-sub">State read from the unit.</div></div></div>${recoveryBody()}</div>`,foot:F(back(S.manage.from||'goal'),'')}}
/* ---- D15 Manage: health ---- */
function arraysOf(u){const disks=u.disks.map(x=>x[0]);
 if(S.manage.layout==='hyb'){const cap=capacity('hyb1',u.disks),a=[{n:'md0',l:'RAID 1 ×3',sz:'32 GiB',mem:disks.slice(0,3),role:'sys'}];cap.slices.filter(s=>s.L!=='unused').forEach((s,i)=>a.push({n:'md'+(i+1),l:s.L+' ×'+s.mem.length,sz:TiB(s.u),mem:s.mem.map(k=>disks[k]),role:'data',slice:s.id}));return a}
 return [{n:'md0',l:'RAID 1 ×3',sz:'32 GiB',mem:disks.slice(0,3),role:'sys'},{n:'md1',l:'RAID 5 ×'+disks.length,sz:TiB(capacity('raid5',u.disks).usable),mem:disks,role:'data'}]}
function healthData(h){const u=cur();
  const base={overall:'ok',warnings:[],arrays:arraysOf(u).map(a=>({...a,st:'Clean',v:'ok'})),
   disks:u.disks.map(x=>[x[0],x[0]==='sdc'?38:37-(x[0].charCodeAt(2)%3),'ok','OK']),vols:[['/',41],['/srv/data',16]],svc:[['ssh',1],['synofurb-mgmt',1],['smartd',1],['mdadm',1],['unattended-upgrades',1]],upd:{n:12,sec:3,reboot:false},boot:'Debian 13 · 6.12.57'};
  const dataArr=base.arrays.filter(a=>a.role==='data'),lastData=dataArr[dataArr.length-1];
  if(h==='rebuild'){base.overall='warn';base.warnings=[lastData.n+' rebuilding 24%'];Object.assign(lastData,{st:'Rebuilding',v:'info',pct:24});const k=base.disks.findIndex(x=>x[0]==='sdc');if(k>=0)base.disks[k]=['sdc',38,'warn','8 realloc.']}
  if(h==='faildisk'){base.overall='crit';const k=base.disks.findIndex(x=>x[0]==='sdc');if(k>=0)base.disks[k]=['sdc','—','err','Failed'];base.arrays.forEach(a=>{if(a.mem.includes('sdc'))Object.assign(a,a.role==='sys'?{st:'Degraded',v:'warn'}:{st:'Degraded',v:'err'})});base.warnings=['sdc failed',...base.arrays.filter(a=>a.v==='err').map(a=>a.n+' degraded')]}
  if(h==='svcdown'&&!S.manage.svcFixed){base.overall='warn';base.warnings=['2 services down'];base.svc[1][1]=0;base.svc[3][1]=0;base.upd.reboot=true}
  if(h==='unreachable'){base.overall='unknown'}
  return base}
function mgmtRow(H){const m=S.manage.mgmt,unr=S.manage.health==='unreachable',unit='0.9.1',app='0.9.2',cur_=m==='updated'?app:unit,ok=H.overall==='ok';
 const stat=m==='updated'?tag('Up to date',{v:'ok',i:'circle-check'}):m==='updating'?tag('Updating',{v:'info',i:'loader-circle'}):m==='failed'?tag('Rolled back',{v:'warn',i:'undo-2'}):unit!==app?tag('Update available',{v:'info',i:'download'}):tag('Up to date',{v:'ok',i:'circle-check'});
 const steps=['Upload','Install','Restart','Health check'];const k=Math.floor(S.manage.mgmtPct/26);
 const updOff=unr?'Unit unreachable.':m==='updating'?'Updating now.':cur_===app?'Already up to date.':'';
 const upPrimary=ok&&!updOff;
 return `<div class="card"><div class="mgmt" style="flex-wrap:wrap"><div class="col grow" style="gap:.125rem"><div class="row"><b>Management app</b>${infoBtn('mgmtv','Management app')}${stat}</div><div class="row gap12 sub"><span>On unit</span>${mono(cur_)}<span>In this app</span>${mono(app)}</div></div>
  ${m==='updated'?btn('Roll back',{cls:'ghost',icon:'undo-2',act:'rollback'}):''}${btn('Update',{cls:upPrimary?'primary':'',icon:'download',act:'mgmtupdate',off:updOff})}${btn('Open web UI',{cls:H.overall==='crit'?'primary':'',icon:'external-link',act:'openweb',off:unr?'Unit unreachable.':''})}</div>
  ${m==='updating'?`<div class="card-b" style="border-top:1px solid var(--border);padding:.75rem 1rem" role="status"><div class="prog" role="progressbar" aria-valuenow="${S.manage.mgmtPct}"><i style="width:${S.manage.mgmtPct}%"></i></div><div class="row gap16 sub" style="margin-top:.5rem">${steps.map((s,i)=>`<span style="${i===k?'font-weight:600;color:var(--fg)':''}">${i<k?ic('check','s'):''} ${s}</span>`).join('')}</div></div>`:''}
  ${m==='failed'?`<div style="padding:0 1rem 1rem">${alertH('warn','undo-2','<b>Update failed.</b> '+d('Health check timed out. Rolled back to 0.9.1.'),`<button class="btn" data-act="mgmtupdate">Retry</button>`)}</div>`:''}</div>`}
function manage(){const t=S.manage.tab,u=cur(),hs=S.manage.health,H=healthData(hs);
 const tabs=[['overview','Overview'],['storage','Storage'],['logins','Logins'],['recovery','Recovery'],['history','History']];
 let body='';
 const IC={ok:'circle-check',info:'refresh-cw',warn:'triangle-alert',err:'octagon-x'};
 if(t==='overview'){
  const ov={ok:['Healthy','ok','circle-check',false],warn:['Needs attention','warn','triangle-alert',false],crit:['Critical','err','octagon-x',true],unknown:['Unreachable','','wifi-off',false]}[H.overall];
  const un=H.overall==='unknown';
  const card=(t_,ico,inner,pop)=>`<section class="card"><div class="card-h">${ic(ico,'s')}${t_}${pop?infoBtn(pop,t_):''}</div><div class="card-b" style="padding:.5rem 1rem">${un?'<div class="skel" style="margin:.75rem 0;width:70%"></div><div class="skel" style="margin:.75rem 0;width:50%"></div>':inner}</div></section>`;
  const dsk=H.disks.map(x=>`<div class="mini"><span class="nm">${x[0]}</span>${mono(x[1]==='—'?'—':x[1]+' °C')}<span class="grow"></span>${tag(d(x[3]),{v:x[2],i:x[2]==='ok'?'circle-check':x[2]==='err'?'circle-x':'triangle-alert',t:x[2]==='ok'?'SMART passed.':'SMART reports a problem; plan a replacement.'})}${x[2]==='err'?`<button class="link" data-act="openweb-raid">${ic('external-link','s')}Replace ${x[0]}</button>`:''}</div>`).join('');
  const arr=H.arrays.map(a=>`<div class="mini" style="height:auto;padding:.25rem 0;flex-wrap:wrap"><span class="nm">${a.n}</span>${d(a.l)}<span class="grow"></span>${tag(d(a.st),{v:a.v,i:IC[a.v]})}${a.v==='err'?`<button class="link" data-act="openweb-raid">${ic('external-link','s')}Repair ${a.n}</button>`:''}${a.pct!==undefined?`<div style="flex:0 0 100%" class="row gap12"><div class="prog grow" role="progressbar" aria-valuenow="${a.pct}"><i style="width:${a.pct}%"></i></div>${mono(a.pct+'%')}</div>`:''}</div>`).join('');
  const vol=H.vols.map(v=>`<div class="mini"><span class="nm" style="min-width:5.25rem">${v[0]}</span><div class="prog grow ${v[1]>85?'warn':''}" role="progressbar" aria-valuenow="${v[1]}"><i style="width:${v[1]}%"></i></div>${mono(v[1]+'%')}</div>`).join('');
  const svc=`<div class="row wrap" style="padding:.5rem 0;gap:.5rem">${H.svc.map(s=>`<span class="svc">${tag(d(s[0]),{v:s[1]?'ok':'err',i:s[1]?'circle-check':'circle-x'})}${s[1]?'':`<button class="btn sm" data-svc="${s[0]}">Restart</button>`}</span>`).join('')}</div>`;
  const upd=`<div class="mini"><span class="grow">${d(H.upd.n+' pending')}</span>${tag(d(H.upd.sec+' security'),{v:'warn',i:'shield-alert'})}</div>${H.upd.reboot?`<div class="mini">${tag('Reboot needed',{v:'warn',i:'rotate-ccw'})}</div>`:''}`;
  body=`<div class="toprow"><div class="card"><div class="card-b row gap12 wrap">${tag(ov[0],{v:ov[1],i:ov[2],lg:true,fill:ov[3],t:'Overall health is the worst component.'})}${H.warnings.map(w=>tag(d(w),{v:H.overall==='crit'?'err':'warn',i:'triangle-alert'})).join('')}<span class="sp"></span><button class="btn ghost icon" data-act="recheck" aria-label="Check again"${tip('Check again.')}>${ic('refresh-cw')}</button></div></div>${mgmtRow(H)}</div>
  ${un?alertH('err','wifi-off','<b>Cannot reach unit.</b> Check power, cable and network.',`<button class="btn" data-act="recheck">${ic('refresh-cw')}Retry</button><button class="btn ghost" data-go="find">Scan again</button>`)+(S.manage.checked?`<span class="sub">Checked just now.</span>`:''):''}
  <div class="hcards">${card('Disks','hard-drive',dsk,'diskcard')}${card('Arrays','layers',arr,'level')}${card('Volumes','database',vol)}${card('Services','activity',svc)}${card('Updates','package',upd)}${card('Boot default','power',`<div class="mini"><span class="grow">${d(H.boot)}</span></div><div class="mini">${d('DSM loader entry remains')}</div>`)}</div>`;
 } else if(t==='storage'){
  const A=H.arrays,hy=S.manage.layout==='hyb',cap=hy?capacity('hyb1',u.disks):null;
  const chipsOf=l=>l.map(x=>`<span class="chip">${x}</span>`).join('');
  const dm=H.disks.map(x=>({n:x[0],st:x[3],v:x[2],arr:A.filter(a=>a.mem.includes(x[0])).map(a=>a.n)}));
  body=`${hs==='faildisk'?alertH('warn','triangle-alert','<b>Array degraded.</b> Replace the disk in the web UI.'):''}
  <div class="card cardtbl"><table class="tbl"><thead><tr><th>Array</th><th>Level ${infoBtn('level','Level')}</th><th>Members</th><th>State</th><th class="r">Size</th></tr></thead><tbody>${A.map(a=>`<tr><td>${mono(a.n)}${a.slice?' '+d(a.slice):''}</td><td>${d(a.l)}</td><td><div class="chips" style="margin:0">${chipsOf(a.mem.map(m=>d(m)))}</div></td><td>${tag(a.st,{v:a.v,i:IC[a.v]})}${a.pct!==undefined?' '+mono(a.pct+'%'):''}</td><td class="r">${mono(a.sz)}</td></tr>`).join('')}</tbody></table></div>
  <div class="card cardtbl"><table class="tbl sm"><thead><tr><th>Disk</th><th>In arrays</th><th>SMART</th></tr></thead><tbody>${dm.map(x=>`<tr><td>${mono(x.n)}</td><td><div class="chips" style="margin:0">${chipsOf(x.arr.map(m=>d(m)))}</div></td><td>${tag(x.st,{v:x.v,i:x.v==='ok'?'circle-check':x.v==='err'?'circle-x':'triangle-alert'})}</td></tr>`).join('')}</tbody></table></div>
  ${hy?`<div class="card"><div class="card-b row wrap gap12">${tag('Hybrid 1-disk',{i:'layers'})}${d('LVM vg0 · /srv/data · '+TiB(cap.usable*0.8))}${infoBtn('hybman','Hybrid layout')}</div></div>`:''}
  <div class="row"><span class="sub">Changes run in the web UI.</span><button class="btn primary" data-act="openweb-raid">${ic('external-link')}Open web UI</button>${infoBtn('webraid','Why the web UI?')}</div>`;
 } else if(t==='logins'){
  body=`<div class="card"><div class="card-b" style="padding:.5rem 1rem"><dl class="dl" style="grid-template-columns:8.75rem 1fr"><dt>SSH command</dt><dd><span class="copyv">${mono('ssh andreas@'+u.ip)}<button class="btn ghost icon sm" data-copy="ssh andreas@${u.ip}" aria-label="Copy command">${ic('copy','s')}</button></span></dd><dt>Web UI</dt><dd>${mono('https://'+u.ip+':8443')}</dd><dt>Password</dt><dd>${tag('Password not kept',{i:'lock',t:'synofurb does not store it; the NAS keeps a hash.'})}</dd></dl></div></div>`;
 } else if(t==='recovery'){
  body=recoveryBody();
 } else {
  body=`<div class="card"><ul class="rows" style="padding:.25rem .5rem">${[['14:41','circle-check','Debian 13 verified'],['14:25','triangle-alert','Point of no return passed'],['14:20','hard-drive','Boot flash backed up'],['14:02','search','Inspected']].map(x=>`<li><div class="row rw">${ic(x[1],'s')}${mono(x[0])}${d(x[2])}</div></li>`).join('')}</ul></div>`;
 }
 return {html:`<div class="page"><div class="ph"><h1 class="t-title">Manage</h1></div>
  <div class="tabs" role="tablist">${tabs.map(([k,l])=>`<button role="tab" aria-selected="${t===k}" data-mtab="${k}">${l}</button>`).join('')}</div>${body}</div>`,
  foot:F(`<button class="btn ghost" data-act="diag"${tip('Copies version, OS and state; no secrets.')}>${ic('copy')}Diagnostics</button>`,'')}}
/* ---- D16 Help ---- */
function helpBody(){const h=S.help,topics=HELP;
  if(h.topic){const t=topics.find(x=>x.id===h.topic);return `<div class="row"><button class="btn ghost sm" data-help-back>${ic('arrow-left')}Back</button></div><h2>${t.title}</h2><div class="md">${t.html}</div><div class="row"><button class="link" data-act="docs">${ic('external-link','s')} Open in docs</button></div>`}
  if(h.tab==='Shortcuts')return `<table class="tbl sm"><tbody>${SHORTCUTS.map(s=>`<tr><td>${s[0].split(' / ').map(k=>`<span class="kbd">${k}</span>`).join(' ')}</td><td>${s[1]}</td></tr>`).join('')}</tbody></table>`;
  const q=h.q.toLowerCase(),l=topics.filter(t=>!q||(t.title+' '+t.text).toLowerCase().includes(q));
  return l.length?`<ul class="help-list">${l.map(t=>`<li><button data-help-topic="${t.id}">${ic('book-open','s')}${t.title}${ic('chevron-right','s r')}</button></li>`).join('')}</ul>`:`<div class="empty"><div class="big">${ic('search','xl')}</div>No topics match</div>`}
const SHORTCUTS=[['F1','Help for the focused control'],['Ctrl+,','Settings'],['Ctrl+F','Search'],['Alt+Left / Alt+Right','Previous / next completed step'],['F6','Jump between regions'],['Ctrl+Shift+C','Show the command'],['Esc','Close popover or dialog; never cancels an install'],['Space','Press the focused button (Enter does not erase)']];
function openHelp(topic){S.help.topic=topic||null;S.help.q=S.help.q||'';S.help.tab=S.help.tab||'Topics';
  const sc=openSheet({title:'Help',body:'<div id="hb-top"></div><div id="hb"></div>',foot:`<button class="link" data-act="docs">${ic('external-link','s')} Open docs</button>`});
  S.help.open=true;document.documentElement.dataset.sheet='open';paintHelp();const i=$('#hq');(topic?$('[data-help-back]',sc):i)&&(topic?$('[data-help-back]',sc):i).focus()}
function paintHelp(){const top=$('#hb-top');if(!top)return;const h=S.help;const ae=document.activeElement,keep=ae&&ae.id==='hq'?ae.selectionStart:null;
  top.innerHTML=h.topic?'':`<div class="inp">${ic('search','s')}<input id="hq" placeholder="Search help" value="${esc(h.q)}" aria-label="Search help"></div><div class="tabs" role="tablist" style="margin-top:.5rem">${['Topics','Shortcuts'].map(t=>`<button role="tab" aria-selected="${h.tab===t}" data-help-tab="${t}">${t}</button>`).join('')}</div>`;
  $('#hb').innerHTML=helpBody();if(keep!==null){const i=$('#hq');if(i){i.focus();i.setSelectionRange(keep,keep)}}}
/* ---- D17 About ---- */
function about(){const cr=[['Debian','DFSG licences'],['Lucide','ISC'],['Inter','OFL 1.1'],['JetBrains Mono','OFL 1.1'],['GPUI Kit, gpui-component','Apache-2.0'],['synodeb note','GPL-2.0, technique only']];
 return {html:`<div class="page narrow"><div class="ph"><h1 class="t-title">About</h1></div>
 <div class="col" style="gap:.75rem;padding:.5rem 0"><div style="width:17.5rem">${LOGO_FULL}</div><div class="sub">Reinstall Synology NAS units with Debian.</div><div>${tag('0.9.2 · build 4c1e9d2',{rd:true})}</div></div>
 <div class="row wrap"><button class="btn primary" data-act="diag"${tip('Version, OS, settings; no secrets.')}>${ic('copy')}Copy diagnostics</button><button class="btn ghost" data-act="docs">${ic('book-open')}Documentation</button><button class="btn ghost" data-act="docs">${ic('external-link')}Source code</button><button class="btn ghost" data-act="docs">${ic('bug')}Report issue</button></div>
 <div>${acc('credits','Credits',`<ul class="rows">${cr.map(c=>`<li><div class="row rw">${d(c[0])}<span class="sp"></span><button class="link" data-dlg="licence">${c[1]}</button></div></li>`).join('')}</ul>`)}</div></div>`,foot:F('',btn('Close',{go:S.back||'find'}))}}
