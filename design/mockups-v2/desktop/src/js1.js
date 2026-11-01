/* ===== synofurb desktop mockup v2 :: data, helpers, widgets ===== */
const $=(s,r=document)=>r.querySelector(s), $$=(s,r=document)=>[...r.querySelectorAll(s)];
const esc=s=>String(s).replace(/[&<>"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const ic=(n,c='')=>`<svg class="ic ${c}" aria-hidden="true"><use href="#i-${n}"/></svg>`;
const d=(s,c='')=>`<span class="d ${c}">${s}</span>`;           // data: not counted in the word budget
const mono=s=>d(s,'mono');
const tip=t=>` data-tip="${esc(t)}"`;
const TiB=v=>v.toFixed(2)+' TiB';
const WORDS=s=>s.trim()?s.trim().split(/\s+/).length:0;

/* ---------- sample data (all of it is sample data) ---------- */
const UNITS=[
 {id:'ds923',model:'DS923+',sn:'2250R8N000912',ip:'192.168.1.47',dsm:'7.2.2',found:['mDNS','UPnP','API'],compat:'community',cpu:'AMD Ryzen R1600',ram:'4 GiB',fw:'UEFI',flash:'editable',con:'memory-mapped UART',bk:'none found',
  disks:[['sda','WDC WD80EFPX',7.276],['sdb','WDC WD80EFPX',7.276],['sdc','WDC WD40EFPX',3.638],['sdd','WDC WD40EFPX',3.638]],vol:'volume1 · btrfs · 9.8 TiB',used:'4.1 TiB'},
 {id:'ds918',model:'DS918+',sn:'1930PDN123456',ip:'192.168.1.40',dsm:'7.2.2',found:['Assistant','mDNS','API'],compat:'untested',cpu:'Celeron J3455',ram:'8 GiB',fw:'BIOS',flash:'editable',con:'ttyS0 115200',bk:'none found',
  disks:[['sda','WDC WD40EFRX',3.638],['sdb','WDC WD40EFRX',3.638],['sdc','WDC WD40EFRX',3.638,'8 reallocated sectors'],['sdd','WDC WD40EFRX',3.638]],vol:'volume1 · btrfs · 10.9 TiB',used:'6.2 TiB'},
 {id:'ds1621',model:'DS1621+',sn:'2060SQR204118',ip:'192.168.1.52',dsm:'7.2.1',found:['Assistant','UPnP'],compat:'untested',cpu:'AMD Ryzen V1500B',ram:'32 GiB ECC',fw:'UEFI',flash:'editable',con:'memory-mapped UART',bk:'none found',
  disks:[['sda','ST2000VN004',1.819],['sdb','WDC WD40EFRX',3.638],['sdc','WDC WD40EFRX',3.638],['sdd','WDC WD40EFRX',3.638],['sde','ST8000VN004',7.276],['sdf','ST8000VN004',7.276]],vol:'volume1 · btrfs · 22.1 TiB',used:'9.4 TiB'}];
const TWIN={id:'ds918b',model:'DS918+',sn:'1930PDN557710',ip:'192.168.1.58',dsm:'7.2.1',found:['mDNS'],compat:'untested',cpu:'Celeron J3455',ram:'8 GiB',fw:'BIOS',flash:'editable',con:'ttyS0 115200',bk:'none found',disks:UNITS[1].disks,vol:'volume1',used:'1 TiB'};
const DEBIAN_UNITS=[{id:'attic',model:'DS218+',ip:'192.168.1.33',os:'Debian 13.2',kernel:'6.12.57'}];
const COMPAT={verified:['shield-check','ok','Verified'],community:['users','info','Community-reported'],untested:['circle-dashed','','Untested'],issues:['triangle-alert','warn','Known issues'],unsupported:['ban','err','Unsupported']};
const tag=(txt,{v='',i='',fill=false,rd=false,lg=false,t='',attrs=''}={})=>`<span class="tag ${v} ${fill?'fill':''} ${rd?'rd':''} ${lg?'lg':''}"${t?tip(t):''}${attrs}>${i?ic(i,'s'):''}${txt}</span>`;
const compatTag=(k,o={})=>{const[c,v,l]=COMPAT[k];return tag(l,{v,i:c,...o})};
const cur=()=>S.unit||UNITS[1];

/* ---------- RAID capacity engine (docs/RAID.md rules; sample data only) ---------- */
const RAID_OPTS=[['auto','Auto'],['none','None'],['raid0','RAID 0'],['raid1','RAID 1'],['raid5','RAID 5'],['raid6','RAID 6'],['raid10','RAID 10'],['hyb1','Hybrid 1-disk'],['hyb2','Hybrid 2-disk']];
const MINDISKS={none:1,raid0:2,raid1:2,raid5:3,raid6:4,raid10:4,hyb1:2,hyb2:4};
const SYS_GIB=32, SYS_TIB=SYS_GIB/1024;
function capacity(raid,disks){
  const n=disks.length, data=disks.map(x=>+(x[2]-SYS_TIB).toFixed(4));
  let lvl=raid; if(raid==='auto') lvl=n===1?'none':n===2?'raid1':n<=5?'raid5':'raid6';
  const min=Math.min(...data), r={level:lvl,n,data,min,slices:[],survives:'0 disks',usable:0,error:null,weak:false};
  if(n<MINDISKS[lvl]){r.error=`${lvl.startsWith('hyb')?'Hybrid needs':'This level needs'} ${MINDISKS[lvl]}+ disks`;return r}
  if(lvl==='none'){r.usable=min;r.survives='0 disks'}
  else if(lvl==='raid0'){r.usable=n*min;r.survives='0 disks'}
  else if(lvl==='raid1'){r.usable=min;r.survives=(n-1)+' disks'}
  else if(lvl==='raid5'){r.usable=(n-1)*min;r.survives='1 disk'}
  else if(lvl==='raid6'){r.usable=(n-2)*min;r.survives='2 disks'}
  else if(lvl==='raid10'){r.usable=Math.floor(n/2)*min;r.survives='1 disk per pair'}
  else{ // hybrid: slices at the sizes of the different disks
    const sizes=[...new Set(data.map(x=>x.toFixed(3)))].map(Number).sort((a,b)=>a-b); let prev=0;
    sizes.forEach((sz,i)=>{const th=+(sz-prev).toFixed(4), mem=data.map((x,k)=>x>=sz-1e-3?k:-1).filter(k=>k>=0);
      let L='unused',u=0; if(mem.length===2){L='RAID 1';u=th}
      else if(mem.length>=3){ if(lvl==='hyb2'&&mem.length>=4){L='RAID 6';u=(mem.length-2)*th} else {L='RAID 5';u=(mem.length-1)*th} }
      const weak=lvl==='hyb2'&&mem.length>=2&&mem.length<4&&L!=='unused';
      r.slices.push({id:'data'+(i+1),th,mem,L,u:+u.toFixed(4),weak,idx:i}); prev=sz});
    r.usable=r.slices.reduce((a,s)=>a+s.u,0); r.survives=lvl==='hyb1'?'1 disk':'2 disks'; r.weak=r.slices.some(s=>s.weak);
  }
  r.usable=+r.usable.toFixed(4); r.raw=+disks.reduce((a,x)=>a+x[2],0).toFixed(4); return r;
}
const LEVEL_LABEL={none:'None',raid0:'RAID 0',raid1:'RAID 1',raid5:'RAID 5',raid6:'RAID 6',raid10:'RAID 10',hyb1:'Hybrid 1-disk',hyb2:'Hybrid 2-disk'};

/* ---------- state ---------- */
const S={screen:'find',unit:null,theme:'light',units:UNITS,twin:false,offline:false,scan:'done',filter:'',sel:null,update:null,
  plan:{mode:'wipe',sys:32,copies:3,raid:'auto',free:20},
  sys:{host:'ds918',user:'andreas',pw:'tundra-ember-Sage-4471',key:'Generate',tz:'Europe/Stockholm',web:true,kernel:'Stable',source:'Online'},
  conn:{port:'22',user:'admin',auth:'Password',pw:'',sudo:true,keep:false,state:'idle',err:null,trusted:false},
  gate:{b1:false,b2:false,b3:false,phrase:''},
  inst:{step:-1,frozen:false,t0:0},fail:null,done:{copied:false},
  manage:{tab:'overview',health:'ok',mgmt:'idle',mgmtPct:0},
  settings:{page:'General'},help:{topic:null,q:''},mark:true,dev:false,lastFocus:null,boot:{key:'Generate',done:false},seenFirst:false};
