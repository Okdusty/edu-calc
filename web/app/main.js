// EduCalculator on the web: the desktop's Simple mode (src/gui/simple/mod.rs): Calculator, Graph and
// Data over one calculator engine in a worker, with everything kept in this browser's storage.
import { GROUPS, NUMBERS, PAGES } from './keys.js';
import * as L from './logic.js';
import { Board, emptyAxes } from './board.js';
import { Table } from './table.js';
import { DEFAULT_OPTIONS } from './engine-core-options.js';

const $ = (sel, root = document) => root.querySelector(sel);
function h(tag, props = {}, ...children) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') e.className = v;
    else if (k === 'text') e.textContent = v;
    else if (k === 'html') e.innerHTML = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (k in e && k !== 'list') e[k] = v;
    else e.setAttribute(k, v);
  }
  e.append(...children);
  return e;
}
/** A button that leaves the focus where it is, like focus_on_click(false): the keypad keeps typing into the entry. */
function press(props, ...children) {
  const b = h('button', { type: 'button', ...props }, ...children);
  b.addEventListener('mousedown', (e) => e.preventDefault());
  return b;
}
const svg = (path, size = 16) => `<svg viewBox="0 0 16 16" width="${size}" height="${size}" fill="currentColor" aria-hidden="true">${path}</svg>`;
const ICON = {
  keyboard: '<path d="M1 4h14a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H1a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zm1 2v1h1V6zm2 0v1h1V6zm2 0v1h1V6zm2 0v1h1V6zm2 0v1h1V6zm2 0v1h2V6zM2 8v1h2V8zm3 0v1h1V8zm2 0v1h1V8zm2 0v1h1V8zm2 0v1h3V8zM4 10v1h8v-1z" fill-rule="evenodd"/>',
  menu: '<path d="M2 3h12v2H2zm0 4h12v2H2zm0 4h12v2H2z"/>',
  trash: '<path d="M6 1h4l1 1h3v2H2V2h3zM3 5h10l-1 10H4z"/>',
};

const store = {
  get(key, fallback) { try { const v = localStorage.getItem(`educalc.${key}`); return v === null ? fallback : JSON.parse(v); } catch { return fallback; } },
  set(key, value) { try { localStorage.setItem(`educalc.${key}`, JSON.stringify(value)); } catch { /* full or blocked: this session keeps working */ } },
  text(key) { try { return localStorage.getItem(`educalc.${key}`) ?? ''; } catch { return ''; } },
  setText(key, value) { try { localStorage.setItem(`educalc.${key}`, value); return null; } catch (e) { return e.message; } },
  remove(key) { try { localStorage.removeItem(`educalc.${key}`); } catch { /* nothing to remove */ } },
};

// ---- the engine, one job at a time: a newer live job of the same kind replaces a queued one -----

class Engine {
  constructor(onRestart) {
    this.queue = [];
    this.running = null;
    this.onRestart = onRestart;
    this.readyWaiters = [];
    this.nextId = 1;
    this.start();
  }

  start() {
    this.ready = false;
    this.worker = new Worker('worker.js', { type: 'module' });
    // a download that hangs counts as failed
    clearTimeout(this.startTimer);
    this.startTimer = setTimeout(() => this.startFailed('The calculator took too long to load.'), 45000);
    this.worker.onmessage = ({ data }) => {
      if (data.failed) { if (!this.ready) this.startFailed(data.failed); return; }
      if (data.ready) {
        clearTimeout(this.startTimer);
        this.starts = 0;
        this.ready = true;
        this.failure = null;
        this.readyWaiters.splice(0).forEach((f) => f());
        this.pump();
        return;
      }
      const job = this.running;
      if (!job || job.id !== data.id) return;
      clearTimeout(this.watchdog);
      this.running = null;
      if (data.ok) job.resolve(data.reply); else job.reject(new Error(data.error));
      this.pump();
    };
    this.worker.onerror = (e) => {
      if (!this.ready) { if (!this.retrying) this.startFailed(e.message); return; }
      // it stopped working later: its jobs fail, and a new one takes over
      if (this.running) { const job = this.running; this.running = null; job.reject(new Error('The calculator stopped; it is restarting.')); }
      clearTimeout(this.watchdog);
      this.worker.terminate();
      this.start();
      this.onRestart();
    };
  }

  /** A failed download is usually a hiccup: try again, longer each time, then say so and keep trying. */
  startFailed(reason) {
    if (this.retrying) return;
    this.retrying = true;
    clearTimeout(this.startTimer);
    this.worker.terminate();
    this.starts = (this.starts ?? 0) + 1;
    if (this.starts >= 3) {
      this.failure = `The calculator could not load (${reason || 'download failed'}). Trying again…`;
      this.onFailure?.(this.failure);
    }
    setTimeout(() => { this.retrying = false; this.start(); }, Math.min(this.starts * 1500, 15000));
  }

  whenReady() { return this.ready ? Promise.resolve() : new Promise((f) => this.readyWaiters.push(f)); }

  /** `key`: a later job with the same key replaces this one if it hasn't started (it resolves null). */
  run(type, args, key = null) {
    return new Promise((resolve, reject) => {
      if (key) {
        const old = this.queue.findIndex((j) => j.key === key);
        if (old >= 0) { this.queue[old].resolve(null); this.queue.splice(old, 1); }
      }
      this.queue.push({ id: this.nextId++, type, args, key, resolve, reject });
      this.pump();
    });
  }

  pump() {
    if (!this.ready || this.running || !this.queue.length) return;
    const job = this.queue.shift();
    this.running = job;
    this.worker.postMessage({ id: job.id, type: job.type, args: job.args });
    // the engine times itself out; this is for a calculation that ignores that
    this.watchdog = setTimeout(() => {
      this.worker.terminate();
      this.running = null;
      job.reject(new Error('This takes too long to calculate. Try something simpler.'));
      this.start();
      this.onRestart();
    }, 20000);
  }
}

// ---- small widgets ------------------------------------------------------------------------------

function ask(message, detail, buttons) {
  return new Promise((resolve) => {
    const d = h('dialog', {}, h('h2', { text: message }), h('p', { text: detail }));
    const row = h('div', { class: 'buttons' });
    buttons.forEach((text, i) => row.append(h('button', { type: 'button', text, class: i === buttons.length - 1 && i > 0 ? 'suggested' : '', onclick: () => { d.close(); resolve(i); } })));
    d.append(row);
    d.addEventListener('cancel', () => resolve(0));
    d.addEventListener('close', () => d.remove());
    document.body.append(d);
    d.showModal();
  });
}

function download(name, blob) {
  const a = h('a', { href: URL.createObjectURL(blob), download: name });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 30000);
}

let openPopover = null;
function closePopover() {
  if (openPopover) { openPopover.pop.remove(); openPopover.button?.setAttribute('aria-expanded', 'false'); openPopover = null; }
}
document.addEventListener('pointerdown', (e) => {
  if (openPopover && !openPopover.pop.contains(e.target) && !openPopover.button.contains(e.target)) closePopover();
});
function showPopover(button, pop) {
  const again = openPopover?.button === button;
  closePopover();
  if (again) return false;
  document.body.append(pop);
  pop.classList.add('popover');
  const b = button.getBoundingClientRect();
  const w = pop.offsetWidth;
  pop.style.top = `${b.bottom + 4}px`;
  pop.style.left = `${Math.max(8, Math.min(b.right - w, window.innerWidth - w - 8))}px`;
  openPopover = { pop, button };
  button.setAttribute('aria-expanded', 'true');
  return true;
}

// ---- the app -------------------------------------------------------------------------------------

/** A CSS color as [r, g, b], or null; the browser is the parser. */
function cssColor(text) {
  const c = document.createElement('canvas').getContext('2d');
  // an invalid color leaves fillStyle as it was, whatever that is
  c.fillStyle = '#000000';
  c.fillStyle = text.trim();
  const v = c.fillStyle;
  c.fillStyle = '#ffffff';
  c.fillStyle = text.trim();
  if (c.fillStyle !== v) return null;
  if (v.startsWith('#')) return [1, 3, 5].map((i) => parseInt(v.slice(i, i + 2), 16));
  const m = /rgba?\(\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)/.exec(v);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

const TABS = ['Calculator', 'Graph', 'Data'];
const CALC_ONLY = ['÷', '×', '+', '=', '@slot'];
const ROW_COLORS = [[36, 105, 205], [205, 66, 64], [25, 139, 91], [139, 72, 192], [188, 112, 15], [17, 134, 151]];

class App {
  constructor() {
    this.options = { ...DEFAULT_OPTIONS, angle: store.get('angle', 'deg'), fraction: store.get('decimal', false) ? 'decimal' : 'auto' };
    this.tab = 'Calculator';
    this.lastExpr = '';
    this.lastAnswer = '';
    this.lastNumber = '';
    this.historyRows = [];
    this.historyStick = true;
    this.calcRev = 0;
    this.graphRev = 0;
    this.graphRows = [];
    this.nextRow = 0;
    this.graphFocus = 0;
    this.graphScale = store.get('graphScale', 1);
    this.sharedTable = { named: [], unnamed: [], misaligned: false };
    this.sharedNames = [];
    this.columnNames = ['X', 'Y'];
    this.appearance = store.get('appearance', 'light');
    this.engine = new Engine(() => this.shareTable(true));

    this.buildHeader();
    this.buildCalculator();
    this.buildGraph();
    this.buildData();
    this.buildKeypad();
    this.userCss = h('style', { id: 'user-css' });
    document.head.append(this.userCss);
    this.applySettings(store.text('qcalcp.ini') || L.DEFAULT_INI, store.text('style.css'), false);
    this.applyAppearance();
    window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => this.applyAppearance());
    this.bindKeys();

    for (const record of L.parseHistory(store.text('history'))) this.historyCard(record);
    const rows = store.get('graphRows', null);
    if (rows?.length) rows.forEach((r) => this.addGraph(r.text, r.visible));
    else this.addGraph('');
    this.select(TABS.includes(store.get('tab', '')) ? store.get('tab', '') : 'Calculator');
    this.refreshData();
    this.engine.onFailure = (text) => { this.message.textContent = text; };
    this.engine.whenReady().then(() => {
      this.message.textContent = '';
      this.setLoading(false);
      this.queueGraph(true);
    });
    this.setLoading(!this.engine.ready);
  }

  setLoading(on) {
    this.preview.classList.toggle('loading', on);
    if (on) this.preview.textContent = 'Loading the calculator…';
    else if (this.preview.classList.contains('loading') || this.preview.textContent === 'Loading the calculator…') this.preview.textContent = '';
    if (!on && this.entry.value.trim() !== '') this.queueCalculation(false);
  }

  // ---- header -----------------------------------------------------------------------------------

  buildHeader() {
    const tabs = $('.tabs');
    this.tabButtons = {};
    for (const name of TABS) {
      const b = h('button', { type: 'button', text: name, role: 'tab', onclick: () => this.select(name) });
      b.addEventListener('mousedown', (e) => e.preventDefault());
      this.tabButtons[name] = b;
      tabs.append(b);
    }
    const end = $('.header-end');
    this.angle = press({ text: 'DEG', onclick: () => this.setAngle(this.options.angle !== 'rad' ? 'rad' : 'deg') });
    this.functions = press({ html: '<span class="long">Functions</span><span class="short">f(x)</span>', class: 'chevron', 'aria-label': 'Functions', title: 'Functions and how to write them; tap one to insert it', 'aria-haspopup': 'true', onclick: () => this.openFunctions() });
    const keypad = press({ class: 'flat icon-button', html: svg(ICON.keyboard, 20), title: 'Show or hide the keypad (F9)', 'aria-label': 'Show or hide the keypad', onclick: () => this.toggleKeypad() });
    const menu = press({ class: 'flat icon-button', html: svg(ICON.menu, 18), title: 'Menu', 'aria-label': 'Menu', 'aria-haspopup': 'true', onclick: () => this.openMenu(menu) });
    end.append(this.angle, this.functions, keypad, menu);
    this.showAngle();
  }

  setAngle(unit) {
    this.options.angle = unit;
    store.set('angle', unit);
    this.showAngle();
    this.queueCalculation(false);
    this.queueGraph(false);
  }

  showAngle() {
    const rad = this.options.angle === 'rad';
    this.angle.textContent = rad ? 'RAD' : 'DEG';
    this.angle.title = rad ? 'Radians. Tap for degrees.' : 'Degrees. Tap for radians.';
  }

  openMenu(button) {
    const close = (f) => () => { closePopover(); f(); };
    const pop = h('div', { class: 'menu', role: 'menu' });
    pop.append(
      press({ text: 'Fullscreen', role: 'menuitem', onclick: close(() => this.toggleFullscreen()) }),
      press({ text: 'Export chart…', role: 'menuitem', onclick: close(() => this.exportChart()) }),
      press({ text: 'Clear history…', role: 'menuitem', onclick: close(() => this.clearHistory()) }),
      press({ text: 'Settings…', role: 'menuitem', onclick: close(() => this.openSettings()) }),
    );
    if (this.installPrompt) pop.append(press({ text: 'Install app…', role: 'menuitem', onclick: close(() => { this.installPrompt.prompt(); this.installPrompt = null; }) }));
    pop.append(h('hr'), h('div', { class: 'menu-title', text: 'Appearance' }));
    for (const [mode, label] of [['system', 'System Default'], ['light', 'Light'], ['dark', 'Dark']]) {
      pop.append(press({ text: label, role: 'menuitemradio', 'aria-checked': String(this.appearance === mode), onclick: close(() => this.setAppearance(mode)) }));
    }
    if (navigator.serviceWorker?.controller) pop.append(h('hr'), h('div', { class: 'menu-note', text: '✓ Works offline. Your work stays in this browser.' }));
    showPopover(button, pop);
  }

  openFunctions() {
    if (this.functions.disabled) return;
    const search = h('input', { type: 'search', placeholder: 'Search functions', 'aria-label': 'Search functions' });
    const list = h('div', { class: 'catalog-list' });
    const groups = [];
    for (const [topic, items] of GROUPS) {
      const head = h('div', { class: 'catalog-topic', text: topic });
      list.append(head);
      const rows = items.map(([template, desc, example]) => {
        const row = press({ class: 'catalog-row', title: 'Tap to insert', onclick: () => { closePopover(); this.press(template); } },
          h('span', { class: 'catalog-name', text: template }), h('span', { class: 'dim', text: desc }), h('span', { class: 'catalog-example', text: example }));
        list.append(row);
        return { row, hay: `${topic} ${template} ${desc}`.toLowerCase() };
      });
      groups.push({ head, rows });
    }
    search.addEventListener('input', () => {
      const words = search.value.toLowerCase().split(/\s+/).filter(Boolean);
      for (const { head, rows } of groups) {
        let any = false;
        for (const { row, hay } of rows) {
          const hit = words.every((w) => hay.includes(w));
          row.hidden = !hit;
          any ||= hit;
        }
        head.hidden = !any;
      }
    });
    search.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter') return;
      e.preventDefault();
      groups.flatMap((g) => g.rows).find(({ row }) => !row.hidden)?.row.click();
    });
    const shown = showPopover(this.functions, h('div', { class: 'catalog simple-catalog' }, search, list));
    // no on-screen keyboard popping up by itself on a touch screen
    if (shown && !matchMedia('(pointer: coarse)').matches) search.focus();
  }

  toggleFullscreen() {
    if (document.fullscreenElement) document.exitFullscreen();
    else document.documentElement.requestFullscreen?.().catch(() => {});
  }

  async exportChart() {
    const i = await ask('Export chart', 'PNG for documents and slides; SVG stays sharp at any size.', ['Cancel', 'SVG', 'PNG']);
    if (i === 0) return;
    const board = this.tab === 'Data' ? this.dataBoard : this.graph;
    const ext = i === 2 ? 'png' : 'svg';
    download(`graph.${ext}`, await board.exportChart(ext));
    board.status.textContent = `Exported graph.${ext}`;
  }

  setAppearance(mode) {
    this.appearance = mode;
    store.set('appearance', mode);
    // the page's early script reads this before main.js runs
    this.applyAppearance();
    // and the ini says the same, like the desktop rewriting its file
    this.iniText = L.withAppearance(this.iniText ?? L.DEFAULT_INI, mode);
    if (this.iniText !== L.DEFAULT_INI) store.setText('qcalcp.ini', this.iniText);
  }

  /** qcalcp.ini and style.css in effect: plot colors and font, zoom step, keys, appearance, the page's look. */
  applySettings(ini, css, fromUser = true) {
    this.iniText = ini;
    this.cssText = css;
    this.config = L.parseConfig(ini, cssColor);
    this.userCss.textContent = css;
    for (const board of [this.graph, this.dataBoard]) {
      board.plotOverrides = this.config.plot;
      board.wheelStep = this.config.zoomStep;
      board.redraw();
    }
    if (fromUser && this.config.appearance && this.config.appearance !== this.appearance) {
      this.appearance = this.config.appearance;
      store.set('appearance', this.appearance);
      this.applyAppearance();
    }
    return this.config.warnings;
  }

  /** The desktop's two settings files, edited here. A desktop file can be opened or pasted as it is. */
  openSettings() {
    const files = { 'qcalcp.ini': this.iniText, 'style.css': this.cssText };
    let current = 'qcalcp.ini';
    const area = h('textarea', { class: 'settings-text', spellcheck: false, 'aria-label': 'Settings file', value: files[current] });
    const hint = h('p', { class: 'settings-hint' });
    const problems = h('div', { class: 'settings-problems error' });
    const tabs = {};
    const show = (name) => {
      files[current] = area.value;
      current = name;
      area.value = files[name];
      for (const [n, b] of Object.entries(tabs)) b.setAttribute('aria-selected', String(n === name));
      hint.textContent = name === 'style.css'
        ? 'CSS over the page, applied on Save. Names: .keypad button, .simple-entry, .simple-preview, .simple-result, .simple-card, .simple-keypad, .simple-side, .simple-catalog, .graph-row, .board, .table-scroll.'
        : 'Plot colors and font, zoom step and key bindings. A file from the desktop app works as it is; keys for Advanced have no effect here.';
    };
    for (const name of Object.keys(files)) tabs[name] = h('button', { type: 'button', text: name, role: 'tab', onclick: () => show(name) });
    const pick = h('input', { type: 'file', hidden: true, accept: '.ini,.css,.txt,text/*' });
    pick.addEventListener('change', async () => {
      if (pick.files[0]) area.value = await pick.files[0].text();
      pick.value = '';
    });
    const d = h('dialog', { class: 'settings' }, h('h2', { text: 'Settings' }), h('div', { class: 'linked settings-tabs' }, ...Object.values(tabs)), hint, area, problems, pick);
    const close = () => d.close();
    const row = h('div', { class: 'buttons' },
      h('button', { type: 'button', text: 'Open file…', onclick: () => pick.click() }),
      h('button', { type: 'button', text: 'Download', onclick: () => download(current, new Blob([area.value], { type: 'text/plain' })) }),
      h('button', { type: 'button', text: 'Defaults', onclick: () => { area.value = current === 'qcalcp.ini' ? L.DEFAULT_INI : ''; } }),
      h('span', { class: 'spacer' }),
      h('button', { type: 'button', text: 'Cancel', onclick: close }),
      h('button', { type: 'button', text: 'Save', class: 'suggested', onclick: () => {
        files[current] = area.value;
        store.setText('qcalcp.ini', files['qcalcp.ini']);
        store.setText('style.css', files['style.css']);
        const warnings = this.applySettings(files['qcalcp.ini'], files['style.css']);
        // what's wrong stays in view, like the desktop's console; everything else is already applied
        problems.textContent = warnings.join('\n');
        if (!warnings.length) close();
      } }));
    d.append(row);
    d.addEventListener('close', () => { d.remove(); this.focus(); });
    document.body.append(d);
    show(current);
    d.showModal();
  }

  applyAppearance() {
    const dark = this.appearance === 'dark' || (this.appearance === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
    const name = dark ? 'dark' : 'light';
    document.documentElement.dataset.theme = name;
    $('meta[name="theme-color"]').content = dark ? '#242424' : '#ebebeb';
    this.graph.setTheme(name);
    this.dataBoard.setTheme(name);
  }

  // ---- tabs and keys ----------------------------------------------------------------------------

  select(tab) {
    this.tab = tab;
    store.set('tab', tab);
    for (const name of TABS) {
      this.tabButtons[name].setAttribute('aria-selected', String(name === tab));
      this.views[name].hidden = name !== tab;
    }
    const data = tab === 'Data';
    this.keypad.classList.toggle('data-mode', data);
    this.functions.disabled = data;
    this.angle.hidden = data;
    for (const b of this.calcKeys) b.disabled = data;
    closePopover();
    this.graph.redraw();
    this.dataBoard.redraw();
    this.focus();
  }

  focus() {
    if (this.tab === 'Calculator') this.entry.focus({ preventScroll: true });
    else if (this.tab === 'Graph') this.graphRows.find((r) => r.id === this.graphFocus)?.entry.focus({ preventScroll: true });
    else this.table.focus();
  }

  bindKeys() {
    const actions = {
      run: () => this.run(),
      keypad: () => this.toggleKeypad(),
      data: () => this.select('Data'),
      export: () => this.exportChart(),
      reset_view: () => (this.tab === 'Data' ? this.dataBoard : this.graph).reset(),
      dark: () => this.setAppearance(document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark'),
      fullscreen: () => this.toggleFullscreen(),
      clear_history: () => this.clearHistory(),
    };
    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') { closePopover(); return; }
      if (document.querySelector('dialog[open]')) return;
      const hit = this.config.keys.find(([, a]) => L.accelMatches(a, e));
      if (hit) { e.preventDefault(); actions[hit[0]](); }
    });
    window.addEventListener('beforeinstallprompt', (e) => { e.preventDefault(); this.installPrompt = e; });
  }

  run() {
    if (this.tab === 'Calculator') this.queueCalculation(true);
    else if (this.tab === 'Graph') this.queueGraph(true);
    else this.press('@enter');
  }

  buildKeypad() {
    const wrap = $('.keypad-wrap');
    const grip = h('div', { class: 'keypad-grip', tabIndex: 0, title: 'Drag up or down to resize the keypad. Arrow keys adjust height; double-click or Home resets.' }, h('span'));
    this.keypad = h('div', { class: 'keypad simple-keypad' });
    wrap.append(grip, this.keypad);
    this.calcKeys = [];
    const side = h('div', { class: 'side-keys simple-side' });
    this.pageButtons = {};
    const functions = h('div', { class: 'key-grid functions' });
    const showPage = (name) => {
      for (const [n, b] of Object.entries(this.pageButtons)) b.setAttribute('aria-pressed', String(n === name));
      functions.replaceChildren(...PAGES.find((p) => p[0] === name)[2].map(([face, action]) => this.key(face, action)));
    };
    for (const [name, face] of PAGES) {
      const b = press({ text: face, onclick: () => showPage(name) });
      this.pageButtons[name] = b;
      side.append(b);
    }
    showPage('basic');
    const numbers = h('div', { class: 'key-grid numbers' }, ...NUMBERS.map(([face, action]) => this.key(face, action)));
    const tiles = h('div', { class: 'tiles' }, functions, numbers);
    this.keypad.append(side, tiles);

    const setHeight = (px, save = true) => {
      const max = Math.max(140, window.innerHeight - 240);
      const v = Math.min(Math.max(px, 140), max);
      this.keypad.style.setProperty('--keypad-height', `${v}px`);
      if (save) store.set('keypadHeight', v);
    };
    this.setKeypadHeight = setHeight;
    const saved = store.get('keypadHeight', null);
    if (saved) setHeight(saved, false);
    grip.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      grip.setPointerCapture(e.pointerId);
      const [y0, h0] = [e.clientY, this.keypad.offsetHeight];
      const move = (m) => setHeight(h0 - (m.clientY - y0));
      const stop = () => { grip.removeEventListener('pointermove', move); grip.removeEventListener('pointerup', stop); };
      grip.addEventListener('pointermove', move);
      grip.addEventListener('pointerup', stop);
    });
    const reset = () => { this.keypad.style.removeProperty('--keypad-height'); store.remove('keypadHeight'); };
    grip.addEventListener('dblclick', reset);
    grip.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowUp') setHeight(this.keypad.offsetHeight + 16);
      else if (e.key === 'ArrowDown') setHeight(this.keypad.offsetHeight - 16);
      else if (e.key === 'Home') reset();
      else return;
      e.preventDefault();
    });
    this.keypadGrip = grip;
    if (!store.get('keypadShown', true)) this.toggleKeypad();
  }

  key(face, action) {
    const classes = [];
    if (action === '@enter') classes.push('suggested', 'wide');
    const b = press({ text: face, class: classes.join(' '), onclick: () => this.press(action) });
    if (CALC_ONLY.includes(action)) this.calcKeys.push(b);
    if (this.tab === 'Data' && CALC_ONLY.includes(action)) b.disabled = true;
    return b;
  }

  toggleKeypad() {
    const show = this.keypad.hidden;
    this.keypad.hidden = !show;
    this.keypadGrip.hidden = !show;
    store.set('keypadShown', show);
    const mode = show ? 'none' : 'text';
    this.entry.inputMode = mode;
    for (const r of this.graphRows) r.entry.inputMode = mode;
    this.table.setKeypadShown(show);
  }

  /** A key: □ templates and text go in at the cursor (see logic.expand); @ names are actions. */
  press(text) {
    const data = this.tab === 'Data';
    const entry = data ? this.table.activeInput() : this.tab === 'Calculator' ? this.entry : this.graphRows.find((r) => r.id === this.graphFocus)?.entry;
    if (data && text === '@enter') { const [r, c] = this.table.cur; this.table.go(r + 1, c); return; }
    if (!entry) { if (data) this.table.focus(); return; }
    entry.focus({ preventScroll: true });
    const [a, b] = [entry.selectionStart ?? 0, entry.selectionEnd ?? 0];
    const changed = () => entry.dispatchEvent(new Event('input', { bubbles: true }));
    switch (text) {
      case '@left': entry.setSelectionRange(Math.max(a - 1, 0), Math.max(a - 1, 0)); return;
      case '@right': entry.setSelectionRange(Math.min(a + 1, entry.value.length), Math.min(a + 1, entry.value.length)); return;
      case '@clear': entry.value = ''; changed(); return;
      case '@back':
        if (a < b) entry.setRangeText('', a, b, 'end');
        else if (a > 0) entry.setRangeText('', a - 1, a, 'end');
        changed();
        return;
      case '@enter': entry.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true })); return;
      case '@slot': {
        const i = L.nextSlot(entry.value, a, b, false);
        if (i !== null) entry.setSelectionRange(i, i + 1);
        return;
      }
      default:
    }
    let tpl = text;
    if (text === '@ans') tpl = data ? this.lastNumber : `(${this.lastAnswer})`;
    if (text === '−' && data) tpl = '-';
    const [out, slot] = L.expand(tpl, entry.value.slice(a, b));
    entry.setRangeText(out, a, b, 'end');
    if (slot !== null) entry.setSelectionRange(a + slot, a + slot + 1);
    changed();
  }

  slotKeys(input) {
    input.addEventListener('keydown', (e) => {
      if (e.key !== 'Tab') return;
      const i = L.nextSlot(input.value, input.selectionStart, input.selectionEnd, e.shiftKey);
      if (i !== null) { e.preventDefault(); input.setSelectionRange(i, i + 1); }
    });
  }

  // ---- calculator -------------------------------------------------------------------------------

  buildCalculator() {
    this.clearButton = press({ class: 'flat clear-history', html: `${svg(ICON.trash)}<span>Clear history</span>`, title: 'Delete the saved answers', hidden: true, onclick: () => this.clearHistory() });
    this.history = h('div', { class: 'history' });
    this.historyScroll = h('div', { class: 'history-scroll' }, this.history);
    this.preview = h('div', { class: 'preview simple-preview' });
    this.message = h('div', { class: 'message error' });
    this.tableNote = h('div', { class: 'table-note dim' });
    this.tableBar = h('div', { class: 'table-bar', hidden: true });
    this.entry = h('input', { type: 'text', class: 'simple-entry', placeholder: 'Try 12 × 8, 3/4 + 1/2, or sin(30)', autocomplete: 'off', spellcheck: false, inputMode: 'none', 'aria-label': 'Calculation' });
    this.decimal = press({ text: 'Decimal', title: 'Show results as decimals instead of fractions', 'aria-pressed': String(this.options.fraction === 'decimal'), onclick: () => this.toggleDecimal() });
    this.graphThis = press({ text: 'Graph', title: 'Graph this function of x', disabled: true, onclick: () => {
      this.addGraph(this.entry.value.trim() === '' ? this.lastExpr : this.entry.value);
      this.select('Graph');
    } });
    this.slotKeys(this.entry);
    this.entry.addEventListener('input', () => this.queueCalculation(false));
    this.entry.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); this.queueCalculation(true); } });
    // Only the user's own scrolling lets go of the newest answer: a scroll event can also be left over
    // from a tab switch, and arrive when new answers have already made it look scrolled up.
    const touched = () => { this.historyTouched = performance.now(); };
    for (const type of ['wheel', 'touchmove', 'pointerdown', 'keydown']) this.historyScroll.addEventListener(type, touched, { passive: true });
    this.historyScroll.addEventListener('scroll', () => {
      const s = this.historyScroll;
      if (s.scrollTop >= s.scrollHeight - s.clientHeight - 1) this.historyStick = true;
      else if (performance.now() - (this.historyTouched ?? -Infinity) < 1500) {
        this.historyStick = false;
        touched(); // a flick keeps gliding after the finger lifts
      }
    });
    // the answers rise with the entry: the newest stays in view when the space changes
    const stick = () => { if (this.historyStick) this.scrollHistoryDown(); };
    const ro = new ResizeObserver(stick);
    ro.observe(this.historyScroll);
    ro.observe(this.history);
    const view = h('div', { class: 'view calc-view' }, this.clearButton, this.historyScroll, this.preview, this.message, this.tableNote, this.tableBar, h('div', { class: 'input-row' }, this.entry, this.decimal, this.graphThis));
    this.views = { Calculator: view };
    $('.workspace').append(view);
  }

  scrollHistoryDown() {
    const s = this.historyScroll;
    s.scrollTop = s.scrollHeight - s.clientHeight;
  }

  toggleDecimal() {
    const on = this.options.fraction !== 'decimal';
    this.options.fraction = on ? 'decimal' : 'auto';
    store.set('decimal', on);
    this.decimal.setAttribute('aria-pressed', String(on));
    if (this.entry.value.trim() === '' && this.lastExpr !== '') { this.entry.value = this.lastExpr; this.entry.dispatchEvent(new Event('input')); }
    else this.queueCalculation(false);
  }

  queueCalculation(commit) {
    clearTimeout(this.calcPending);
    const id = ++this.calcRev;
    this.message.textContent = '';
    this.graphThis.disabled = true;
    const text = this.entry.value.trim();
    if (text === '') { if (!this.preview.classList.contains('loading')) this.preview.textContent = ''; return; }
    if (text.includes(L.SLOT)) {
      this.preview.textContent = '';
      if (commit) {
        this.message.textContent = 'Fill each □ before calculating.';
        this.press('@slot');
      }
      return;
    }
    const send = () => {
      const job = this.engine.run('calculate', [text, { ...this.options }], commit ? null : 'preview');
      job.then((a) => { if (a) this.answered(id, text, commit, a, null); }, (e) => this.answered(id, text, commit, null, e.message));
    };
    if (commit) send(); else this.calcPending = setTimeout(send, 250);
  }

  answered(id, text, commit, a, error) {
    const current = id === this.calcRev;
    if (!commit && !current) return;
    if (error !== null) {
      if (current) { this.preview.textContent = ''; this.message.textContent = error; }
      return;
    }
    if (current) {
      this.preview.classList.remove('loading');
      this.preview.textContent = `= ${a.result}${a.approx === '' ? '' : `  ≈ ${a.approx}`}`;
      this.graphThis.disabled = !(a.freeVars.length === 1 && a.freeVars[0] === 'x');
    }
    if (!commit) return;
    this.lastNumber = L.answerNumber(a.result, a.approx);
    const saved = { expr: text, input: a.input, result: a.result, approx: a.approx };
    this.historyCard(saved);
    const history = L.parseHistory(store.text('history'));
    history.push(saved);
    const failed = store.setText('history', L.historyText(history.slice(-L.KEEP)));
    if (failed) this.message.textContent = `Answer calculated; history could not be saved: ${failed}`;
    this.lastExpr = text;
    this.lastAnswer = a.result;
    if (this.entry.value === text || this.entry.value.trim() === text) {
      this.entry.value = '';
      this.preview.textContent = '';
    }
  }

  historyCard(saved) {
    this.clearButton.hidden = false;
    const answer = h('div', { class: 'history-answer' }, h('span', { class: 'history-result simple-result', text: saved.result }));
    if (saved.approx !== '') answer.append(h('span', { class: 'dim', text: `≈ ${saved.approx}` }));
    const card = h('button', { type: 'button', class: 'history-card simple-card', title: 'Tap to use again', onclick: () => {
      this.entry.value = saved.expr;
      this.entry.focus({ preventScroll: true });
      this.entry.setSelectionRange(saved.expr.length, saved.expr.length);
      this.entry.dispatchEvent(new Event('input'));
    } }, h('div', { class: 'history-input', text: saved.input }), answer);
    this.history.append(card);
    this.historyRows.push(card);
    if (this.historyRows.length > L.KEEP) this.historyRows.shift().remove();
    this.historyStick = true;
  }

  async clearHistory() {
    const saved = L.parseHistory(store.text('history')).length;
    if (saved > 0) {
      const i = await ask('Clear the calculator history?', `${saved} saved answer${saved === 1 ? '' : 's'} will be deleted. This can't be undone.`, ['Cancel', 'Clear History']);
      if (i !== 1) return;
    }
    store.remove('history');
    for (const row of this.historyRows.splice(0)) row.remove();
    this.lastExpr = '';
    this.clearButton.hidden = true;
    if (this.entry.value.trim() === '') this.graphThis.disabled = true;
    this.focus();
  }

  // ---- the table in the calculator --------------------------------------------------------------

  shareTable(force) {
    const table = L.tableColumns(this.table.grid());
    if (!force && JSON.stringify(table) === JSON.stringify(this.sharedTable)) return;
    const now = table.named.map((c) => c[0]);
    const gone = this.sharedTable.named.map((c) => c[0]).filter((n) => !now.includes(n));
    this.sharedTable = table;
    this.engine.run('share', [table.named, gone]).then((names) => {
      this.sharedNames = names;
      this.rebuildTableBar();
      if (this.entry.value.trim() !== '') this.queueCalculation(false);
    }, () => {});
  }

  tablePair() {
    const taken = this.sharedNames;
    const pick = (i) => { const n = this.columnNames[i]?.trim(); return n !== undefined && taken.includes(n) ? n : null; };
    const y = pick(this.ycol.selectedIndex) ?? taken[1];
    if (y === undefined) return null;
    const xi = this.xcol.selectedIndex - 1;
    const x0 = xi >= 0 ? pick(xi) : null;
    const x = x0 !== null && x0 !== y ? x0 : taken.find((n) => n !== y);
    return x === undefined ? null : [x, y];
  }

  rebuildTableBar() {
    const taken = this.sharedNames;
    this.tableBar.replaceChildren();
    this.tableBar.hidden = !taken.length;
    if (taken.length) this.tableBar.append(h('span', { class: 'caption-label', text: 'Table' }));
    for (const name of taken) this.tableBar.append(press({ text: name, title: 'This table column. For example: mean(X)', onclick: () => this.press(name) }));
    if (taken.length > 1) {
      for (const [text, tip, make] of [
        ['Correlation r', "Pearson's r of the charted columns", (x, y) => `pearson(${x}, ${y})`],
        ['Slope', 'Slope m of their best-fit line y = mx + b', (x, y) => `slope(${x}, ${y})`],
        ['Intercept', 'Intercept b of their best-fit line', (x, y) => `intercept(${x}, ${y})`],
      ]) {
        this.tableBar.append(press({ text, title: tip, onclick: () => { const p = this.tablePair(); if (p) this.press(make(p[0], p[1])); } }));
      }
    }
    const shared = this.sharedTable;
    const unusable = [...shared.unnamed, ...shared.named.map((c) => c[0]).filter((n) => !taken.includes(n))];
    const notes = [];
    if (unusable.length) notes.push(`Not available here: ${unusable.join(', ')}. Name a column with one word that isn't a unit or constant.`);
    if (shared.misaligned) notes.push('Blank cells are skipped, so the columns no longer line up row by row. Fill them before using r or the slope here.');
    this.tableNote.textContent = notes.join(' ');
  }

  // ---- graph ------------------------------------------------------------------------------------

  buildGraph() {
    this.graph = new Board(true, 'light');
    this.graph.changed = () => this.queueGraph(true);
    this.graphList = h('div', { class: 'graph-list' });
    const add = press({ text: 'Add function', onclick: () => this.addGraph('') });
    const sizes = h('span', { class: 'linked' },
      press({ text: 'A−', title: 'Smaller equations', onclick: () => this.scaleEquations(1 / 1.2) }),
      press({ text: 'A+', title: 'Bigger equations', onclick: () => this.scaleEquations(1.2) }));
    const left = h('div', { class: 'pane-left graph-side' }, this.graphList, h('div', { class: 'graph-tools' }, add, sizes));
    this.graphSide = left;
    new ResizeObserver(() => this.styleEquations()).observe(left);
    const view = this.paned(left, h('div', { class: 'pane-right' }, this.graph.root), 'graphSplit', 0.25);
    view.classList.add('view');
    this.views.Graph = view;
    $('.workspace').append(view);
  }

  /** Two panes that keep their proportions when the window resizes; the handle moves the split. */
  paned(left, right, key, initial) {
    const handle = h('div', { class: 'pane-handle', role: 'separator', title: 'Drag to resize' });
    const view = h('div', { class: 'paned' }, left, handle, right);
    view.style.setProperty('--split', store.get(key, initial));
    handle.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      handle.setPointerCapture(e.pointerId);
      const box = view.getBoundingClientRect();
      const move = (m) => {
        const f = Math.min(Math.max((m.clientX - box.left) / box.width, 0.1), 0.75);
        view.style.setProperty('--split', f);
        store.set(key, f);
      };
      const stop = () => { handle.removeEventListener('pointermove', move); handle.removeEventListener('pointerup', stop); };
      handle.addEventListener('pointermove', move);
      handle.addEventListener('pointerup', stop);
    });
    return view;
  }

  scaleEquations(factor) {
    this.graphScale = Math.min(Math.max(this.graphScale * factor, 0.6), 3);
    store.set('graphScale', this.graphScale);
    this.styleEquations();
  }

  /** Equations scale with the space the list has; every touch target stays at least 44 px. */
  styleEquations() {
    const { clientWidth: w, clientHeight: hh } = this.graphSide;
    if (!w || !hh) return;
    const px = L.graphTextPx(w, hh, this.graphScale);
    this.equationPx = px;
    const s = this.graphSide.style;
    s.setProperty('--eq', `${px}px`);
    s.setProperty('--eq-target', `${Math.max(px * 2.1, 44)}px`);
    s.setProperty('--eq-check', `${Math.max(px * 1.2, 26)}px`);
    s.setProperty('--eq-dot', `${Math.max(px * 0.8, 14)}px`);
    s.setProperty('--eq-icon', `${Math.max(px * 1.1, 20)}px`);
    for (const r of this.graphRows) this.fitEquation(r.entry);
  }

  /** A long equation shrinks to stay readable whole in its entry. */
  fitEquation(entry) {
    const px = this.equationPx;
    if (!px) return;
    this.measure ??= document.createElement('canvas').getContext('2d');
    this.measure.font = `${px}px ${getComputedStyle(entry).fontFamily}`;
    const scale = L.fitScale(this.measure.measureText(entry.value).width, entry.clientWidth - 24);
    entry.style.fontSize = scale < 1 ? `${px * scale}px` : '';
  }

  saveGraphRows() {
    store.set('graphRows', this.graphRows.map((r) => ({ text: r.entry.value, visible: r.visible.checked })));
  }

  addGraph(text, visible = true) {
    const id = this.nextRow++;
    const color = ROW_COLORS[id % ROW_COLORS.length];
    const check = h('input', { type: 'checkbox', checked: visible, 'aria-label': 'Show on the graph' });
    const entry = h('input', { type: 'text', placeholder: 'y = x²', value: text, autocomplete: 'off', spellcheck: false, inputMode: this.keypad && this.keypad.hidden ? 'text' : 'none', 'aria-label': 'Function' });
    const error = h('div', { class: 'graph-error', hidden: true });
    const remove = press({ class: 'flat graph-delete', html: svg('<path d="M8 1a7 7 0 1 1 0 14A7 7 0 0 1 8 1zm0 2a5 5 0 1 0 0 10A5 5 0 0 0 8 3zM5.900 4.500 8 6.600l2.100-2.100 1.400 1.400L9.400 8l2.100 2.100-1.400 1.400L8 9.400l-2.100 2.100-1.400-1.400L6.600 8 4.500 5.900z" fill-rule="evenodd"/>'), title: 'Delete', 'aria-label': 'Delete' });
    const root = h('div', { class: 'graph-row' },
      h('div', { class: 'graph-line' },
        h('label', { class: 'graph-check', title: 'Show on the graph' }, check),
        h('span', { class: 'graph-dot', style: `background: rgb(${color.join(',')})` }),
        entry, remove),
      error);
    const row = { id, root, entry, visible: check, error, color };
    this.graphList.append(root);
    this.slotKeys(entry);
    entry.addEventListener('input', () => { this.fitEquation(entry); this.saveGraphRows(); this.queueGraph(false); });
    entry.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); this.queueGraph(true); this.addGraph(''); } });
    entry.addEventListener('focus', () => { this.graphFocus = id; });
    check.addEventListener('change', () => { this.saveGraphRows(); this.queueGraph(true); });
    remove.addEventListener('click', () => {
      const i = this.graphRows.indexOf(row);
      this.graphRows.splice(i, 1);
      root.remove();
      if (this.graphRows.length) this.graphFocus = this.graphRows[this.graphRows.length - 1].id;
      else this.addGraph('');
      this.saveGraphRows();
      this.queueGraph(true);
      this.focus();
    });
    this.graphFocus = id;
    this.graphRows.push(row);
    this.saveGraphRows();
    this.fitEquation(entry);
    if (this.tab === 'Graph') entry.focus({ preventScroll: true });
    if (text !== '') this.queueGraph(false);
  }

  queueGraph(immediate) {
    clearTimeout(this.graphPending);
    const id = ++this.graphRev;
    const send = () => {
      const rows = this.graphRows.filter((r) => r.visible.checked && r.entry.value.trim() !== '').map((r) => [r.id, r.entry.value]);
      // half a view more on each side, so a pan shows curve before the next samples land
      const [lo, hi] = this.graph.range();
      const pad = (hi - lo) / 2;
      this.engine.run('graph', [rows, { ...this.options }, lo - pad, hi + pad], 'graph').then((reply) => {
        if (reply && id === this.graphRev) this.showCurves(reply.rows);
      }, () => {});
    };
    if (immediate) send(); else this.graphPending = setTimeout(send, 300);
  }

  showCurves(results) {
    const a = this.graph.axes;
    const old = a.series.map((s) => s.label);
    a.series = [];
    for (const row of this.graphRows) {
      row.error.textContent = '';
      row.error.hidden = true;
      const r = results.find((c) => c.id === row.id);
      if (!r) continue;
      if (r.error) { row.error.textContent = r.error; row.error.hidden = false; continue; }
      a.series.push({ kind: 'line', x: r.x, y: r.y, color: row.color, width: 2.5, label: row.entry.value });
      if (r.y.every((v) => !Number.isFinite(v))) { row.error.textContent = 'No real values in this view.'; row.error.hidden = false; }
    }
    // pins stay while only the view moved
    const now = a.series.map((s) => s.label);
    if (now.length !== old.length || now.some((l, i) => l !== old[i])) a.pins = [];
    this.graph.redraw();
  }

  // ---- data -------------------------------------------------------------------------------------

  buildData() {
    this.table = new Table({ confirm: ask, download, loadBook: () => import('./vendor/xlsx.mjs') });
    const saved = store.get('table', null);
    if (saved?.length) { this.table.setGrid(saved, [1, 0], false); this.table.refresh(); }
    this.dataBoard = new Board(false, 'light');
    this.dataBoard.axes = emptyAxes(false);
    this.chart = h('select', { 'aria-label': 'Chart type' }, ...['Scatter', 'Line', 'Bar', 'Histogram'].map((c, i) => new Option(c, i)));
    this.xcol = h('select', { 'aria-label': 'x column' });
    this.ycol = h('select', { 'aria-label': 'y column' });
    this.fit = h('select', { 'aria-label': 'Fit', title: 'Curve through the points, with its equation and r²' }, ...L.FIT_NAMES.map((n, i) => new Option(n, i)));
    this.fit.selectedIndex = store.get('fit', 1);
    this.chart.selectedIndex = store.get('chart', 0);
    this.fillSelectors(['X', 'Y'], store.get('xcol', 1), store.get('ycol', 1));
    this.statsLine = h('div', { class: 'stats' });
    this.moreStats = h('div');
    this.more = h('details', { class: 'more-stats' }, h('summary', { text: 'More statistics' }), this.moreStats);
    const selectors = h('div', { class: 'selectors' }, this.chart, h('label', { text: 'x' }), this.xcol, h('label', { text: 'y' }), this.ycol, h('label', { text: 'fit' }), this.fit);
    for (const d of [this.chart, this.xcol, this.ycol, this.fit]) {
      d.addEventListener('change', () => {
        store.set('chart', this.chart.selectedIndex);
        store.set('xcol', this.xcol.selectedIndex);
        store.set('ycol', this.ycol.selectedIndex);
        store.set('fit', this.fit.selectedIndex);
        this.refreshData();
      });
    }
    this.table.connectChanged(() => {
      clearTimeout(this.dataPending);
      this.dataPending = setTimeout(() => this.refreshData(), 200);
    });
    const left = h('div', { class: 'pane-left' }, this.table.root);
    const right = h('div', { class: 'pane-right' }, selectors, this.dataBoard.root, this.statsLine, this.more);
    const view = this.paned(left, right, 'dataSplit', 0.28);
    view.classList.add('data-view');
    view.style.setProperty('--min-left', '300px');
    view.classList.add('view');
    this.views.Data = view;
    $('.workspace').append(view);
  }

  fillSelectors(names, x, y) {
    this.xcol.replaceChildren(new Option('Row number', 0), ...names.map((n, i) => new Option(n, i + 1)));
    this.ycol.replaceChildren(...names.map((n, i) => new Option(n, i)));
    this.xcol.selectedIndex = Math.min(x, names.length);
    this.ycol.selectedIndex = Math.min(y, Math.max(names.length - 1, 0));
  }

  refreshData() {
    const grid = this.table.grid();
    store.set('table', grid);
    const names = (grid[0] ?? []).map((n, i) => (n.trim() === '' ? `Column ${i + 1}` : n));
    if (names.length !== this.columnNames.length || names.some((n, i) => n !== this.columnNames[i])) {
      this.fillSelectors(names, this.xcol.selectedIndex, this.ycol.selectedIndex);
      this.columnNames = names;
    }
    this.shareTable(false);
    const chart = this.chart.selectedIndex;
    const paired = chart !== 3;
    this.xcol.disabled = !paired;
    this.fit.disabled = !paired;
    const x = this.xcol.selectedIndex >= 1 ? this.xcol.selectedIndex - 1 : null;
    const y = Math.max(this.ycol.selectedIndex, 0);
    const c = L.dataChart(grid, chart, x, y);
    const points = paired ? L.pairs(grid, x, y) : [];
    const kind = paired ? this.fit.selectedIndex : 0;
    let fitted = null;
    let failed = null;
    if (kind !== 0) {
      try { fitted = L.fit(points, kind); } catch (e) { failed = e.message; }
    }
    const axes = emptyAxes(false);
    axes.xlabel = c.xlabel;
    axes.ylabel = c.ylabel;
    axes.series = [c.series];
    if (fitted) {
      const lo = points.reduce((m, p) => Math.min(m, p[0]), Infinity);
      const hi = points.reduce((m, p) => Math.max(m, p[0]), -Infinity);
      const xs = Array.from({ length: 201 }, (_, i) => lo + ((hi - lo) * i) / 200);
      axes.series.push({ kind: 'line', x: xs, y: xs.map((v) => L.fitAt(fitted, v)), color: [205, 66, 64], width: 2.5 });
    }
    this.dataBoard.axes = axes;
    this.dataBoard.redraw();

    const fmt = (v) => (v !== null && Number.isFinite(v) ? L.shown(v) : null);
    const s = c.stats;
    const lines = [];
    if (s.count > 0) {
      lines.push([['n', s.count], ['mean', s.mean], ['median', s.median], ['sd', s.std], ['min', s.min], ['max', s.max]].filter(([, v]) => fmt(v) !== null).map(([n, v]) => `${n} = ${fmt(v)}`).join('   '));
    }
    const r = paired ? L.pearson(points) : null;
    const part = (name, v) => (fmt(v) === null ? '' : `   ${name} = ${fmt(v)}`);
    const name = L.FIT_NAMES[kind];
    if (fitted && kind === 1) lines.push(`Best-fit line: ${L.fitEquation(fitted)}${part('r', r)}${part('r²', fitted.r2)}`);
    else if (fitted) lines.push(`${name} fit: ${L.fitEquation(fitted)}${part('r²', fitted.r2)}${part('Pearson r', r)}`);
    else if (failed && (points.length >= 2 || (kind !== 1 && points.length))) lines.push(`No ${name.toLowerCase()} fit: ${failed}`);
    else if (kind === 0 && paired && r !== null) lines.push(`Pearson r = ${fmt(r)}`);
    const skipped = Math.max(c.skipped, s.skipped);
    if (skipped > 0) lines.push(`${skipped} row${skipped === 1 ? '' : 's'} skipped: not a number`);
    this.statsLine.textContent = lines.join('\n');
    this.showMoreStats(s, points, y);
  }

  showMoreStats(s, points, y) {
    const fmt = (v) => (v !== null && Number.isFinite(v) ? L.shown(v) : '–');
    const both = (a, b, f) => (a !== null && b !== null ? f(a, b) : null);
    const section = (title, items) => h('div', {}, h('div', { class: 'more-title', text: title }),
      h('div', { class: 'more-grid' }, ...items.map(([n, v]) => h('div', {}, h('span', { class: 'label', text: n }), h('span', { class: 'value', text: v })))));
    const body = [section(`Column ${this.columnNames[y] ?? 'y'}`, [
      ['Count n', String(s.count)], ['Sum', fmt(s.sum)], ['Mean', fmt(s.mean)], ['Median', fmt(s.median)],
      ['Mode', s.modes.length ? s.modes.map(fmt).join(', ') : 'none repeats'],
      ['Minimum', fmt(s.min)], ['Q1', fmt(s.q1)], ['Q3', fmt(s.q3)], ['Maximum', fmt(s.max)],
      ['Range', fmt(both(s.max, s.min, (a, b) => a - b))], ['IQR', fmt(both(s.q3, s.q1, (a, b) => a - b))],
      ['SD, sample', fmt(s.std)], ['SD, population', fmt(s.popStd)],
      ['Variance, sample', fmt(s.std === null ? null : s.std * s.std)], ['Variance, population', fmt(s.popStd === null ? null : s.popStd * s.popStd)],
      ['Standard error', fmt(s.std === null ? null : s.std / Math.sqrt(s.count))],
    ])];
    if (points.length >= 2) {
      const r = L.pearson(points);
      let line = null;
      try { line = L.leastSquares(points); } catch { /* the x values don't vary */ }
      const cov = L.covariance(points);
      const n = points.length;
      body.push(section('x and y together', [
        ['Complete rows', String(n)], ['Pearson r', fmt(r)], ['r²', fmt(r === null ? null : r * r)], ['Spearman ρ', fmt(L.spearman(points))],
        ['Slope m', fmt(line ? line[0] : null)], ['Intercept b', fmt(line ? line[1] : null)],
        ['Covariance, sample', fmt(cov === null ? null : (cov * n) / (n - 1))], ['Covariance, population', fmt(cov)],
      ]));
    }
    body.push(h('div', { class: 'more-hint', text: 'In the Calculator: mean(X), pearson(X, Y), slope(X, Y)… see Functions.' }));
    this.moreStats.replaceChildren(...body);
  }
}

window.app = new App();

// offline: the service worker keeps the whole app, and the browser is asked to keep the storage
if ('serviceWorker' in navigator) {
  // back online after the offline copy couldn't be made: try again
  const register = () => navigator.serviceWorker.register('sw.js').catch(() => {});
  register();
  window.addEventListener('online', register);
  navigator.storage?.persist?.().catch(() => {});
}
