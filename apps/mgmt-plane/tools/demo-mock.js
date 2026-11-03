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


  /* ---------- RAID (simulated: plans mirror the server's wording and rules; state changes and syncs tick in the browser) ---------- */
  const RA = () => U.raid.arrays;
  const GIBn = 1073741824, MIBn = 1048576;
  const BUSY = ['resync', 'recovery', 'recover', 'reshape', 'check', 'repair'];
  const step = (desc, argv, kind) => ({desc, argv: argv || null, kind: kind || 'cmd', cmd: argv ? argv.join(' ') : ''});
  const sysStep = (name, f, v) => ({desc: 'Write ' + f, argv: null, kind: 'sysfs', cmd: 'echo ' + v + ' > /sys/block/' + name + '/md/' + f});
  const minActive = (lvl, n) => ({raid0: n, raid1: 1, raid4: n - 1, raid5: n - 1, raid6: n - 2, raid10: Math.max(1, Math.floor(n / 2))}[lvl] ?? n);
  function startSync(a, action, rate, onDone, p0) {
    a._sync = {action, p0: p0 || 0, t0: Date.now(), rate, onDone};
    a.sync = {action, percent: p0 || 0, finish_min: Math.round((100 - (p0 || 0)) / rate / 60), speed_kbs: 88000}; a.busy = true;
  }
  function tickSync(a) {
    const y = a._sync;
    if (!y) return;
    const p = Math.min(100, y.p0 + (Date.now() - y.t0) / 1000 * y.rate);
    if (p >= 100) { const f = y.onDone; a._sync = null; a.sync = null; a.busy = false; if (f) f(a); a.state = a.degraded ? 'degraded' : 'clean'; return; }
    a.sync.percent = Math.round(p * 10) / 10; a.sync.finish_min = Math.round((100 - p) / y.rate / 60); a.busy = true;
  }
  U.raid.arrays.forEach(a => {
    if (a.sync && a.sync.percent != null) { a._sync = {action: a.sync.action, p0: a.sync.percent, t0: Date.now(), rate: 0.15, onDone: null}; }
  });
  const need = (a, what) => { if (a.busy) throw bad(409, a.name + ' is ' + (a.sync && a.sync.action) + '; wait until it is idle before ' + what); };
  function arrByName(n) {
    if (typeof n !== 'string' || !/^md\d{1,4}$/.test(n)) throw bad(400, 'invalid array name (expected md0, md127, ...)');
    const a = RA().find(x => x.name === n);
    if (!a) throw bad(404, 'no such array ' + n);
    return a;
  }
  function freeDev(p, minB) {
    const c = U.raid.candidates.find(x => x.path === p);
    if (!c) { const x = U.raid.excluded.find(y => y.path === p); throw bad(x ? 409 : 400, x ? p + ' cannot be used: ' + x.reason : p + ': no such block device'); }
    if (minB && c.size < minB) throw bad(409, p + ' is smaller than the array members (' + (Math.floor(minB / GIBn) + 1) + ' GiB needed)');
    return c;
  }
  const memberMin = a => Math.min(...a.members.map(m => m.size || Infinity));
  const mdadmM = (a, ...rest) => ['mdadm', '--manage', a.device].concat(rest);
  const devsOf = v => { if (v === undefined || v === null || v === '') return []; if (!Array.isArray(v)) throw bad(400, 'devices must be a list'); return v; };
  function raidPlan(op, b) {
    const warn = [], follow = [];
    let steps = [], confirm = null, title = '', arr = null, info = {};
    if (!U.raid.available && op !== 'assemble') throw bad(409, 'mdadm is not installed (apt install mdadm)');
    if (op === 'assemble') {
      return {arr: null, plan: {op, title: 'Assemble arrays', steps: [step('Assemble arrays', ['mdadm', '--assemble', '--scan']), step('Scan LVM', ['vgscan']), step('Activate logical volumes', ['vgchange', '-ay'])], warnings: [], confirm: null, array: null, follow_ups: [], destructive: false}};
    }
    if (op === 'tuning' && !b.array) {
      if (!['enable', 'disable'].includes(b.scrub_schedule)) throw bad(400, 'nothing to change');
      steps = b.scrub_schedule === 'enable' ? [step('Write the monthly timer', null, 'unitfile'), step('Enable the monthly check', ['systemctl', 'enable', '--now', 'synofurb-raid-scrub.timer'])] : [step('Disable the monthly check', ['systemctl', 'disable', '--now', 'synofurb-raid-scrub.timer'])];
      return {arr: null, plan: {op, title: 'Tune RAID checks', steps, warnings: [], confirm: null, array: null, follow_ups: [], destructive: false}};
    }
    if (op === 'create') {
      const label = b.name;
      if (typeof label !== 'string' || !/^[a-z][a-z0-9_]{0,23}$/.test(label)) throw bad(400, 'array name must match [a-z][a-z0-9_]{0,23}');
      const level = b.level, devs = devsOf(b.devices), spares = devsOf(b.spares);
      if (!['raid0', 'raid1', 'raid5', 'raid6', 'raid10'].includes(level)) throw bad(400, 'level must be one of: raid0, raid1, raid5, raid6, raid10');
      const sizes = devs.concat(spares).map(d => freeDev(d).size);
      if (devs.length < RAID_MIN[level]) throw bad(400, level + ' needs at least ' + RAID_MIN[level] + ' devices (got ' + devs.length + ')');
      if (level === 'raid10' && devs.length % 2) throw bad(400, 'raid10 needs an even number of devices');
      if (RA().some(x => x.label === label || x.name === label)) throw bad(409, 'an array named ' + label + ' already exists');
      devs.concat(spares).forEach(d => steps.push(step('Wipe old signatures on ' + d, ['wipefs', '-a', d])));
      steps.push(step('Create ' + level + ' array /dev/md/' + label + ' from ' + devs.length + ' devices', ['mdadm', '--create', '/dev/md/' + label, '--run', '--level=' + level, '--raid-devices=' + devs.length, '--metadata=1.2', '--name=' + label].concat(level === 'raid0' ? [] : ['--bitmap=internal'], devs, spares)));
      steps.push(step('Wait for device nodes', ['udevadm', 'settle', '--timeout=30']), step('Record arrays in /etc/mdadm/mdadm.conf', ['mdadm', '--detail', '--scan'], 'mdconf'), step('Update initramfs so the arrays assemble at boot', ['update-initramfs', '-u']));
      if (b.volume) { const fs = b.volume.fs || 'xfs'; steps.push(step('Create LVM physical volume', ['pvcreate', '--yes', '/dev/md/' + label]), step('Create volume group vg_' + label, ['vgcreate', 'vg_' + label, '/dev/md/' + label]), step('Create logical volume ' + label, ['lvcreate', '--yes', '-n', label, '-l', '100%FREE', 'vg_' + label]), step('Create ' + fs + ' filesystem', ['mkfs.' + fs, '/dev/vg_' + label + '/' + label]), step('Mount /srv/' + label, ['mount', '/srv/' + label])); }
      const mn = Math.min(...sizes.slice(0, devs.length));
      warn.push('ALL DATA ON THE SELECTED DEVICES WILL BE DESTROYED.');
      if (Math.max(...sizes) > mn * 1.1) warn.push('Devices differ in size; only the smallest size is used per member.');
      if (level === 'raid0') warn.push('RAID0 has no redundancy.');
      confirm = label; title = 'Create ' + level + ' array ' + label; info = {usable_bytes: usable(level, sizes.slice(0, devs.length)), label, level, devs, spares};
    } else if (op === 'shr-create') {
      const name = b.name, mode = b.mode || b.level, devs = devsOf(b.devices);
      if (typeof name !== 'string' || !reVol.test(name)) throw bad(400, 'volume name must match [a-z][a-z0-9_]{0,23}');
      if (!['shr1', 'shr2'].includes(mode)) throw bad(400, 'mode must be shr1 or shr2');
      if (devs.length < (mode === 'shr1' ? 2 : 4)) throw bad(400, (mode === 'shr1' ? 'SHR-1 needs at least 2' : 'SHR-2 needs at least 4') + ' disks (got ' + devs.length + ')');
      const sizes = {};
      devs.forEach(d => { const c = freeDev(d); if (c.type !== 'disk') throw bad(400, d + ' is a partition; SHR needs whole disks'); sizes[d] = Math.max(0, c.size - 2 * MIBn); });
      if (U.volumes.vgs.some(g => g.vg_name === 'vg_' + name) || RA().some(a => a.vgs.includes('vg_' + name))) throw bad(409, 'volume group vg_' + name + ' already exists');
      const r = window.shrSlices(sizes, mode), us = r.slices.filter(s => s.level);
      if (!us.length) throw bad(400, 'these disks give no usable SHR capacity');
      const per = {};
      us.forEach(s => s.members.forEach(d => { (per[d] = per[d] || []).push(s); }));
      Object.keys(per).forEach(d => { steps.push(step('Wipe ' + d, ['wipefs', '-a', d]), step('Create a GPT on ' + d, ['parted', '-s', d, 'mklabel', 'gpt'])); per[d].forEach((s, k) => steps.push(step('Create partition ' + name + '_' + s.index + ' on ' + d, ['parted', '-s', '-a', 'optimal', d, 'mkpart', name + '_' + s.index, (1 + s.start / MIBn) + 'MiB', (1 + (s.start + s.length) / MIBn) + 'MiB']))); });
      steps.push(step('Re-read partition tables', ['partprobe'].concat(Object.keys(per))), step('Wait for device nodes', ['udevadm', 'settle', '--timeout=30']));
      const pvs = [];
      us.forEach(s => { const parts = s.members.map(d => d + (per[d].indexOf(s) + 1)); steps.push(step('Create ' + s.level + ' array /dev/md/' + name + '_' + s.index + ' from ' + parts.length + ' devices', ['mdadm', '--create', '/dev/md/' + name + '_' + s.index, '--run', '--level=' + s.level, '--raid-devices=' + parts.length, '--metadata=1.2', '--name=' + name + '_' + s.index].concat(parts))); pvs.push('/dev/md/' + name + '_' + s.index); s._parts = parts; });
      steps.push(step('Record arrays in /etc/mdadm/mdadm.conf', ['mdadm', '--detail', '--scan'], 'mdconf'), step('Update initramfs so the arrays assemble at boot', ['update-initramfs', '-u']),
        step('Create LVM physical volumes', ['pvcreate', '--yes'].concat(pvs)), step('Create volume group vg_' + name, ['vgcreate', 'vg_' + name].concat(pvs)), step('Create logical volume ' + name, ['lvcreate', '--yes', '-n', name, '-l', '100%FREE', 'vg_' + name]),
        step('Create ' + (b.fs || 'xfs') + ' filesystem', ['mkfs.' + (b.fs || 'xfs'), '/dev/vg_' + name + '/' + name]), step('Mount /srv/' + name, ['mount', '/srv/' + name]));
      warn.push('ALL DATA ON THE SELECTED DISKS WILL BE DESTROYED.', ...r.slices.filter(s => s.unusable).map(s => 'Slice ' + s.index + ': ' + Math.floor(s.length / GIBn) + ' GiB on ' + s.members[0] + ' has no partner disk and stays unused.'), 'SHR-style layout managed by Debian; DSM will not adopt it.');
      confirm = name; title = 'Create ' + (mode === 'shr1' ? 'SHR-1' : 'SHR-2') + ' volume ' + name;
      info = {shr: {mode, slices: r.slices, usable_bytes: r.usable, warnings: []}, usable_bytes: r.usable, name, mountpoint: '/srv/' + name, level: mode, _slices: us, _name: name};
    } else if (op === 'shr-add-disk' || op === 'shr-replace-disk') {
      const vol = b.volume, arrs = RA().filter(a => new RegExp('^' + vol + '_\\d+$').test(a.label) && a.vgs.includes('vg_' + vol)).sort((x, y) => x.label.localeCompare(y.label, undefined, {numeric: true}));
      if (!arrs.length) throw bad(404, 'no SHR volume ' + vol + ' created here');
      arrs.forEach(a => { if (a.busy || a.degraded) throw bad(409, 'array ' + a.name + ' is ' + (a.degraded ? 'degraded' : 'busy') + '; wait until it is healthy'); });
      const dev = b.device, c = U.raid.candidates.find(x => x.path === dev);
      if (!c || c.type !== 'disk') throw bad(400, dev + ' must be a whole disk');
      if (op === 'shr-add-disk') {
        let free = Math.max(0, c.size - 2 * MIBn), n = 0; const joined = [];
        arrs.forEach(a => { const len = Math.floor(memberMin(a) / MIBn) * MIBn; if (free >= len) { n++; joined.push([a, dev + n]); free -= len; } });
        if (joined.length) steps.push(step('Wipe ' + dev, ['wipefs', '-a', dev]), step('Create a GPT on ' + dev, ['parted', '-s', dev, 'mklabel', 'gpt']));
        joined.forEach(([a, p]) => { steps.push(step('Create partition ' + vol + ' on ' + dev, ['parted', '-s', '-a', 'optimal', dev, 'mkpart', a.label, '1MiB', '1MiB'])); });
        steps.push(step('Re-read partition tables', ['partprobe', dev]));
        joined.forEach(([a, p]) => { const nn = a.raid_devices + 1; steps.push(step('Add ' + p + ' to ' + a.name, mdadmM(a, '--add', p)), step('Reshape ' + a.name + ' to ' + nn + ' devices', ['mdadm', '--grow', a.device, '--raid-devices=' + nn, '--backup-file=/var/lib/synofurb-mgmt/md-' + a.name + '.backup'])); });
        if (!steps.length) throw bad(409, dev + ' adds no usable capacity to this volume');
        follow.push(step('Grow the physical volumes', ['pvresize'].concat(joined.map(([a]) => a.device))), step('Extend the logical volume into the new space', ['lvextend', '-l', '+100%FREE', '/dev/vg_' + vol + '/' + vol]));
        warn.push('Reshaping runs for hours; the volume stays usable but slower. Do not remove disks meanwhile.');
        info = {volume: vol, joined: joined.map(([a, p]) => [a.name, p, dev]), after_reshape: true}; title = 'Add ' + dev + ' to volume ' + vol;
      } else {
        const old = b.old, oldk = (old || '').replace('/dev/', ''), mine = arrs.filter(a => a.members.some(m => m.disk === oldk));
        if (!mine.length) throw bad(409, old + ' is not part of volume ' + vol);
        steps.push(step('Wipe ' + dev, ['wipefs', '-a', dev]), step('Create a GPT on ' + dev, ['parted', '-s', dev, 'mklabel', 'gpt']));
        mine.forEach((a, k) => { const mb = a.members.find(m => m.disk === oldk); steps.push(step('Create partition ' + a.label + ' on ' + dev, ['parted', '-s', '-a', 'optimal', dev, 'mkpart', a.label, '1MiB', '1MiB']), step('Add ' + dev + (k + 1) + ' to ' + a.name + ' as a spare', mdadmM(a, '--add', dev + (k + 1))), step('Replace ' + mb.device + ' by ' + dev + (k + 1) + ' in ' + a.name, mdadmM(a, '--replace', mb.device, '--with', dev + (k + 1)))); follow.push(step('When ' + a.name + ' has finished: remove ' + mb.device, mdadmM(a, '--remove', mb.device))); });
        warn.push('Rebuilding takes hours; keep the other disks healthy until it finishes.');
        info = {volume: vol, old, new: dev, _mine: mine.map(a => a.name)}; title = 'Replace ' + old + ' by ' + dev + ' in ' + vol;
      }
      confirm = vol;
    } else {
      arr = arrByName(b.array);
      const a = arr, dev = b.device;
      if (op === 'add-spare') {
        freeDev(dev, memberMin(a)); steps = [step('Add ' + dev + ' to ' + a.name + ' as a spare', mdadmM(a, '--add', dev))]; warn.push(dev + ' will be overwritten.'); confirm = a.name; title = 'Add spare to ' + a.name;
      } else if (op === 'fail' || op === 'remove') {
        const m = a.members.find(x => x.device === dev); if (!m) throw bad(400, dev + ' is not a member of ' + a.name);
        if (op === 'fail') { need(a, 'failing a disk'); if (['faulty', 'removed'].includes(m.state)) throw bad(409, dev + ' has already failed; remove it instead'); if (m.state === 'active' && a.members.filter(x => x.state === 'active').length - 1 < minActive(a.level, a.raid_devices)) throw bad(409, 'failing ' + dev + ' would make ' + a.name + ' unreadable'); steps = [step('Mark ' + dev + ' as failed in ' + a.name, mdadmM(a, '--fail', dev))]; title = 'Fail ' + dev + ' in ' + a.name; }
        else { if (['active', 'rebuilding'].includes(m.state)) throw bad(409, dev + ' is working; fail it first'); steps = [step('Remove ' + dev + ' from ' + a.name, mdadmM(a, '--remove', dev))]; title = 'Remove ' + dev + ' from ' + a.name; }
        confirm = a.name;
      } else if (op === 'replace') {
        const old = b.old, nw = b.new, m = a.members.find(x => x.device === old); if (!m) throw bad(400, old + ' is not a member of ' + a.name);
        need(a, 'replacing a disk'); if (a.level === 'raid0') throw bad(409, 'RAID0 has no redundancy to replace from'); freeDev(nw, memberMin(a));
        if (m.state === 'faulty') { steps = [step('Remove ' + old + ' from ' + a.name, mdadmM(a, '--remove', old)), step('Add ' + nw + ' to ' + a.name, mdadmM(a, '--add', nw))]; warn.push('The new disk is rebuilt from the others; the array is exposed until it finishes.'); info.mode = 'failed'; }
        else { if (a.degraded) throw bad(409, a.name + ' is degraded; replace the failed disk first'); steps = [step('Add ' + nw + ' to ' + a.name + ' as a spare', mdadmM(a, '--add', nw)), step('Copy ' + old + ' onto ' + nw, mdadmM(a, '--replace', old, '--with', nw))]; follow.push(step('When the copy has finished: remove ' + old, mdadmM(a, '--remove', old))); warn.push('Redundancy is kept during the copy. ' + old + ' becomes a failed member when it is done.'); info.mode = 'healthy'; }
        confirm = a.name; title = 'Replace ' + old + ' in ' + a.name; info.old = old; info.nw = nw;
      } else if (op === 'grow') {
        need(a, 'growing'); if (a.degraded) throw bad(409, a.name + ' is degraded; rebuild it before growing'); if (!['raid1', 'raid5', 'raid6'].includes(a.level)) throw bad(409, a.level + ' cannot be grown');
        const add = devsOf(b.add); if (add.length && b.size_max) throw bad(400, 'add disks and use all space in two separate steps'); if (!add.length && !b.size_max && !a.spares.length) throw bad(400, 'choose disks to add, or use all space');
        add.forEach(d => freeDev(d, memberMin(a)));
        if (b.size_max) { steps = [step('Use all space on the members of ' + a.name, ['mdadm', '--grow', a.device, '--size=max'])]; warn.push('Only useful after every member was replaced by a larger disk.'); }
        else { const nn = a.raid_devices + add.length; steps = add.map(d => step('Add ' + d + ' to ' + a.name, mdadmM(a, '--add', d))); steps.push(step('Reshape ' + a.name + ' to ' + nn + ' devices', ['mdadm', '--grow', a.device, '--raid-devices=' + nn].concat(a.level === 'raid1' ? [] : ['--backup-file=/var/lib/synofurb-mgmt/md-' + a.name + '.backup']))); warn.push('The reshape runs for hours; the array stays usable but slower.'); }
        follow.push(step('Grow the physical volume on ' + a.name, ['pvresize', a.device])); a.lvs.forEach(l => follow.push(step('Extend ' + l, ['lvextend', '-l', '+100%FREE', '/dev/' + l])));
        confirm = a.name; title = 'Grow ' + a.name; info = {add, after_reshape: true, size_max: !!b.size_max};
      } else if (op === 'change-level') {
        need(a, 'changing the level'); if (a.degraded) throw bad(409, a.name + ' is degraded; rebuild it first');
        const tr = {'raid1>raid5': 0, 'raid5>raid6': 1, 'raid6>raid5': -1, 'raid0>raid5': 1}[a.level + '>' + b.target];
        if (tr === undefined) throw bad(409, 'mdadm cannot convert ' + a.level + ' to ' + b.target + '. Supported: raid0 to raid5; raid1 to raid5; raid5 to raid6; raid6 to raid5');
        const add = devsOf(b.add); if ((tr === 1) && add.length + a.spares.length < 1) throw bad(409, 'add 1 more disk(s) first'); if (a.level === 'raid1' && a.raid_devices !== 2) throw bad(409, 'raid1 to raid5 needs exactly 2 members (this array has ' + a.raid_devices + ')');
        add.forEach(d => freeDev(d, memberMin(a)));
        steps = add.map(d => step('Add ' + d + ' to ' + a.name, mdadmM(a, '--add', d)));
        const nn = a.raid_devices + tr + (a.level === 'raid1' ? add.length : 0);
        steps.push(step('Convert ' + a.name + ' from ' + a.level + ' to ' + b.target, ['mdadm', '--grow', a.device, '--level=' + b.target.slice(4)].concat(nn !== a.raid_devices ? ['--raid-devices=' + nn] : [], ['--backup-file=/var/lib/synofurb-mgmt/md-' + a.name + '.backup'])));
        warn.push('The conversion rewrites all data and runs for hours; do not power off.'); confirm = a.name; title = 'Change ' + a.name + ' to ' + b.target; info = {to: b.target, add, n: nn, after_reshape: true};
      } else if (op === 'scrub') {
        const act = b.action || 'check'; if (!['check', 'repair', 'idle'].includes(act)) throw bad(400, 'action must be one of: check, repair, idle');
        if (a.inactive) throw bad(409, a.name + ' is inactive');
        if (act !== 'idle') { if (a.level === 'raid0') throw bad(409, a.name + ' has no redundancy to check'); need(a, 'starting a check'); if (a.degraded && act === 'repair') throw bad(409, 'repair on a degraded array is refused; rebuild first'); }
        steps = [sysStep(a.name, 'sync_action', act)]; title = (act === 'idle' ? 'Stop the running task on ' : act === 'check' ? 'Check ' : 'Repair ') + a.name; info = {action: act, writes: act === 'repair'};
        if (act === 'repair') warn.push('Repair rewrites blocks that disagree; use it only after a check found mismatches.');
      } else if (op === 'stop') {
        if (a.mounts.length) throw bad(409, a.name + ' is mounted at ' + a.mounts.join(', ')); if (a.holds_system) throw bad(409, a.name + ' holds the system'); if (a.lvs.length) throw bad(409, a.name + ' is in use by ' + a.lvs.join(', ')); if (a.busy) throw bad(409, a.name + ' is ' + a.sync.action + '; stopping now would interrupt it');
        steps = [step('Stop ' + a.name, ['mdadm', '--stop', a.device])]; confirm = a.name; title = 'Stop ' + a.name;
      } else if (op === 'tuning') {
        ['speed_min', 'speed_max'].forEach(k => { const v = b[k]; if (v === undefined || v === null || v === '') return; if (v !== 'system' && !(Number.isInteger(v) && v >= 0 && v <= 4000000)) throw bad(400, k + " must be 'system' or a whole number of KiB/s up to 4000000"); steps.push(sysStep(a.name, 'sync_' + k.replace('_', '_speed_').replace('speed_speed', 'speed').replace('sync_speed_', 'sync_speed_'), v)); });
        steps.forEach(s => { s.cmd = s.cmd.replace('sync_speed_speed_', 'sync_speed_'); });
        if (!steps.length) throw bad(400, 'nothing to change'); title = 'Tune ' + a.name;
      } else throw bad(404, 'unknown RAID operation');
    }
    if (arr && arr.foreign && !(['scrub', 'assemble'].includes(op) && !info.writes)) {
      warn.unshift('This array was not created by synofurb (' + arr.foreign_reasons.join('; ') + '). Changes can make DSM unable to use it.'); confirm = confirm || arr.name; info.needs_foreign_ack = true;
    }
    return {arr, plan: Object.assign({op, title, steps, warnings: warn, confirm, array: arr ? arr.name : null, follow_ups: follow, destructive: confirm !== null}, info)};
  }
  function raidApply(op, p, b) {
    const a = p.arr, pl = p.plan, now = Date.now();
    const mem = (dev, state, extra) => { const c = U.raid.candidates.find(x => x.path === dev); return Object.assign({device: dev, name: dev.replace('/dev/', ''), slot: null, state, write_mostly: false, disk: dev.replace(/^\/dev\//, '').replace(/p?\d+$/, ''), model: c && c.model, serial: c && c.serial, size: c && c.size}, extra || {}); };
    const take = d => { const i = U.raid.candidates.findIndex(c => c.path === d); if (i >= 0) { const c = U.raid.candidates.splice(i, 1)[0]; U.raid.excluded.push(Object.assign(c, {reason: 'already a RAID member'})); } };
    if (op === 'create') {
      const n = 'md' + (st.mdn++), sizes = pl.devs.map(d => (U.raid.candidates.find(c => c.path === d) || {}).size);
      const arr = {name: n, device: '/dev/' + n, label: pl.label, level: pl.level, state: 'clean', flags: [], degraded: false, size: pl.usable_bytes, used_dev_size: Math.min(...sizes), raid_devices: pl.devs.length, chunk_bytes: 524288, layout: null,
        bitmap: pl.level === 'raid0' ? null : 'internal', members: pl.devs.map((d, i) => mem(d, 'active', {slot: i})).concat(pl.spares.map(d => mem(d, 'spare'))), spares: pl.spares.slice(), failed: [], sync: null, busy: false, mismatch_cnt: 0, last_sync_action: null,
        sync_speed_min: 'system', sync_speed_max: 'system', stripe_cache_size: null, uuid: null, events: 1, foreign: false, foreign_reasons: [], vgs: [], lvs: [], mounts: [], in_use: false, holds_system: false, inactive: false};
      pl.devs.concat(pl.spares).forEach(take); U.raid.arrays.push(arr); if (pl.level !== 'raid0') startSync(arr, 'resync', 0.4, null);
    } else if (op === 'shr-create') {
      const name = pl._name;
      pl._slices.forEach(s => {
        const n = 'md' + (st.mdn++), parts = s._parts;
        const arr = {name: n, device: '/dev/' + n, label: name + '_' + s.index, level: s.level, state: 'clean', flags: [], degraded: false, size: s.usable, used_dev_size: s.length, raid_devices: parts.length, chunk_bytes: 524288, layout: null, bitmap: 'internal',
          members: parts.map((p, i) => Object.assign(mem(p, 'active', {slot: i}), {disk: p.replace('/dev/', '').replace(/\d+$/, ''), size: s.length})), spares: [], failed: [], sync: null, busy: false, mismatch_cnt: 0, sync_speed_min: 'system', sync_speed_max: 'system', foreign: false, foreign_reasons: [], vgs: ['vg_' + name], lvs: [], mounts: [], in_use: false, holds_system: false, inactive: false};
        U.raid.arrays.push(arr); startSync(arr, 'resync', 0.3, null);
      });
      Object.keys(b.devices.reduce((o, d) => (o[d] = 1, o), {})).forEach(take); U.raid.shr_volumes.push(name);
    } else if (op === 'add-spare') { a.members.push(mem(b.device, 'spare')); a.spares.push(b.device); take(b.device); if (a.degraded) { const m = a.members.find(x => x.device === b.device); m.state = 'rebuilding'; a.spares = []; startSync(a, 'recovery', 0.5, x => { m.state = 'active'; x.degraded = false; }); } }
    else if (op === 'fail') { const m = a.members.find(x => x.device === b.device); m.state = 'faulty'; a.failed.push(b.device); a.degraded = true; a.state = 'degraded'; }
    else if (op === 'remove') { const m = a.members.find(x => x.device === b.device); m.state = 'removed'; m.device = null; m.name = null; a.failed = a.failed.filter(x => x !== b.device); a.spares = a.spares.filter(x => x !== b.device); }
    else if (op === 'replace') {
      const old = a.members.find(x => x.device === b.old), nm = mem(b.new, 'rebuilding'); take(b.new); a.members.push(nm);
      if (pl.mode === 'failed') { a.members = a.members.filter(x => x !== old || x.state === 'removed'); old.state = 'removed'; old.device = null; old.name = null; }
      startSync(a, 'recovery', 0.5, x => { nm.state = 'active'; if (pl.mode === 'failed') { x.degraded = false; x.failed = []; } else { old.state = 'faulty'; x.failed = [b.old]; } });
    } else if (op === 'grow') {
      if (pl.size_max) { startSync(a, 'reshape', 1, null); }
      else { pl.add.forEach(d => { a.members.push(mem(d, 'active', {slot: a.members.length})); take(d); }); const old = a.raid_devices; a.raid_devices += pl.add.length; startSync(a, 'reshape', 0.4, x => { x.size = Math.round(x.size / (old - 1) * (x.raid_devices - 1)); }); }
    } else if (op === 'change-level') {
      pl.add.forEach(d => { a.members.push(mem(d, 'active', {slot: a.members.length})); take(d); }); startSync(a, 'reshape', 0.4, x => { x.level = pl.to; x.raid_devices = pl.n; });
    } else if (op === 'scrub') {
      if (pl.action === 'idle') { a._sync = null; a.sync = null; a.busy = false; }
      else startSync(a, pl.action, pl.action === 'check' ? 1.2 : 0.6, x => { if (pl.action === 'repair') x.mismatch_cnt = 0; });
    } else if (op === 'stop') { U.raid.arrays = U.raid.arrays.filter(x => x !== a); }
    else if (op === 'tuning') { if (b.speed_min !== undefined && b.speed_min !== null && b.speed_min !== '') a.sync_speed_min = b.speed_min; if (b.speed_max !== undefined && b.speed_max !== null && b.speed_max !== '') a.sync_speed_max = b.speed_max; }
    else if (op === 'shr-add-disk') {
      pl.joined.forEach(([name, part, dev]) => { const x = RA().find(y => y.name === name); x.members.push(Object.assign(mem(part, 'active', {slot: x.members.length}), {disk: dev.replace('/dev/', ''), size: memberMin(x)})); const old = x.raid_devices; x.raid_devices += 1; startSync(x, 'reshape', 0.35, y => { y.size = Math.round(y.size / Math.max(1, old - 1) * (y.raid_devices - 1)); }); });
      take(b.device);
    } else if (op === 'shr-replace-disk') {
      pl._mine.forEach(name => { const x = RA().find(y => y.name === name), old = x.members.find(m => m.disk === pl.old.replace('/dev/', '')); const nm = Object.assign(mem(b.device + '1', 'rebuilding'), {disk: b.device.replace('/dev/', ''), size: old.size}); x.members.push(nm); startSync(x, 'recovery', 0.45, () => { nm.state = 'active'; old.state = 'faulty'; }); });
      take(b.device);
    }
    U.raid.arrays.forEach(x => { x.state = x.sync ? x.state : (x.degraded ? 'degraded' : 'clean'); x.spares = x.members.filter(m => m.state === 'spare').map(m => m.device); x.failed = x.members.filter(m => m.state === 'faulty').map(m => m.device); });
    U.volumes.candidates = U.volumes.candidates.filter(c => U.raid.candidates.some(r => r.path === c.path));
  }
  function raidRun(op, b, execute) {
    const p = raidPlan(op, b);
    if (!execute) return Object.assign({ok: true}, p.plan);
    if (p.plan.confirm) needConfirm(b, p.plan.confirm);
    if (p.plan.needs_foreign_ack && b.foreign_ack !== true) throw bad(400, "this array was not created here: tick 'I understand this is not mine' to change it");
    raidApply(op, p, b);
    const results = p.plan.steps.map(s => ({desc: s.desc, cmd: s.cmd, rc: 0, out: s.cmd.startsWith('mdadm --create') ? 'mdadm: array started.' : s.cmd.startsWith('mdadm --manage') ? 'mdadm: hot added.' : '', err: ''}));
    return {ok: true, op, results, follow_ups: p.plan.follow_ups, title: p.plan.title};
  }
  R['GET /api/raid'] = () => { U.raid.arrays.forEach(tickSync); const o = clone(U.raid); o.arrays.forEach(a => { delete a._sync; }); return o; };
  R['POST /api/raid/plan'] = b => { if (typeof b.op !== 'string') throw bad(400, 'op is required'); return raidRun(b.op, b, false); };
  ['create', 'add-spare', 'fail', 'remove', 'replace', 'grow', 'change-level', 'scrub', 'stop', 'assemble', 'tuning', 'shr-create', 'shr-add-disk', 'shr-replace-disk'].forEach(op => {
    R['POST /api/raid/' + op] = b => {
      if (op === 'tuning' && b.scrub_schedule && !b.array) { U.raid.scrub_timer.enabled = b.scrub_schedule === 'enable' ? 'enabled' : 'disabled'; U.raid.scrub_timer.installed = true; return {ok: true, op, results: raidRun(op, b, false).steps.map(s => ({desc: s.desc, cmd: s.cmd, rc: 0, out: '', err: ''})), follow_ups: []}; }
      return raidRun(op, b, true);
    };
  });
  R['POST /api/raid/after-grow'] = b => { const a = arrByName(b.array); need(a, 'extending the volume'); needConfirm(b, a.name); return {ok: true, results: [{desc: 'Grow the physical volume on ' + a.name, cmd: 'pvresize ' + a.device, rc: 0, out: '', err: ''}].concat(a.lvs.map(l => ({desc: 'Extend ' + l, cmd: 'lvextend -l +100%FREE /dev/' + l, rc: 0, out: '', err: ''})))}; };


  /* ---------- updates of the management plane itself (?update=available|current|offline|rollback|busy|unhealthy) ---------- */
  const UM = qs.get('update') || 'available';
  const NEWV = (v => v[0] + '.' + (Number(v[1]) + 1) + '.0')(String(DATA.version).split('.'));
  const SHA = '3f9c2a71b8e4d05a6c7e1f20944b8d3ac51e7f60';
  const nowS = () => Date.now() / 1000;
  const upd = {installed: DATA.version, previous: '0.4.0', channel: 'stable', ref: 'main', check: null, down: 0,
    jobs: [{id: 'a1b2c3d4e5f60718', kind: 'update', source: 'app', ref: '', commit: '', by: 'andreas', via: 'cli', from: '0.4.0', to: DATA.version, state: 'ok', exit: 0, started: nowS() - 3 * 86400, finished: nowS() - 3 * 86400 + 38, unsigned: false, step: 'result', message: 'updated 0.4.0 -> ' + DATA.version},
           {id: '0f1e2d3c4b5a6978', kind: 'update', source: 'github-stable', ref: '', commit: '', by: 'admin', via: 'web', from: '0.3.1', to: '0.4.0', state: 'rolled_back', exit: 3, started: nowS() - 9 * 86400, finished: nowS() - 9 * 86400 + 61, unsigned: true, step: 'result', message: 'update to 0.4.0 failed; rolled back'}]};
  if (UM === 'busy') upd.busy = 'addon';
  function updCmp(c) { const a = c.version.split('.').map(Number), b = upd.installed.split('.').map(Number); for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i] ? 'available' : 'older'; return c.channel === 'main' ? 'same' : 'current'; }
  function updState() {
    const c = upd.check ? Object.assign({}, upd.check, {installed: upd.installed, state: updCmp(upd.check)}) : null;
    const act = upd.jobs.find(j => j.state === 'running');
    return {ok: true, installed: upd.installed, sha256: 'demo', previous: upd.previous, channel: upd.channel, ref: upd.ref, check: c, active: act ? act.id : null, busy: upd.busy || null, jobs: clone(upd.jobs), signed: false, repo: 'andreaswiren/synofurb'};
  }
  R['GET /api/update'] = () => updState();
  R['POST /api/update/check'] = b => {
    if (UM === 'offline') throw bad(502, 'Cannot reach GitHub (gaierror)');
    if (!['stable', 'main'].includes(b.channel)) throw bad(400, 'channel must be stable or main');
    upd.channel = b.channel; upd.ref = b.ref || upd.ref;
    const main = b.channel === 'main', v = UM === 'current' && !main ? upd.installed : NEWV;
    upd.check = {channel: b.channel, version: v, tag: main ? '' : 'v0.3.0', asset: main ? '' : 'synofurb-mgmt-' + v + '.tar.gz', ref: main ? (b.ref || 'main') : '', commit: main ? SHA : '', published: '2026-05-12', mode: 'unsigned', blocked: null,
      notes: main ? '• 3f9c2a7 Updates page: check, update and roll back from the web UI (2026-05-12)\n• 81d0c4e Keep the previous version after an update (2026-05-09)' : 'Management plane ' + v + '\n• Update from GitHub inside the web UI\n• Version history shows who updated and how',
      checked_at: Math.floor(nowS()), repo: 'andreaswiren/synofurb'};
    return {ok: true, check: updState().check};
  };
  function updJob(id) { const j = upd.jobs.find(x => x.id === id); if (!j) throw bad(404, 'no such job'); return j; }
  function updRun(j, rollbackJob, force) {
    const to = j.to, fail = UM === 'rollback' && !rollbackJob, refuse = UM === 'unhealthy' && !rollbackJob && !force;
    const plan = refuse ? [['check', 'start', 'asking GitHub'], ['check', 'ok', 'release ' + to + ', available (installed ' + j.from + ')'], ['health', 'fail', 'the unit is in a critical state (array md1 degraded); an update needs force']]
      : rollbackJob ? [['rollback', 'start', 'restoring /opt/synofurb-mgmt.prev'], ['restart', 'start', 'restarting synofurb-mgmt.service'], ['rollback', 'ok', 'version ' + to + ' is running'], ['result', 'ok', 'rolled back to ' + to]]
      : [['check', 'start', 'asking GitHub'], ['check', 'ok', 'release ' + to + ', available (installed ' + j.from + ')'], ['download', 'start', 'downloading'],
         ['download', 'ok', 'synofurb-mgmt-' + to + '.tar.gz 212 KiB, SHA-256 9f2c1a7b4e0d... matches SHA256SUMS (unsigned)'], ['verify', 'start', 'checking'], ['verify', 'ok', 'manifest verified, version ' + to],
         ['stage', 'start', 'copying'], ['stage', 'ok', '4 files staged'], ['swap', 'start', 'swapping'], ['swap', 'ok', 'previous version kept'], ['restart', 'start', 'restarting synofurb-mgmt.service'],
         ['restart', 'ok', 'service restarted'], ['health', 'start', 'waiting up to 30 s for the new version']]
      .concat(fail ? [['health', 'fail', 'health check failed (no valid answer)'], ['rollback', 'start', 'restoring the previous version'], ['rollback', 'ok', 'previous version ' + j.from + ' is running again'], ['result', 'fail', 'update to ' + to + ' failed; rolled back']]
                   : [['health', 'ok', 'version ' + to + ' answers'], ['result', 'ok', 'updated ' + j.from + ' -> ' + to]]);
    j.events = []; j.state = 'running';
    plan.forEach(([step, status, message], i) => setTimeout(() => {
      j.events.push({t: Math.floor(nowS()), step, status, message}); j.step = step; j.message = message;
      if (step === 'restart' && status === 'start') upd.down = Date.now() + 3200;
      if (refuse && step === 'health') { j.state = 'refused'; j.exit = 2; j.finished = nowS(); }
      if (step === 'result') {
        j.state = rollbackJob ? 'ok' : fail ? 'rolled_back' : 'ok'; j.exit = j.state === 'ok' ? 0 : 3; j.finished = nowS();
        if (j.state === 'ok') { if (!rollbackJob) upd.previous = upd.installed; else upd.previous = upd.installed; upd.installed = to; }
      }
    }, 700 * (i + 1) + (step === 'restart' && status === 'ok' ? 2400 : 0)));
  }
  function updStart(src, to, extra) {
    if (upd.busy || upd.jobs.some(j => j.state === 'running')) throw bad(409, 'another job is running (' + (upd.busy || 'update') + '): wait for it to finish');
    const j = Object.assign({id: Array.from({length: 16}, () => '0123456789abcdef'[Math.floor(Math.random() * 16)]).join(''), kind: 'update', source: src, ref: '', commit: '', by: st.user, via: 'web', from: upd.installed, to, state: 'queued', exit: null,
      started: nowS(), finished: null, unsigned: true, step: null, message: null, events: []}, extra || {});
    upd.jobs.unshift(j);
    return j;
  }
  R['POST /api/update/start'] = b => {
    if (!['stable', 'main'].includes(b.channel)) throw bad(400, 'channel must be stable or main');
    if (typeof b.expect !== 'string' || !b.expect) throw bad(400, 'expect must be the version or commit that was shown');
    if (b.channel === 'main') needConfirm(b, b.expect.slice(0, 7));
    const j = updStart('github-' + b.channel, upd.check ? upd.check.version : NEWV, b.channel === 'main' ? {ref: b.ref || 'main', commit: b.expect} : {});
    updRun(j, false, b.force === true);
    return {ok: true, job: j.id};
  };
  R['POST /api/update/rollback'] = () => {
    if (!upd.previous) throw bad(409, 'no previous version is kept');
    const j = updStart('rollback', upd.previous, {kind: 'rollback', unsigned: false});
    updRun(j, true);
    return {ok: true, job: j.id};
  };
  R['POST /api/update/job'] = b => {
    if (Date.now() < upd.down) throw bad(401, 'authentication required');
    const j = updJob(b.job), o = clone(j); delete o.events;
    return {ok: true, job: o, events: clone(j.events || [])};
  };
  R['POST /api/update/progress'] = b => {
    const j = updJob(b.job);
    return {ok: true, job: {id: j.id, kind: j.kind, state: j.state, step: j.step, from: j.from, to: j.to, finished: j.finished}};
  };
  PUBLIC.add('POST /api/update/progress');

  /* ---------- add-ons: the list and details come from the real handlers; jobs are simulated with timers ---------- */
  const AD = U.addons, ADJ = {};
  const adRow = id => AD.list.addons.find(x => x.id === id);
  const adDet = id => AD.detail[id];
  function adSync(id) {
    const r = adRow(id), d = adDet(id);
    for (const o of [r, d]) { o.installed = st.ad[id].installed; o.status = st.ad[id].status; o.installed_version = st.ad[id].installed ? '1' : ''; o.update_available = false; o.detail = st.ad[id].detail || ''; o.link = o.installed && o.web ? {scheme: o.web.scheme, port: o.web.port, path: o.web.path, hint: o.web.hostname_hint} : null; }
    d.values = st.ad[id].values || d.values;
  }
  st.ad = {};
  AD.list.addons.forEach(a => { st.ad[a.id] = {installed: a.installed, status: a.status, detail: '', values: null}; });
  /* ?addons=states|problems|nodocker: other states of the page (default: Docker and Portainer running, Frigate and Plex available) */
  const ADS = qs.get('addons') || '';
  const adPreset = {
    states: {docker: ['running'], portainer: ['stopped'], plex: ['needs-attention', 'No space left on device while downloading the images. Free some space and retry.'], frigate: ['updating']},
    problems: {docker: ['running'], portainer: ['unhealthy', 'The health check at https://nas:9443 does not answer.'], plex: ['error', 'The container plex keeps stopping; the last log line says: permission denied on /data.'], frigate: ['starting']},
    nodocker: {docker: ['not-installed'], portainer: ['not-installed'], plex: ['not-installed'], frigate: ['not-installed']},
  }[ADS];
  if (adPreset) for (const [id, [status, detail]] of Object.entries(adPreset)) { st.ad[id] = {installed: status !== 'not-installed', status, detail: detail || '', values: null}; }
  function adEffPrivs(d, settings) {
    const out = d.privileges.map(p => p.id);
    d.settings.filter(s => s.type === 'choice').forEach(s => { const c = s.choices.find(x => x.value === (settings[s.key] || d.values[s.key])); if (c) c.privileges.forEach(p => { if (!out.includes(p)) out.push(p); }); });
    return out;
  }
  function adJobView(j, full) {
    const el = (Date.now() - j.t0) / 1000, n = j.steps.length, per = j.dur / n, idx = Math.min(n - 1, Math.floor(el / per)), done = el >= j.dur;
    if (done && j.state === 'running') {
      j.state = 'done'; j.ok = true; st.ad[j.addon].status = j.final.status; st.ad[j.addon].installed = j.final.installed; st.ad[j.addon].detail = '';
      if (j.final.values) st.ad[j.addon].values = j.final.values;
      adSync(j.addon);
    }
    const v = {id: j.id, kind: j.kind, addon: j.addon, state: j.state, step: j.steps[idx], steps: j.steps, progress: done ? 100 : Math.round(100 * el / j.dur), message: done ? '' : j.msgs[idx], ok: j.ok, error: '', started: j.t0 / 1000, finished: done ? Date.now() / 1000 : null, warnings: []};
    if (full) v.log = j.msgs.slice(0, done ? n : idx + 1).map(x => '== ' + x).concat(done ? ['Done'] : []);
    return v;
  }
  function adStart(id, kind, final, steps, msgs) {
    const act = Object.values(ADJ).find(j => j.state === 'running');
    if (act) throw bad(409, 'Another job is running (' + act.kind + ' ' + act.addon + '). Wait until it ends.');
    const jid = 'j' + Math.random().toString(16).slice(2, 14).padEnd(12, '0');
    ADJ[jid] = {id: jid, kind, addon: id, state: 'running', ok: null, t0: Date.now(), dur: kind === 'install' || kind === 'update' ? 5 : 2.4, steps, msgs, final};
    st.ad[id].status = {install: 'installing', update: 'updating', remove: 'removing'}[kind] || st.ad[id].status; adSync(id);
    return {ok: true, job: adJobView(ADJ[jid], false)};
  }
  function adAction(id, action, b) {
    const d = adDet(id), s = st.ad[id];
    if (!d) throw bad(404, "unknown add-on '" + id + "'");
    if (action === 'install') {
      if (s.installed && s.status !== 'needs-attention') throw bad(400, d.name + ' is already installed.');
      const miss = d.requires.filter(r => !st.ad[r].installed);
      if (miss.length) throw bad(400, d.name + ' needs ' + miss.map(r => adDet(r).name).join(', ') + ' to be installed first.');
      const set = b.settings || {};
      for (const k of Object.keys(set)) if (!d.settings.some(x => x.key === k)) throw bad(400, "unknown setting '" + k + "'");
      for (const x of d.settings) {
        const v = set[x.key];
        if (x.required && (v === '' || (v == null && !d.values[x.key]))) throw bad(400, x.label + ' is required');
        if (x.type === 'int' && v !== '' && v != null && (!Number.isInteger(Number(v)) || Number(v) < x.min || Number(v) > x.max)) throw bad(400, x.label + ' must be between ' + x.min + ' and ' + x.max);
      }
      const privs = adEffPrivs(d, set);
      if (privs.length && b.accept !== true) throw bad(400, d.name + ' needs these privileges: ' + privs.join(', ') + '. Confirm them to continue.');
      const vals = Object.assign({}, d.values);
      d.settings.forEach(x => { if (x.type === 'secret') vals[x.key] = {set: !!set[x.key] || (vals[x.key] && vals[x.key].set)}; else if (set[x.key] != null) vals[x.key] = set[x.key]; });
      return adStart(id, 'install', {status: 'running', installed: true, values: vals}, ['check', 'render', 'pull', 'start', 'health'],
        ['Checking Docker', 'Writing the configuration', 'Downloading images (needs Internet)', 'Starting', 'Waiting for ' + d.name + ' to answer']);
    }
    if (!s.installed) throw bad(400, d.name + ' is not installed.');
    if (action === 'update') return adStart(id, 'update', {status: 'running', installed: true}, ['check', 'render', 'pull', 'start', 'health'], ['Checking Docker', 'Writing the configuration', 'Downloading images (needs Internet)', 'Starting', 'Waiting for ' + d.name + ' to answer']);
    if (action === 'remove') {
      if (d.kind === 'system') throw bad(400, d.name + ' was installed with apt and stays on this unit.');
      const users = AD.list.addons.filter(x => x.requires.includes(id) && st.ad[x.id].installed);
      if (users.length) throw bad(400, d.name + ' is used by ' + users.map(x => x.id).join(', ') + '. Remove those first.');
      if (b.delete_data === true && b.confirm !== id) throw bad(400, 'to delete the data, type the add-on name ' + JSON.stringify(id).replace(/"/g, "'") + ' to confirm');
      return adStart(id, 'remove', {status: 'not-installed', installed: false}, ['stop', 'files', 'forget'], ['Stopping and removing containers', b.delete_data ? 'Deleting data' : 'Removing the compose file', 'Forgetting the settings']);
    }
    if (action === 'stop' && d.kind === 'system' && AD.list.addons.some(x => x.requires.includes(id) && st.ad[x.id].installed)) throw bad(400, d.name + ' is used by other add-ons. Remove or stop those first.');
    return adStart(id, action, {status: action === 'stop' ? 'stopped' : 'running', installed: true}, [action], [{start: 'Starting', stop: 'Stopping', restart: 'Restarting'}[action]]);
  }
  R['GET /api/addons'] = () => { const o = clone(AD.list); const act = Object.values(ADJ).find(j => j.state === 'running'); o.job = act ? adJobView(act, false) : null; return o; };
  R['GET /api/jobs'] = () => ({ok: true, jobs: Object.values(ADJ).map(j => adJobView(j, false))});
  const DYN = [
    [/^GET \/api\/addons\/([a-z0-9-]+)$/, m => { const d = adDet(m[1]); if (!d) throw bad(404, "unknown add-on '" + m[1] + "'"); return {ok: true, addon: clone(d)}; }],
    [/^GET \/api\/addons\/([a-z0-9-]+)\/logs$/, m => ({ok: true, logs: '2026-11-03T08:00:01Z ' + m[1] + ' | starting\n2026-11-03T08:00:03Z ' + m[1] + ' | listening\n2026-11-03T08:00:09Z ' + m[1] + ' | ready\n'})],
    [/^GET \/api\/jobs\/(j[0-9a-f]{12})$/, m => { const j = ADJ[m[1]]; if (!j) throw bad(404, 'no such job'); return {ok: true, job: adJobView(j, true)}; }],
    [/^POST \/api\/addons\/([a-z0-9-]+)\/(install|start|stop|restart|update|remove)$/, (m, b) => adAction(m[1], m[2], b)],
  ];
  function dynRoute(route) { for (const [rx, fn] of DYN) { const m = rx.exec(route); if (m) return b => fn(m, b); } return null; }
  AD.list.addons.forEach(a => adSync(a.id));
  if (ADS === 'states') {                                           // Frigate is being updated: a job that outlasts the screenshot
    const jid = 'jabcdef012345';
    ADJ[jid] = {id: jid, kind: 'update', addon: 'frigate', state: 'running', ok: null, t0: Date.now() - 2200000, dur: 3600, steps: ['check', 'render', 'pull', 'start', 'health'],
      msgs: ['Checking Docker', 'Writing the configuration', 'Downloading images (needs Internet)', 'Starting', 'Waiting for Frigate NVR to answer'], final: {status: 'running', installed: true}};
  }

/*__EDGE_MOCK__*/
  window.fetch = async function (url, opt) {
    const u = new URL(typeof url === 'string' ? url : url.url, location.href);
    if (!/\/api\//.test(u.pathname)) return origFetch ? origFetch(url, opt) : Promise.reject(new Error('offline demo'));
    const method = ((opt && opt.method) || 'GET').toUpperCase();
    const route = method + ' ' + u.pathname.slice(u.pathname.indexOf('/api/'));
    await sleep(route.includes('volumes/create') ? 900 : 90 + Math.random() * 90);
    const fn = R[route] || dynRoute(route);
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
