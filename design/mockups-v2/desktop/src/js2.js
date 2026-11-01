/* ===== widgets: tooltip, popover, dialog, sheet, toast ===== */
const L=()=>$('#layer'), WIN=()=>$('.win');
function place(el,anchor,pref='below'){ // position inside .win
  const w=WIN().getBoundingClientRect(), a=anchor.getBoundingClientRect(), e=el.getBoundingClientRect();
  let x=a.left-w.left+a.width/2-e.width/2, y=pref==='above'?a.top-w.top-e.height-6:a.bottom-w.top+6;
  if(pref==='below'&&y+e.height>w.height-8) y=a.top-w.top-e.height-6;
  if(pref==='above'&&y<4) y=a.bottom-w.top+6;
  x=Math.max(8,Math.min(x,w.width-e.width-8)); el.style.left=x+'px'; el.style.top=y+'px';
}
/* tooltip: HOVER ONLY, like gpui-component's Tooltip (no focus path). It may only sit on controls that are already focusable and only carry non-essential extras; every sentence a keyboard user needs lives on a focusable info button (infoTxt) or is visible text. check_tips.js enforces it. */
let tipEl=null,tipT=null;
function showTip(t){hideTip();const txt=t.getAttribute('data-tip');if(!txt)return;tipEl=document.createElement('div');tipEl.className='tip';tipEl.setAttribute('role','tooltip');tipEl.id='tooltip';tipEl.textContent=txt;L().appendChild(tipEl);place(tipEl,t,'below');t.setAttribute('aria-describedby','tooltip')}
function hideTip(){clearTimeout(tipT);if(tipEl){tipEl.remove();tipEl=null}$$('[aria-describedby=tooltip]').forEach(e=>e.removeAttribute('aria-describedby'))}
document.addEventListener('mouseover',e=>{const t=e.target.closest('[data-tip]');if(!t)return;clearTimeout(tipT);tipT=setTimeout(()=>showTip(t),window.__fast?0:400)});
document.addEventListener('mouseout',e=>{if(e.target.closest('[data-tip]'))hideTip()});
document.addEventListener('focusout',hideTip);document.addEventListener('keydown',()=>hideTip(),true);
/* popover (and Select popup): GPUI tooltips are hover-only, so anything needed to act or stay safe is a focusable trigger that opens a popover */
let popEl=null,popAnchor=null;
function closePop(ret=true){if(popEl){popEl.remove();popEl=null;if(popAnchor&&popAnchor.getAttribute&&popAnchor.hasAttribute('aria-expanded'))popAnchor.setAttribute('aria-expanded','false');if(ret&&popAnchor&&document.contains(popAnchor))popAnchor.focus({preventScroll:true})}popAnchor=null}
function showPop(anchor,P){
  if(popEl&&popAnchor===anchor){closePop();return}
  closePop(false);hideTip();
  popEl=document.createElement('div');popEl.className='pop';popEl.setAttribute('role','dialog');popEl.setAttribute('aria-label',P.title);popAnchor=anchor;
  popEl.innerHTML=`<h3>${P.title}</h3><div>${P.body}</div>${P.help?`<div class="row"><button class="link" data-help-open="${P.help}">Open Help</button></div>`:''}`;
  L().appendChild(popEl);place(popEl,anchor,'below');
  const f=$('button,[tabindex]',popEl);if(f)f.focus();
}
function openPop(anchor,key){const P=POP[key]?POP[key]():null;if(P)showPop(anchor,P)}
function openText(anchor,title,text,help){showPop(anchor,{title,body:`<p>${esc(text)}</p>`,help})}
document.addEventListener('click',e=>{if(popEl&&!e.target.closest('.pop,.lb')&&!e.target.closest('[data-pop],[data-select],[data-ptext],[data-reason]'))closePop(false)},true);
/* Select: closed = input-like button, open = popup list with check on the selected item; Up/Down/Enter/Esc */
const SEL={};
function selectH(id,{opts,get,set,label,disabled,w}){SEL[id]={opts,get,set};const o=opts(),c=o.find(x=>x[0]===get())||o[0];
  return `<button type="button" class="inp sel" id="${id}" data-select="${id}" role="combobox" aria-haspopup="listbox" aria-expanded="false" aria-label="${label}"${disabled?' aria-disabled="true" data-reason="Not available in Keep mode."':''}${w?` style="width:${w}"`:''}><span class="grow">${c[1]}</span>${ic('chevron-down')}</button>`}
function openSelect(btn){
  if(popEl&&popAnchor===btn){closePop();return}
  closePop(false);hideTip();const C=SEL[btn.dataset.select],o=C.opts(),v=C.get();
  const m=document.createElement('div');m.className='lb';m.setAttribute('role','listbox');m.setAttribute('aria-label',btn.getAttribute('aria-label'));
  m.innerHTML=o.map(([k,l])=>`<button type="button" class="opt" role="option" aria-selected="${k===v}" data-v="${k}">${l}${ic('check','s')}</button>`).join('');
  const r=btn.getBoundingClientRect();m.style.minWidth=r.width+'px';L().appendChild(m);place(m,btn,'below');
  m.style.left=Math.max(8,r.left-WIN().getBoundingClientRect().left)+'px';popEl=m;popAnchor=btn;btn.setAttribute('aria-expanded','true');
  const ops=$$('.opt',m);(ops.find(b=>b.getAttribute('aria-selected')==='true')||ops[0]).focus();
  m.addEventListener('click',e=>{const b=e.target.closest('.opt');if(!b)return;const val=b.dataset.v;closePop();C.set(val)});
  m.addEventListener('keydown',e=>{const i=ops.indexOf(document.activeElement);
    if(e.key==='ArrowDown'||e.key==='ArrowUp'){e.preventDefault();ops[(i+(e.key==='ArrowDown'?1:-1)+ops.length)%ops.length].focus()}
    else if(e.key==='Home'){e.preventDefault();ops[0].focus()}else if(e.key==='End'){e.preventDefault();ops[ops.length-1].focus()}
    else if(e.key==='Tab'){e.preventDefault();closePop()}})}
/* dialog and sheet with focus trap */
const dlgStack=[];
function trap(root,ev){if(ev.key!=='Tab')return;const f=$$('button:not([aria-disabled=true]),a[href],input:not([type=hidden]),select,[tabindex]:not([tabindex="-1"])',root).filter(x=>x.offsetParent!==null);if(!f.length)return;const a=f[0],z=f[f.length-1];
  if(ev.shiftKey&&document.activeElement===a){ev.preventDefault();z.focus()}else if(!ev.shiftKey&&document.activeElement===z){ev.preventDefault();a.focus()}else if(!root.contains(document.activeElement)){ev.preventDefault();a.focus()}}
function openDialog({id,title,icon,body,buttons=[],wide=false,danger=false,onopen,onclose,initial}){
  hideTip();closePop(false);const prev=document.activeElement;
  const sc=document.createElement('div');sc.className='scrim';sc.dataset.dname=id;
  sc.innerHTML=`<div class="dlg ${wide?'wide':''} ${danger?'dng':''}" role="dialog" aria-modal="true" aria-labelledby="dt-${id}"><div class="dlg-h">${icon?ic(icon,'l'):''}<span id="dt-${id}">${title}</span></div><div class="dlg-b">${body}</div><div class="dlg-f">${buttons.map((b,i)=>`<button class="btn ${b.cls||''}" data-dbtn="${i}"${b.off?` aria-disabled="true" tabindex="-1" data-reason="${esc(typeof b.off==='string'?b.off:'Type the phrase first.')}"`:''}>${b.icon?ic(b.icon):''}${b.label}</button>`).join('')}</div></div>`;
  L().appendChild(sc);const dlg=$('.dlg',sc);
  const close=()=>{sc.remove();dlgStack.pop();onclose&&onclose();if(prev&&document.contains(prev))prev.focus({preventScroll:true})};
  const api={el:sc,close,btn:i=>$(`[data-dbtn="${i}"]`,sc)};
  sc.addEventListener('keydown',ev=>{if(ev.key==='Escape'){ev.stopPropagation();if(!buttons.some(b=>b.noesc))close()}trap(dlg,ev)});
  sc.addEventListener('click',ev=>{const b=ev.target.closest('[data-dbtn]');if(b&&b.getAttribute('aria-disabled')!=='true'){const bt=buttons[+b.dataset.dbtn];if(bt.fn){if(bt.fn(api)===false)return}if(!bt.keep)close()}});
  dlgStack.push(api);onopen&&onopen(api);
  (initial?$(initial,sc):$('.btn.primary,.btn.danger,input,.btn',dlg)).focus();return api}
function openSheet({title,body,foot,onopen}){
  hideTip();closePop(false);const prev=document.activeElement;closeSheet(false);
  const sc=document.createElement('div');sc.id='sheet';sc.innerHTML=`<div class="sheetscrim"></div><aside class="sheet" role="dialog" aria-modal="true" aria-labelledby="sheet-t"><div class="sheet-h"><span id="sheet-t" class="grow">${title}</span><button class="btn ghost icon" data-sheet-close aria-label="Close"${tip('Close (Esc)')}>${ic('x')}</button></div><div class="sheet-b">${body}</div>${foot?`<div class="sheet-f">${foot}</div>`:''}</aside>`;
  L().appendChild(sc);sc._prev=prev;
  sc.addEventListener('keydown',ev=>{if(ev.key==='Escape'){ev.stopPropagation();closeSheet()}trap($('.sheet',sc),ev)});
  sc.addEventListener('click',ev=>{if(ev.target.matches('.sheetscrim,[data-sheet-close]')||ev.target.closest('[data-sheet-close]'))closeSheet()});
  onopen&&onopen(sc);return sc}
function closeSheet(ret=true){const s=$('#sheet');if(!s)return;const p=s._prev;s.remove();S.help.open=false;document.documentElement.dataset.sheet='';if(ret&&p&&document.contains(p))p.focus({preventScroll:true})}
function toast(msg,icon='check'){const t=document.createElement('div');t.className='toast';t.setAttribute('role','status');t.innerHTML=ic(icon)+esc(msg);$('.toasts').appendChild(t);setTimeout(()=>t.remove(),2600)}
function copy(text,label){try{navigator.clipboard&&navigator.clipboard.writeText(text)}catch(e){}toast(label||'Copied')}

/* ---------- popover registry (short paragraph + Open Help) ---------- */
const POP={
 status:()=>({title:'Status',body:'<p>How much this model is proven. <b>Verified</b> needs maintainer reports; none yet. <b>Untested</b> is the usual case.</p>',help:'compat-status'}),
 found:()=>({title:'Scan details',body:`<div class="col">${[['mDNS','ok'],['UPnP','ok'],['Synology Assistant','ok'],['DSM API','ok'],['Subnet sweep','warn']].map(([n,v])=>`<div class="row">${tag(v==='ok'?'ok':'slow',{v,i:v==='ok'?'circle-check':'triangle-alert'})}${n}</div>`).join('')}</div>`,help:'troubleshooting'}),
 compat:()=>{const k=cur().compat;return {title:COMPAT[k][2],body:k==='untested'?'<p>No one has reported an install on this model. The hardware class should work in theory; your install is the test.</p>':'<p>One report exists, from another project. Not yet to the project standard.</p>',help:'compat-status'}},
 flash:()=>({title:'Boot flash',body:'<p>Synology’s internal boot module. We add entries to it; the whole flash is saved to this computer first.</p>',help:'boot-manager'}),
 whatread:()=>({title:'What we read',body:'<p>Hardware, disks, RAID and network state. Nothing is written.</p><button class="link" data-dlg="script">Show script</button>',help:'first-install'}),
 bootmgr:()=>({title:'Boot manager',body:'<p>Adds a small SSH agent to the boot flash. Disks stay untouched; DSM stays the default.</p>',help:'boot-manager'}),
 install:()=>({title:'Install Debian',body:'<p>Erases the disks; needs your backup. About 25 minutes. Data layout from None to RAID 10 or Hybrid.</p>',help:'first-install'}),
 restore:()=>({title:'Restore / Recover',body:'<p>Reads the unit’s state before offering anything.</p>',help:'recovery'}),
 raid:()=>({title:'Data RAID',body:`<table class="tbl sm"><thead><tr><th>Level</th><th>Min</th><th>Survives</th></tr></thead><tbody>${[['RAID 1',2,'n−1'],['RAID 5',3,'1'],['RAID 6',4,'2'],['RAID 10',4,'1 / pair'],['Hybrid 1-disk',2,'1'],['Hybrid 2-disk',4,'2']].map(r=>`<tr><td>${r[0]}</td><td>${r[1]}</td><td>${r[2]}</td></tr>`).join('')}</tbody></table>`,help:'raid-levels'}),
 kernel:()=>({title:'Kernel',body:'<p>Stable is 6.12 LTS. Newest is 7.x from backports and falls back to Stable.</p>',help:'first-install'}),
 source:()=>({title:'Source',body:'<p>Offline uses a downloaded bundle for this CPU architecture (Settings, Data).</p>',help:'offline-mode'}),
 bootvia:()=>({title:'Boot via',body:'<p>Debian starts through the boot flash on this unit. Backed up first; DSM stays default.</p>',help:'boot-manager'}),
 before25:()=>({title:'Before you start',body:'<ul><li>Keep this computer awake and on the network.</li><li>The NAS needs Internet unless Source is Offline.</li><li>Unplug USB and eSATA disks.</li><li>The first RAID sync takes about 9 hours.</li></ul>',help:'first-install'}),
 phases:()=>({title:'Phases',body:`<ol>${PHASES.map((p,i)=>`<li${i===phaseIdx()?' style="font-weight:700"':''}>${p}</li>`).join('')}</ol>`,help:'first-install'}),
 cert:()=>({title:'Certificate',body:`<p class="mono" style="word-break:break-all">5E:91:C4:0B:7A:D2:38:F6:19:AC:63:E0:B4:27:8D:55:F1:0A:9C:42:E7:3B:68:D0:14:AF:C9:76:2D:83:3C:A1</p><button class="btn sm" data-copy="5E:91:C4:0B:7A:D2:38:F6:19:AC:63:E0:B4:27:8D:55:F1:0A:9C:42:E7:3B:68:D0:14:AF:C9:76:2D:83:3C:A1">${ic('copy')}Copy</button>`,help:'troubleshooting'}),
 wayback:()=>({title:'Way back',body:`<p>${wayBack()}</p>`,help:'recovery'}),
 backup:()=>({title:'Backup',body:`<p>${esc(cur().bk==='none found'?'No Hyper Backup, USB Copy or rsync task was found on this NAS.':'A backup task exists. Test a restore before you erase.')}</p><p>Copy everything you need to another device first. After step 3 DSM and its data may be gone.</p>`,help:'backups'}),
 bootrisk:()=>({title:'Boot flash',body:'<p>The whole flash is saved on this computer first, then entries and the agent are added. DSM stays the default entry.</p>',help:'boot-manager'}),
 oneshot:()=>({title:'One-shot agent',body:'<p>It resets the default to DSM when it starts. kexec starts Linux kernels only.</p>',help:'boot-manager'}),
 level:()=>({title:'Level',body:'<p>RAID level and minimum disks.</p>',help:'raid-levels'}),
 mgmtv:()=>({title:'Management app',body:'<p>The web UI on the unit. This app bundles a newer build and can update it over SSH.</p>',help:'first-install'}),
 diskcard:()=>({title:'Disks',body:'<p>SMART status and temperature, read over SSH from <span class="mono">synofurb-cli manage health</span>.</p>',help:'troubleshooting'})};
const PHASES=['Connecting','Inspecting','Staging','Booting installer','Installing','Waiting for reboot','Verifying'];
function phaseIdx(){const s=S.inst.step;return s<0?2:s<4?(s<3?2:3):s<16?4:s<17?5:6}
function wayBack(){const r=risk(),pl=['setup','plan','system','confirm'].includes(S.screen);
  return {'Read-only':'Nothing to undo.','Plan only':pl?'Nothing is written yet. From installer step 3 there is no undo: a failure opens a 15-minute rescue shell, then the DSM loader. No data promise.':'Nothing to undo.','Flash changed':'Power-cycle or Cancel: DSM boots. Our entries stay until removed.','Disks changed':'Rescue shell 15 min, then the DSM loader; no data promise.','Failed safely':'DSM boots again by itself.','Debian running':'Recovery stays under Manage.','Needs attention':'Recovery stays under Manage. Repairs happen in the web UI.','Critical':'Recovery stays under Manage. Repairs happen in the web UI.','Unreachable':'Check power and network, then Retry.'}[r.t]||''}
