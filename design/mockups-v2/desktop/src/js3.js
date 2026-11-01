/* ===== shell pieces + screens D1..D12 ===== */
const STEPS=['Find','Connect','Inspect','Goal','Plan','System','Confirm','Install','Done'];
const STEP_OF={find:0,connect:1,inspect:2,goal:3,boot:3,plan:4,system:5,confirm:6,install:7,failed:7,done:8};
const SCREEN_OF=['find','connect','inspect','goal','plan','system','confirm','install','done'];
const NOSTEPPER=['recovery','manage','settings','about'];
const instN=()=>S.inst.step>=4?S.inst.step-3:0;
function risk(){const s=S.screen;
  if(s==='failed')return S.fail.kind==='pre'?{t:'Failed safely',v:'warn',i:'shield-check'}:{t:'Disks changed',v:'warn',i:'triangle-alert'};
  if(s==='install'){if(S.inst.step>=18)return{t:'Debian running',v:'ok',i:'circle-check'};return instN()>=3?{t:'Disks changed',v:'warn',i:'triangle-alert'}:S.inst.step>=1?{t:'Flash changed',v:'warn',i:'triangle-alert'}:{t:'Read-only',v:'',i:'lock'}}
  if(s==='done'||s==='manage')return{t:'Debian running',v:'ok',i:'circle-check'};
  if(s==='boot'&&S.boot.done)return{t:'Flash changed',v:'warn',i:'triangle-alert'};
  if(['plan','system','confirm'].includes(s))return{t:'Plan only',v:'',i:'square-pen'};
  return{t:'Read-only',v:'',i:'lock'}}
function stepperHTML(){const s=S.screen,cur=STEP_OF[s],failed=s==='failed';let h='<nav class="stepper" aria-label="Progress">';
  STEPS.forEach((n,i)=>{const done=i<cur||(s==='done'&&i<=8)&&i<8||(s==='done'&&i===8&&false);let c=i<cur?'done':i===cur?(failed?'fail':'cur'):'';if(s==='done'&&i===8)c='done';
    const inner=`<span class="n">${c==='done'?ic('check','s'):c==='fail'?ic('x','s'):i+1}</span><span class="lbl">${n}</span>`;
    const click=i<cur&&!['install','failed','done'].includes(s);
    h+=click?`<button class="step ${c}" data-go="${SCREEN_OF[i]}" aria-label="Back to ${n}"${tip('Back to '+n)}>${inner}</button>`:`<span class="step ${c}"${c==='cur'||c==='fail'?' aria-current="step"':''}>${inner}</span>`;
    if(i<8)h+=`<span class="step-line ${i<cur?'done':''}"></span>`});return h+'</nav>'}
function unitHdr(){const u=cur();const t=S.screen;if(!S.unit||['find','settings','about'].includes(t))return '';
  const twin=S.twin&&u.id==='ds918';
  return `<header class="unithdr"><div class="tile">${ic('server','l')}</div><div class="grow"><div class="row gap12"><span class="nm">${d(u.model)}</span><span class="meta-i d mono sub">${esc(u.ip)} \u00b7 SN ${esc(u.sn)}</span>${compatTag(u.compat,{t:u.compat==='untested'?'No one has reported this model; your install is the test.':'One report, from another project.'})}${twin?tag('Twin',{v:'warn',i:'triangle-alert',t:'Another unit of this model is on the network; check serials.'}):''}</div>
  <div class="meta">${mono(u.ip)}${mono('SN '+u.sn)}${d('DSM '+u.dsm)}</div></div><div class="facts">${d(u.disks.length+' disks · '+TiB(u.disks.reduce((a,x)=>a+x[2],0)))}${d(u.cpu)}${d(u.ram)}</div></header>`}
function statusbar(){const r=risk();return `<footer class="statusbar">${tag(r.t,{v:r.v,i:r.i})}${S.offline?tag('Offline',{v:'warn',i:'wifi-off',t:'Compatibility list is 12 days old; LAN scan still works.'}):''}<span class="sp"></span><button class="link" data-pop="wayback" aria-label="Way back">Way back</button></footer>`}
function titlebar(){return `<header class="titlebar"><div class="brand">${LOGO}<span>synofurb</span></div><span class="sp"></span>
 <button class="btn ghost icon" data-help-open="" aria-label="Help"${tip('Help (F1)')}>${ic('circle-question-mark')}</button>
 <button class="btn ghost icon" data-go="settings" aria-label="Settings"${tip('Settings (Ctrl+,)')}>${ic('settings')}${S.update&&S.update.state==='available'?'<span style="position:absolute;margin:-14px 0 0 12px;width:8px;height:8px;border-radius:50%;background:var(--accent)"></span>':''}</button>
 <button class="btn ghost icon" data-menu aria-label="Menu"${tip('Menu')}>${ic('menu')}</button>
 <div class="wc" aria-hidden="true"><i><svg viewBox="0 0 10 10"><path d="M0 5h10"/></svg></i><i><svg viewBox="0 0 10 10"><rect x=".6" y=".6" width="8.8" height="8.8"/></svg></i><i><svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0 0 10"/></svg></i></div></header>`}
const F=(left,right)=>`<footer class="foot">${left||''}<span class="sp"></span>${right||''}</footer>`;
const btn=(label,{cls='',icon='',go='',act='',off='',tipT='',id='',trail=''}={})=>`<button class="btn ${cls}"${id?` id="${id}"`:''}${go?` data-go="${go}"`:''}${act?` data-act="${act}"`:''}${off?` aria-disabled="true"`:''}${(off||tipT)?tip(off||tipT):''}>${icon?ic(icon):''}${label}${trail?ic(trail):''}</button>`;
const back=(go)=>btn('Back',{cls:'ghost',icon:'arrow-left',go});
const cmdBtn=()=>`<button class="btn ghost icon" data-act="cmd" aria-label="Copy command"${tip('Copies the equivalent synofurb command.')}>${ic('terminal')}</button>`;
const infoBtn=(k,l)=>`<button class="ib" data-pop="${k}" aria-label="${l||'Info'}"${tip(l||'More')}>${ic('info','s')}</button>`;
const alertH=(v,i,body,acts='')=>`<div class="alert ${v}" role="${v==='err'||v==='warn'?'alert':'status'}">${ic(i)}<div class="grow">${body}</div>${acts?`<div class="acts">${acts}</div>`:''}</div>`;

/* ---- D1 Find ---- */
function find(){
  const list=(S.twin?[UNITS[0],UNITS[1],TWIN,UNITS[2]]:UNITS).filter(u=>(u.model+u.ip+u.sn).toLowerCase().includes(S.filter.toLowerCase()));
  let body;
  if(S.scan==='loading') body=`<div class="card"><table class="tbl"><tbody>${[1,2,3].map(()=>`<tr><td><div class="skel" style="width:70%"></div></td><td><div class="skel" style="width:60%"></div></td><td><div class="skel" style="width:40%"></div></td><td><div class="skel" style="width:50%"></div></td><td><div class="skel" style="width:60%"></div></td></tr>`).join('')}</tbody></table></div>`;
  else if(!list.length) body=`<div class="card"><div class="empty"><div class="big">${ic('search','xl')}</div><h2>No NAS found</h2><div class="row"><button class="btn" data-act="rescan">${ic('refresh-cw')}Scan again</button><button class="btn" data-dlg="addip">${ic('plus')}Add by IP</button><button class="link" data-help-open="troubleshooting">Help</button></div></div></div>`;
  else body=`<div class="card cardtbl"><table class="tbl"><thead><tr><th>Model</th><th>Address</th><th>DSM</th><th class="hide-md">Found by ${infoBtn('found','Scan details')}</th><th>Status ${infoBtn('status','Status')}</th></tr></thead><tbody>${list.map(u=>`<tr tabindex="0" role="row" data-sel="${u.id}" class="${S.sel===u.id?'sel':''}" aria-selected="${S.sel===u.id}"><td><b>${d(u.model)}</b> ${mono('SN '+u.sn)}${S.twin&&u.model==='DS918+'?' '+tag('Twin',{v:'warn',i:'triangle-alert',t:'Another unit of this model is on the network; check serials.'}):''}</td><td>${mono(u.ip)}</td><td>${d(u.dsm)}</td><td class="hide-md">${d(u.found.join(' · '))}</td><td>${compatTag(u.compat,{t:{untested:'No reports yet.',community:'One report, from another project.'}[u.compat]})}</td></tr>`).join('')}</tbody></table></div>`;
  const deb=`<div class="col"><h2>Running Debian</h2><div class="card cardtbl"><table class="tbl sm"><tbody>${DEBIAN_UNITS.map(u=>`<tr><td><b>${d(u.model)}</b> ${d('attic')}</td><td>${mono(u.ip)}</td><td>${d(u.os+' · '+u.kernel)}</td><td class="r"><button class="btn ghost sm" data-act="manage">Manage</button></td></tr>`).join('')}</tbody></table></div></div>`;
  const upd=S.update&&S.update.state==='available';
  return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Find NAS</h1><div class="sub t-sub">Select the unit to work on.</div></div><span class="sp"></span><div class="inp" style="width:200px">${ic('search','s')}<input id="flt" placeholder="Filter units" value="${esc(S.filter)}" aria-label="Filter units"></div>
   <button class="btn ghost" data-act="rescan"${tip('Five discovery methods run at once.')}>${S.scan==='loading'?ic('loader-circle','spin'):ic('refresh-cw')}Rescan</button><button class="btn ghost" data-dlg="addip">${ic('plus')}Add by IP</button></div>${body}${S.scan==='loading'||!list.length?'':deb}</div>`,
   foot:F('',btn('Connect',{cls:'primary',id:'f-connect',off:S.sel?'':'Select a unit first.',trail:'arrow-right',act:'connect'})), banner:upd};
}
/* ---- D2 Connect ---- */
function connect(){const c=S.conn,u=cur();
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Connect ${infoBtn('whatread','What we read')}</h1><div class="sub t-sub"><span${tip('This step only reads.')}>Sign in over SSH. Nothing changes.</span></div></div></div>
 <div class="splash"><div class="card"><div class="card-b col gap16">${c.err==='off'?alertH('err','octagon-x','<b>SSH is off</b>',`<button class="link" data-help-open="troubleshooting">How to enable</button>`):c.err==='login'?alertH('err','octagon-x','<b>Login failed</b>',''):''}
  <div class="row gap12"><div class="fld grow"><span class="lab">User</span><input class="inp" id="c-user" value="${esc(c.user)}" aria-label="User"></div><div class="fld" style="width:96px"><span class="lab">Port</span><input class="inp mono" id="c-port" value="${esc(c.port)}" aria-label="Port"></div></div>
  <div class="fld"><span class="lab">Sign in with</span><div class="seg" role="group" style="align-self:flex-start">${['Password','Key'].map(k=>`<button aria-pressed="${c.auth===k}" data-conn-auth="${k}">${k}</button>`).join('')}</div></div>
  <div class="fld"><span class="lab">${c.auth==='Key'?'Key':'Password'}</span><div class="inp ${c.err==='login'?'err':''}"><input id="c-pw" type="password" value="${esc(c.pw)}" aria-label="Password"><button class="btn ghost icon sm" data-act="peek" aria-label="Show password"${tip('Show or hide')}>${ic('eye','s')}</button></div></div>
  <label class="chk"><input type="checkbox" data-conn="sudo" ${c.sudo?'checked':''}><span class="bx">${ic('check')}</span><span${tip('DSM asks for the password to read disk details.')}>Same for sudo</span></label>
  <label class="chk"><input type="checkbox" data-conn="keep" ${c.keep?'checked':''}><span class="bx">${ic('check')}</span><span${tip('Otherwise the password is forgotten on exit.')}>Keep in keychain</span></label></div></div>
  <div class="path" aria-hidden="true"><div class="node">${ic('laptop','xl')}${d('this computer')}</div><div class="line"></div><div class="node">${ic('server','xl')}${mono(u.ip+':'+c.port)}</div></div></div></div>`,
  foot:F(back('find'),btn(c.state==='busy'?'Connecting':'Connect',{cls:'primary '+(c.state==='busy'?'busy':''),act:'dosignin',icon:c.state==='busy'?'loader-circle':'',trail:c.state==='busy'?'':'arrow-right'}))}}
/* ---- D3 Inspect ---- */
function inspect(){const u=cur(),bad=u.disks.find(x=>x[3]);
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Inspection</h1><div class="sub t-sub">Read-only results.</div></div></div>
 ${bad?alertH('warn','triangle-alert',`<b>Disk warning</b> <span class="d">${bad[0]} · ${bad[3]}</span>`):''}
 <div class="grid2" style="display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1.3fr);gap:20px;align-items:start"><div class="card"><div class="card-b" style="padding:8px 16px"><dl class="dl"><dt>Model</dt><dd>${d(u.model)} ${compatTag(u.compat)} ${infoBtn('compat','Status')}</dd><dt>CPU</dt><dd>${d(u.cpu)}</dd><dt>RAM</dt><dd>${d(u.ram)}</dd><dt>Firmware</dt><dd>${d(u.fw)}</dd><dt>Boot flash ${infoBtn('flash','Boot flash')}</dt><dd>${d(u.flash)}</dd><dt>Console</dt><dd>${d(u.con)}</dd><dt>Backups</dt><dd><span${tip('No Hyper Backup, USB Copy or rsync tasks on this NAS.')}>${tag(u.bk,{v:'warn',i:'triangle-alert'})}</span></dd></dl></div></div>
 <div class="card cardtbl"><table class="tbl sm"><thead><tr><th>Disk</th><th>Model</th><th class="r">Size</th><th>SMART</th></tr></thead><tbody>${u.disks.map(x=>`<tr><td>${mono(x[0])}</td><td>${d(x[1])}</td><td class="r">${mono(TiB(x[2]))}</td><td>${x[3]?tag(x[3].replace(' sectors',''),{v:'warn',i:'triangle-alert',t:'It is in the data RAID; consider replacing it first.'}):tag('OK',{v:'ok',i:'circle-check'})}</td></tr>`).join('')}</tbody></table></div></div>
 <div class="col">${acc('dsm','DSM volumes',`<dl class="dl"><dt>${d('volume1')}</dt><dd>${d(u.vol+' · '+u.used+' used')}</dd></dl>`)}${acc('cur','Current layout',`<div id="dg-now"></div>`)}</div></div>`,
 foot:F(back('connect'),btn('Choose goal',{cls:'primary',go:'goal',trail:'arrow-right'})),after(){const el=$('#dg-now');if(el)el.innerHTML=diagram('now')}}}
function acc(id,title,body,open=false,extra=''){const o=S.acc&&S.acc[id]!==undefined?S.acc[id]:open;return `<div class="acc"><button class="acc-h" aria-expanded="${o}" data-acc="${id}">${title}${extra}${ic('chevron-down','ch')}</button>${o?`<div class="acc-b">${body}</div>`:''}</div>`}
S.acc={};
/* ---- D4 Goal ---- */
function goal(){const u=cur(),unsupported=u.compat==='unsupported',rec=u.compat==='untested';
 const card=(k,icon,title,line,tg,btnH,recd)=>`<section class="card goal ${recd?'rec':''}">${recd?`<span class="tag fill acc" style="position:absolute;right:16px;top:16px"${tip('No one has reported this model; try the agent first.')}>${ic('star','s')}Recommended</span>`:''}<div class="big">${ic(icon,'xl')}</div><div><h2 class="row">${title} ${infoBtn(k,title)}</h2><div class="sub">${line}</div></div><div>${tg}</div>${btnH}</section>`;
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Choose goal</h1><div class="sub t-sub">What should happen to this NAS?</div></div></div><div class="cards3">
 ${card('bootmgr','terminal','Boot manager','Try Linux. Disks untouched.',tag('Disks untouched',{v:'ok',i:'shield-check'}),btn('Set up',{cls:rec?'primary':'',go:'boot'}),rec)}
 ${card('install','hard-drive','Install Debian','Replace DSM with Debian.',tag('Erases disks',{v:'err',i:'trash'}),btn('Plan install',{cls:rec?'':'primary',go:'plan',off:unsupported?'Debian 13 cannot run on this hardware':''}),false)}
 ${card('restore','life-buoy','Restore / Recover','Undo or repair.',tag('Reads state first',{i:'search'}),btn('Open recovery',{go:'recovery'}),false)}</div></div>`,
 foot:F(back('inspect'),'')}}
/* ---- diagram ---- */
function diagram(kind){const u=cur(),p=S.plan,cap=capacity(p.raid,u.disks),mx=Math.max(...u.disks.map(x=>x[2]));
 const keep=p.mode==='keep',rows=u.disks.map((x,i)=>{const w=(x[2]/mx*100).toFixed(1);let segs='';
  if(kind==='now'){segs=`<i class="dsm" style="flex:0 0 14px" title="p1"></i><i class="swap" style="flex:0 0 8px"></i><i class="dsm" style="flex:1"></i>`}
  else{ segs=`<i class="bios" style="flex:0 0 6px"></i><i class="${i<p.copies?'sys':'unused'}" style="flex:0 0 ${keep?Math.max(12,Math.min(p.sys,64)/64*80):Math.max(14,p.sys/64*90)}px"></i>`;
   if(keep) segs+=`<i class="kept" style="flex:1"></i>`;
   else if(cap.slices.length){segs+=cap.slices.map(s=>`<i class="s${s.idx%4} ${s.L==='unused'||!s.mem.includes(i)?'':''} ${s.weak&&s.mem.includes(i)?'warnseg':''}" style="flex:${s.th};${s.mem.includes(i)?'':'visibility:hidden'}${s.L==='unused'?';background:transparent;border-style:dashed':''}"></i>`).join('')}
   else segs+=`<i class="data" style="flex:1"></i>`}
  return `<div class="dg-row"><span>${x[0]}</span><div style="width:${w}%"><div class="dg-bar">${segs}</div></div><span style="text-align:right">${x[2].toFixed(2)}</span></div>`}).join('');
 let chips='';
 if(kind==='now')chips=`<span class="chip">md0 RAID 1 · DSM</span><span class="chip">md1 swap</span><span class="chip">md2 RAID 5 · SHR</span><span class="chip">${esc(u.vol)}</span>`;
 else{const dv=+(cap.usable*(1-p.free/100)).toFixed(2);chips=`<span class="chip">md0 RAID 1 ×${p.copies} · ${p.sys} GiB</span><span class="chip">/ ${Math.max(p.sys-4,4)} GiB · swap 4</span>`+(keep?`<span class="chip">DSM data kept</span>`:cap.error?'':cap.slices.length?cap.slices.map(s=>`<span class="chip">${s.id} ${s.L} · ${TiB(s.u)}</span>`).join('')+`<span class="chip">/srv/data ${TiB(dv)}</span>`:`<span class="chip">md1 ${LEVEL_LABEL[cap.level]} · ${TiB(cap.usable)}</span><span class="chip">/srv/data ${TiB(dv)}</span><span class="chip">free ${p.free}%</span>`)}
 return `<div class="dg">${rows}<div class="chips">${chips}</div></div>`}
