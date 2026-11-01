/* ===== Setup (Storage + System), Confirm, Install, Install stopped, Done, Settings ===== */
/* ---- D5 Setup / Storage ---- */
function hybGain(u,cap){const mx=Math.max(...u.disks.map(x=>x[2])),mn=Math.min(...u.disks.map(x=>x[2]));if(mx/mn<1.1||/^hyb/.test(S.plan.raid))return 0;const h=capacity('hyb1',u.disks);return h.error||cap.error?0:+(h.usable-cap.usable).toFixed(2)}
function planDyn(){const u=cur(),p=S.plan,cap=capacity(p.raid,u.disks),keep=p.mode==='keep',bad=u.disks.find(x=>x[3]);
  const err=keep&&p.sys>9?'Max 9 GiB':keep&&p.sys<8?'Min 8 GiB':cap.error&&!keep?cap.error:null;
  const dv=cap.error?0:+(cap.usable*(1-p.free/100)).toFixed(2);
  const raw=TiB(u.disks.reduce((a,x)=>a+x[2],0)),sysAll=u.disks.length*10;
  const alerts=(err?alertH('err','octagon-x',`<b>${err}</b>`,keep?`<button class="btn" data-act="setsys">${p.sys>9?'Set 9 GiB':'Set 8 GiB'}</button>`:`<button class="btn" data-act="rawauto">Set Auto</button>`):'')
   +(cap.weak&&!keep?alertH('warn','triangle-alert',`<b>Weaker slice</b> ${infoBtn('slices2','Weaker slice')}`):'')
   +(bad?alertH('warn','triangle-alert',`<b>Disk warning</b> <span class="d">${bad[0]} · ${bad[3]}</span>`):'');
  const right=`<div class="dgs"><div class="card"><div class="card-h">Now ${tag(d('DSM'),{i:'server'})}</div><div class="card-b">${diagram('now')}</div></div>
   <div class="card"><div class="card-h">After ${infoBtn('legend','Colour key')}<span class="sp"></span>${tag(keep?'Keep':'Wipe',{v:keep?'warn':'err',i:keep?'shield-check':'trash'})}</div><div class="card-b">${err&&!keep?`<div class="sub">${d(err)}</div>`:diagram('after')}</div></div></div>`;
  const sum=`<span class="sumbar">${tag('Destroyed',{v:'err',i:'trash',t:keep?'DSM system and swap · '+sysAll+' GiB':u.disks.map(x=>x[0]).join(' ')+' · '+raw+' · '+u.vol.split(' · ')[0]})} ${d(keep?sysAll+' GiB':raw)}${tag('Changed',{v:'warn',i:'triangle-alert',t:'Boot flash · entries added, backed up first'})} ${d('Boot flash')}${tag('Preserved',{i:'shield-check',t:keep?'volume1 · best effort, no guarantee':'Wipe mode preserves nothing on the disks.'})} ${d(keep?'volume1':'Nothing')}</span>`;
  return {cap,err,alerts,right,sum,dv}}
function planTags(cap,u){if(cap.error)return '';const g=hybGain(u,cap);return `<span class="hc">${tag('Usable',{i:'database'})}${mono(TiB(cap.usable))}</span><span class="hc">${tag('Survives',{i:'shield-check'})}${d(cap.survives)}</span>${cap.slices.length?`<button class="link" data-dlg="slices">Slices</button>`:''}${g>0.5?`<button class="tag info" data-pop="hybsug">${ic('layers','s')}Hybrid: +${g.toFixed(1)} TiB</button>`:''}`}
function storageTab(){const u=cur(),p=S.plan,D=planDyn(),keep=p.mode==='keep',cap=D.cap;
 const ctrl=`<div class="card"><div class="card-b col gap16">
  <div class="fld"><span class="lab">Data RAID ${infoBtn('raid','Data RAID')}</span>${selectH('pl-raid',{opts:()=>RAID_OPTS,get:()=>S.plan.raid,set:v=>{S.plan.raid=v;render()},label:'Data RAID',disabled:keep})}
   <div class="row wrap" id="pl-tags">${keep?'':planTags(cap,u)}</div></div>
  <div class="fld"><span class="lab">${lab('Mode','Wipe erases all '+u.disks.length+' disks.')}</span><div class="seg" role="group" style="align-self:flex-start"><button aria-pressed="${!keep}" data-plan-mode="wipe">${ic('trash','s')}Wipe all</button>${keep?`<button aria-pressed="true" data-plan-mode="keep">Keep</button>`:''}</div><button class="link" data-dlg="keep" style="align-self:flex-start">Advanced</button></div>
  <div class="fld"><span class="lab">${lab('System size','Mirrored on every copy; 32 GiB is recommended.')}</span><div class="slider"><input type="range" id="pl-sys" min="4" max="64" step="1" value="${p.sys}" aria-label="System size in GiB"><span class="val d" id="pl-sysv">${p.sys} GiB</span></div></div>
  <div class="fld"><span class="lab">${lab('Mirror copies','More copies survive more disk failures.')}</span><div class="seg" role="group" style="align-self:flex-start">${[2,3,4].map(n=>`<button aria-pressed="${p.copies===n}" data-plan-copies="${n}"${n>u.disks.length?` aria-disabled="true" data-reason="This unit has only ${u.disks.length} disks."`:''}>${n}</button>`).join('')}</div></div>
  <div class="fld"><span class="lab">${lab('Free space','Left unallocated for growth and snapshots.')}</span><div class="slider"><input type="range" id="pl-free" min="0" max="50" step="5" value="${p.free}" aria-label="Free space percent"><span class="val d" id="pl-freev">${p.free}%</span></div></div></div></div>`;
 return `<div class="plan"><div class="col gap16">${ctrl}</div><div class="col gap16"><div id="pl-right">${D.right}</div><div id="pl-sum">${D.sum}</div><div id="pl-alerts" class="col">${D.alerts}</div></div></div>`}
/* ---- D6 Setup / System ---- */
function systemTab(){const y=S.sys,u=cur(),cap=capacity(S.plan.raid,u.disks);
 const seg=(k,opts,v)=>`<div class="seg" role="group" style="align-self:flex-start">${opts.map(o=>`<button aria-pressed="${v===o}" data-sys-${k}="${o}">${o}</button>`).join('')}</div>`;
 const f=(l,h,tp,full)=>`<div class="fld"${full?' style="grid-column:1/-1"':''}><span class="lab">${tp?lab(l,tp):l}</span>${h}</div>`;
 return `<div class="card"><div class="card-b fgrid">
  ${f('Hostname',`<input class="inp mono" id="s-host" value="${esc(y.host)}" aria-label="Hostname">`)}${f('User',`<input class="inp" id="s-user" value="${esc(y.user)}" aria-label="User">`)}
  ${f('Password',`<div class="inp"><input class="mono" type="password" id="s-pw" value="${esc(y.pw)}" aria-label="Password"><button class="btn ghost icon sm" data-act="peek2" aria-label="Show password">${ic('eye','s')}</button><button class="btn ghost icon sm" data-act="copy-pw" aria-label="Copy password">${ic('copy','s')}</button><button class="btn ghost icon sm" data-act="regen" aria-label="Regenerate"${tip('Generate a new password.')}>${ic('refresh-cw','s')}</button></div>`,'Generated; copy it on the last screen.')}
  ${f('SSH key',seg('key',['Generate','Paste','None'],y.key)+(y.key==='Paste'?`<input class="inp mono" id="s-pub" placeholder="ssh-ed25519 AAAA…" value="${esc(y.pubkey)}" aria-label="Public key">`:''),'The key pair is created on this computer.')}
  ${f('Time zone',selectH('s-tz',{opts:()=>[['Europe/Stockholm','Europe/Stockholm'],['Europe/Berlin','Europe/Berlin'],['America/New_York','America/New_York']],get:()=>S.opt.tz,set:v=>{S.opt.tz=v;render()},label:'Time zone'}))}
  ${f('Console',`<div class="mono" style="padding:.25rem 0">${d(u.con)}</div>`,'Read from DSM; a USB serial adapter also works.')}
  ${f('Kernel',seg('kernel',['Stable','Newest'],y.kernel)+'')}
  ${f('Source',seg('source',['Online','Offline'],y.source))}
  ${f('Web UI',`<label class="sw"><input type="checkbox" data-sys-web ${y.web?'checked':''} aria-label="Web UI"><span></span></label>`,'HTTPS on port 8443, self-signed certificate.')}
  ${f('Data layout',`<div class="row"><button class="tag" data-go="plan"${tip('Chosen under Storage; click to change it.')}>${ic('layers','s')}${esc(LEVEL_LABEL[cap.level]||'RAID')} · ${cap.error?'—':TiB(cap.usable)}</button></div>`)}
  ${f('Boot via',`<div class="row">${tag('Boot flash',{i:'hard-drive'})}${infoBtn('bootvia','Boot via')}</div>`)}
  ${y.source==='Offline'&&!S.bundle?`<div class="fld" style="grid-column:1/-1">${alertH('err','octagon-x','<b>No bundle</b>',`<button class="btn" data-go="settings" data-settings="Data">${ic('download')}Download</button>`)}</div>`:''}
 </div></div>`}
function setup(){const tab=S.setup.tab,D=planDyn(),y=S.sys,off=tab==='storage'&&D.err?D.err:(y.source==='Offline'&&!S.bundle?'No offline bundle yet.':''),toSys=tab==='storage'&&!S.setup.seenSys;
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Setup</h1><div class="sub t-sub">Nothing is written yet.</div></div></div>
  <div class="tabs" role="tablist">${[['storage','Storage'],['system','System']].map(([k,l])=>`<button role="tab" aria-selected="${tab===k}" data-setup-tab="${k}">${l}</button>`).join('')}</div>${tab==='storage'?storageTab():systemTab()}</div>`,
  foot:F(back('goal')+cmdBtn(),btn(toSys?'Next: System':'Next: Confirm',{cls:'primary',id:'f-next',trail:'arrow-right',go:toSys?'':'confirm',act:toSys?'nextsys':'',off:toSys&&D.err?D.err:(!toSys?off:'')})) }}
function refreshPlan(){const D=planDyn(),u=cur();$('#pl-alerts').innerHTML=D.alerts;$('#pl-right').innerHTML=D.right;$('#pl-sum').innerHTML=D.sum;const t=$('#pl-tags');if(t)t.innerHTML=S.plan.mode==='keep'?'':planTags(D.cap,u);
  const n=$('#f-next');if(n){n.setAttribute('aria-disabled',!!D.err);if(D.err)n.setAttribute('data-reason',D.err);else n.removeAttribute('data-reason');const w=n.closest('.dbw');if(w){const i=$('.ib',w);if(i&&D.err)i.dataset.reason=D.err}if(!D.err&&n.closest('.dbw')){render()}else if(D.err&&!n.closest('.dbw')){render()}}}
/* ---- command = plan (the one accessible path must carry every reviewed choice) ---- */
const CMDKEYS=[['mode','mode'],['raid','raid'],['copies','copies'],['system','system-size'],['free','free'],['hostname','hostname'],['login','login'],['key','ssh-key'],['kernel','kernel'],['source','source'],['webui','web-ui'],['tz','timezone'],['boot','boot-via']];
function planObj(){const p=S.plan,y=S.sys,keep=p.mode==='keep',u=cur();return {host:u.ip,user:S.conn.user,mode:p.mode,raid:keep?'keep-volumes':p.raid,copies:String(p.copies),system:p.sys+'G',free:keep?'0':String(p.free),hostname:y.host,login:y.user,key:y.key.toLowerCase(),kernel:y.kernel.toLowerCase(),source:y.source.toLowerCase(),webui:y.web?'on':'off',tz:S.opt.tz,boot:'flash'}}
function buildCmd(gate){const o=planObj(),a=['synofurb install','--host '+o.host,'--user '+o.user];CMDKEYS.forEach(([k,f])=>{if(k==='raid'&&o.mode==='keep')return;a.push('--'+f+' '+o[k])});if(gate)a.push('--have-backup','--accept-data-loss','--confirm "'+PHRASE()+'"');return a.join(' ')}
function parseCmd(s){const o={},re=/--([a-z-]+)(?: ("[^"]*"|[^\s-][^\s]*))?/g;let m;while((m=re.exec(s)))o[m[1]]=(m[2]||'').replace(/"/g,'');return o}
function cmdMatches(s){const o=planObj(),p=parseCmd(s);return o.host===p.host&&o.user===p.user&&CMDKEYS.every(([k,f])=>(k==='raid'&&o.mode==='keep')||p[f]===o[k])}
/* ---- D7 Confirm ---- */
const PHRASE=()=>{const u=cur();return 'ERASE '+u.model+(twinOf(u)?' '+u.sn.slice(-4):'')};
const norm=s=>s.trim();
function gateOk(){const g=S.gate;return g.b1&&g.b2&&g.b3&&norm(g.phrase)===PHRASE()}
function missingList(){const g=S.gate,m=[];if(!g.b1)m.push('tested backup');if(!g.b2)m.push('data loss accepted');if(!g.b3)m.push('unit confirmed');if(norm(g.phrase)!==PHRASE())m.push('the phrase');return m}
const missing=()=>'Missing: '+missingList().join(', ')+'.';
const hostOf=u=>u.model.toLowerCase().replace(/[^a-z0-9]/g,'');
function afterText(){const u=cur(),p=S.plan,cap=capacity(p.raid,u.disks);if(p.mode==='keep')return 'Debian 13 · volume1 kept · system '+p.sys+' GiB × '+p.copies+' · '+S.sys.host;return 'Debian 13 · '+(cap.error?'—':LEVEL_LABEL[cap.level]+' · '+TiB(cap.usable)+' usable')+' · system '+p.sys+' GiB × '+p.copies+' · '+S.sys.host}
function confirm(){const u=cur(),g=S.gate,bad=u.disks.find(x=>x[3]),keep=S.plan.mode==='keep',tw=twinOf(u),nobk=u.bk==='none found',raw=TiB(u.disks.reduce((a,x)=>a+x[2],0));
 const li=(i,t,sz)=>`<li>${ic(i,'s')}<span>${d(t)}</span><span class="sz d">${sz}</span></li>`;
 const chk=(k,l)=>`<label class="chk"><input type="checkbox" data-gate="${k}" ${g[k]?'checked':''}><span class="bx">${ic('check')}</span><span>${l}</span></label>`;
 const ok=gateOk();
 const bkTxt=keep?`<b>${nobk?'No backup found.':'Test your backup.'}</b> ${d('volume1 · '+u.used)} may still be lost. ${infoBtn('backup','About backups')}`:`<b>${nobk?'No backup found.':'Test your backup.'}</b> ${d(u.used+' on volume1')} is erased. ${infoBtn('backup','About backups')}`;
 const bk=alertH(nobk?'err':'warn',nobk?'octagon-x':'triangle-alert',bkTxt);
 const twa=tw?alertH('warn','triangle-alert',`<b>Twin</b> ${d(tw.ip+' · SN '+snTail(tw))}`):'';
 const tri=`<div class="triad"><div class="tri dn"><div class="h">${ic('trash','s')}Destroyed</div><div class="v">${d(keep?u.disks.length*10+' GiB DSM system':u.disks.length+' disks · '+raw)}</div></div><div class="tri ch"><div class="h">${ic('triangle-alert','s')}Changed ${infoTxt('Entries and files are added; the whole boot flash is backed up first.','About the boot flash')}</div><div class="v">${d('Boot flash · backed up first')}</div></div><div class="tri ok"><div class="h">${ic('shield-check','s')}Preserved ${infoTxt(keep?'volume1 is left alone on a best-effort basis, with no guarantee.':'Wipe mode preserves nothing on the disks.','About preserved data')}</div><div class="v">${d(keep?'volume1 · best effort':'Nothing')}</div></div></div>`;
 const list=`<div class="grp" role="group" aria-label="Details"><ul>${li('layers',afterText(),'')}${(keep?li('hard-drive','DSM system and swap',u.disks.length*10+' GiB')+li('hard-drive','DSM settings · packages','—'):u.disks.map((x,i)=>li('hard-drive',x[0]+' · '+x[1]+' · '+dsn(u,i),TiB(x[2]))).join('')+li('database',u.vol.split(' · ').slice(0,2).join(' · '),u.used+' used')+li('server','DSM · settings · users',''))}</ul></div>`;
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Confirm erase</h1></div><span class="sp"></span>${bad?tag(d(bad[0]),{v:'warn',i:'triangle-alert',t:bad[0]+': '+bad[3]+'; consider replacing it first.'}):''}</div>
 <div class="cgrid"><div class="ca col gap16">${bk}${twa}</div>
  <aside class="dz" aria-label="Confirmation"><div class="dz-h">${ic('octagon-x','l')}<span>${d(PHRASE())}</span></div><div class="dz-b">
   ${chk('b1','I have a tested backup')}${chk('b2','I accept data loss')}${chk('b3','This is '+d(u.ip+' · '+snTail(u)))}
   <div class="fld"><span class="lab">${d(PHRASE(),'code')}</span><div class="inp ${phraseState()==='ok'?'ok':''}" id="phw"><input id="g-phrase" autocomplete="off" spellcheck="false" value="${esc(g.phrase)}" aria-label="Type the phrase" class="mono"><span id="phtag">${phraseTag()}</span></div>${tw?`<span class="sub">Add serial last 4.</span>`:''}</div></div></aside>
  <div class="cr col gap16"><div class="row"><span class="sub">Free to cancel until step 3; then no undo.</span>${infoBtn('recov','How recovery works')}</div>${tri}${list}</div></div></div>`,
  foot:F(back('setup')+cmdBtn(),(ok?'':`<button type="button" class="ib" data-reason="${esc(missing())}" aria-label="Why locked"${tip('Why locked')}>${ic('info','s')}</button>`)+`<button class="btn danger" id="f-erase" data-act="erase" ${ok?'':`aria-disabled="true" tabindex="-1" data-reason="${esc(missing())}"`}>${ic('trash')}Erase and install</button>`)}}
/* the typed-phrase feedback is neutral progress ("n of m") while the text is a prefix of the phrase; a red "No match" only after a wrong character */
function phraseState(){const g=S.gate,v=norm(g.phrase),P=PHRASE();if(!v)return '';if(v===P)return 'ok';return P.startsWith(v)?'typing':'bad'}
function phraseTag(){const s=phraseState();return s==='ok'?tag('Matches',{v:'ok',i:'circle-check'}):s==='bad'?tag('No match',{v:'err',i:'circle-x'}):s==='typing'?tag(d(norm(S.gate.phrase).length+' of '+PHRASE().length),{}):''}
function refreshGate(){const ok=gateOk(),b=$('#f-erase');if(b){b.setAttribute('aria-disabled',!ok);if(ok){b.removeAttribute('data-reason');b.removeAttribute('tabindex')}else{b.setAttribute('data-reason',missing());b.setAttribute('tabindex','-1')}const w=b.parentElement;let i=$('.ib[aria-label="Why locked"]',w);if(ok&&i)i.remove();else if(!ok&&!i){b.insertAdjacentHTML('beforebegin',`<button type="button" class="ib" data-reason="${esc(missing())}" aria-label="Why locked"${tip('Why locked')}>${ic('info','s')}</button>`)}else if(i)i.dataset.reason=missing()}
  const t=$('#phtag');if(t)t.innerHTML=phraseTag();const w=$('#phw');if(w)w.classList.toggle('ok',phraseState()==='ok')}
/* ---- D8 Install ---- */
const FLASH=['Boot flash backup','Boot agent added','Reboot into the agent','Send installer'];
const ISTEPS=['Load installer components','Preflight: match disks','Release RAID/LVM, wipe reclaimed areas','Partition the disks','Create RAID arrays','Create LVM volumes','Create filesystems','Mount the new system','Install the Debian base system','Write fstab and unpack the payload','Configure packages, users, boot','Finish: save logs, unmount, reboot'];
const LOGS=[['14:19:44','i','dom: streaming boot flash 128 MiB'],['14:19:58','i','dom: sha256 NAS == local: OK'],['14:20:41','i','ssh: reboot into SYNOFURB_BOOT'],['14:21:52','i','agent: installer download 73/118 MiB'],['14:25:07','i','@@STEP 3/12 release RAID/LVM'],['14:25:49','w','mdadm: md1 initial resync will run in the background (about 9 h)'],['14:27:02','i','@@STEP 9/12 install the Debian base system'],['14:28:51','i','debootstrap: unpacking libc6:amd64 (2.41-12)']];
const LVL={i:['info','INFO'],w:['triangle-alert','WARN'],e:['circle-x','ERROR']};
const logLine=l=>`<div class="${l[1]}"><time>${l[0]}</time><span class="lv">${ic(LVL[l[1]][0],'s')}${LVL[l[1]][1]}</span><span class="m">${esc(l[2])}</span></div>`;
function install(){const s=S.inst.step,n=instN(),pct=Math.min(100,Math.round(Math.max(0,s)/17*100)),post=n>=3;
 const flash=FLASH.map((t,i)=>{const c=s>i?'done':s===i?'run':'';return `<li class="d ${c}"><span class="si">${c==='done'?ic('check'):''}</span>${t}</li>`}).join('');
 const steps=ISTEPS.map((t,i)=>{const k=i+4,c=s>k?'done':s===k?'run':'';return (i===2?`<li class="pnr" role="presentation">${ic('flag','s')}No return</li>`:'')+`<li class="d ${c}"><span class="si">${c==='done'?ic('check'):''}</span><span class="grow">${i+1}. ${t}</span></li>`}).join('');
 const nl=Math.max(1,Math.min(LOGS.length,Math.ceil((s+1)/2.4))),lg=LOGS.slice(Math.max(0,nl-6),nl).map(logLine).join(''),last=LOGS[nl-1];
 const stepName=s<4?FLASH[Math.max(0,s)]:s<16?ISTEPS[s-4]:s===16?'Waiting for reboot':'Verifying';
 const where=s<4?'Phase 3 of 7':s<16?'Step '+Math.max(1,n)+' of 12':s===16?'Phase 6 of 7':'Phase 7 of 7';
 const ticks=Array.from({length:12},(_,i)=>`<i class="t ${n>i+1?'done':n===i+1?'run':''}"></i>`).join('');
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Installing</h1><div class="sub t-sub">Keep this window open.</div></div><span class="sp"></span><button class="tag acc rd lg" data-pop="phases">${ic('loader-circle','s')}${PHASES[phaseIdx()]}</button></div>
  ${S.inst.lost?alertH('err','wifi-off','<b>Connection lost.</b> The NAS keeps installing.',`<button class="btn" data-act="reconnect">${ic('refresh-cw')}Reconnect</button>`):''}
  <div class="card"><div class="card-b col"><div class="row"><span class="big-num">${pct}%</span><div class="grow col" style="gap:.125rem">${d('<b>'+stepName+'</b>')}${d(where)}</div><span class="mono sub" id="eta">${d('about '+Math.max(1,Math.round((17-s)*1.3))+' min left')}</span></div><div class="prog lg ${s<0?'ind':''}" role="progressbar" aria-valuenow="${pct}" aria-valuemin="0" aria-valuemax="100"><i style="width:${pct}%"></i></div>
   <div class="pstrip" aria-label="Installer steps"><div class="ticks">${ticks}</div><div class="pflag ${post?'passed':''}" style="left:${(2/12*100).toFixed(2)}%">${ic('flag','s')}<span>No return${post?' passed':''}</span></div></div></div></div>
  <div class="col">${acc('isteps','Steps',`<ol class="stp">${steps}</ol>`,false)}${acc('iflash','Boot flash',`<ol class="stp d">${flash}</ol>`,s<4,tag('Backed up',{v:'ok',i:'shield-check'}))}${acc('ilog','Log',`<div class="logbox d" role="log">${lg}</div><div class="row" style="margin-top:.5rem"><button class="btn sm" data-act="copylog">${ic('copy')}Copy</button><button class="btn sm" data-act="openlog">${ic('file-text')}Open file</button></div>`,false,`<span class="logone d">${S.acc.ilog?'':esc(last[0]+' '+last[2])}</span>`)}</div></div>`,
  foot:F('',btn('Cancel',{id:'f-cancel',act:'cancelinst',off:post?'Not possible after step 3.':''})),after(){const r=$('.stp li.run');if(r&&S.acc.isteps)r.scrollIntoView({block:'nearest'})}}}
/* ---- D9 Install stopped ----
   kinds: stopped (by you, before step 3), pre (failed before step 3), agent (staging: the agent never answered; 30 min watchdog), backup (staging: flash backup did not verify; nothing changed), post (after step 3). */
const STAGE_NAMES=['Connecting','Inspecting','Boot flash backup','Boot agent added','Reboot into the agent','Send installer','Verifying'];
function mkFail(kind,o={}){const post=kind==='post',stage=kind==='agent'||kind==='backup';return Object.assign({kind,keep:false,phase:kind==='backup'?'done':'clock',t:post?900:kind==='agent'?1800:60,el:0,out:null,step:kind==='stopped'?Math.max(1,instN()||1):post?11:kind==='agent'?5:kind==='backup'?3:2,rpw:false},o)}
const fmt=t=>t>=3600?Math.floor(t/3600)+':'+String(Math.floor(t%3600/60)).padStart(2,'0')+':'+String(t%60).padStart(2,'0'):Math.floor(t/60)+':'+String(t%60).padStart(2,'0');
const STEPNAMES=['Load installer components','Preflight: match disks','Release RAID/LVM, wipe reclaimed areas','Partition the disks','Create RAID arrays','Create LVM volumes','Create filesystems','Mount the new system','Install the Debian base system','Write fstab and unpack the payload','Configure packages, users, boot','Finish: save logs, unmount, reboot'];
function failed(){const f=S.fail,post=f.kind==='post',stopped=f.kind==='stopped',stage=f.kind==='agent'||f.kind==='backup',u=cur(),ph=f.phase;
 const untouched=!post&&!(S.F.disks);
 const sub=stage?(f.kind==='backup'?'Phase 3 of 7':'Phase 5 of 7'):'Step '+f.step+' of 12';
 const cause={pre:'Disk changed after confirm',post:'Boot entry check failed',agent:'The agent did not answer',backup:'Flash backup did not verify'}[f.kind];
 const meaning=post?(f.step>=11?'Debian is installed.':'Disks are partly changed.'):(stage?(f.kind==='backup'?'Nothing was changed. DSM is untouched.':'DSM is untouched.'):'');
 const alertTop=stopped?alertH('info','info','<b>Stopped by you.</b>'):alertH('err','octagon-x',`<b>${cause}</b> ${infoBtn('cause','Engine message')}`);
 const clockLbl=post?'Rescue shell closes in':f.kind==='agent'?'Agent reboots to DSM in':'DSM returns in';
 const clockInfo=post?'Then DSM becomes the default and the NAS reboots into the DSM loader.':f.kind==='agent'?'The agent’s watchdog sets DSM as the default and reboots. Power-cycling the NAS now also boots DSM.':'Then DSM becomes the default and the NAS reboots.';
 let card='';
 if(ph==='clock')card=`<div class="card"><div class="card-b row gap16"><div class="col" style="gap:.25rem"><span class="sub">${clockLbl} ${infoTxt(clockInfo,'About the clock')}</span><span class="cd" id="cd" role="timer">${fmt(f.t)}</span></div><span class="sp"></span>${meaning?`<span class="sub">${meaning}</span>`:''}</div></div>`;
 else if(ph==='waiting')card=`<div class="card"><div class="card-b row gap16"><div class="col" style="gap:.25rem"><span class="sub">Waiting for DSM</span><span class="cd" id="cd" role="timer">${fmt(f.el)}</span></div><span class="sp"></span>${post?'<span class="sub">The shell closed. The NAS is rebooting.</span>':''}${ic('loader-circle','xl spin')}</div></div>`;
 else if(ph==='done')card=alertH('ok','shield-check',`<b>${meaning}</b> DSM is still running.`);
 else if(ph==='back')card=alertH('ok','circle-check','<b>DSM is back.</b> The NAS answered on its DSM address.');
 else if(ph==='notback')card=alertH('err','octagon-x','<b>DSM did not return.</b> Power-cycle the NAS.')
 else if(ph==='loader')card=alertH('warn','triangle-alert','<b>The NAS is in the DSM loader.</b> DSM’s system may be gone.');
 else if(ph==='lost')card=alertH('warn','wifi-off','<b>Lost contact with the NAS.</b> The clock keeps running on the NAS.');
 const acts={notback:`<button class="btn" data-act="waitdsm">${ic('refresh-cw')}Check again</button>`,loader:`<button class="btn" data-dlg="reinstall">${ic('life-buoy')}Reinstall DSM</button>`,lost:`<button class="btn" data-act="reconnect">${ic('refresh-cw')}Reconnect</button>`}[ph];
 const cardH=acts?`<div class="alrow">${card}${acts}</div>`:card;
 const shellOpen=post&&(ph==='clock'||ph==='lost');
 const rescue=shellOpen?`<section class="card" aria-label="Rescue"><div class="card-h">${ic('terminal','s')}Rescue ${infoBtn('rescueinfo','Rescue shell')}<span class="sp"></span><span class="sub">Export first.</span></div><div class="card-b col">
   <div class="copyv">${mono('ssh root@'+u.ip)}<button class="btn ghost icon sm" data-copy="ssh root@${u.ip}" aria-label="Copy command"${tip('Copy')}>${ic('copy','s')}</button></div>
   <div class="copyv"><span class="sub">Password</span><span class="mono" id="rpw">${f.rpw?'rescue-Tn4k-Qe7':'•'.repeat(12)}</span><button class="btn ghost icon sm" data-act="showrpw" aria-label="${f.rpw?'Hide':'Show'} password">${ic(f.rpw?'eye-off':'eye','s')}</button><button class="btn ghost icon sm" data-copy="rescue-Tn4k-Qe7" aria-label="Copy password"${tip('Copy')}>${ic('copy','s')}</button></div>
   <div class="row"><button class="btn sm" data-dlg="rescuenotes">${ic('file-text')}Rescue notes</button></div></div></section>`:'';
 const dsmBack=ph==='back';
 const st=[['Disks','hard-drive',untouched?['Untouched','ok','shield-check']:['Changed','err','triangle-alert']],
  ['Boot flash','power',f.kind==='backup'?['Untouched','ok','shield-check']:post?['Restored','ok','circle-check']:['Entries added','warn','triangle-alert']],
  ['DSM','server',f.kind==='backup'?['Running','ok','circle-check']:(post&&!dsmBack)||ph==='loader'?['Loader only','warn','triangle-alert']:['Default','ok','circle-check']]];
 const rec=btn('Recovery options',{go:'recovery'});
 const foot=ph==='back'?F('',rec+btn('Done',{cls:'primary',go:'find'}))
  :ph==='done'?F('',rec+btn('Back to Confirm',{cls:'primary',go:'confirm',icon:'arrow-left'}))
  :ph==='notback'?F('',rec+btn('Check again',{cls:'primary',act:'waitdsm',icon:'refresh-cw'}))
  :ph==='loader'?F('',rec+btn('Reinstall DSM',{cls:'primary',act:'reinstall',icon:'life-buoy'}))
  :ph==='lost'?F('',rec+btn('Reconnect',{cls:'primary',act:'reconnect',icon:'refresh-cw'}))
  :ph==='waiting'?F('',rec)
  :!post?F('',rec+btn('Wait for DSM',{cls:'primary',act:'waitdsm',icon:'loader-circle'}))
  :F('',btn('Export diagnostics',{tipT:'Do this before the clock ends; secrets are removed.',icon:'download',act:'export'})+rec+btn('Open rescue shell',{cls:'primary',act:'rescue',icon:'terminal'}));
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Install stopped</h1><div class="sub t-sub">${sub}</div></div></div>
  ${ph==='clock'||ph==='waiting'||ph==='done'?alertTop:''}${cardH}${rescue}
  <div class="cards3 statecards">${st.map(([l,i,t])=>`<div class="card"><div class="card-b row">${ic(i,'l')}<b>${l}</b><span class="sp"></span>${tag(t[0],{v:t[1],i:t[2]})}</div></div>`).join('')}</div>
  <div class="row"><button class="link" data-dlg="next">What happens next</button></div>${acc('flog','Log excerpt',`<div class="logbox d">${logLine(['14:27:19','e',stopped?'user: stop requested before step 3':f.kind==='backup'?'dom: sha256 NAS != local (flash backup rejected)':f.kind==='agent'?'ssh: agent did not answer within 180 s':pre_(f)?'preflight: sdc partition table differs from the plan':'dom: read-back checksum mismatch on p2 (2/2)'])}${logLine(['14:27:19','e',stage?'@@FAIL phase '+(f.kind==='backup'?3:5)+'/7 staging stopped':'@@FAIL '+f.step+'/12 refusing to continue'])}</div>`)}</div>`,foot}}
const pre_=f=>f.kind==='pre';
/* ---- D10 Done ---- */
function done(){const u=cur(),y=S.sys,dn=S.done;
 const row=(l,v,extra='',tp)=>`<dt>${tp?lab(l,tp):l}</dt><dd><span class="copyv">${v}${extra}</span></dd>`;
 const cp=(t,l)=>`<button class="btn ghost icon sm" data-copy="${esc(t)}" aria-label="Copy ${l||''}"${tip('Copy')}>${ic('copy','s')}</button>`;
 const bad=u.disks.find(x=>x[3]);
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Done</h1><div class="sub t-sub">Debian is running.</div></div><span class="sp"></span>${tag('Verified over SSH',{v:'ok',i:'circle-check',lg:true})}</div>
 <div class="card"><div class="card-b" style="padding:.5rem 1rem"><dl class="dl" style="grid-template-columns:8.75rem 1fr">
 ${row('Address',mono(u.ip),cp(u.ip,'address'))}${row('User',mono(y.user),cp(y.user,'user'))}
 ${row('Password',`<span class="mono" id="pwv">${dn.show?esc(y.pw):'•'.repeat(12)}</span>`,`<button class="btn ghost icon sm" data-act="showpw" aria-label="${dn.show?'Hide':'Show'} password">${ic(dn.show?'eye-off':'eye','s')}</button><button class="btn sm" data-act="copy-pw2">${ic(dn.copied?'check':'copy','s')}${dn.copied?'Copied':'Copy password'}</button>`,'synofurb does not store it and will not show it again.')}
 ${row('SSH',mono('ssh -i ~/.ssh/synofurb_ds918 '+y.user+'@'+u.ip),cp('ssh -i ~/.ssh/synofurb_ds918 '+y.user+'@'+u.ip,'command'))}
 ${row('Host key',mono('SHA256:Hd8vN0cYk2tqLxP4…KyVd6E'),cp('SHA256:Hd8vN0cYk2tqLxP4wZr1eJm7aB9sUf3oGiX5nKyVd6E','host key'),'Compare the full value when the browser warns.')}
 ${row('Web UI',mono('https://'+u.ip+':8443'),btn('Open',{cls:'sm',icon:'external-link',act:'openweb'}))}
 ${row('Certificate',mono('5E:91:C4:0B…D8:3C:A1'),`<button class="btn sm" data-pop="cert">Show full</button>`)}</dl></div></div>
 ${bad?alertH('warn','triangle-alert',`<b>Replace ${d(bad[0])} soon.</b> ${d(bad[3])}`):''}
 <div class="row gap16 wrap"><span class="sp"></span><button class="btn" data-dlg="report">${ic('external-link')}Report result</button></div>
 ${acc('next','Next steps',`<ul style="margin-left:1.25rem"><li>${d('Open the web UI and sign in')}</li><li>${d('RAID sync runs for about 9 hours')}</li><li>${d('Reboot once to check it starts')}</li></ul>`)}</div>`,
 foot:F('',btn('Finish',{cls:'primary',act:'finish',id:'f-finish',off:dn.copied?'':'Copy the password first.',icon:'check'}))}}
/* ---- D11 update banner/dialog ---- */
function updBanner(){const up=S.update;if(!up||up.state==='hidden')return '';return `<div class="alrow banner"><div class="alert info" role="status">${ic('download')}<span class="grow"><b>Update available</b> ${infoTxt('Never offered while an install is running.','About updates')} ${tag('0.9.3',{v:'info'})}</span></div><button class="btn ghost" data-dlg="update">Details</button><button class="btn ghost" data-act="later">Later</button></div>`}
/* ---- D12 Settings ---- */
function settings(){const pg=S.settings.page,pages=[['General','settings'],['Updates','download'],['Data','database'],['Logs','file-text'],['Privacy','lock']];
 const it=(l,c,tp)=>`<div class="setitem"><span class="lab">${tp?lab(l,tp):l}</span>${c}</div>`;
 const segm=(k,o,v)=>`<div class="seg" role="group">${o.map(x=>`<button aria-pressed="${v===x}" data-set-${k}="${x}">${x}</button>`).join('')}</div>`;
 const P={General:it('Theme',segm('theme',['System','Light','Dark'],(S.themeMode==='System'?'System':S.theme==='dark'?'Dark':'Light')))+it('Text size',segm('size',['Default','Large','Largest'],S.textSize||'Default'),'Scales the whole window.'),
  Updates:it('Channel',segm('chan',['Stable','Beta'],'Stable'))+it('Auto-update',selectH('set-auto',{opts:()=>[['Ask','Ask'],['Download','Download'],['Off','Off']],get:()=>S.opt.auto,set:v=>{S.opt.auto=v;render()},label:'Auto-update',w:'10rem'}),'Never installs while a conversion is running.')+it('Compatibility list',`${d('v2026.10.3')}<button class="btn sm" data-act="refreshlist">${ic('refresh-cw')}Refresh</button>`),
  Data:it('Offline bundle',`${selectH('set-arch',{opts:()=>[['x86_64','x86_64'],['arm64','arm64']],get:()=>S.opt.arch,set:v=>{S.opt.arch=v;render()},label:'Offline bundle',w:'10rem'})}${S.bundle?tag('Ready',{v:'ok',i:'circle-check'}):tag('None',{})}<button class="btn sm" data-act="dlbundle">${ic('download')}Download</button>`),
  Logs:it('Log folder',`${mono('~/.local/share/synofurb/logs')}<button class="btn sm" data-act="openfolder">${ic('folder-open')}Open</button>`),
  Privacy:it('Send results',`<label class="sw"><input type="checkbox" aria-label="Send results"><span></span></label>`,'Asks before every report; serial numbers are left out.')}[pg];
 return {html:`<div class="page"><div class="ph"><h1 class="t-title">Settings</h1></div><div class="set"><nav class="nav" aria-label="Settings">${pages.map(([n,i])=>`<button aria-current="${pg===n}" data-set-page="${n}">${ic(i)}${n}</button>`).join('')}</nav><div class="card"><div class="card-b" style="padding:.25rem 1.25rem">${P}</div></div></div></div>`,foot:F('',btn('Close',{go:S.back||'find'}))}}
