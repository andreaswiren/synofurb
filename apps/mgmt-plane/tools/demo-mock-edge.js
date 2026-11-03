  /* ---------- certificates and the HAProxy front (inlined into demo-mock.js by build-demo.py; simulated, nothing leaves this page) ---------- */
  /* ?certs=empty|unset|states (default: two certificates)   ?proxy=on|pending (default: the front is off) */
  const CEQ = qs.get('certs') || '', PXQ = qs.get('proxy') || '';
  const CE_DAY = 86400, ceNow = () => Math.floor(Date.now() / 1000);
  const CE = {
    settings: {email: CEQ === 'unset' ? '' : 'admin@example.org', tos_accepted: CEQ === 'unset' ? null : ceNow() - 5 * CE_DAY, environment: 'production', key_type: 'ec256', dns_skip_check: false, dns_resolvers: '',
      addr_tools_url: 'https://challenges.addr.tools', addr_tools_ca: '', terms_url: 'https://letsencrypt.org/repository/', secret: {set: CEQ !== 'unset', target: CEQ === 'unset' ? null : '8d4e7a1c0b9f3a62d5c8e1f7a0b3c6d9e2f5a8b1c4d7e0f3a6b9c2d5.challenges.addr.tools', length: 43}},
    certs: [], jobs: {},
  };
  const CE_TARGET = '8d4e7a1c0b9f3a62d5c8e1f7a0b3c6d9e2f5a8b1c4d7e0f3a6b9c2d5.challenges.addr.tools';
  const ceMake = (primary, names, ch, left, o) => Object.assign({id: primary.replace(/^\*\./, '_wildcard.'), primary, names, challenge: ch, environment: 'production', key_type: 'ec256', created: ceNow() - 60 * CE_DAY,
    renewed: null, not_before: ceNow() - (90 - left) * CE_DAY, expires: ceNow() + left * CE_DAY, fingerprint: 'B6:53:18:2F:BF:3E:D6:EE:DD:F1:ED:E3:BA:10:CB:3B:1E:7A:04:06:29:27:A1:53:42:F9:16:5D:08:4D:F7:25',
    last_ok: ceNow() - (90 - left) * CE_DAY, last_attempt: ceNow() - (90 - left) * CE_DAY, last_error: '', days_left: left, status: left <= 14 ? 'expiring' : 'valid', has_cert: true, used_by: [], cnames: []}, o || {});
  if (CEQ !== 'empty') {
    CE.certs.push(ceMake('nas.example.org', ['nas.example.org', '*.example.org'], 'dns-01', 62), ceMake('media.example.org', ['media.example.org'], 'http-01', 11));
    if (CEQ === 'states') {
      CE.certs.push(ceMake('old.example.org', ['old.example.org'], 'tls-alpn-01', -3, {status: 'expired', last_error: 'Let\'s Encrypt could not connect. Is port 80 (HTTP-01) or 443 (TLS-ALPN-01) open from the Internet and forwarded to this unit? (Timeout during connect)'}),
        ceMake('new.example.org', ['new.example.org'], 'dns-01', 0, {status: 'failed', expires: null, not_before: null, fingerprint: null, days_left: null, has_cert: false, last_error: 'Create the CNAME record first: _acme-challenge.new.example.org should point at the challenge service (see the target in the dialog).'}),
        ceMake('stage.example.org', ['stage.example.org'], 'dns-01', 80, {environment: 'staging'}));
    }
  }
  const ceFix = c => { c.cnames = c.challenge === 'dns-01' ? [...new Set(c.names.map(n => n.replace(/^\*\./, '')))].map(b => ({name: b, record: '_acme-challenge.' + b, target: CE.settings.secret.target})) : []; return c; };
  const ceJobs = {}, ceCert = id => CE.certs.find(c => c.id === id);
  function ceJobView(j, full) {
    const el = (Date.now() - j.t0) / 1000, n = j.steps.length, per = j.dur / n, idx = Math.min(n - 1, Math.floor(el / per)), done = el >= j.dur;
    if (done && j.state === 'running') { j.state = 'done'; j.ok = true; if (j.final) j.final(); }
    const v = {id: j.id, kind: j.kind, subject: j.subject, state: j.state, step: j.steps[idx], steps: j.steps, progress: done ? 100 : Math.round(100 * el / j.dur), message: done ? '' : j.msgs[idx], ok: j.ok, error: '',
      started: j.t0 / 1000, finished: done ? Date.now() / 1000 : null, warnings: [], result: done ? (j.result || {}) : {}};
    if (full) v.log = j.msgs.slice(0, done ? n : idx + 1).map(x => '== ' + x).concat(done ? ['Done'] : []);
    return v;
  }
  function ceStart(kind, subject, steps, msgs, final, result, dur) {
    const act = Object.values(ceJobs).find(j => j.state === 'running');
    if (act) throw bad(409, 'Another job is running (certs ' + act.subject + ')');
    const jid = 'e' + Math.random().toString(16).slice(2, 14).padEnd(12, '0');
    ceJobs[jid] = {id: jid, kind, subject, state: 'running', ok: null, t0: Date.now(), dur: dur || 4.5, steps, msgs, final, result};
    return {ok: true, job: ceJobView(ceJobs[jid], false)};
  }
  const CE_STEPS = ['check', 'tools', 'challenge', 'obtain', 'verify', 'store', 'publish'];
  const CE_MSGS = ['Checking the request', 'Checking lego', 'Preparing the challenge', 'Asking Let\'s Encrypt', 'Checking the new certificate', 'Storing it', 'Publishing the certificate'];
  const reDom = /^(\*\.)?([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,}$/;
  function ceNames(b) {
    const names = (Array.isArray(b.names) ? b.names : String(b.names || '').split(/[,\s]+/)).map(x => String(x).trim().toLowerCase()).filter(Boolean);
    if (!names.length) throw bad(400, 'give at least one name');
    for (const n of names) { if (!reDom.test(n)) throw bad(400, 'not a valid name: ' + JSON.stringify(n).replace(/"/g, "'")); if (n.startsWith('*.') && b.challenge !== 'dns-01') throw bad(400, 'a wildcard (' + n + ') needs the DNS challenge'); }
    return [...new Set(names)];
  }
  function ceNeedAccount() { if (!CE.settings.email || !CE.settings.tos_accepted) throw bad(400, 'Set your e-mail address and accept the Let\'s Encrypt terms first (Settings).'); }
  R['GET /api/certs'] = () => {
    const act = Object.values(ceJobs).find(j => j.state === 'running');
    const certs = CE.certs.map(c => { const o = clone(ceFix(c)); o.used_by = pxUsedBy(c.id); if (act && (act.subject === c.id || act.subject === '*')) o.status = 'working'; return o; });
    return {ok: true, certs, settings: clone(CE.settings), tools: {lego: {installed: true}, haproxy: {installed: PX.installed}}, job: act ? ceJobView(act, false) : null,
      renew: {ts: ceNow() - 8 * 3600, ok: true, renewed: 0, failed: 0, results: []}, timer: {installed: true, enabled: true}, proxy: {enabled: PX.enabled, pending: pxPending()}};
  };
  R['POST /api/certs/settings'] = b => {
    const s = CE.settings;
    if ('email' in b) { if (b.email && !/^[^@\s]+@[^@\s]+\.[a-z]{2,}$/i.test(b.email)) throw bad(400, 'that does not look like an email address'); s.email = b.email; }
    if ('accept_tos' in b) s.tos_accepted = b.accept_tos ? ceNow() : null;
    if (b.environment) s.environment = b.environment;
    if (b.key_type) s.key_type = b.key_type;
    let once = null;
    if (b.generate_secret) once = 'Qm3-v8Zk1xT0aYb7LcR2nPd9sWe4HfJ6uGiAoKtXz5E';
    else if (b.secret) { if (String(b.secret).length < 16) throw bad(400, 'the secret needs 16 to 128 characters'); once = b.secret; }
    if (once) s.secret = {set: true, target: CE_TARGET, length: once.length};
    const o = clone(s); if (once) o.secret_once = once; o.ok = true; return o;
  };
  R['POST /api/certs/issue'] = b => {
    ceNeedAccount();
    const names = ceNames(b), id = names[0].replace(/^\*\./, '_wildcard.');
    if (ceCert(id)) throw bad(400, id + ' exists already: use Change or Renew');
    if (b.challenge === 'dns-01' && !CE.settings.secret.set) throw bad(400, 'Set the challenge secret first (Settings), then create the CNAME record.');
    return ceStart('issue', id, CE_STEPS, CE_MSGS, () => { CE.certs.push(ceMake(names[0], names, b.challenge, 89, {environment: b.environment || 'production', renewed: null})); }, {id});
  };
  R['POST /api/certs/precheck'] = b => {
    const names = ceNames(b);
    if (b.challenge === 'dns-01') return {ok: true, ok_all: true, challenge: b.challenge, names: names.map(n => ({name: n, state: 'ok', message: 'points at the challenge service'})), ports: []};
    const port = b.challenge === 'http-01' ? 80 : 443;
    return {ok: true, ok_all: false, challenge: b.challenge, names: names.map((n, i) => ({name: n, state: 'ok', addresses: ['192.168.1.40'], local: true, private: i === 0, message: 'resolves to this unit'})),
      ports: [{port, front: PX.enabled, busy: false, state: 'ok', message: PX.enabled ? 'answered through the HAProxy front' : 'free on this unit'}], note: 'reachability from the Internet cannot be tested from here'};
  };
  R['POST /api/certs/check-dns'] = b => {
    const n = String(b.name || ''), rec = '_acme-challenge.' + n.replace(/^\*\./, '');
    if (!CE.settings.secret.set) return {ok: true, name: n, record: rec, state: 'nosecret', target: null, found: null, message: 'no secret is set'};
    if (/^media|^new/.test(n)) return {ok: true, name: n, record: rec, state: 'missing', target: CE_TARGET, found: null, message: 'no CNAME record found'};
    if (/^old/.test(n)) return {ok: true, name: n, record: rec, state: 'wrong', target: CE_TARGET, found: 'ab12cd34.challenges.addr.tools', message: 'points somewhere else'};
    return {ok: true, name: n, record: rec, state: 'ok', target: CE_TARGET, found: CE_TARGET, message: 'points at the challenge service'};
  };
  R['POST /api/certs/check-secret'] = () => ({ok: true, reached: CE.settings.secret.set, state: CE.settings.secret.set ? 'ok' : 'nosecret', message: 'the service derives the same sub-domain'});
  DYN.push(
    [/^GET \/api\/certs\/job\/(e[0-9a-f]{12})$/, m => { const j = ceJobs[m[1]]; if (!j) throw bad(404, 'no such job'); return {ok: true, job: ceJobView(j, true)}; }],
    [/^GET \/api\/certs\/([a-z0-9_][a-z0-9._-]*)$/, m => { const c = ceCert(m[1]); if (!c) throw bad(404, 'no such certificate'); return {ok: true, cert: clone(ceFix(c))}; }],
    [/^POST \/api\/certs\/([a-z0-9_][a-z0-9._-]*)\/(renew|change|remove)$/, (m, b) => {
      const c = ceCert(m[1]); if (!c) throw bad(404, 'no such certificate');
      if (m[2] === 'remove') {
        if (b.confirm !== c.id) throw bad(400, "type the certificate name '" + c.id + "' to confirm");
        const u = pxUsedBy(c.id); if (u.length) throw bad(400, 'it is used by ' + u.map(x => x.hostname).join(', ') + ': choose another certificate for those hostnames first');
        CE.certs.splice(CE.certs.indexOf(c), 1); return {ok: true, id: c.id};
      }
      ceNeedAccount();
      if (m[2] === 'change') { const names = ceNames(b); if (names[0] !== c.primary) throw bad(400, 'the first name of a certificate cannot change: issue a new certificate instead'); }
      return ceStart(m[2], c.id, CE_STEPS, CE_MSGS, () => { c.expires = ceNow() + 89 * CE_DAY; c.days_left = 89; c.status = 'valid'; c.last_error = ''; if (m[2] === 'change') { c.names = ceNames(b); c.challenge = b.challenge; c.environment = b.environment; } }, {id: c.id});
    }],
  );

  /* the front */
  const PX = {enabled: PXQ === 'on' || PXQ === 'pending', installed: true, pending: PXQ === 'pending' ? {deadline: Date.now() + 95000, kind: 'apply', seconds: 120} : null, hosts: {mgmt: {hostname: PXQ ? 'nas.example.org' : '', cert: PXQ ? 'nas.example.org' : ''}}, sites: {}};
  if (PXQ) PX.sites.portainer = {hostname: 'portainer.example.org', cert: 'nas.example.org', scheme: 'https'};
  function pxUsedBy(cid) {
    const out = [];
    if (PX.hosts.mgmt.hostname && PX.hosts.mgmt.cert === cid) out.push({kind: 'mgmt', id: 'mgmt', hostname: PX.hosts.mgmt.hostname});
    for (const [id, s] of Object.entries(PX.sites)) if (s.hostname && s.cert === cid) out.push({kind: 'addon', id, hostname: s.hostname});
    return out;
  }
  function pxPending() { if (!PX.pending) return null; return {kind: PX.pending.kind, deadline: PX.pending.deadline / 1000, seconds_left: Math.max(0, Math.round((PX.pending.deadline - Date.now()) / 1000)), seconds: PX.pending.seconds}; }
  function pxRows() {
    const rows = [{kind: 'mgmt', id: 'mgmt', name: '', hostname: PX.hosts.mgmt.hostname, cert: PX.hosts.mgmt.cert, hint: '', scheme: 'https', port: 8443}];
    for (const a of AD.list.addons.filter(x => x.installed && x.web && x.kind !== 'system')) {
      const s = PX.sites[a.id] || {hostname: '', cert: '', scheme: ''};
      rows.push({kind: 'addon', id: a.id, name: a.name, icon: a.icon, hostname: s.hostname, cert: s.cert, hint: a.web.hostname_hint, scheme: s.scheme || a.web.scheme, port: a.web.port});
    }
    return rows;
  }
  function pxApplyBody(b) {
    const certs = CE.certs.filter(c => c.expires);
    const check = (host, cert) => {
      if (!host) return '';
      if (!reDom.test(host)) throw bad(400, 'not a valid name: ' + JSON.stringify(host).replace(/"/g, "'"));
      const c = certs.find(x => x.id === cert); if (!c) throw bad(400, 'choose a certificate for ' + host);
      if (!c.names.some(n => n.startsWith('*.') ? host.endsWith(n.slice(1)) && !host.slice(0, host.length - n.length + 1).includes('.') : n === host)) throw bad(400, 'the certificate ' + cert + ' does not cover ' + host);
      return cert;
    };
    const seen = new Set(), hosts = {mgmt: PX.hosts.mgmt}, sites = Object.assign({}, PX.sites);
    if (b.mgmt) hosts.mgmt = {hostname: b.mgmt.hostname || '', cert: check(b.mgmt.hostname || '', b.mgmt.cert)};
    for (const [id, v] of Object.entries(b.sites || {})) sites[id] = {hostname: v.hostname || '', cert: check(v.hostname || '', v.cert), scheme: v.scheme || ''};
    for (const h of [hosts.mgmt.hostname, ...Object.values(sites).map(s => s.hostname)].filter(Boolean)) { if (seen.has(h)) throw bad(400, 'a hostname can be assigned only once'); seen.add(h); }
    return {hosts, sites};
  }
  R['GET /api/proxy'] = () => {
    const act = Object.values(ceJobs).find(j => j.state === 'running');
    return {ok: true, enabled: PX.enabled, rows: pxRows(), certs: CE.certs.filter(c => c.expires).map(c => ({id: c.id, names: c.names, expires: c.expires})),
      haproxy: {installed: PX.installed, version: '3.0.11', active: PX.enabled, enabled: PX.enabled}, pending: pxPending(), urls: PX.enabled ? ['https://nas.example.org/', 'https://192.168.1.40/'] : [], mgmt_port: 8443,
      reverted: null, job: act ? ceJobView(act, false) : null, recovery: 'python3 /opt/synofurb-mgmt/synofurb_mgmt.py proxy disable'};
  };
  R['POST /api/proxy/hostnames'] = b => { if (PX.enabled) throw bad(400, 'The front is on: use Apply, which checks and publishes the change.'); const r = pxApplyBody(b); PX.hosts = r.hosts; PX.sites = r.sites; return {ok: true, rows: pxRows()}; };
  R['POST /api/proxy/test'] = b => { pxApplyBody(b || {}); return {ok: true, ok_check: true, installed: true, output: '', config: '# Managed by synofurb-mgmt (synofurb_edge.py).\nfrontend fe_http\n    bind 0.0.0.0:80\nfrontend fe_tls\n    bind 0.0.0.0:443\n    use_backend be_acme_alpn if { req.ssl_alpn acme-tls/1 }\n'}; };
  const pxJob = (kind, b) => {
    const r = pxApplyBody(b || {});
    const steps = kind === 'disable' ? ['listener', 'stop'] : ['tools', 'config', 'check', 'apply', 'listener', 'confirm'];
    const msgs = kind === 'disable' ? ['Restoring the direct web address', 'Stopping HAProxy'] : ['Checking HAProxy', 'Building the configuration', 'Checking it with haproxy -c', 'Applying it', kind === 'enable' ? 'Moving the management plane behind the front' : 'Saving the hostnames', 'Waiting for you to confirm'];
    return ceStart('proxy-' + kind, 'proxy', steps, msgs, () => {
      PX.hosts = r.hosts; PX.sites = r.sites;
      if (kind === 'disable') { PX.enabled = false; PX.pending = null; } else { PX.enabled = true; PX.pending = {deadline: Date.now() + 120000, kind, seconds: 120}; }
    }, kind === 'disable' ? {kind, restart: true, urls: ['https://192.168.1.40:8443/']} : {kind, seconds: 120, urls: ['https://nas.example.org/', 'https://192.168.1.40/'], restart: kind === 'enable'}, 3.2);
  };
  R['POST /api/proxy/enable'] = b => { if (PX.enabled) throw bad(400, 'The HAProxy front is on already: use Apply.'); return pxJob('enable', b); };
  R['POST /api/proxy/apply'] = b => { if (!PX.enabled) throw bad(400, 'The HAProxy front is off: enable it first.'); return pxJob('apply', b); };
  R['POST /api/proxy/disable'] = () => { if (!PX.enabled) throw bad(400, 'The HAProxy front is off already.'); return pxJob('disable', {}); };
  R['POST /api/proxy/confirm'] = () => { if (!PX.pending) throw bad(400, 'Nothing waits for confirmation: it may have been reverted already.'); PX.pending = null; return {ok: true}; };
  R['POST /api/proxy/revert'] = () => { if (!PX.pending) throw bad(400, 'Nothing to revert.'); PX.pending = null; PX.enabled = false; return {ok: true, problems: []}; };
  { const meBase = R['GET /api/me']; R['GET /api/me'] = () => Object.assign(meBase(), {edge_pending: pxPending()}); }
