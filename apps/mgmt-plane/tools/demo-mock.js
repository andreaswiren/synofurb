/* Demo layer for the synofurb management UI: replaces fetch('/api/...') with canned data and simulated actions.
   Nothing here talks to a server. Generated into demo/index.html by tools/build-demo.py. */
(function () {
  'use strict';
  const DATA = /*__DATA__*/null;
  const GIB = 1073741824;
  const qs = new URLSearchParams(location.search);
  const key = /1621|amd/i.test(qs.get('unit') || '') ? 'ds1621' : 'ds918';
  const SRC = DATA.units[key];
  const U = JSON.parse(JSON.stringify(SRC));
  const st = {login: qs.get('login') !== '1', user: 'admin', pending: null, prev: null, timer: null, boot: qs.get('boot') !== 'none', t0: Date.now(), hostname: SRC.meta.host, mdn: 5};
  const clone = o => JSON.parse(JSON.stringify(o));
  const J = (code, obj) => new Response(JSON.stringify(obj), {status: code, headers: {'Content-Type': 'application/json'}});
  const E = (code, msg) => J(code, {ok: false, error: msg});
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const origFetch = window.fetch ? window.fetch.bind(window) : null;
  const fmt = n => (n / GIB).toFixed(0) + ' GiB';

  /* ---------- validators (same rules as the server) ---------- */
  const reUser = /^[a-z_][a-z0-9_-]{0,31}$/;
  const reVol = /^[a-z][a-z0-9_]{0,23}$/;
  const RESERVED = ['sys', 'data', 'root', 'swap', 'boot', 'efi'];
  const reMp = /^\/(srv|mnt)\/[A-Za-z0-9][A-Za-z0-9._-]{0,63}(\/[A-Za-z0-9][A-Za-z0-9._-]{0,63}){0,2}$/;
  const reHost = /^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?(\.[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*$/;
  const reKey = /^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp(256|384|521)) [A-Za-z0-9+/]{16,}={0,3}( [ -~]{0,100})?$/;
  const bad = (code, msg) => { const e = new Error(msg); e.code = code; return e; };
  function needConfirm(b, expected) {
    if (typeof b.confirm !== 'string' || b.confirm !== expected) throw bad(400, "confirmation does not match: type '" + expected + "' to confirm");
  }
  function ip4(s) {
    const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(s || '');
    if (!m) return null;
    const o = m.slice(1).map(Number);
    return o.some(x => x > 255) ? null : ((o[0] * 16777216) + (o[1] << 16) + (o[2] << 8) + o[3]) >>> 0;
  }
  function cidr4(s) {
    if (typeof s !== 'string' || !s.includes('/')) throw bad(400, 'IPv4 address needs a prefix length, e.g. 192.168.1.10/24');
    const [a, p] = s.split('/'), n = ip4(a), pl = Number(p);
    if (n === null || !/^\d{1,2}$/.test(p) || pl < 1 || pl > 32) throw bad(400, 'invalid IPv4 address/prefix');
    return {addr: a, prefix: pl, n, net: (n & (pl === 32 ? 0xffffffff : (~((1 << (32 - pl)) - 1) >>> 0))) >>> 0};
  }
  const isV6 = s => typeof s === 'string' && /^[0-9a-fA-F:]+$/.test(s) && s.includes(':');
  const list = v => (Array.isArray(v) ? v : String(v || '').split(/[,\s]+/)).map(x => String(x).trim()).filter(Boolean).slice(0, 8);

  /* ---------- derived state ---------- */
  function pendingView() {
    if (!st.pending) return null;
    return {ifname: st.pending.ifname, deadline: st.pending.deadline / 1000, seconds_left: Math.max(0, Math.round((st.pending.deadline - Date.now()) / 1000))};
  }
  function syncStatus() {
    const v = U.volumes;
    U.status.lvm = {vgs: clone(v.vgs), lvs: v.lvs.map(l => ({lv_name: l.lv_name, vg_name: l.vg_name, lv_size: l.lv_size, lv_path: l.lv_path, lv_attr: l.lv_attr})), pvs: clone(v.pvs)};
    U.status.md.arrays = clone(v.md);
    U.status.mounts = clone(U.mounts.mounts);
    U.status.hostname = st.hostname;
  }
  function bootDefault(id) {
    U.boot.entries.forEach(e => { e.default = e.id === id; });
    const e = U.boot.entries.find(x => x.id === id);
    U.boot.default_id = id; U.boot.default_name = e.name; U.boot.default_valid = true;
  }
  function overlay(title, text, label) {
    const ov = document.createElement('div');
    ov.id = 'demo-ov';
    ov.innerHTML = '<div role="alertdialog" aria-modal="true" aria-labelledby="demo-ov-t"><h2 id="demo-ov-t"></h2><p></p><button type="button" class="btn primary"></button></div>';
    ov.querySelector('h2').textContent = title; ov.querySelector('p').textContent = text;
    const b = ov.querySelector('button'); b.textContent = label || 'Back to the demo'; b.onclick = () => ov.remove();
    document.body.append(ov); b.focus();
  }
  const rollbackNow = () => {
    if (!st.pending) return false;
    const i = U.network.interfaces.findIndex(x => x.ifname === st.pending.ifname);
    if (i >= 0 && st.prev) U.network.interfaces[i] = st.prev;
    clearTimeout(st.timer); st.pending = null; st.prev = null;
    return true;
  };

  /* ---------- volumes (port of plan_volume_create) ---------- */
  const RAID_MIN = {raid0: 2, raid1: 2, raid5: 3, raid6: 4, raid10: 4};
  function usable(level, sizes) {
    if (!sizes.length) return 0;
    const mn = Math.min(...sizes), n = sizes.length;
    return {none: sizes.reduce((a, b) => a + b, 0), raid0: mn * n, raid1: mn, raid5: mn * (n - 1), raid6: mn * (n - 2), raid10: Math.floor(mn * n / 2)}[level];
  }
  function volPlan(b) {
    const name = b.name;
    if (typeof name !== 'string' || !reVol.test(name)) throw bad(400, 'volume name must match [a-z][a-z0-9_]{0,23}');
    if (RESERVED.includes(name)) throw bad(400, "volume name '" + name + "' is reserved");
    const level = b.level || 'none', fs = b.fs || 'xfs';
    if (!['none', 'raid0', 'raid1', 'raid5', 'raid6', 'raid10'].includes(level)) throw bad(400, 'RAID level must be one of: none, raid0, raid1, raid5, raid6, raid10');
    if (!['xfs', 'ext4', 'btrfs'].includes(fs)) throw bad(400, 'filesystem must be one of: xfs, ext4, btrfs');
    const mp = b.mountpoint || '/srv/' + name;
    if (!reMp.test(mp)) throw bad(400, 'mountpoint must be /srv/<name> or /mnt/<name> (max 3 levels)');
    let size = null;
    if (!(b.size_gib === null || b.size_gib === '' || b.size_gib === undefined || b.size_gib === 'all')) {
      size = Number(b.size_gib);
      if (!Number.isInteger(size)) throw bad(400, 'size_gib must be a whole number or empty');
    }
    if (!Array.isArray(b.devices)) throw bad(400, 'devices must be a list');
    const V = U.volumes, sizes = [];
    for (const d of b.devices) {
      if (!/^\/dev\/[A-Za-z0-9][A-Za-z0-9_.+-]{0,63}(\/[A-Za-z0-9][A-Za-z0-9_.+-]{0,63})?$/.test(d)) throw bad(400, 'invalid device path');
      const c = V.candidates.find(x => x.path === d);
      if (!c) { const x = V.excluded.find(y => y.path === d); throw bad(400, x ? d + ' cannot be used: ' + x.reason : d + ': no such block device'); }
      sizes.push(c.size);
    }
    if (V.vgs.some(g => g.vg_name === 'vg_' + name)) throw bad(409, 'volume group vg_' + name + ' already exists');
    if (U.mounts.mounts.some(x => x.target === mp) || U.mounts.fstab.some(x => x.mountpoint === mp)) throw bad(409, mp + ' is already mounted or listed in fstab');
    if (new Set(b.devices).size !== b.devices.length) throw bad(400, 'duplicate devices selected');
    if (level === 'none' && b.devices.length < 1) throw bad(400, 'select at least one device');
    if (level !== 'none' && b.devices.length < RAID_MIN[level]) throw bad(400, level + ' needs at least ' + RAID_MIN[level] + ' devices (got ' + b.devices.length + ')');
    if (level === 'raid10' && b.devices.length % 2) throw bad(400, 'raid10 needs an even number of devices');
    if (size !== null && size < 1) throw bad(400, 'size must be a whole number of GiB, or empty for all');
    const us = usable(level, sizes);
    if (size !== null && size * GIB > us) throw bad(400, 'requested ' + size + ' GiB exceeds the usable capacity (' + Math.floor(us / GIB) + ' GiB)');
    const vg = 'vg_' + name, lvp = '/dev/' + vg + '/' + name, md = '/dev/md/' + name, steps = [];
    const S = (desc, argv, kind) => steps.push({desc, argv, kind: kind || 'cmd', cmd: argv ? argv.join(' ') : ''});
    b.devices.forEach(d => S('Wipe old signatures on ' + d, ['wipefs', '-a', d]));
    let pvs = b.devices;
    if (level !== 'none') {
      S('Create ' + level + ' array ' + md + ' from ' + b.devices.length + ' devices', ['mdadm', '--create', md, '--run', '--level=' + level, '--raid-devices=' + b.devices.length, '--metadata=1.2', '--name=' + name].concat(b.devices));
      S('Wait for device nodes', ['udevadm', 'settle', '--timeout=30']);
      steps.push({desc: 'Record array in /etc/mdadm/mdadm.conf', argv: ['mdadm', '--detail', '--scan', md], kind: 'mdconf', cmd: 'mdadm --detail --scan ' + md + '  >> /etc/mdadm/mdadm.conf'});
      S('Update initramfs so the array assembles at boot', ['update-initramfs', '-u']);
      pvs = [md];
    }
    S('Create LVM physical volume(s)', ['pvcreate', '--yes'].concat(pvs));
    S('Create volume group ' + vg, ['vgcreate', vg].concat(pvs));
    S('Create logical volume ' + name, ['lvcreate', '--yes', '-n', name].concat(size === null ? ['-l', '100%FREE'] : ['-L', size + 'G'], [vg]));
    S('Wait for device nodes', ['udevadm', 'settle', '--timeout=30']);
    const label = fs === 'xfs' ? name.slice(0, 12) : name.slice(0, 16);
    S('Create ' + fs + ' filesystem (label ' + label + ')', {xfs: ['mkfs.xfs', '-f', '-L', label, lvp], ext4: ['mkfs.ext4', '-F', '-L', label, lvp], btrfs: ['mkfs.btrfs', '-f', '-L', label, lvp]}[fs]);
    const opts = 'defaults,nofail,x-systemd.device-timeout=30';
    steps.push({desc: 'Add fstab entry (by UUID, nofail)', argv: null, kind: 'fstab', cmd: '[edit /etc/fstab] ' + mp + ' ' + fs + ' ' + opts});
    steps.push({desc: 'Create mountpoint ' + mp, argv: null, kind: 'mkdir', cmd: 'mkdir -p ' + mp});
    S('Reload systemd units', ['systemctl', 'daemon-reload']);
    S('Mount ' + mp, ['mount', mp]);
    const warnings = ['ALL DATA ON THE SELECTED DEVICES WILL BE DESTROYED.'];
    if (sizes.length && Math.max(...sizes) > Math.min(...sizes) * 1.1 && level !== 'none') warnings.push('Devices differ in size; only the smallest size is used per member.');
    if (level === 'none' && b.devices.length > 1) warnings.push('No RAID: the volume spans the devices without redundancy; losing one loses everything.');
    if (level === 'raid0') warnings.push('RAID0 has no redundancy.');
    return {name, vg, level, fs, mountpoint: mp, usable_bytes: us, warnings, steps, size, devices: b.devices, sizes, lvp, md};
  }
  const FAKE_OUT = {mdadm: 'mdadm: array started.', mkfs: 'meta-data=... 1 log, 1 data block groups created', lvcreate: 'Logical volume created.', vgcreate: 'Volume group created.', pvcreate: 'Physical volume successfully created.', wipefs: ''};
  function runResults(steps, ms) {
    return steps.map(s => ({desc: s.desc, cmd: s.cmd, rc: 0, out: (s.argv && FAKE_OUT[s.argv[0].split('.')[0]]) || '', err: ''}));
  }
  function applyCreate(p) {
    const V = U.volumes, lvSize = p.size !== null ? p.size * GIB : p.usable_bytes, vgFree = p.usable_bytes - lvSize;
    p.devices.forEach(d => {
      const i = V.candidates.findIndex(c => c.path === d);
      const c = V.candidates.splice(i, 1)[0];
      V.excluded.push(Object.assign(c, {reason: p.level === 'none' ? 'already an LVM physical volume' : 'already a RAID member'}));
    });
    V.vgs.push({vg_name: p.vg, vg_size: p.usable_bytes, vg_free: vgFree, pv_count: p.level === 'none' ? p.devices.length : 1, lv_count: 1});
    V.pvs.push({pv_name: p.level === 'none' ? p.devices[0] : p.md, vg_name: p.vg, pv_size: p.usable_bytes, pv_free: vgFree});
    V.lvs.push({lv_name: p.name, vg_name: p.vg, lv_size: lvSize, lv_path: p.lvp, lv_attr: '-wi-ao----', fstype: p.fs, mountpoints: [p.mountpoint], label: p.name});
    if (p.level !== 'none') {
      const n = 'md' + (st.mdn++);
      V.md.push({name: n, device: '/dev/' + n, state: 'active', flags: [], level: p.level, members: p.devices.map((d, i) => ({name: d.replace('/dev/', ''), role: i, failed: false, spare: false})),
        blocks: Math.floor(p.usable_bytes / 1024), size: p.usable_bytes, total: p.devices.length, active: p.devices.length, health: 'U'.repeat(p.devices.length), degraded: false,
        sync: p.level === 'raid0' ? null : {action: 'resync', percent: 0.4, finish_min: 640, speed_kbs: 95000}});
    }
    U.mounts.mounts.push({target: p.mountpoint, source: '/dev/mapper/' + p.vg + '-' + p.name, fstype: p.fs, options: 'rw,relatime', size: Math.floor(lvSize * 0.99), used: Math.floor(lvSize * 0.002), avail: Math.floor(lvSize * 0.98), real: true});
    U.mounts.fstab.push({spec: 'UUID=7f3c9e52-1a04-4b6d-8e2f-5c0d3a9b1e77', mountpoint: p.mountpoint, fstype: p.fs, options: 'defaults,nofail,x-systemd.device-timeout=30', managed: true, tag: p.name});
    syncStatus();
  }

  /* ---------- routes ---------- */
  const R = {};
  const PUBLIC = new Set(['GET /api/tls-fingerprint', 'POST /api/login']);
  R['GET /api/tls-fingerprint'] = () => Object.assign(clone(U.tls), {hostname: st.hostname, version: DATA.version});
  R['POST /api/login'] = b => {
    if (typeof b.username !== 'string' || typeof b.password !== 'string' || !b.username || !b.password) throw bad(400, 'username and password are required');
    if (b.password === 'wrong') throw bad(401, 'Invalid credentials');
    st.login = true; st.user = b.username.slice(0, 32);
    return {ok: true, user: st.user, csrf: 'demo-csrf'};
  };
  R['POST /api/logout'] = () => { st.login = false; return {ok: true}; };
  R['GET /api/me'] = () => ({ok: true, user: st.user, csrf: 'demo-csrf', hostname: st.hostname, version: DATA.version, dry_run: false, net_pending: pendingView()});
  R['GET /api/status'] = () => { U.status.uptime = SRC.meta.uptime + (Date.now() - st.t0) / 1000; syncStatus(); return clone(U.status); };
  R['GET /api/smart'] = () => clone(U.smart);
  /* users */
  const users = () => U.users.users;
  const sudoCount = pred => users().filter(u => u.sudo && !u.root && pred(u)).length;
  function findUser(n) {
    if (typeof n !== 'string' || !reUser.test(n)) throw bad(400, 'invalid user name (a-z, 0-9, _ and -, max 32, must start with a letter or _)');
    const u = users().find(x => x.name === n);
    if (!u) throw bad(404, 'no such user');
    if (u.root) throw bad(400, 'refusing to touch system accounts or root');
    return u;
  }
  const pwOk = p => { if (typeof p !== 'string' || p.length < 8 || p.length > 128) throw bad(400, 'password must be 8-128 characters'); };
  R['GET /api/users'] = () => Object.assign(clone(U.users), {current: st.user});
  R['POST /api/users/create'] = b => {
    if (typeof b.username !== 'string' || !reUser.test(b.username)) throw bad(400, 'invalid user name (a-z, 0-9, _ and -, max 32, must start with a letter or _)');
    pwOk(b.password);
    if (users().some(u => u.name === b.username)) throw bad(409, 'user ' + b.username + ' already exists');
    const known = U.users.groups, groups = list(b.groups);
    for (const g of groups) { if (!known.includes(g)) throw bad(400, 'group ' + JSON.stringify(g).replace(/"/g, "'") + ' is not allowed or does not exist'); }
    if (b.sudo && !groups.includes('sudo')) groups.push('sudo');
    if (b.ssh_key && !reKey.test(String(b.ssh_key).trim())) throw bad(400, 'not a valid single-line SSH public key (options prefixes are not allowed)');
    const uid = Math.max(...users().map(u => u.uid)) + 1;
    users().push({name: b.username, uid, home: '/home/' + b.username, shell: '/bin/bash', groups: [b.username].concat(groups).sort(), locked: false, has_password: true, sudo: groups.includes('sudo'), root: false,
      ssh_keys: b.ssh_key ? 1 : 0, web_login: groups.includes('sudo')});
    return {ok: true, user: b.username, groups};
  };
  R['POST /api/users/delete'] = b => {
    const u = findUser(b.username); needConfirm(b, u.name);
    if (u.name === st.user) throw bad(400, 'you cannot delete the account you are logged in with');
    if (u.sudo && sudoCount(() => true) === 1) throw bad(400, 'refusing to delete the last member of group sudo');
    U.users.users = users().filter(x => x !== u);
    return {ok: true};
  };
  R['POST /api/users/password'] = b => { findUser(b.username); pwOk(b.password); return {ok: true}; };
  function setLock(b, lock) {
    const u = findUser(b.username);
    if (lock) {
      if (u.name === st.user) throw bad(400, 'you cannot lock your own account');
      if (u.sudo && sudoCount(x => !x.locked && x !== u) === 0) throw bad(400, 'refusing to lock the last unlocked administrator');
    }
    u.locked = lock; u.web_login = u.sudo && u.has_password && !u.locked;
    return {ok: true};
  }
  R['POST /api/users/lock'] = b => setLock(b, true);
  R['POST /api/users/unlock'] = b => setLock(b, false);
  R['POST /api/users/ssh-key'] = b => {
    const u = findUser(b.username);
    if (!reKey.test(String(b.ssh_key || '').trim())) throw bad(400, 'not a valid single-line SSH public key (options prefixes are not allowed)');
    u.ssh_keys += 1; return {ok: true};
  };
  /* network */
  R['GET /api/network'] = () => { const o = clone(U.network); o.hostname = st.hostname; o.pending = pendingView(); return o; };
  R['POST /api/network/apply'] = b => {
    const ifc = U.network.interfaces.find(x => x.ifname === b.ifname);
    if (!ifc) throw bad(400, 'unknown interface');
    if (!['dhcp', 'static'].includes(b.mode || 'dhcp')) throw bad(400, 'mode must be dhcp or static');
    const cfg = {mode: b.mode || 'dhcp', ipv4: null, ipv6: null, gw4: null, gw6: null, dns: [], search: []};
    let addrs = [];
    if (cfg.mode === 'static') {
      const c = cidr4(b.ipv4);
      const gw = ip4(b.gateway4);
      if (gw === null) throw bad(400, 'invalid IP address: ' + JSON.stringify(String(b.gateway4 || '').slice(0, 64)));
      if ((gw & (c.prefix === 32 ? 0xffffffff : (~((1 << (32 - c.prefix)) - 1) >>> 0))) >>> 0 !== c.net) throw bad(400, 'gateway ' + b.gateway4 + ' is not inside the subnet of ' + b.ipv4);
      cfg.ipv4 = b.ipv4; cfg.gw4 = b.gateway4; addrs.push({family: 'inet', addr: c.addr, prefix: c.prefix, scope: 'global'});
      if (b.ipv6) {
        if (!b.ipv6.includes('/') || !isV6(b.ipv6.split('/')[0])) throw bad(400, 'IPv6 address needs a prefix length, e.g. 2001:db8::10/64');
        cfg.ipv6 = b.ipv6; addrs.push({family: 'inet6', addr: b.ipv6.split('/')[0], prefix: Number(b.ipv6.split('/')[1]), scope: 'global'});
        if (b.gateway6) { if (!isV6(b.gateway6)) throw bad(400, 'invalid IP address: ' + JSON.stringify(b.gateway6)); cfg.gw6 = b.gateway6; }
      }
      cfg.dns = list(b.dns);
      for (const d of cfg.dns) if (ip4(d) === null && !isV6(d)) throw bad(400, 'invalid IP address: ' + JSON.stringify(d));
      cfg.search = list(b.search);
      for (const d of cfg.search) if (!/^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?(\.[A-Za-z0-9-]+)*$/.test(d)) throw bad(400, 'invalid search domain: ' + JSON.stringify(d));
    } else {
      const orig = SRC.network.interfaces.find(x => x.ifname === ifc.ifname);
      addrs = clone(orig.addrs.filter(a => a.scope !== 'link'));
    }
    if (st.pending) throw bad(409, 'a previous network change is still waiting for confirmation');
    st.prev = clone(ifc);
    ifc.addrs = (ifc.state === 'UP' || cfg.mode === 'static' ? addrs : []).concat(ifc.addrs.filter(a => a.scope === 'link'));
    ifc.state = cfg.mode === 'static' || ifc.state === 'UP' ? 'UP' : ifc.state;
    ifc.managed = true; ifc.config = cfg; ifc.shadowed_by = [];
    st.pending = {ifname: ifc.ifname, deadline: Date.now() + 90000};
    clearTimeout(st.timer); st.timer = setTimeout(rollbackNow, 90000);
    return {ok: true, pending: pendingView(), reconfigure_rc: 0, config: '# Managed by synofurb-mgmt (demo)\n[Match]\nName=' + ifc.ifname + '\n',
      message: 'Applied. Confirm within 90 s from the NEW address or it is rolled back.'};
  };
  R['POST /api/network/confirm'] = () => {
    if (!st.pending) throw bad(409, 'no network change is waiting for confirmation');
    clearTimeout(st.timer); st.pending = null; st.prev = null; return {ok: true};
  };
  R['POST /api/network/rollback'] = () => { if (!rollbackNow()) throw bad(409, 'nothing to roll back'); return {ok: true}; };
  R['POST /api/hostname'] = b => {
    if (typeof b.hostname !== 'string' || b.hostname.length < 1 || b.hostname.length > 64 || !reHost.test(b.hostname)) throw bad(400, 'invalid hostname (letters, digits, \'-\' and dots; no leading/trailing \'-\')');
    st.hostname = b.hostname.toLowerCase(); syncStatus(); return {ok: true, hostname: st.hostname};
  };
  /* volumes */
  R['GET /api/volumes'] = () => clone(U.volumes);
  R['POST /api/volumes/plan'] = b => {
    const p = volPlan(b);
    return {ok: true, name: p.name, vg: p.vg, level: p.level, fs: p.fs, mountpoint: p.mountpoint, usable_bytes: p.usable_bytes, warnings: p.warnings, steps: p.steps, confirm: p.name};
  };
  R['POST /api/volumes/create'] = b => {
    if (typeof b.name !== 'string' || !reVol.test(b.name)) throw bad(400, 'volume name must match [a-z][a-z0-9_]{0,23}');
    needConfirm(b, b.name);
    const p = volPlan(b); applyCreate(p);
    return {ok: true, results: runResults(p.steps), mountpoint: p.mountpoint};
  };
  function findLv(vg, lv) {
    const l = U.volumes.lvs.find(x => x.vg_name === vg && x.lv_name === lv);
    if (!l) throw bad(404, 'no such logical volume ' + vg + '/' + lv);
    return l;
  }
  R['POST /api/volumes/extend'] = b => {
    const l = findLv(b.vg, b.lv), g = U.volumes.vgs.find(x => x.vg_name === l.vg_name);
    const add = (b.add_gib === null || b.add_gib === '' || b.add_gib === undefined) ? null : Number(b.add_gib);
    if (add !== null && (!Number.isInteger(add) || add < 1)) throw bad(400, 'add_gib must be a whole number or empty');
    if (g.vg_free <= 0 || (add !== null && add * GIB > g.vg_free)) throw bad(400, 'not enough free space in ' + g.vg_name + ' (' + Math.floor(g.vg_free / GIB) + ' GiB free)');
    const inc = add === null ? g.vg_free : add * GIB;
    l.lv_size += inc; g.vg_free -= inc;
    const pv = U.volumes.pvs.find(x => x.vg_name === g.vg_name); if (pv) pv.pv_free = g.vg_free;
    const mt = U.mounts.mounts.find(x => x.target === (l.mountpoints[0] || '')); if (mt) { mt.size += inc; mt.avail += inc; }
    syncStatus();
    const grow = l.fstype === 'xfs' ? ['xfs_growfs ' + l.mountpoints[0]] : l.fstype === 'btrfs' ? ['btrfs filesystem resize max ' + l.mountpoints[0]] : ['resize2fs ' + l.lv_path];
    return {ok: true, results: [{desc: 'Extend logical volume ' + g.vg_name + '/' + l.lv_name, cmd: 'lvextend ' + (add === null ? '-l +100%FREE ' : '-L +' + add + 'G ') + l.lv_path, rc: 0, out: 'Size of logical volume changed.', err: ''},
      {desc: 'Grow ' + l.fstype + ' filesystem', cmd: grow[0], rc: 0, out: '', err: ''}]};
  };
  R['POST /api/volumes/delete'] = b => {
    const l = findLv(b.vg, b.lv); needConfirm(b, l.lv_name);
    if (l.vg_name === 'vg_sys' || ['root', 'swap'].includes(l.lv_name)) throw bad(400, 'refusing to delete system volumes');
    if (l.mountpoints.some(m => ['/', '/boot', '/usr', '/var'].includes(m))) throw bad(400, 'volume holds a system filesystem');
    const V = U.volumes, res = [];
    l.mountpoints.forEach(mp => {
      res.push({desc: 'Unmount ' + mp, cmd: 'umount ' + mp, rc: 0, out: '', err: ''});
      U.mounts.mounts = U.mounts.mounts.filter(x => x.target !== mp); U.mounts.fstab = U.mounts.fstab.filter(x => x.mountpoint !== mp);
    });
    V.lvs = V.lvs.filter(x => x !== l);
    const g = V.vgs.find(x => x.vg_name === l.vg_name); g.lv_count -= 1; g.vg_free += l.lv_size;
    res.push({desc: 'Remove logical volume ' + l.vg_name + '/' + l.lv_name, cmd: 'lvremove --yes ' + l.lv_path, rc: 0, out: 'Logical volume removed.', err: ''});
    if (b.remove_vg && g.lv_count === 0) { V.vgs = V.vgs.filter(x => x !== g); V.pvs = V.pvs.filter(x => x.vg_name !== g.vg_name); res.push({desc: 'Remove volume group ' + g.vg_name, cmd: 'vgremove --yes ' + g.vg_name, rc: 0, out: '', err: ''}); }
    else { const pv = V.pvs.find(x => x.vg_name === g.vg_name); if (pv) pv.pv_free = g.vg_free; }
    syncStatus();
    return {ok: true, results: res};
  };
  /* mounts */
  R['GET /api/mounts'] = () => clone(U.mounts);
  R['POST /api/mounts/mount'] = b => {
    const M = U.mounts, d = M.devices.find(x => x.path === b.device);
    if (!d) throw bad(404, 'no such block device');
    if (!reMp.test(String(b.mountpoint || ''))) throw bad(400, 'mountpoint must be /srv/<name> or /mnt/<name> (max 3 levels)');
    if (M.mounts.some(x => x.target === b.mountpoint)) throw bad(409, b.mountpoint + ' is already a mountpoint');
    const ro = 'ro' in b ? !!b.ro : d.foreign, persist = !!b.persist;
    if (persist && !d.uuid) throw bad(400, 'device has no UUID; cannot persist in fstab');
    const type = {ntfs: 'ntfs-3g'}[d.fstype] || d.fstype;
    let opts = 'defaults,nofail' + (['vfat', 'exfat', 'ntfs'].includes(d.fstype) ? ',uid=1000,gid=1000,umask=0022' : '') + (ro ? ',ro' : '');
    M.devices = M.devices.filter(x => x !== d);
    M.mounts.push({target: b.mountpoint, source: d.path, fstype: type, options: opts.replace('defaults,nofail', 'rw,relatime').replace('rw,relatime,ro', 'ro,relatime'), size: d.size, used: Math.floor(d.size * 0.42), avail: Math.floor(d.size * 0.58), real: true, _dev: d});
    if (persist) M.fstab.push({spec: 'UUID=' + d.uuid, mountpoint: b.mountpoint, fstype: type, options: opts + ',x-systemd.device-timeout=30', managed: true, tag: 'mount'});
    syncStatus();
    return {ok: true, results: [{desc: 'Create mountpoint ' + b.mountpoint, cmd: 'mkdir -p ' + b.mountpoint, rc: 0, out: '', err: ''},
      {desc: 'Mount ' + d.path + ' on ' + b.mountpoint, cmd: 'mount -t ' + type + ' -o ' + opts + ' ' + d.path + ' ' + b.mountpoint, rc: 0, out: '', err: ''}]};
  };
  R['POST /api/mounts/unmount'] = b => {
    if (!reMp.test(String(b.mountpoint || ''))) throw bad(400, 'mountpoint must be /srv/<name> or /mnt/<name> (max 3 levels)');
    needConfirm(b, b.mountpoint);
    const M = U.mounts, mt = M.mounts.find(x => x.target === b.mountpoint);
    if (!mt) throw bad(404, b.mountpoint + ' is not mounted');
    if (mt.target === '/srv/data' && st.user === 'demo-block') throw bad(500, 'unmount failed (busy?)');
    M.mounts = M.mounts.filter(x => x !== mt);
    if (mt._dev) M.devices.push(mt._dev);
    if (b.remove_fstab) M.fstab = M.fstab.filter(x => !(x.mountpoint === mt.target && x.managed));
    syncStatus();
    return {ok: true, results: [{desc: 'Unmount ' + mt.target, cmd: 'umount ' + mt.target, rc: 0, out: '', err: ''}]};
  };
  R['POST /api/mounts/activate'] = () => ({ok: true, results: clone(U.activate.results), devices: clone(U.mounts.devices), other: clone(U.mounts.other), md: clone(U.volumes.md), lvs: clone(U.volumes.lvs)});
  /* services, system */
  R['GET /api/services'] = () => clone(U.services);
  R['POST /api/services/restart'] = b => { if (!U.services.restartable.includes(b.unit)) throw bad(400, 'unit not in the restart allow-list'); return {ok: true}; };
  R['POST /api/system/power'] = b => {
    if (!['reboot', 'poweroff'].includes(b.action)) throw bad(400, 'action must be reboot or poweroff');
    needConfirm(b, st.hostname);
    setTimeout(() => overlay(b.action === 'reboot' ? 'Rebooting (simulated)' : 'Powered off (simulated)', 'In the real app the NAS would ' + (b.action === 'reboot' ? 'restart now and come back in about 90 seconds.' : 'shut down now.') + ' This is a demo: nothing happens.'), 600);
    return {ok: true, message: b.action + ' scheduled'};
  };
  /* boot */
  R['GET /api/boot'] = () => st.boot ? Object.assign({ok: true, available: true}, clone(U.boot)) : {ok: true, available: false, message: 'not a boot-flash unit (synofurb-dom is not installed)'};
  function bootEntry(b) {
    if (!['dsm', 'debian', 'debian-old', 'agent'].includes(b.entry)) throw bad(400, 'entry must be one of: dsm, debian, debian-old, agent');
    if (!st.boot) throw bad(409, 'not a boot-flash unit (synofurb-dom is not installed)');
    if (b.entry !== 'debian') needConfirm(b, b.entry);
    const e = U.boot.entries.find(x => x.id === b.entry);
    if (!e.present) throw bad(404, 'boot entry ' + b.entry + ' is not present on the boot flash');
    return b.entry;
  }
  R['POST /api/boot/default'] = b => { const id = bootEntry(b); bootDefault(id); return {ok: true, entry: id, output: ''}; };
  R['POST /api/boot/reboot-to'] = b => {
    const id = bootEntry(b); bootDefault(id);
    setTimeout(() => overlay('Rebooting into ' + U.boot.entries.find(x => x.id === id).label + ' (simulated)', 'The NAS would restart in 5 seconds. This is a demo: nothing happens.'), 800);
    return {ok: true, entry: id, message: 'Default set to ' + id + '; rebooting in 5 s.'};
  };

  window.fetch = async function (url, opt) {
    const u = new URL(typeof url === 'string' ? url : url.url, location.href);
    if (!/\/api\//.test(u.pathname)) return origFetch ? origFetch(url, opt) : Promise.reject(new Error('offline demo'));
    const method = ((opt && opt.method) || 'GET').toUpperCase();
    const route = method + ' ' + u.pathname.slice(u.pathname.indexOf('/api/'));
    await sleep(route.includes('volumes/create') ? 900 : 90 + Math.random() * 90);
    const fn = R[route];
    if (!fn) return E(404, 'not found');
    if (!PUBLIC.has(route) && !st.login) return E(401, 'authentication required');
    let body = {};
    if (method === 'POST') {
      try { body = opt && opt.body ? JSON.parse(opt.body) : {}; } catch (e) { return E(400, 'invalid JSON'); }
      const h = (opt && opt.headers) || {};
      if (!PUBLIC.has(route) && h['X-CSRF-Token'] !== 'demo-csrf') return E(403, 'missing or invalid CSRF token');
    }
    try { return J(200, fn(body)); } catch (e) { if (e.code) return E(e.code, e.message); console.error(e); return E(500, 'internal error'); }
  };

  /* ---------- ribbon ---------- */
  function ribbon() {
    const el = document.createElement('div');
    el.id = 'demo-ribbon'; el.setAttribute('role', 'region'); el.setAttribute('aria-label', 'Demo controls');
    el.innerHTML = '<button type="button" id="demo-pill" aria-expanded="false" aria-controls="demo-panel"><span class="dot"></span>Demo data</button>' +
      '<button type="button" id="demo-login">Show login page</button>' +
      '<div id="demo-panel" hidden><p>Simulated data. Every action only changes this page, nothing is sent anywhere. Sign in with any user name; the password "wrong" is rejected.</p>' +
      '<label>Unit<select id="demo-unit"><option value="ds918">DS918+ (Intel, 4 bays)</option><option value="ds1621">DS1621+ (AMD, 6 bays)</option></select></label>' +
      '<label class="chk"><input type="checkbox" id="demo-boot"> This unit has a boot flash</label>' +
      '<button type="button" id="demo-reset">Reset the demo</button></div>';
    document.body.append(el);
    const pill = el.querySelector('#demo-pill'), panel = el.querySelector('#demo-panel');
    pill.onclick = () => { const open = panel.hidden; panel.hidden = !open; pill.setAttribute('aria-expanded', String(open)); };
    document.addEventListener('keydown', e => { if (e.key === 'Escape' && !panel.hidden) { panel.hidden = true; pill.setAttribute('aria-expanded', 'false'); pill.focus(); } });
    el.querySelector('#demo-unit').value = key;
    el.querySelector('#demo-unit').onchange = e => { const p = new URLSearchParams(location.search); p.set('unit', e.target.value); location.search = p.toString(); };
    const bc = el.querySelector('#demo-boot'); bc.checked = st.boot;
    bc.onchange = () => { st.boot = bc.checked; if (st.login && typeof show === 'function' && S.page === 'boot') show('boot'); };
    el.querySelector('#demo-login').onclick = () => { st.login = false; clearTimeout(st.timer); st.pending = null; st.prev = null; showLogin(); };
    el.querySelector('#demo-reset').onclick = () => { const p = new URLSearchParams(location.search); p.delete('login'); location.search = p.toString(); };
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', ribbon); else ribbon();
})();
