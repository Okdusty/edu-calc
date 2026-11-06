// The spreadsheet: row 0 holds the column names (echoed in the sticky titles), the rows below the
// numbers. Enter / Down, Shift+Enter / Up, Tab, Shift+Tab move between cells, moving past the last
// row adds one. Only the rows in view exist as widgets, so thousands are fine (src/gui/data.rs).
import { cells, parseText, splitHeader, toNum, writeCells } from './logic.js';

const ROW_H = 40;
const TEXT_TIP = 'Enter a finite number. This cell is excluded from charts and statistics.';
const BOOK = /\.(xlsx|xlsm|xlsb|xls|ods)$/i;
const ICON = {
  open: '<path d="M1 3h5l2 2h7v2H4L2 13H1zm3 5h12l-2 6H2z"/>',
  save: '<path d="M7 1h2v7l3-3 1.400 1.400L8 11.800 2.600 6.400 4 5l3 3zM2 13h12v2H2z"/>',
  paste: '<path d="M5 2h1a2 2 0 0 1 4 0h1v2H5zM3 3h1v2h8V3h1a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1z"/>',
};
const iconHtml = (name) => `<svg viewBox="0 0 16 16" width="18" height="18" fill="currentColor" aria-hidden="true">${ICON[name]}</svg>`;

export class Table {
  /** `ui`: { confirm(message, detail, buttons) -> Promise<index>, download(name, blob), loadBook() -> Promise<XLSX module> }. */
  constructor(ui) {
    this.ui = ui;
    this.g = [];
    this.cols = 0;
    this.cur = [1, 0];
    this.active = null;
    this.widths = [];
    this.dirty = false;
    this.source = null;
    this.pristine = false;
    this.book = null;
    this.keypadShown = true;
    this.changedCb = () => {};
    this.rows = new Map();
    this.root = document.createElement('div');
    this.root.className = 'table-pane';
    this.root.innerHTML = `
      <div class="toolbar">
        <button type="button" class="flat" data-do="import" title="Import csv, tsv, txt, xlsx, xls, ods…" aria-label="Import">${iconHtml('open')}</button>
        <button type="button" class="flat" data-do="export" title="Export as .csv or .tsv…" aria-label="Export">${iconHtml('save')}</button>
        <button type="button" class="flat" data-do="paste" title="Paste cells from a spreadsheet or text at the current cell (Ctrl+V)" aria-label="Paste">${iconHtml('paste')}</button>
        <select class="sheets" title="Sheet" hidden></select>
      </div>
      <div class="table-edit">
        <span class="dim">Rows</span>
        <span class="linked">
          <button type="button" data-do="addRow" title="Insert a row below the selected cell" aria-label="Insert a row"><svg viewBox="0 0 16 16" width="20" height="20" fill="currentColor" aria-hidden="true"><rect x="0.5" y="1.5" width="4.5" height="3" rx="0.5" opacity="0.4"/><rect x="5.5" y="1.5" width="4.5" height="3" rx="0.5" opacity="0.4"/><rect x="0.5" y="6.5" width="4.5" height="3" rx="0.5" opacity="1"/><rect x="5.5" y="6.5" width="4.5" height="3" rx="0.5" opacity="1"/><rect x="0.5" y="11.5" width="4.5" height="3" rx="0.5" opacity="0.4"/><rect x="5.5" y="11.5" width="4.5" height="3" rx="0.5" opacity="0.4"/><rect class="success" x="11" y="7.25" width="5" height="1.5"/><rect class="success" x="12.75" y="5.5" width="1.5" height="5"/></svg></button>
          <button type="button" data-do="delRow" title="Delete the selected cell's row" aria-label="Delete the row"><svg viewBox="0 0 16 16" width="20" height="20" fill="currentColor" aria-hidden="true"><rect x="0.5" y="1.5" width="4.5" height="3" rx="0.5" opacity="0.4"/><rect x="5.5" y="1.5" width="4.5" height="3" rx="0.5" opacity="0.4"/><rect x="0.5" y="6.5" width="4.5" height="3" rx="0.5" opacity="1"/><rect x="5.5" y="6.5" width="4.5" height="3" rx="0.5" opacity="1"/><rect x="0.5" y="11.5" width="4.5" height="3" rx="0.5" opacity="0.4"/><rect x="5.5" y="11.5" width="4.5" height="3" rx="0.5" opacity="0.4"/><rect class="error" x="11" y="7.25" width="5" height="1.5"/></svg></button>
        </span>
        <span class="dim columns-caption">Columns</span>
        <span class="linked">
          <button type="button" data-do="addCol" title="Insert a column right of the selected cell" aria-label="Insert a column"><svg viewBox="0 0 16 16" width="20" height="20" fill="currentColor" aria-hidden="true"><rect x="1.5" y="0.5" width="3" height="4.5" rx="0.5" opacity="0.4"/><rect x="1.5" y="5.5" width="3" height="4.5" rx="0.5" opacity="0.4"/><rect x="6.5" y="0.5" width="3" height="4.5" rx="0.5" opacity="1"/><rect x="6.5" y="5.5" width="3" height="4.5" rx="0.5" opacity="1"/><rect x="11.5" y="0.5" width="3" height="4.5" rx="0.5" opacity="0.4"/><rect x="11.5" y="5.5" width="3" height="4.5" rx="0.5" opacity="0.4"/><rect class="success" x="5.5" y="12.75" width="5" height="1.5"/><rect class="success" x="7.25" y="11" width="1.5" height="5"/></svg></button>
          <button type="button" data-do="delCol" title="Delete the selected cell's column" aria-label="Delete the column"><svg viewBox="0 0 16 16" width="20" height="20" fill="currentColor" aria-hidden="true"><rect x="1.5" y="0.5" width="3" height="4.5" rx="0.5" opacity="0.4"/><rect x="1.5" y="5.5" width="3" height="4.5" rx="0.5" opacity="0.4"/><rect x="6.5" y="0.5" width="3" height="4.5" rx="0.5" opacity="1"/><rect x="6.5" y="5.5" width="3" height="4.5" rx="0.5" opacity="1"/><rect x="11.5" y="0.5" width="3" height="4.5" rx="0.5" opacity="0.4"/><rect x="11.5" y="5.5" width="3" height="4.5" rx="0.5" opacity="0.4"/><rect class="error" x="5.5" y="12.75" width="5" height="1.5"/></svg></button>
        </span>
      </div>
      <div class="table-scroll"><table><colgroup></colgroup><thead><tr></tr></thead><tbody><tr class="spacer top"></tr><tr class="spacer bottom"></tr></tbody></table></div>
      <div class="table-status dim"></div>
      <input type="file" class="file" hidden accept=".csv,.tsv,.txt,.dat,.xlsx,.xlsm,.xlsb,.xls,.ods">`;
    this.scroll = this.root.querySelector('.table-scroll');
    this.table = this.root.querySelector('table');
    this.colgroup = this.root.querySelector('colgroup');
    this.head = this.root.querySelector('thead tr');
    this.body = this.root.querySelector('tbody');
    this.topSpacer = this.root.querySelector('.spacer.top');
    this.bottomSpacer = this.root.querySelector('.spacer.bottom');
    this.statusLine = this.root.querySelector('.table-status');
    this.sheets = this.root.querySelector('.sheets');
    this.file = this.root.querySelector('.file');
    this.root.querySelectorAll('[data-do]').forEach((b) => {
      b.addEventListener('mousedown', (e) => e.preventDefault());
      b.addEventListener('click', () => this[b.dataset.do]());
    });
    this.scroll.addEventListener('scroll', () => this.renderWindow());
    new ResizeObserver(() => this.renderWindow()).observe(this.scroll);
    this.body.addEventListener('input', (e) => this.edited(e.target));
    this.body.addEventListener('focusin', (e) => this.focused(e.target));
    this.body.addEventListener('keydown', (e) => this.key(e));
    this.root.addEventListener('paste', (e) => {
      const text = e.clipboardData?.getData('text') ?? '';
      if (this.pasteText(text)) e.preventDefault();
    });
    this.file.addEventListener('change', () => {
      const f = this.file.files[0];
      this.file.value = '';
      if (f) this.load(f);
    });
    this.sheets.addEventListener('change', async () => {
      if (await this.confirmDiscard()) this.showSheet(this.sheets.value);
      else this.sheets.value = this.source?.sheet ?? '';
    });
    this.setGrid([['X', 'Y'], ...Array.from({ length: 10 }, () => ['', ''])], [1, 0], false);
    this.refresh();
  }

  grid() { return this.g.map((r) => [...r]); }
  connectChanged(f) { this.changedCb = f; }
  setKeypadShown(shown) { this.keypadShown = shown; this.renderHints(); }

  setGrid(grid, at, focus = true) {
    const g = grid.map((r) => [...r]);
    while (g.length < 2) g.push([]);
    const w = Math.max(1, ...g.map((r) => r.length));
    for (const r of g) while (r.length < w) r.push('');
    if (w !== this.cols) this.widths = [];
    this.cols = w;
    this.g = g;
    this.cur = [Math.min(this.cur[0], g.length - 1), Math.min(this.cur[1], w - 1)];
    this.active = null;
    this.buildColumns();
    this.clearRows();
    this.renderWindow();
    if (focus && this.root.isConnected && this.root.offsetParent !== null) this.go(Math.min(at[0], g.length - 1), at[1]);
    else this.cur = [Math.min(at[0], g.length - 1), Math.min(Math.max(at[1], 0), w - 1)];
  }

  buildColumns() {
    this.colgroup.innerHTML = '<col class="row-number">';
    this.head.innerHTML = '<th class="row-number"></th>';
    for (let c = 0; c < this.cols; c++) {
      const col = document.createElement('col');
      if (this.widths[c]) col.style.width = `${this.widths[c]}px`;
      this.colgroup.append(col);
      const th = document.createElement('th');
      th.innerHTML = '<span class="title"></span><span class="col-grip"></span>';
      th.querySelector('.title').textContent = this.titleOf(c);
      this.resizable(th.querySelector('.col-grip'), c, col);
      this.head.append(th);
    }
    this.table.style.minWidth = `${56 + this.cols * 110}px`;
    this.topSpacer.innerHTML = `<td colspan="${this.cols + 1}"></td>`;
    this.bottomSpacer.innerHTML = `<td colspan="${this.cols + 1}"></td>`;
  }

  /** Drag a column heading's edge. */
  resizable(grip, c, col) {
    grip.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      grip.setPointerCapture(e.pointerId);
      const start = e.clientX;
      const from = col.getBoundingClientRect().width || this.head.children[c + 1].getBoundingClientRect().width;
      const move = (m) => {
        this.widths[c] = Math.max(60, from + m.clientX - start);
        col.style.width = `${this.widths[c]}px`;
      };
      const stop = () => { grip.removeEventListener('pointermove', move); grip.removeEventListener('pointerup', stop); };
      grip.addEventListener('pointermove', move);
      grip.addEventListener('pointerup', stop);
    });
  }

  titleOf(c) { return `${c + 1}  ${(this.g[0]?.[c] ?? '').trim()}`; }

  clearRows() { for (const tr of this.rows.values()) tr.remove(); this.rows.clear(); }

  renderWindow() {
    const view = this.scroll.clientHeight || 400;
    const headH = this.head.offsetHeight || 32;
    const first = Math.max(Math.floor((this.scroll.scrollTop - headH) / ROW_H) - 4, 0);
    const last = Math.min(Math.ceil((this.scroll.scrollTop + view) / ROW_H) + 4, this.g.length - 1);
    for (const [r, tr] of this.rows) {
      if (r < first || r > last) { tr.remove(); this.rows.delete(r); }
    }
    this.topSpacer.firstChild.style.height = `${first * ROW_H}px`;
    this.topSpacer.hidden = first === 0;
    this.bottomSpacer.firstChild.style.height = `${(this.g.length - 1 - last) * ROW_H}px`;
    this.bottomSpacer.hidden = last >= this.g.length - 1;
    let before = this.bottomSpacer;
    for (let r = last; r >= first; r--) {
      let tr = this.rows.get(r);
      if (!tr) { tr = this.buildRow(r); this.rows.set(r, tr); this.body.insertBefore(tr, before); }
      before = tr;
    }
  }

  buildRow(r) {
    const tr = document.createElement('tr');
    tr.dataset.row = r;
    const n = document.createElement('td');
    n.className = 'row-number dim';
    n.textContent = r === 0 ? '' : r;
    tr.append(n);
    for (let c = 0; c < this.cols; c++) {
      const td = document.createElement('td');
      const e = document.createElement('input');
      e.type = 'text';
      e.dataset.col = c;
      e.autocomplete = 'off';
      e.spellcheck = false;
      e.setAttribute('aria-label', r === 0 ? `Name of column ${c + 1}` : `Row ${r}, column ${c + 1}`);
      e.value = this.g[r][c] ?? '';
      this.style(e, r);
      td.append(e);
      tr.append(td);
    }
    return tr;
  }

  style(e, r) {
    const names = r === 0;
    const t = e.value;
    const n = toNum(t, true);
    const text = !names && t.trim() !== '' && !(n !== null && Number.isFinite(n));
    e.placeholder = names ? 'name' : '';
    e.title = text ? TEXT_TIP : '';
    e.classList.toggle('heading', names);
    e.classList.toggle('warning', text);
    e.inputMode = this.keypadShown && !names ? 'none' : 'text';
  }

  renderHints() {
    for (const [r, tr] of this.rows) tr.querySelectorAll('input').forEach((e) => this.style(e, r));
  }

  where(e) { return [Number(e.closest('tr').dataset.row), Number(e.dataset.col)]; }

  edited(e) {
    const [r, c] = this.where(e);
    if (this.g[r][c] === e.value) return;
    this.g[r][c] = e.value;
    this.style(e, r);
    if (r === 0) this.head.children[c + 1].querySelector('.title').textContent = this.titleOf(c);
    this.changed();
  }

  focused(e) {
    if (e.tagName !== 'INPUT') return;
    this.cur = this.where(e);
    this.active = e;
  }

  key(ev) {
    if (ev.target.tagName !== 'INPUT') return;
    const [r, c] = this.where(ev.target);
    const moves = {
      Enter: () => this.go(ev.shiftKey ? r - 1 : r + 1, c),
      ArrowDown: () => this.go(r + 1, c),
      ArrowUp: () => this.go(r - 1, c),
      Tab: () => this.go(r, ev.shiftKey ? c - 1 : c + 1),
    };
    if (moves[ev.key]) { ev.preventDefault(); moves[ev.key](); }
  }

  /** Focus cell (r, c); Tab past the last column wraps to the next row, moving past the last row adds one. */
  go(r0, c0) {
    const w = this.cols;
    const [r, c] = c0 >= w ? [r0 + 1, 0] : c0 < 0 ? [r0 - 1, w - 1] : [r0, c0];
    if (r < 0) return;
    if (r >= this.g.length) this.g.push(new Array(w).fill(''));
    const row = Math.min(r, this.g.length - 1);
    this.cur = [row, c];
    const top = this.head.offsetHeight + row * ROW_H;
    if (top < this.scroll.scrollTop + this.head.offsetHeight) this.scroll.scrollTop = top - this.head.offsetHeight;
    else if (top + ROW_H > this.scroll.scrollTop + this.scroll.clientHeight) this.scroll.scrollTop = top + ROW_H - this.scroll.clientHeight;
    this.renderWindow();
    const e = this.rows.get(row)?.querySelector(`input[data-col="${c}"]`);
    if (e) { e.focus({ preventScroll: true }); this.active = e; }
  }

  focus() { this.go(this.cur[0], this.cur[1]); }

  /** The keypad's keys. Returns the cell to act on. */
  activeInput() {
    const [r, c] = this.cur;
    return this.rows.get(r)?.querySelector(`input[data-col="${c}"]`) ?? null;
  }

  changed() {
    this.dirty = true;
    this.pristine = false;
    this.refresh();
  }

  say(msg, error) {
    this.statusLine.textContent = msg;
    this.statusLine.classList.toggle('error', error);
    this.statusLine.classList.toggle('dim', !error);
  }

  refresh() {
    const [, rows] = cells(this.g);
    const from = this.source && this.pristine ? ` · ${this.source.name}${this.source.sheet ? ` [${this.source.sheet}]` : ''}` : '';
    if (!rows.length) this.say('Type numbers (Enter moves down), paste a block with Ctrl+V, or import a file', false);
    else this.say(`${rows.length} rows × ${rows[0].length} columns${from}`, false);
    this.changedCb();
  }

  addRow() {
    const [r, c] = this.cur;
    const g = this.grid();
    g.splice(r + 1, 0, []);
    this.setGrid(g, [r + 1, c]);
    this.changed();
  }

  delRow() {
    const [r, c] = this.cur;
    if (r === 0 || r >= this.g.length) { this.say('Pick a data row first (the names row stays)', true); return; }
    const g = this.grid();
    g.splice(r, 1);
    this.setGrid(g, [r, c]);
    this.changed();
  }

  addCol() {
    const [r, c] = this.cur;
    const g = this.grid();
    for (const row of g) row.splice(Math.min(c + 1, row.length), 0, '');
    this.setGrid(g, [r, c + 1]);
    this.changed();
  }

  delCol() {
    const [r, c] = this.cur;
    if (this.cols < 2) { this.say('The last column stays', true); return; }
    const g = this.grid();
    for (const row of g) if (c < row.length) row.splice(c, 1);
    this.setGrid(g, [r, Math.min(c, this.cols - 2)]);
    this.changed();
  }

  /** A block fills cells; one value is left to the focused entry (false). */
  pasteText(text) {
    if (text === '') return false;
    const block = parseText(text);
    if (block.length === 1 && block[0].length === 1 && this.active && document.activeElement === this.active) return false;
    this.put(block);
    return true;
  }

  async paste() {
    let text;
    try { text = await navigator.clipboard.readText(); } catch { this.say('Paste: press Ctrl+V, the browser kept the clipboard private', true); return; }
    if (text === '') { this.say('Paste: no text on the clipboard', true); return; }
    const block = parseText(text);
    const e = this.activeInput();
    if (block.length === 1 && block[0].length === 1 && e) {
      e.focus();
      e.setRangeText(block[0][0], e.selectionStart, e.selectionEnd, 'end');
      this.edited(e);
      return;
    }
    this.put(block);
  }

  /** Paste a block at the current cell, growing the grid. At the top, a header line fills the names row. */
  put(block) {
    if (!block.length) return;
    const [r0, c] = this.cur;
    const g = this.grid();
    const write = (r, cs) => {
      while (g.length <= r) g.push([]);
      const row = g[r];
      while (row.length < c + cs.length) row.push('');
      cs.forEach((v, i) => { row[c + i] = v; });
    };
    const [head, body] = splitHeader(block);
    let r = r0;
    let rows = block;
    if (r0 === 0 || (r0 === 1 && head.length && body.length)) {
      if (head.length) write(0, head[head.length - 1]);
      r = 1;
      rows = body;
    }
    for (const row of rows) write(r++, row);
    this.setGrid(g, [r0, c]);
    this.changed();
  }

  async confirmDiscard() {
    if (!this.dirty) return true;
    const i = await this.ui.confirm('Discard the edited data?', 'Export the table first if you want to keep it.', ['Cancel', 'Discard']);
    if (i === 1) this.dirty = false;
    return i === 1;
  }

  async import() {
    if (await this.confirmDiscard()) this.file.click();
  }

  /** Import a file: a workbook's first sheet (the others in the picker), its last header row names the columns. */
  async load(file) {
    this.book = null;
    try {
      if (BOOK.test(file.name)) {
        const XLSX = await this.ui.loadBook();
        this.book = { XLSX, wb: XLSX.read(await file.arrayBuffer(), { dense: false }) };
        this.source = { name: file.name, sheet: this.book.wb.SheetNames[0] };
        this.sheets.innerHTML = '';
        for (const n of this.book.wb.SheetNames) this.sheets.append(new Option(n, n));
        this.sheets.hidden = this.book.wb.SheetNames.length < 2;
        this.showSheet(this.source.sheet);
      } else {
        this.sheets.hidden = true;
        this.source = { name: file.name, sheet: null };
        this.fill(parseText(await file.text()));
      }
    } catch (e) {
      this.say(`${file.name}: ${e.message}`, true);
    }
  }

  showSheet(name) {
    const { XLSX, wb } = this.book;
    this.source.sheet = name;
    this.sheets.value = name;
    const raw = XLSX.utils.sheet_to_json(wb.Sheets[name], { header: 1, raw: true, blankrows: true, defval: '' });
    this.fill(raw.map((r) => r.map((v) => (typeof v === 'number' ? String(v) : String(v).trim()))));
  }

  fill(g) {
    const [head, body] = splitHeader(g);
    const names = head.length ? head[head.length - 1] : [];
    this.scroll.scrollTop = 0;
    this.setGrid([names, ...body], [1, 0]);
    this.pristine = true;
    this.dirty = false;
    this.refresh();
  }

  async export() {
    const i = await this.ui.confirm('Export the table', 'Comma-separated works in most programs.', ['Cancel', 'TSV', 'CSV']);
    if (i === 0) return;
    const [ext, sep] = i === 2 ? ['csv', ','] : ['tsv', '\t'];
    const [names, rows] = cells(this.g);
    if (names.some((n) => n !== '')) rows.unshift(names);
    this.ui.download(`data.${ext}`, new Blob([writeCells(rows, sep)], { type: i === 2 ? 'text/csv' : 'text/tab-separated-values' }));
    this.dirty = false;
    this.say(`Exported to data.${ext}`, false);
  }
}
