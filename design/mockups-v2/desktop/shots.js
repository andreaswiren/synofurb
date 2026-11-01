// usage: node shots.js [filter] [outdir]   -> screenshots 1280x800 light+dark (+960x640 for a few)
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const path = require('path'), fs = require('fs');
const H = __dirname, OUT = process.argv[3] || path.join(H, '..', '..', 'screenshots');
const list = JSON.parse(fs.readFileSync(path.join(H, 'screens.json'), 'utf8'));
const BIG = new Set(['find', 'plan', 'confirm-ready', 'install', 'failed-post']);   // also at Text size Largest (130%) on 960x640
const filt = process.argv[2] && process.argv[2] !== 'all' ? new RegExp(process.argv[2]) : null;
(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const b = await chromium.launch({ executablePath: process.env.CHROME || '/usr/bin/google-chrome', args: ['--no-sandbox'], ignoreDefaultArgs: ['--hide-scrollbars'] });
  for (const s of list) {
    if (filt && !filt.test(s.n)) continue;
    for (const [w, h, suffix, extra] of [[1280, 800, '', ''], [960, 640, '-960', ''], ...(BIG.has(s.n) ? [[960, 640, '-960-130', '&size=largest']] : [])]) for (const th of ['light', 'dark']) {
      const p = await b.newPage({ viewport: { width: w, height: h } });
      const errs = []; p.on('pageerror', e => errs.push(e.message));
      await p.goto(`file://${H}/index.html?bare=1&theme=${th}&${s.q}${extra}#${s.h}`);
      await p.waitForTimeout(350);
      const over = await p.evaluate(() => {const m=document.querySelector('.main'),sb=document.querySelector('.sheet-b');return document.documentElement.scrollWidth>document.documentElement.clientWidth||(m&&m.scrollWidth>m.clientWidth)||(sb&&sb.scrollWidth>sb.clientWidth)});
      await p.screenshot({ path: path.join(OUT, `v2-mockup-desktop-${s.n}${suffix}-${th}.png`) });
      if (errs.length || over) console.log(s.n, suffix, th, errs.join('|'), over ? 'HSCROLL' : '');
      await p.close();
    }
  }
  await b.close();
})();
