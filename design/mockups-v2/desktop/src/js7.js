/* ===== hidden "Components" page: the widgets as they look in each state (review aid, not a product screen) ===== */
NOSTEPPER.push('specimens');SCREEN_HELP.specimens=null;
function specimens(){
 const row=(l,h)=>`<div class="spec-row"><span class="sub" style="min-width:8rem">${l}</span>${h}</div>`;
 const b=(label,cls,st='')=>`<button class="btn ${cls} ${st}"${st==='dis'?' aria-disabled="true"':''}>${label}</button>`;
 const dis=(label,cls)=>`<button class="btn ${cls}" aria-disabled="true">${label}</button>`;
 const mini=(n,f)=>{let h='<nav class="stepper" style="position:static;padding:0;border:0;height:auto;grid-row:auto;background:transparent">';['Find','Connect','Goal','Setup','Confirm','Install'].forEach((l,i)=>{const c=i<n?'done':i===n?(f?'fail':'cur'):'';h+=`<div class="item"><span class="step ${c}"><span class="n">${c==='done'?ic('check','s'):c==='fail'?ic('circle-x','s'):i+1}</span><span class="lbl">${l}</span></span><span class="step-line"></span></div>`});return h+'</nav>'};
 const sbox=(cls)=>`<div class="sbbox ${cls}"><i></i></div>`;
 return {html:`<div class="page"><div class="ph"><div><h1 class="t-title">Components</h1><div class="sub t-sub">Widget states in this theme.</div></div></div><div class="specs">
 <section class="card"><div class="card-h">Stepper</div><div class="card-b">${row('Step 3 current',mini(2))}${row('Install failed',mini(5,true))}</div></section>
 <section class="card"><div class="card-h">Buttons</div><div class="card-b">
  ${row('Default',b('Cancel','')+b('Cancel','','hv')+b('Cancel','','pr')+dis('Cancel',''))}
  ${row('Primary',b('Connect','primary')+b('Connect','primary','hv')+b('Connect','primary','pr')+dis('Connect','primary'))}
  ${row('Danger',b('Erase','danger')+b('Erase','danger','hv')+b('Erase','danger','pr')+dis('Erase','danger'))}
  ${row('Ghost',b('Back','ghost')+b('Back','ghost','hv')+b('Back','ghost','pr')+dis('Back','ghost'))}</div></section>
 <section class="card"><div class="card-h">Tags</div><div class="card-b">${row('Compatibility',['verified','community','untested','issues','unsupported'].map(k=>compatTag(k)).join(''))}${row('Status',tag('Healthy',{v:'ok',i:'circle-check'})+tag('Needs attention',{v:'warn',i:'triangle-alert'})+tag('Critical',{v:'err',i:'octagon-x',fill:true})+tag('Updating',{v:'info',i:'loader-circle'})+tag('Unreachable',{i:'wifi-off'}))}</div></section>
 <section class="card"><div class="card-h">Alerts</div><div class="card-b">${alertH('info','info','<b>Information.</b> One sentence.')}${alertH('warn','triangle-alert','<b>Warning.</b> One sentence.',`<button class="btn">Action</button>`)}${alertH('err','octagon-x','<b>Error.</b> One sentence.')}</div></section>
 <section class="card"><div class="card-h">Select and Table</div><div class="card-b"><div style="width:14rem">${selectH('sp-sel',{opts:()=>RAID_OPTS,get:()=>'raid5',set:()=>{},label:'Example'})}</div>
  <div class="lb" style="position:static;width:14rem">${RAID_OPTS.slice(0,5).map(([k,l],i)=>`<button type="button" class="opt ${i===3?'hl':''}" role="option" aria-selected="${k==='raid5'}">${l}${ic('check','s')}</button>`).join('')}</div>
  <div class="card cardtbl"><table class="tbl sm"><tbody><tr><td class="rc"><span class="rad"><i></i></span></td><td>${d('DS923+')}</td><td>${d('7.2.2')}</td></tr><tr class="sel"><td class="rc"><span class="rad"><i></i></span></td><td>${d('DS918+')}</td><td>${d('7.2.2')}</td></tr></tbody></table></div></div></section>
 <section class="card"><div class="card-h">Scrollbar</div><div class="card-b"><div class="sbdemo">${sbox('')}${sbox('h')}</div><span class="sub">Idle (6 px) and hover or drag (8 px). Pill thumb, no arrows.</span></div></section>
 <section class="card"><div class="card-h">Disabled reason</div><div class="card-b">${row('Visible info icon',btn('Erase and install',{cls:'danger',off:'Missing: the phrase.'}))}<span class="sub">The icon is focusable and opens a popover; the button is not the trigger.</span></div></section>
 </div></div>`,foot:F(back('find'),'')}}
SCREENS.specimens=specimens;
init();
