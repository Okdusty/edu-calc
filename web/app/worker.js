// The calculator engine on its own thread, one job at a time (src/gui/simple/worker.rs).
import createQalc from './engine/qalc.mjs';
import { Engine } from './engine-core.js';

let engine;
try {
  // fetched here for its status: a server error is known at once, not when the loader gives up
  const response = await fetch(new URL('engine/qalc.wasm', import.meta.url));
  if (!response.ok) throw new Error(`the server answered ${response.status}`);
  const wasmBinary = await response.arrayBuffer();
  engine = new Engine(await createQalc({ wasmBinary, onAbort: (what) => self.postMessage({ failed: String(what) }) }));
} catch (e) {
  // a failed download of the engine: the page tries again with a new worker
  self.postMessage({ failed: e.message });
  throw e;
}
self.onmessage = ({ data: { id, type, args } }) => {
  try {
    const reply = jobs[type](...args);
    self.postMessage({ id, ok: true, reply }, reply?.transfer ?? []);
  } catch (e) {
    self.postMessage({ id, ok: false, error: e.message });
  }
};
const jobs = {
  calculate: (text, options) => engine.classroomEval(text, options),
  graph(rows, options, lo, hi) {
    const transfer = [];
    const out = rows.map(([id, text]) => {
      try {
        const [x, y] = engine.graphRow(text, options, lo, hi);
        const c = { id, x: Float64Array.from(x), y: Float64Array.from(y) };
        transfer.push(c.x.buffer, c.y.buffer);
        return c;
      } catch (e) {
        return { id, error: e.message };
      }
    });
    return { rows: out, transfer };
  },
  share: (cols, gone) => engine.shareColumns(cols, gone),
};
self.postMessage({ ready: true });
