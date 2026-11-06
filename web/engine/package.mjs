// web/app -> web/dist: the site as published. The service worker gets the list of every file and a
// version from their contents, so any change to the app reaches browsers as a new offline copy.
import { createHash } from 'node:crypto';
import { cpSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';

const root = new URL('../', import.meta.url).pathname;
const [app, dist] = [join(root, 'app'), join(root, 'dist')];
rmSync(dist, { recursive: true, force: true });
cpSync(app, dist, { recursive: true });
const walk = (d) => readdirSync(d).flatMap((f) => { const p = join(d, f); return statSync(p).isDirectory() ? walk(p) : [p]; });
const files = walk(dist).map((p) => relative(dist, p).split(sep).join('/')).filter((f) => f !== 'sw.js' && !f.endsWith('README.txt')).sort();
if (!files.includes('engine/qalc.wasm')) throw new Error('build the engine first: web/engine/build.sh');
const hash = createHash('sha256');
for (const f of files) hash.update(f).update(readFileSync(join(dist, f)));
const version = hash.digest('hex').slice(0, 16);
const sw = readFileSync(join(dist, 'sw.js'), 'utf8').replace('__VERSION__', version).replace('__FILES__', JSON.stringify(['./', ...files], null, 1));
writeFileSync(join(dist, 'sw.js'), sw);
writeFileSync(join(dist, '.nojekyll'), '');
const size = files.reduce((n, f) => n + statSync(join(dist, f)).size, 0);
console.log(`${files.length} files, ${(size / 1048576).toFixed(1)} MB, version ${version}`);
