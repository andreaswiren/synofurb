/* ===== shell pieces + screens: Find, Connect, Inspect, Goal ===== */
const STEPS=['Find','Connect','Goal','Setup','Confirm','Install'];
const STEP_OF={find:0,connect:1,inspect:1,goal:2,boot:2,plan:3,system:3,setup:3,confirm:4,install:5,failed:5,done:5};
const SCREEN_OF=['find','inspect','goal','plan','confirm','install'];
const NOSTEPPER=['recovery','manage','settings','about','boot'];
const SCREEN_HELP={find:'compat-status',connect:'troubleshooting',inspect:'backups',goal:'first-install',boot:'boot-manager',plan:'raid-levels',system:'first-install',setup:'raid-levels',confirm:'recovery',install:'first-install',failed:'recovery',done:'first-install',recovery:'recovery',manage:'troubleshooting',settings:null,about:null};
const instN=()=>S.inst.step>=4?Math.min(12,S.inst.step-3):0;
/* Risk Tag = ONE pure function of the unit's facts (S.F), never of the screen name. Settings and About are not about a unit: no Tag. */
function syncFacts(){const F=S.F,s=S.screen;
  if(s==='install'){if(S.inst.step>=1)F.flash=true;if(instN()>=3)F.disks=true;if(S.inst.step>=18){F.debian=true;F.dflt='debian'}}
  if(s==='failed'&&S.fail){F.flash=true;if(S.fail.kind!=='stopped')F.fail=true;if(S.fail.kind==='post')F.disks=true}
  if(s==='done'){F.flash=true;F.disks=true;F.debian=true;F.dflt='debian'}
  if(s==='boot'&&S.boot.done)F.flash=true}
function risk(){const s=S.screen,F=S.F;if(['settings','about'].includes(s))return null;
  if(s==='manage'){const h=S.manage.health;
    if(h==='unreachable')return{t:'Unreachable',v:'',i:'wifi-off'};
    if(h==='faildisk')return{t:'Critical',v:'err',i:'octagon-x'};
    if(h==='rebuild'||h==='svcdown'&&!S.manage.svcFixed)return{t:'Needs attention',v:'warn',i:'triangle-alert'};
    return{t:'Debian running',v:'ok',i:'circle-check'}}
  if(F.debian&&!F.fail)return{t:'Debian running',v:'ok',i:'circle-check'};
  if(F.disks)return{t:'Disks changed',v:'warn',i:'triangle-alert'};
  if(F.fail)return{t:'Failed safely',v:'warn',i:'shield-check'};
  if(F.flash)return{t:'Flash changed',v:'warn',i:'triangle-alert'};
  if(['plan','system','setup','confirm'].includes(s))return{t:'Plan only',v:'',i:'square-pen'};
  return{t:'Read-only',v:'',i:'lock'}}
function stepperHTML(){const s=S.screen,cur=STEP_OF[s],failed=s==='failed';let h='<nav class="stepper" aria-label="Progress">';
  STEPS.forEach((n,i)=>{const c=s==='done'?'done':i<cur?'done':i===cur?(failed?'fail':'cur'):'';
    const inner=`<span class="n">${c==='done'?ic('check','s'):c==='fail'?ic('circle-x','s'):i+1}</span><span class="lbl">${n}</span>`;
    const click=i<cur&&!['install','failed','done'].includes(s);
    h+=`<div class="item">${click?`<button class="step ${c}" tabindex="-1" data-go="${SCREEN_OF[i]}" aria-label="Back to ${n}">${inner}</button>`:`<span class="step ${c}"${c==='cur'||c==='fail'?' aria-current="step"':''}>${inner}</span>`}<span class="step-line${s==='done'||i<cur?' done':''}"></span></div>`});
  return h+'</nav>'}
function unitHdr(){const u=cur(),t=S.screen;if(!S.unit||['find','settings','about'].includes(t))return '';
  const deb=S.F.debian||t==='manage',tw=twinOf(u);
  return `<header class="unithdr"><div class="tile">${ic('server','l')}</div><div class="grow"><div class="row gap12"><span class="nm">${d(u.model)}</span>${deb?tag('Debian 13',{v:'ok',i:'circle-check'}):compatTag(u.compat)}${tw?tag('Twin',{v:'warn',i:'triangle-alert',t:'Another '+u.model+' is on the network ('+tw.ip+', SN '+snTail(tw)+'). Check the serial.'}):''}</div>
  <div class="meta">${mono(u.ip)}${mono('SN '+u.sn)}${d(deb?'Debian 13 · 6.12.57':'DSM '+u.dsm)}</div></div><div class="facts">${d(u.disks.length+' disks · '+TiB(u.disks.reduce((a,x)=>a+x[2],0)))}${d(u.cpu)}${d(u.ram)}</div></header>`}
function statusbar(){const r=risk();return `<footer class="statusbar">${r?tag(r.t,{v:r.v,i:r.i}):''}${S.offline?tag('Offline',{v:'warn',i:'wifi-off',t:'Compatibility list is 12 days old; LAN scan still works.'}):''}<span class="sp"></span>${r?`<button class="link" data-pop="wayback" aria-label="Way back">Way back</button>`:''}</footer>`}
function titlebar(){return `<header class="titlebar"><div class="brand"><span class="mk">${LOGO}</span><span class="wm">syno<b>furb</b></span></div><span class="sp"></span>
 <button class="btn ghost icon" data-help-open="" aria-label="Help"${tip('Help (F1)')}>${ic('circle-question-mark')}</button>
 <button class="btn ghost icon" data-go="settings" aria-label="Settings"${tip('Settings (Ctrl+,)')}>${ic('settings')}${S.update&&S.update.state==='available'?'<span class="udot"></span>':''}</button>
 <button class="btn ghost icon" data-menu aria-label="Menu"${tip('Menu')}>${ic('menu')}</button>
 <div class="wc" aria-hidden="true"><i>${ic('minus','s')}</i><i>${ic('square','s')}</i><i>${ic('x','s')}</i></div></header>`}
const F=(left,right)=>`<footer class="foot">${left||''}<span class="sp"></span>${right||''}</footer>`;
/* A control that is unavailable keeps the widget's disabled look and stays focusable; the reason is a visible info icon and a popover (GPUI tooltips are hover only, a disabled Button cannot be focused). */
const btn=(label,{cls='',icon='',go='',act='',off='',tipT='',id='',trail=''}={})=>{
  const b=`<button class="btn ${cls}"${id?` id="${id}"`:''}${go?` data-go="${go}"`:''}${act?` data-act="${act}"`:''}${off?` aria-disabled="true" tabindex="-1" data-reason="${esc(off)}"`:''}${(!off&&tipT)?tip(tipT):''}>${icon?ic(icon):''}${label}${trail?ic(trail):''}</button>`;
  return off?`<span class="dbw">${b}<button type="button" class="ib" data-reason="${esc(off)}" aria-label="Why unavailable"${tip('Why unavailable')}>${ic('info','s')}</button></span>`:b};
const back=(go)=>btn('Back',{cls:'ghost',icon:'arrow-left',go});
const cmdBtn=()=>`<button class="btn ghost" data-act="cmd"${tip('Shows the equivalent command for the plan.')}>${ic('terminal')}Command</button>`;
const infoTxt=(text,l)=>`<button type="button" class="ib" data-ptext="${esc(text)}" aria-label="${esc(l||'More')}"${tip(l||'More')}>${ic('info','s')}</button>`;
const lab=(l,text)=>`${l} ${infoTxt(text,l)}`;
const infoBtn=(k,l)=>`<button class="ib" data-pop="${k}" aria-label="${l||'Info'}"${tip(l||'More')}>${ic('info','s')}</button>`;
const alertH=(v,i,body,acts='')=>{const a=`<div class="alert ${v}" role="${v==='err'||v==='warn'?'alert':'status'}">${ic(i)}<div class="grow">${body}</div></div>`;return acts?`<div class="alrow">${a}${acts}</div>`:a};

/* ---- D1 Find ---- */
function find(){
  const list=allUnits().filter(u=>(u.model+u.ip+u.sn).toLowerCase().includes(S.filter.toLowerCase()));
  let body;
  if(S.scan==='loading') body=`<div class="card"><table class="tbl"><tbody>${[1,2,3].map(()=>`<tr><td class="rc"></td>${[70,60,40,50,60].map(w=>`<td><div class="skel" style="width:${w}%"></div></td>`).join('')}</tr>`).join('')}</tbody></table></div>`;
  else if(!list.length) body=`<div class="card"><div class="empty"><div class="big">${ic('search','xl')}</div><h2>No NAS found</h2><div class="row"><button class="btn" data-act="rescan">${ic('refresh-cw')}Scan again</button><button class="btn" data-dlg="addip">${ic('plus')}Add by IP</button><button class="link" data-help-open="troubleshooting">Help</button></div></div></div>`;
  else body=`<div class="card cardtbl"><table class="tbl"><thead><tr><th class="rc"></th><th>Model</th><th>Address</th><th>DSM</th><th class="hide-md">Found by ${infoBtn('found','Scan details')}</th><th>Status ${infoBtn('status','Status')}</th></tr></thead><tbody>${list.map(u=>`<tr tabindex="0" role="row" data-sel="${u.id}" class="${S.sel===u.id?'sel':''}" aria-selected="${S.sel===u.id}"><td class="rc"><span class="rad" aria-hidden="true"><i></i></span></td><td><b>${d(u.model)}</b> ${mono('SN '+u.sn)}${twinOf(u)?' '+tag('Twin',{v:'warn',i:'triangle-alert',t:'Another '+u.model+' is on the network; check serials.'}):''}${S.boot.unit===u.id?' '+tag('Boot agent installed',{v:'info',i:'terminal',t:'Our entries are on the boot flash; remove them under Recovery.'}):''}</td><td>${mono(u.ip)}</td><td>${d(u.dsm)}</td><td class="hide-md">${d(u.found.join(' · '))}</td><td>${compatTag(u.compat)}</td></tr>`).join('')}</tbody></table></div>`;
  const deb=`<div class="col"><h2>Running Debian</h2><div class="card cardtbl"><table class="tbl sm"><tbody>${DEBIAN_UNITS.map(u=>`<tr><td><b>${d(u.model)}</b> ${d('attic')}</td><td>${mono(u.ip)}</td><td>${d(u.os+' · '+u.kernel)}</td><td class="r"><button class="btn ghost sm" data-act="manage">Manage</button></td></tr>`).join('')}</tbody></table></div></div>`;
  const upd=S.update&&S.update.state==='available';
  return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Find NAS</h1><div class="sub t-sub">Select the unit to work on.</div></div><span class="sp"></span><div class="inp" style="width:12.5rem">${ic('search','s')}<input id="flt" placeholder="Filter units" value="${esc(S.filter)}" aria-label="Filter units"></div>
   <button class="btn ghost" data-act="rescan"${tip('Five discovery methods run at once.')}>${S.scan==='loading'?ic('loader-circle','spin'):ic('refresh-cw')}Rescan</button><button class="btn ghost" data-dlg="addip">${ic('plus')}Add by IP</button></div>${body}${S.scan==='loading'||!list.length?'':deb}</div>`,
   foot:F('',btn('Connect',{cls:'primary',id:'f-connect',off:S.sel?'':'Select a unit first.',trail:'arrow-right',act:'connect'})), banner:upd};
}
/* ---- D2 Connect (with its failure states) ---- */
function connect(){const c=S.conn,u=cur(),e=c.err;
 const msg={login:['Wrong password',`<button class="ib" data-pop="authblock" aria-label="About auto-block">${ic('info','s')}</button>`,''],
   off:['SSH is off',`<button class="ib" data-pop="sshoff" aria-label="How to turn SSH on">${ic('info','s')}</button>`,'Retry'],
   unreach:['No answer from '+d(u.ip),`<button class="ib" data-pop="unreach" aria-label="What to check">${ic('info','s')}</button>`,'Retry'],
   hostchanged:['Host key changed',`<button class="ib" data-pop="keychg" aria-label="About the changed key">${ic('info','s')}</button>`,'']}[e];
 const al=e?alertH('err','octagon-x',`<b>${msg[0]}</b> ${msg[1]}`,msg[2]?`<button class="btn" data-act="dosignin">${ic('refresh-cw')}${msg[2]}</button>`:''):'';
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Connect ${infoBtn('whatread','What we read')}</h1><div class="sub t-sub">Sign in over SSH. Nothing changes.</div></div></div>
 <div class="splash"><div class="col gap16">${al}<div class="card"><div class="card-b col gap16">
  <div class="row gap12"><div class="fld grow"><span class="lab">User</span><input class="inp" id="c-user" value="${esc(c.user)}" aria-label="User"></div><div class="fld" style="width:6rem"><span class="lab">Port</span><input class="inp mono" id="c-port" value="${esc(c.port)}" aria-label="Port"></div></div>
  <div class="fld"><span class="lab">Sign in with</span><div class="seg" role="group" style="align-self:flex-start">${['Password','Key'].map(k=>`<button aria-pressed="${c.auth===k}" data-conn-auth="${k}">${k}</button>`).join('')}</div></div>
  <div class="fld"><span class="lab">${c.auth==='Key'?'Key':'Password'}</span><div class="inp ${e==='login'||c.field==='pw'?'err':''}"><input id="c-pw" type="password" value="${esc(c.pw)}" aria-label="${c.auth==='Key'?'Key passphrase':'Password'}"${c.field==='pw'?' aria-invalid="true" aria-describedby="c-pw-e"':''}><button class="btn ghost icon sm" data-act="peek" aria-label="Show password"${tip('Show or hide')}>${ic('eye','s')}</button></div>${c.field==='pw'?`<span class="ferr" id="c-pw-e">${ic('circle-x','s')}Enter a password</span>`:''}</div>
  <label class="chk"><input type="checkbox" data-conn="sudo" ${c.sudo?'checked':''}><span class="bx">${ic('check')}</span><span>Same for sudo</span>${infoTxt('DSM asks for the password to read disk details.','About sudo')}</label>
  <label class="chk"><input type="checkbox" data-conn="keep" ${c.keep?'checked':''}><span class="bx">${ic('check')}</span><span>Keep in keychain</span>${infoTxt('Otherwise the password is forgotten when the app closes.','About the keychain')}</label></div></div></div>
  <div class="path" aria-hidden="true"><div class="node">${ic('laptop','xl')}${d('this computer')}</div><div class="line"></div><div class="node">${ic('server','xl')}${mono(u.ip+':'+c.port)}</div></div></div></div>`,
  foot:F(back('find'),btn(c.state==='busy'?'Connecting':'Connect',{cls:'primary '+(c.state==='busy'?'busy':''),act:'dosignin',icon:c.state==='busy'?'loader-circle':'',trail:c.state==='busy'?'':'arrow-right'}))}}
/* ---- D3 Inspect ---- */
function inspect(){const u=cur(),bad=u.disks.find(x=>x[3]),nobk=u.bk==='none found';
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Inspection</h1><div class="sub t-sub">Read-only results.</div></div></div>
 ${bad?alertH('warn','triangle-alert',`<b>Disk warning</b> <span class="d">${bad[0]} · ${bad[3]}</span>`):''}
 <div class="grid2"><div class="card"><div class="card-b" style="padding:.5rem 1rem"><dl class="dl"><dt>CPU</dt><dd>${d(u.cpu)}</dd><dt>RAM</dt><dd>${d(u.ram)}</dd><dt>Firmware</dt><dd>${d(u.fw)}</dd><dt>Boot flash ${infoBtn('flash','Boot flash')}</dt><dd>${d(u.flash)}</dd><dt>Console</dt><dd>${d(u.con)}</dd><dt>Backups</dt><dd>${tag(u.bk,{v:nobk?'warn':'ok',i:nobk?'triangle-alert':'circle-check',t:nobk?'No Hyper Backup, USB Copy or rsync tasks on this NAS.':'A backup task exists; test a restore.'})}</dd></dl></div></div>
 <div class="card cardtbl"><table class="tbl sm"><thead><tr><th>Disk</th><th>Model</th><th class="r">Size</th><th>SMART</th></tr></thead><tbody>${u.disks.map((x,i)=>`<tr><td>${mono(x[0])}</td><td>${d(x[1]+' · '+dsn(u,i))}</td><td class="r">${mono(TiB(x[2]))}</td><td>${x[3]?tag(x[3].replace(' sectors',''),{v:'warn',i:'triangle-alert',t:'It is in the data RAID; consider replacing it first.'}):tag('OK',{v:'ok',i:'circle-check'})}</td></tr>`).join('')}</tbody></table></div></div>
 <div class="col">${acc('dsm','DSM volumes',`<dl class="dl"><dt>${d('volume1')}</dt><dd>${d(u.vol+' · '+u.used+' used')}</dd></dl>`)}${acc('cur','Current layout',`<div id="dg-now"></div>`)}</div></div>`,
 foot:F(back('connect'),btn('Choose goal',{cls:'primary',go:'goal',trail:'arrow-right'})),after(){const el=$('#dg-now');if(el)el.innerHTML=diagram('now')}}}
function acc(id,title,body,open=false,extra=''){const o=S.acc&&S.acc[id]!==undefined?S.acc[id]:open;return `<div class="acc"><button class="acc-h" aria-expanded="${o}" data-acc="${id}">${title}${extra}${ic('chevron-down','ch')}</button>${o?`<div class="acc-b">${body}</div>`:''}</div>`}
S.acc={};
/* ---- D4 Goal (with Quick setup) ---- */
function goal(){const u=cur(),unsupported=u.compat==='unsupported',rec=u.compat==='untested';
 const card=(k,icon,title,line,tg,btnH,recd)=>`<section class="card goal ${recd?'rec':''}">${recd?tag('Recommended',{v:'acc',fill:true,i:'star',t:'No one has reported this model; try the agent first.',attrs:' style="position:absolute;right:1rem;top:1rem"'}):''}<div class="big">${ic(icon,'xl')}</div><div><h2 class="row">${title} ${infoBtn(k,title)}</h2><div class="sub">${line}</div></div><div>${tg}</div><div class="row wrap gbtn">${btnH}</div></section>`;
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Choose goal</h1><div class="sub t-sub">What should happen to this NAS?</div></div></div><div class="cards3">
 ${card('bootmgr','terminal','Boot manager','Try Linux. Disks untouched.',tag('Disks untouched',{v:'ok',i:'shield-check'}),btn('Set up',{cls:rec?'primary':'',go:'boot'}),rec)}
 ${card('install','hard-drive','Install Debian','Replace DSM with Debian.',tag('Erases disks',{v:'err',i:'trash'}),btn('Quick setup',{cls:rec?'':'primary',act:'quick',off:unsupported?'Debian 13 cannot run on this hardware.':''})+btn('Customize',{go:'plan',off:unsupported?'Debian 13 cannot run on this hardware.':''}),false)}
 ${card('restore','life-buoy','Restore / Recover','Undo or repair.',tag('Reads state first',{i:'search'}),btn('Open recovery',{go:'recovery'}),false)}</div></div>`,
 foot:F(back('inspect'),'')}}
/* ---- diagram (custom: no widget exists) ---- */
function diagram(kind){const u=cur(),p=S.plan,cap=capacity(p.raid,u.disks),mx=Math.max(...u.disks.map(x=>x[2]));
 const keep=p.mode==='keep',R=v=>(v/16).toFixed(3)+'rem',rows=u.disks.map((x,i)=>{const w=(x[2]/mx*100).toFixed(1);let segs='';
  if(kind==='now'){segs=`<i class="dsm" style="flex:0 0 .875rem"></i><i class="swap" style="flex:0 0 .5rem"></i><i class="dsm" style="flex:1"></i>`}
  else{ segs=`<i class="bios" style="flex:0 0 .375rem"></i><i class="${i<p.copies?'sys':'unused'}" style="flex:0 0 ${R(keep?Math.max(12,Math.min(p.sys,64)/64*80):Math.max(14,p.sys/64*90))}"></i>`;
   if(keep) segs+=`<i class="kept" style="flex:1"></i>`;
   else if(cap.slices.length){segs+=cap.slices.map(s=>`<i class="s${s.idx%4} ${s.weak&&s.mem.includes(i)?'warnseg':''}" style="flex:${s.th};${s.mem.includes(i)?'':'visibility:hidden'}${s.L==='unused'?';background:transparent;border-style:dashed':''}"></i>`).join('')}
   else segs+=`<i class="data" style="flex:1"></i>`}
  return `<div class="dg-row"><span>${x[0]}</span><div style="width:${w}%"><div class="dg-bar">${segs}</div></div><span style="text-align:right">${x[2].toFixed(2)}</span></div>`}).join('');
 let chips='';
 if(kind==='now')chips=`<span class="chip">md0 RAID 1 · DSM</span><span class="chip">md1 swap</span><span class="chip">md2 RAID 5 · SHR</span><span class="chip">${esc(u.vol)}</span>`;
 else{const dv=+(cap.usable*(1-p.free/100)).toFixed(2);chips=`<span class="chip"><i class="sq sys"></i>md0 RAID 1 ×${p.copies} · ${p.sys} GiB</span><span class="chip">/ ${Math.max(p.sys-4,4)} GiB · swap 4</span>`+(keep?`<span class="chip"><i class="sq kept"></i>volume1 kept</span>`:cap.error?'':cap.slices.length?cap.slices.map(s=>`<span class="chip"><i class="sq s${s.idx%4}"></i>${s.id} ${s.L} · ${TiB(s.u)}</span>`).join('')+`<span class="chip">/srv/data ${TiB(dv)}</span>`:`<span class="chip"><i class="sq data"></i>md1 ${LEVEL_LABEL[cap.level]} · ${TiB(cap.usable)}</span><span class="chip">/srv/data ${TiB(dv)}</span><span class="chip">free ${p.free}%</span>`)}
 return `<div class="dg d">${rows}<div class="chips">${chips}</div></div>`}
