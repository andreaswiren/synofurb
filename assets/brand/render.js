// Rasterises the brand SVGs with Chromium (Playwright). usage: node render.js <outdir> svg:name:size ...
const { chromium } = require(process.env.PW || '/opt/node-tools/node_modules/playwright');
const fs = require('fs'), path = require('path');
(async () => {
  const [out, ...jobs] = process.argv.slice(2);
  const b = await chromium.launch({ executablePath: process.env.CHROME || undefined, args: ['--no-sandbox'] });
  const p = await b.newPage();
  for (const j of jobs) {
    const [src, name, size, bg] = j.split(':'); const s = +size;
    const svg = fs.readFileSync(src, 'utf8');
    await p.setViewportSize({ width: s, height: s });
    await p.setContent(`<html><body style="margin:0;background:${bg || 'transparent'}"><div style="width:${s}px;height:${s}px;display:flex;align-items:center;justify-content:center"><div style="width:${s}px;height:${s}px">${svg.replace('<svg ', '<svg width="100%" height="100%" ')}</div></div></body></html>`);
    await p.screenshot({ path: path.join(out, name), omitBackground: !bg, clip: { x: 0, y: 0, width: s, height: s } });
  }
  await b.close();
})();
