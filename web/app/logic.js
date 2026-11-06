// Everything without a screen: a port of the desktop app's Rust logic (src/gui/simple/model.rs,
// keypad.rs, src/plot/analysis.rs, cursor.rs, render.rs, builtins/io.rs), so both give one answer.

export const SLOT = '□';
export const MINUS = '−';

// ---- numbers and text ----------------------------------------------------------------------

const FLOAT = /^[+-]?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?$/;
const SPECIAL = /^[+-]?(inf|infinity|nan)$/i;

function parseFloatStrict(s) {
  if (FLOAT.test(s)) return Number(s);
  if (SPECIAL.test(s)) {
    if (/nan/i.test(s)) return NaN;
    return s.startsWith('-') ? -Infinity : Infinity;
  }
  return null;
}

/** Cell text -> number ("1.5", "-2e3", "Inf", "NaN"); `comma` also takes a decimal comma. Null for blanks and text. */
export function toNum(text, comma) {
  const s = String(text).trim();
  const v = parseFloatStrict(s);
  if (v !== null) return v;
  return comma && s.includes(',') ? parseFloatStrict(s.replaceAll(',', '.')) : null;
}

/** A finite number in a cell, or null. */
export function number(text) {
  const v = toNum(text, true);
  return v !== null && Number.isFinite(v) ? v : null;
}

/** Compact number: 4 significant digits, trailing zeros dropped, scientific outside 1e-4..1e6. */
export function num(v) {
  if (v === 0) return '0';
  if (Number.isNaN(v)) return 'NaN';
  if (!Number.isFinite(v)) return v > 0 ? 'inf' : '-inf';
  const e = Math.floor(Math.log10(Math.abs(v)));
  const trim = (s) => (s.includes('.') ? s.replace(/0+$/, '').replace(/\.$/, '') : s);
  if (e >= -4 && e < 6) return trim(v.toFixed(Math.max(3 - e, 0)));
  const [m, x] = v.toExponential(3).split('e');
  return `${trim(m)}e${x.replace('+', '')}`;
}

/** `num` with a typographic minus, for what's on screen. */
export const shown = (v) => num(v).replaceAll('-', MINUS);

/** Number -> cell text: shortest round-trip form, exponent when huge or tiny. */
export function fmtNum(x) {
  if (Number.isNaN(x)) return 'NaN';
  if (!Number.isFinite(x)) return x > 0 ? 'Inf' : '-Inf';
  if (x !== 0 && (Math.abs(x) >= 1e15 || Math.abs(x) < 1e-5)) return x.toExponential().replace('e+', 'e');
  return String(x);
}

/** Column heading -> identifier: whitespace dropped with the next letter capitalized, other invalid characters '_', a leading non-letter gets an 'x'. */
export function validName(s) {
  let out = '';
  let up = false;
  for (const ch of s.trim()) {
    if (/\s/.test(ch)) { up = true; continue; }
    const c = /^[A-Za-z0-9_]$/.test(ch) ? ch : '_';
    out += up ? c.toUpperCase() : c;
    up = false;
  }
  return /^[A-Za-z]/.test(out) ? out : `x${out}`;
}

function splitQuoted(text, sep) {
  const g = [];
  let row = [];
  let cell = '';
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === '"' && quoted && text[i + 1] === '"') { cell += '"'; i++; }
    else if (ch === '"' && quoted) quoted = false;
    else if (ch === '"' && cell.trim() === '') { cell = ''; quoted = true; }
    else if (quoted) cell += ch;
    else if (ch === '\n') { row.push(cell.trim()); g.push(row); row = []; cell = ''; }
    else if (ch === sep) { row.push(cell.trim()); cell = ''; }
    else cell += ch;
  }
  if (cell.trim() !== '' || row.length) { row.push(cell.trim()); g.push(row); }
  return g;
}

function sniff(text) {
  const head = text.split(/\r?\n/).slice(0, 20).join('\n');
  for (const sep of ['\t', ';', ',']) {
    const rows = splitQuoted(head, sep).filter((r) => r.some((c) => c !== ''));
    if (2 * rows.filter((r) => r.length > 1).length > rows.length) return sep;
  }
  return ' ';
}

/** Delimited text -> rows of trimmed cells; the separator is sniffed unless given. */
export function parseText(input, sepGiven = null) {
  const text = input.startsWith('﻿') ? input.slice(1) : input;
  const sep = sepGiven ?? sniff(text);
  const g = sep === ' '
    ? text.split(/\r?\n/).filter((l, i, all) => i < all.length - 1 || l !== '').map((l) => l.split(/\s+/).filter(Boolean))
    : splitQuoted(text, sep);
  if (sep !== ',') {
    for (const r of g) {
      for (let i = 0; i < r.length; i++) {
        if (toNum(r[i], false) === null && toNum(r[i], true) !== null) r[i] = r[i].replaceAll(',', '.');
      }
    }
  }
  return g;
}

/** Leading rows without any number (titles, column names) and the data rows after them. */
export function splitHeader(g) {
  let n = g.findIndex((r) => r.some((c) => toNum(c, false) !== null));
  if (n < 0) n = g.length;
  return [g.slice(0, n), g.slice(n)];
}

/** Delimited file text; fields holding the separator, quotes or newlines get quoted. */
export function writeCells(rows, sep) {
  const quote = (s) => (s.includes(sep) || /["\n\r]/.test(s) ? `"${s.replaceAll('"', '""')}"` : s);
  return rows.map((r) => `${r.map(quote).join(sep)}\n`).join('');
}

/** Column names and data rows: trailing blank rows and columns dropped, decimal commas made points. */
export function cells(grid) {
  const g = grid.map((r) => [...r]);
  const names = g.length ? g.shift() : [];
  const filled = (r) => { for (let i = r.length - 1; i >= 0; i--) if (r[i].trim() !== '') return i + 1; return 0; };
  let last = g.length;
  while (last > 0 && filled(g[last - 1]) === 0) last--;
  g.length = last;
  const w = g.reduce((m, r) => Math.max(m, filled(r)), 0);
  const rows = g.map((r) => r.slice(0, w).map((c0) => {
    const c = c0.trim();
    return toNum(c, false) === null && toNum(c, true) !== null ? c.replaceAll(',', '.') : c;
  }));
  return [names.slice(0, w).map((n) => n.trim()), rows];
}

// ---- keypad text ----------------------------------------------------------------------------

/** A key's template around the selection: the text and the □ to select next. */
export function expand(tpl, sel) {
  const slotAt = (s, from) => { const j = s.indexOf(SLOT, from); return j < 0 ? null : j; };
  const wrap = sel !== '' && [...sel].some((c) => c !== SLOT);
  const i = tpl.indexOf(SLOT);
  if (i >= 0 && wrap) {
    const before = tpl.slice(0, i);
    const arg = /[([,; ]$/.test(before);
    const atom = /^[\p{L}\p{N}._]+$/u.test(sel) || (sel.startsWith('(') && sel.endsWith(')'));
    const body = arg || atom ? sel : `(${sel})`;
    const out = before + body + tpl.slice(i + 1);
    const next = slotAt(out, i + body.length);
    return [out, next ?? slotAt(out, 0)];
  }
  return [tpl, slotAt(tpl, 0)];
}

export function slots(text) {
  const out = [];
  for (let i = 0; i < text.length; i++) if (text[i] === SLOT) out.push(i);
  return out;
}

/** The □ after the selection (before it when `back`), wrapping around; null without one. */
export function nextSlot(text, start, end, back) {
  const s = slots(text);
  if (!s.length) return null;
  if (back) { for (let i = s.length - 1; i >= 0; i--) if (s[i] < start) return s[i]; return s[s.length - 1]; }
  return s.find((i) => i >= end) ?? s[0];
}

// ---- graph input ----------------------------------------------------------------------------

const BINARY = ['*', '/', '^', '×', '÷', '·', '∙'];

/** Accept a bare expression or y = expression, never an implicit equation or script. Throws the reason. */
export function expression(input) {
  let text = input.trim();
  const eq = text.indexOf('=');
  if (eq >= 0) {
    if (text.slice(0, eq).trim() !== 'y') throw new Error('Use y = a function of x, for example y = x².');
    text = text.slice(eq + 1).trim();
  }
  if (text === '') throw new Error('Enter a function of x.');
  if (/[=<>;:[\]□]/.test(text)) throw new Error("Complete a function of x. Equations and inequalities can't be graphed here.");
  syntax(text);
  return text;
}

function syntax(text) {
  const op = (c) => BINARY.includes(c) || c === '+' || c === '-' || c === MINUS;
  const cs = [...text].filter((c) => !/\s/.test(c));
  const name = (i) => { let j = i; while (j > 0 && /[\p{L}\p{N}]/u.test(cs[j - 1])) j--; return cs.slice(j, i).join(''); };
  const open = [];
  for (let i = 0; i < cs.length; i++) {
    const c = cs[i];
    const prev = i > 0 ? cs[i - 1] : null;
    const next = i + 1 < cs.length ? cs[i + 1] : null;
    if (c === '(') open.push(i);
    else if (c === ')') {
      if (!open.length) throw new Error('There is a “)” without a “(” before it. Remove it, or add the missing “(”.');
      const start = open.pop();
      if (start + 1 === i) {
        const f = name(start);
        throw new Error(`“${f}()” has nothing inside the brackets. Put a value in, for example ${f === '' ? '(x + 1)' : `${f}(x)`}.`);
      }
    } else if (c === ',' && !open.length) {
      throw new Error('A comma only separates the values of a function, as in log(x, 2). For decimals use a point: 2.5');
    } else if (c === ',' && (prev === '(' || prev === ',' || next === null || next === ')' || next === ',')) {
      throw new Error('A value is missing next to a comma. Write one on each side, as in log(x, 2).');
    } else if (op(c)) {
      if (next === null) throw new Error(`“${c}” at the end needs something after it, for example x ${c} 1.`);
      if (next === ')' || next === ',') throw new Error(`“${c}” right before “${next}” needs something after it.`);
      if (BINARY.includes(next) && !(c === '*' && next === '*')) throw new Error(`“${c}${next}” is two operators in a row. Remove one, or put a number between them.`);
      if (BINARY.includes(c) && prev !== '*') {
        if (prev === null) throw new Error(`“${c}” at the start needs something before it, for example 2 ${c} x.`);
        if (prev === '(' || prev === ',') throw new Error(`“${c}” right after “${prev}” needs something before it.`);
      }
    }
  }
  if (open.length) {
    const start = open[0];
    const from = start - [...name(start)].length;
    const what = open.length === 1 ? 'a closing “)”' : `${open.length} closing “)”`;
    throw new Error(`“${cs.slice(from).join('')}” is missing ${what}.`);
  }
  for (const run of cs.join('').split(/[^0-9.]/)) {
    if ((run.match(/\./g) ?? []).length > 1) throw new Error(`“${run}” has more than one decimal point.`);
  }
}

export const UNITS_ERROR = 'This uses units or physical constants (like m, g or c). Graph a function of x with plain numbers.';

/** A calculator error for the graph, in a student's words. `functions`: known names; `signature(name)`. */
export function explain(text, err, functions, signature) {
  if (err === UNITS_ERROR) {
    const w = unknownWord(text, functions);
    return w === null ? err : `EduCalc doesn't know “${w}”. Use x as the variable, with functions such as sin, sqrt or ln.`;
  }
  const quoted = () => { const q = err.split('"')[1]; return q ? q : null; };
  if (err.startsWith('Misplaced operator')) return `“${quoted() ?? 'an operator'}” is in a place where it doesn't fit. Check what comes before and after it.`;
  if (err.includes('is not a valid variable/function/unit')) return `EduCalc doesn't understand “${quoted() ?? err}” in a function.`;
  if (err.startsWith('You need at least') && err.includes('in function ')) {
    const f = err.split('in function ')[1].replace(/\.$/, '').replace(/\(\)$/, '');
    const sig = signature(f);
    return sig ? `${f} needs more values inside its brackets: ${sig}` : `${f} needs more values inside its brackets.`;
  }
  if (err.endsWith('timed out')) return 'This function takes too long to calculate. Try a simpler one.';
  return err;
}

/** The first word that isn't x, π, e or known function names run together (sinx is sin x). */
export function unknownWord(text, functions) {
  const known = (w) => {
    if (['x', 'pi', 'π', 'e'].includes(w)) return true;
    const ok = new Array(w.length + 1).fill(false);
    ok[0] = true;
    for (let j = 1; j <= w.length; j++) {
      for (let i = 0; i < j; i++) {
        const part = w.slice(i, j);
        if (ok[i] && (functions.has(part) || ['x', 'pi', 'π', 'e'].includes(part))) { ok[j] = true; break; }
      }
    }
    return ok[w.length];
  };
  for (const w of text.split(/[^\p{L}]+/u)) if (w !== '' && !known(w)) return w;
  return null;
}

// ---- statistics -----------------------------------------------------------------------------

function quantile(sorted, p) {
  if (!sorted.length) return null;
  const h = (sorted.length - 1) * p;
  const lo = Math.floor(h);
  const hi = Math.ceil(h);
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (h - lo);
}

/** Statistics of a column; `values` holds numbers, or null for what isn't one. */
export function summary(values) {
  const v = values.filter((n) => n !== null && Number.isFinite(n)).sort((a, b) => a - b);
  const n = v.length;
  const mean = n > 0 ? v.reduce((s, x) => s + x / n, 0) : null;
  const spread = (div) => Math.sqrt(v.reduce((s, x) => s + (x - mean) ** 2 / div, 0));
  const runs = [];
  for (const x of v) {
    if (runs.length && runs[runs.length - 1][0] === x) runs[runs.length - 1][1]++;
    else runs.push([x, 1]);
  }
  const most = runs.reduce((m, r) => Math.max(m, r[1]), 0);
  return {
    count: n,
    skipped: values.length - n,
    mean,
    median: n > 0 ? (n % 2 === 1 ? v[(n - 1) / 2] : v[n / 2 - 1] / 2 + v[n / 2] / 2) : null,
    min: n > 0 ? v[0] : null,
    max: n > 0 ? v[n - 1] : null,
    std: n > 1 ? spread(n - 1) : null,
    sum: n > 0 ? v.reduce((s, x) => s + x, 0) : null,
    q1: quantile(v, 0.25),
    q3: quantile(v, 0.75),
    popStd: n > 0 ? spread(n) : null,
    modes: most > 1 ? runs.filter((r) => r[1] === most).map((r) => r[0]) : [],
  };
}

/** The complete, finite X/Y pairs; no x column = the row number. */
export function pairs(grid, xcol, ycol) {
  const out = [];
  grid.slice(1).forEach((row, i) => {
    const x = xcol === null ? i + 1 : (row[xcol] === undefined ? null : number(row[xcol]));
    if (x === null || row[ycol] === undefined) return;
    const y = number(row[ycol]);
    if (y !== null) out.push([x, y]);
  });
  return out;
}

const meanOf = (p, k) => p.reduce((s, q) => s + q[k] / p.length, 0);

/** Pearson's r; null when either list doesn't vary. Scaled first, so large values don't overflow. */
export function pearson(p) {
  const [mx, my] = [meanOf(p, 0), meanOf(p, 1)];
  const sx = p.reduce((a, q) => Math.max(a, Math.abs(q[0] - mx)), 0);
  const sy = p.reduce((a, q) => Math.max(a, Math.abs(q[1] - my)), 0);
  if (p.length < 2 || sx === 0 || sy === 0) return null;
  const sum = (f) => p.reduce((s, q) => s + f(q), 0);
  const xy = sum((q) => ((q[0] - mx) / sx) * ((q[1] - my) / sy));
  const xx = sum((q) => ((q[0] - mx) / sx) ** 2);
  const yy = sum((q) => ((q[1] - my) / sy) ** 2);
  const r = Math.min(1, Math.max(-1, xy / Math.sqrt(xx * yy)));
  return Number.isFinite(r) ? r : null;
}

/** Ranks from 1, ties sharing their average. */
export function ranks(v) {
  const order = v.map((_, i) => i).sort((a, b) => v[a] - v[b]);
  const out = new Array(v.length).fill(0);
  let i = 0;
  while (i < order.length) {
    let j = i;
    while (j + 1 < order.length && v[order[j + 1]] === v[order[i]]) j++;
    for (let k = i; k <= j; k++) out[order[k]] = (i + j) / 2 + 1;
    i = j + 1;
  }
  return out;
}

export function spearman(p) {
  const rx = ranks(p.map((q) => q[0]));
  const ry = ranks(p.map((q) => q[1]));
  return pearson(rx.map((x, i) => [x, ry[i]]));
}

/** The population's covariance, like the calculator's cov(). */
export function covariance(p) {
  if (!p.length) return null;
  const [mx, my] = [meanOf(p, 0), meanOf(p, 1)];
  return p.reduce((s, q) => s + ((q[0] - mx) * (q[1] - my)) / p.length, 0);
}

/** Ordinary least squares: [slope, intercept]. Throws the reason. */
export function leastSquares(p) {
  if (p.length < 2) throw new Error('need at least 2 valid X/Y pairs');
  const [mx, my] = [meanOf(p, 0), meanOf(p, 1)];
  const sx = p.reduce((a, q) => Math.max(a, Math.abs(q[0] - mx)), 0);
  const sy = p.reduce((a, q) => Math.max(a, Math.abs(q[1] - my)), 0);
  if (sx === 0) throw new Error('X values must differ');
  let slope = 0;
  if (sy !== 0) {
    const xx = p.reduce((s, q) => s + ((q[0] - mx) / sx) ** 2, 0);
    const xy = p.reduce((s, q) => s + ((q[0] - mx) / sx) * ((q[1] - my) / sy), 0);
    slope = (xy / xx) * (sy / sx);
  }
  const intercept = -slope * mx + my;
  if (![slope, intercept, sx, sy].every(Number.isFinite)) throw new Error('values exceed numeric range');
  return [slope, intercept];
}

export const FIT_NAMES = ['No fit', 'Linear', 'Quadratic', 'Exponential', 'Power', 'Logarithmic'];

export function fitAt(f, x) {
  const c = f.coef;
  switch (f.kind) {
    case 1: return c[0] * x + c[1];
    case 2: return (c[0] * x + c[1]) * x + c[2];
    case 3: return c[0] * Math.exp(c[1] * x);
    case 4: return c[0] * x ** c[1];
    case 5: return c[0] + c[1] * Math.log(x);
    default: return NaN;
  }
}

export function fitEquation(f) {
  const then = (v) => ` ${v < 0 ? MINUS : '+'} ${shown(Math.abs(v))}`;
  const c = f.coef;
  switch (f.kind) {
    case 1: return `y = ${shown(c[0])}x${then(c[1])}`;
    case 2: return `y = ${shown(c[0])}x²${then(c[1])}x${then(c[2])}`;
    case 3: return `y = ${shown(c[0])}·e^(${shown(c[1])}x)`;
    case 4: return `y = ${shown(c[0])}·x^${shown(c[1])}`;
    case 5: return `y = ${shown(c[0])}${then(c[1])}·ln(x)`;
    default: return '';
  }
}

function solve3(a, b) {
  const m = a.map((r, i) => [...r, b[i]]);
  for (let col = 0; col < 3; col++) {
    let pivot = col;
    for (let r = col + 1; r < 3; r++) if (Math.abs(m[r][col]) > Math.abs(m[pivot][col])) pivot = r;
    if (m[pivot][col] === 0) return null;
    [m[col], m[pivot]] = [m[pivot], m[col]];
    for (let r = 0; r < 3; r++) {
      if (r === col) continue;
      const k = m[r][col] / m[col][col];
      for (let j = col; j < 4; j++) m[r][j] -= k * m[col][j];
    }
  }
  return m.map((r, i) => r[3] / r[i]);
}

/** Least squares of FIT_NAMES[kind] over the points. Throws the reason. Exponential, power and logarithmic refuse points logarithms can't take. */
export function fit(points, kind) {
  const need = kind === 2 ? 3 : 2;
  if (points.length < need) throw new Error(`it needs at least ${need} complete rows`);
  const positive = (ok, what) => { if (!ok) throw new Error(`every ${what} must be above 0`); };
  let coef;
  if (kind === 1) coef = leastSquares(points);
  else if (kind === 2) {
    const mx = meanOf(points, 0);
    const sx = points.reduce((a, p) => Math.max(a, Math.abs(p[0] - mx)), 0);
    if (new Set(points.map((p) => p[0])).size < 3) throw new Error('it needs at least 3 different x values');
    const a = [[0, 0, 0], [0, 0, 0], [0, 0, 0]];
    const v = [0, 0, 0];
    for (const p of points) {
      const u = (p[0] - mx) / sx;
      const row = [u * u, u, 1];
      for (let i = 0; i < 3; i++) {
        v[i] += row[i] * p[1];
        for (let j = 0; j < 3; j++) a[i][j] += row[i] * row[j];
      }
    }
    const s = solve3(a, v);
    if (!s) throw new Error("the points don't determine a curve");
    const [qa, qb] = [s[0] / (sx * sx), s[1] / sx];
    coef = [qa, qb - 2 * qa * mx, s[2] - qb * mx + qa * mx * mx];
  } else if (kind === 3) {
    positive(points.every((p) => p[1] > 0), 'y');
    const [b, lnA] = leastSquares(points.map((p) => [p[0], Math.log(p[1])]));
    coef = [Math.exp(lnA), b];
  } else if (kind === 4) {
    positive(points.every((p) => p[0] > 0), 'x');
    positive(points.every((p) => p[1] > 0), 'y');
    const [b, lnA] = leastSquares(points.map((p) => [Math.log(p[0]), Math.log(p[1])]));
    coef = [Math.exp(lnA), b];
  } else if (kind === 5) {
    positive(points.every((p) => p[0] > 0), 'x');
    const [b, a] = leastSquares(points.map((p) => [Math.log(p[0]), p[1]]));
    coef = [a, b];
  } else throw new Error('no fit picked');
  if (!coef.every(Number.isFinite)) throw new Error('the values exceed the numeric range');
  const f = { kind, coef, r2: null };
  const my = meanOf(points, 1);
  const total = points.reduce((s, p) => s + (p[1] - my) ** 2, 0);
  const rest = points.reduce((s, p) => s + (p[1] - fitAt(f, p[0])) ** 2, 0);
  f.r2 = total > 0 ? Math.max(1 - rest / total, 0) : null;
  return f;
}

/** A calculator answer as a table cell: its decimal form when it has one, with a plain minus. */
export const answerNumber = (result, approx) => (approx === '' ? result : approx).replaceAll(MINUS, '-');

/** What the calculator can see of the table. The first column of a repeated name wins. */
export function tableColumns(grid) {
  const out = { named: [], unnamed: [], misaligned: false };
  const names = grid[0] ?? [];
  const rows = grid.slice(1);
  const used = [];
  names.forEach((raw, c) => {
    const values = rows.map((r) => (r[c] === undefined ? null : number(r[c]))).filter((v) => v !== null);
    if (!values.length) return;
    const name = raw.trim();
    if (name === '' || validName(name) !== name) out.unnamed.push(name === '' ? `Column ${c + 1}` : name);
    else if (out.named.every(([n]) => n !== name)) { out.named.push([name, values]); used.push(c); }
  });
  out.misaligned = rows.some((r) => {
    const filled = used.filter((c) => r[c] !== undefined && number(r[c]) !== null).length;
    return filled > 0 && filled < used.length;
  });
  return out;
}

/** The Data tab's chart: series and axis names, statistics, rows skipped. chart: 0 scatter, 1 line, 2 bar, 3 histogram. */
export function dataChart(grid, chart, xcol, ycol) {
  const names = grid[0] ?? [];
  let rows = grid.slice(1);
  let last = rows.length;
  while (last > 0 && !rows[last - 1].some((s) => s.trim() !== '')) last--;
  rows = rows.slice(0, last);
  const at = (r, c) => (r[c] === undefined ? null : number(r[c]));
  const stats = summary(rows.map((r) => at(r, ycol)));
  let xlabel = xcol !== null && names[xcol] !== undefined ? names[xcol] : 'Row number';
  let ylabel = names[ycol] ?? 'Value';
  let skipped = 0;
  let series;
  if (chart === 3) {
    const values = rows.map((r) => at(r, ycol)).filter((v) => v !== null);
    skipped = rows.length - values.length;
    xlabel = ylabel;
    ylabel = 'Frequency';
    const lo = values.length ? values.reduce((a, b) => Math.min(a, b)) : 0;
    const hi = values.length ? values.reduce((a, b) => Math.max(a, b)) : 1;
    const bins = Math.min(40, Math.max(1, Math.ceil(Math.sqrt(values.length))));
    const width = hi > lo ? (hi - lo) / bins : 1;
    const start = hi > lo ? lo : lo - 0.5;
    const x = Array.from({ length: bins }, (_, i) => start + (i + 0.5) * width);
    const y = new Array(bins).fill(0);
    for (const v of values) y[Math.min(Math.max(Math.floor((v - start) / width), 0), bins - 1)] += 1;
    series = { kind: 'bar', x, y, width: width * 0.95 };
  } else {
    const xs = [];
    const ys = [];
    rows.forEach((row, i) => {
      const x = xcol === null ? i + 1 : at(row, xcol);
      const y = at(row, ycol);
      if (x !== null && y !== null) { xs.push(x); ys.push(y); }
      else {
        skipped++;
        if (chart === 1) { xs.push(NaN); ys.push(NaN); }
      }
    });
    if (chart === 2) {
      const sorted = xs.filter(Number.isFinite).sort((a, b) => a - b);
      let d = Infinity;
      for (let i = 1; i < sorted.length; i++) if (sorted[i] - sorted[i - 1] > 0) d = Math.min(d, sorted[i] - sorted[i - 1]);
      series = { kind: 'bar', x: xs, y: ys, width: (d === Infinity ? 1 : d) * 0.8 };
    } else series = { kind: 'line', x: xs, y: ys, marker: true, line: chart !== 0, width: 1.5 };
  }
  return { series, xlabel, ylabel, stats, skipped };
}

// ---- sampled curves -------------------------------------------------------------------------

function quantiles(v) {
  const f = v.filter(Number.isFinite).sort((a, b) => a - b);
  if (!f.length) return null;
  const k = Math.floor(f.length / 20);
  return [f[k], f[f.length - 1 - k]];
}

function finiteRange(v) {
  let lo = Infinity;
  let hi = -Infinity;
  for (const x of v) if (Number.isFinite(x)) { lo = Math.min(lo, x); hi = Math.max(hi, x); }
  return [lo, hi];
}

function robustYlim(ys) {
  const q = quantiles(ys);
  if (!q) return null;
  const [min, max] = finiteRange(ys);
  const r = Math.max(q[1] - q[0], 1e-12);
  return max - min > 5 * r ? [Math.max(q[0] - 0.5 * r, min), Math.min(q[1] + 0.5 * r, max)] : null;
}

/** Break a sampled curve at jumps (tan, 1/x, floor) so no vertical lines get drawn through poles. */
export function breakJumps(xs, ys) {
  const q = quantiles(ys);
  if (!q) return [[...xs], [...ys]];
  const d = [];
  for (let i = 1; i < ys.length; i++) d.push(Math.abs(ys[i] - ys[i - 1]));
  const sorted = d.filter(Number.isFinite).sort((a, b) => a - b);
  const mid = sorted.length ? sorted[Math.floor(sorted.length / 2)] : 0;
  const lim = Math.max(mid * 20, (q[1] - q[0]) * 0.25);
  const x = [];
  const y = [];
  for (let k = 0; k < xs.length; k++) {
    if (k > 0 && lim > 0 && d[k - 1] > lim) {
      // NaN (beside a pole) counts as 0, as in Rust's f64::max; Math.max would give NaN
      const near = Math.max(k >= 2 ? d[k - 2] || 0 : 0, d[k] || 0);
      if (ys[k] * ys[k - 1] < 0 || d[k - 1] > 3 * near) { x.push(NaN); y.push(NaN); }
    }
    x.push(xs[k]);
    y.push(ys[k]);
  }
  return [x, y];
}

function medianOf(values) {
  if (!values.length) return 0;
  return [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
}

function pivots(y, a, b, tol) {
  const out = [];
  let dir = 0;
  let ext = 0;
  for (let i = 1; i < b - a; i++) {
    const yi = y[a + i];
    if (dir === 0) {
      if (Math.abs(yi - y[a]) > tol) { dir = Math.sign(yi - y[a]); ext = i; }
    } else if ((yi - y[a + ext]) * dir > 0) ext = i;
    else if ((y[a + ext] - yi) * dir > tol) { out.push([ext, dir > 0]); dir = -dir; ext = i; }
  }
  return out;
}

function peak(x, y, i) {
  const [a, b] = [x[i - 1] - x[i], x[i + 1] - x[i]];
  const [fa, fb] = [y[i - 1] - y[i], y[i + 1] - y[i]];
  const c2 = (fa / a - fb / b) / (a - b);
  const c1 = fa / a - c2 * a;
  const t = -c1 / (2 * c2);
  return Number.isFinite(t) && t >= a && t <= b ? [x[i] + t, y[i] + (c1 * t) / 2] : [x[i], y[i]];
}

function crossing(x, v, m) {
  if (m + 1 >= v.length) return null;
  const [a, b] = [v[m], v[m + 1]];
  if (a === 0) return x[m];
  if (a * b < 0) return x[m] + (a / (a - b)) * (x[m + 1] - x[m]);
  return null;
}

function tidy(v, span) {
  const s = Math.abs(span);
  if (Math.abs(v) < s * 1e-9) return 0;
  if (Math.abs(v - Math.round(v)) < s * 1e-5) return Math.round(v) + 0;
  return v;
}

/** Key points of y(x), as the desktop's plot::analysis; null without 5 usable samples in increasing x. */
export function analyze(x0, y0) {
  const n0 = Math.min(x0.length, y0.length);
  const f = (i) => (i >= 0 && i < n0 ? y0[i] : NaN);
  const hole = (i) => {
    const [a, b] = [f(i - 1), f(i + 1)];
    return !Number.isFinite(f(i)) && Number.isFinite(a) && Number.isFinite(b) && Math.abs(b - a) <= 2 * (Math.abs(a - f(i - 2)) + Math.abs(f(i + 2) - b));
  };
  const kept = [];
  for (let i = 0; i < n0; i++) if (!hole(i)) kept.push(i);
  const [bx, by] = breakJumps(kept.map((i) => x0[i]), kept.map((i) => y0[i]));
  const xs = [];
  const ys = [];
  const runs = [];
  let start = 0;
  for (let i = 0; i <= bx.length; i++) {
    const [a, b] = i < bx.length ? [bx[i], by[i]] : [NaN, NaN];
    if (Number.isFinite(a) && Number.isFinite(b)) { xs.push(a); ys.push(b); }
    else if (xs.length - start < 3) { xs.length = start; ys.length = start; }
    else { runs.push([start, xs.length]); start = xs.length; }
  }
  if (xs.length < 5) return null;
  for (let i = 1; i < xs.length; i++) if (!(xs[i] > xs[i - 1])) return null;

  const curveAt = (q) => {
    const run = runs.find(([a, b]) => xs[a] <= q && q <= xs[b - 1]);
    if (!run) return null;
    const [a, b] = run;
    let lo = a;
    let hi = b;
    while (lo < hi) { const m = (lo + hi) >> 1; if (xs[m] < q) lo = m + 1; else hi = m; }
    const j = Math.min(Math.max(lo, a + 1), b - 2);
    const [x0_, y0_, x1, y1, x2, y2] = [xs[j - 1], ys[j - 1], xs[j], ys[j], xs[j + 1], ys[j + 1]];
    const v = (y0_ * (q - x1) * (q - x2)) / ((x0_ - x1) * (x0_ - x2)) + (y1 * (q - x0_) * (q - x2)) / ((x1 - x0_) * (x1 - x2)) + (y2 * (q - x0_) * (q - x1)) / ((x2 - x0_) * (x2 - x1));
    return [v, Math.abs(y2 - 2 * y1 + y0_) / 4];
  };
  const mirrors = (h, k, tol) => {
    let count = 0;
    for (let i = 0; i < xs.length; i++) {
      const at = curveAt(2 * h - xs[i]);
      if (!at) continue;
      if (Math.abs(at[0] - (k === null ? ys[i] : 2 * k - ys[i])) > tol + at[1]) return false;
      count++;
    }
    return count >= Math.max(Math.floor(xs.length / 5), 5);
  };

  const rl = robustYlim(ys) ?? finiteRange(ys);
  const r = rl[1] - rl[0];
  const xspan = xs[xs.length - 1] - xs[0];
  const d4 = [];
  for (const [a, b] of runs) {
    for (let i = a + 2; i < b - 2; i++) d4.push(Math.abs(ys[i - 2] - 4 * ys[i - 1] + 6 * ys[i] - 4 * ys[i + 1] + ys[i + 2]));
  }
  const few = d4.length < 20;
  const sigma = few ? 0 : medianOf(d4) / 5.64;
  const smooth = sigma <= r * 1e-6;
  const eps = r * 1e-9;
  const k = { max: [], min: [], roots: [], inflection: [], yInt: null, vertex: null, symmetry: { kind: 'none' } };

  let tol = Math.max(12 * sigma, r * 1e-6);
  const piv = [];
  for (const [a, b] of runs) {
    for (const [i0, up] of pivots(ys, a, b, tol)) {
      const i = a + i0;
      const flat = (j) => j < b && Math.abs(ys[j] - ys[i]) <= eps;
      if (!(flat(i + 1) && flat(i + 2))) piv.push([i, up]);
    }
  }
  for (const [i, up] of piv) (up ? k.max : k.min).push(smooth && !few ? peak(xs, ys, i) : [xs[i], ys[i]]);

  tol = Math.max(6 * sigma, eps);
  for (const [a, b] of runs) {
    let last = null;
    for (let j = a; j < b; j++) {
      if (Math.abs(ys[j]) <= tol) continue;
      if (last !== null && ys[last] > 0 !== ys[j] > 0) {
        let zeros = 0;
        for (let m = last; m < j; m++) if (ys[m] === 0) zeros++;
        if (zeros < 3) {
          const cross = [];
          for (let m = last; m < j; m++) { const c = crossing(xs, ys, m); if (c !== null) cross.push([m, c]); }
          if (cross.length) {
            const [m, c] = cross[Math.floor(cross.length / 2)];
            const slope = (ys[m + 1] - ys[m]) / (xs[m + 1] - xs[m]);
            const at = curveAt(c);
            let x = c;
            if (at && ys[m] !== 0 && Number.isFinite(c - at[0] / slope)) x = Math.min(Math.max(c - at[0] / slope, xs[m]), xs[m + 1]);
            k.roots.push([x, 0]);
          }
        }
      }
      last = j;
    }
  }
  if (smooth) for (const p of [...k.max, ...k.min]) if (Math.abs(p[1]) <= tol) k.roots.push([p[0], 0]);
  k.roots.sort((p, q) => p[0] - q[0]);

  const yAt = curveAt(0);
  k.yInt = yAt ? [0, yAt[0]] : null;

  const g = new Array(xs.length).fill(NaN);
  for (const [a, b] of runs) {
    for (let i = a + 1; i < b - 1; i++) {
      const sl = (ys[i] - ys[i - 1]) / (xs[i] - xs[i - 1]);
      const sr = (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]);
      g[i] = (sr - sl) / (xs[i + 1] - xs[i - 1]);
    }
  }
  tol = Math.max(12 * sigma, eps);
  for (const [a, b] of runs) {
    let held = null;
    let streak = null;
    for (let j = a + 1; j < b - 1; j++) {
      if (!(Math.abs((g[j] * (xs[j + 1] - xs[j - 1]) ** 2) / 2) > tol)) continue;
      const s = g[j] > 0;
      streak = streak && streak[0] === s ? [s, streak[1], streak[2] + 1] : [s, j, 1];
      const [, first, len] = streak;
      if (len < 2) continue;
      if (held && held[0] !== s && len === 2) {
        const cross = [];
        for (let m = held[1]; m < first; m++) { const c = crossing(xs, g, m); if (c !== null) cross.push(c); }
        if (cross.length) {
          const c = cross[Math.floor(cross.length / 2)];
          const at = curveAt(c);
          if (at) k.inflection.push([c, at[0]]);
        }
      }
      held = [s, j];
    }
  }

  if (smooth && runs.length === 1 && piv.length === 1) {
    const f2 = g.filter(Number.isFinite);
    const med = medianOf(f2);
    if (med !== 0 && f2.every((v) => Math.abs(v - med) <= 1e-3 * Math.abs(med))) {
      const v = peak(xs, ys, piv[0][0]);
      if (piv[0][1]) k.max[0] = v; else k.min[0] = v;
      k.vertex = v;
    }
  }

  if (r > eps) {
    const tolS = r * 1e-6 + 8 * sigma;
    const mid = (xs[0] + xs[xs.length - 1]) / 2;
    const near = (v) => [...v].sort((p, q) => Math.abs(p[0] - mid) - Math.abs(q[0] - mid)).slice(0, 3);
    const ext = [...k.max, ...k.min];
    if (mirrors(0, null, tolS)) k.symmetry = { kind: 'even' };
    else if (mirrors(0, 0, tolS)) k.symmetry = { kind: 'odd' };
    else {
      const axis = near(ext).find((p) => mirrors(p[0], null, tolS));
      if (axis) k.symmetry = { kind: 'axis', h: axis[0] };
      else {
        const point = near(k.inflection).find((p) => mirrors(p[0], p[1], tolS));
        if (point) k.symmetry = { kind: 'point', h: point[0], v: point[1] };
      }
    }
  }

  const t = (p) => [tidy(p[0], xspan), tidy(p[1], r)];
  k.max = k.max.map(t);
  k.min = k.min.map(t);
  k.roots = k.roots.map(t);
  k.inflection = k.inflection.map(t);
  if (k.yInt) k.yInt = t(k.yInt);
  if (k.vertex) k.vertex = t(k.vertex);
  if (k.symmetry.kind === 'axis') k.symmetry.h = tidy(k.symmetry.h, xspan);
  if (k.symmetry.kind === 'point') { k.symmetry.h = tidy(k.symmetry.h, xspan); k.symmetry.v = tidy(k.symmetry.v, r); }
  return k;
}

// ---- axes -----------------------------------------------------------------------------------

/** Tick label with just enough digits for `step`. */
function fmtTick(v, step, sci) {
  if (Math.abs(v) < step * 1e-6) return '0';
  let s;
  if (sci) {
    const digits = Math.min(8, Math.max(0, Math.floor(Math.log10(Math.abs(v))) - Math.floor(Math.log10(step))));
    s = v.toExponential(digits).replace('e+', 'e');
  } else {
    let prec = 10;
    for (let d = 0; d < 10; d++) {
      const x = step * 10 ** d;
      if (Math.abs(x - Math.round(x)) < 1e-6 * Math.max(x, 1)) { prec = d; break; }
    }
    s = v.toFixed(prec);
  }
  return s.replaceAll('-', MINUS);
}

/** About n nice major ticks in [lo, hi]: [[value, label]...]. */
export function ticks(a, b, n0) {
  const [lo, hi, n] = [Math.min(a, b), Math.max(a, b), Math.max(n0, 1)];
  const raw = (hi - lo) / n;
  if (!(Number.isFinite(raw) && raw > 0)) return [];
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw * 0.999) ?? 10 * mag;
  const first = Math.ceil(lo / step - 1e-6);
  const last = Math.floor(hi / step + 1e-6);
  const big = Math.max(Math.abs(lo), Math.abs(hi));
  const sci = !(big >= 1e-4 && big < 1e6);
  const out = [];
  for (let i = first; i <= last; i++) out.push([i * step, fmtTick(i * step, step, sci)]);
  return out;
}

/** Axis equal in a w x h px plot area: the range with more pixels per unit widens about its center. */
export function equalRanges(bx, by, w, h) {
  const kx = w / Math.abs(bx[1] - bx[0]);
  const ky = h / Math.abs(by[1] - by[0]);
  if (!(Number.isFinite(kx) && Number.isFinite(ky) && kx > 0 && ky > 0)) return [bx, by];
  const widen = (r, span) => {
    const c = (r[0] + r[1]) / 2;
    const half = (Math.sign(r[1] - r[0]) * span) / 2;
    return [c - half, c + half];
  };
  return kx > ky ? [widen(bx, w / ky), by] : [bx, widen(by, h / kx)];
}

/** Shift range `r` by `shift` spans, then scale it by `factor` about `anchor` (fraction from the low end). */
export function adjust(r, shift, factor, anchor) {
  const span = r[1] - r[0];
  const c = r[0] + span * (shift + anchor);
  const out = [c - span * anchor * factor, c + span * (1 - anchor) * factor];
  const ok = out.every(Number.isFinite) && Math.abs(out[1] - out[0]) > Math.max(Math.abs(out[0]), Math.abs(out[1])) * 1e-12;
  return ok ? out : r;
}

/** Text and line scale for a w x h board: 1 up to about 1000 x 600, growing up to 1.6. */
export const grow = (w, h) => Math.min(1.6, Math.max(1, Math.min(w / 1000, h / 600)));

/** Equation text for a w x h function list at `scale`: more room reads bigger. */
export const graphTextPx = (w, h, scale) => Math.min(56, Math.max(15, Math.min(w * 0.065, h * 0.06) * scale));

/** How much a line shrinks to fit `room`: 1 when it fits, never under 60 %. */
export const fitScale = (textPx, room) => (textPx <= room || room <= 0 ? 1 : Math.max(room / textPx, 0.6));

// ---- data cursor ----------------------------------------------------------------------------

/** Readout: "x = 1.5, y = 0.9975". */
export const readout = (p) => `x = ${num(p[0])}, y = ${num(p[1])}`;

// ---- history file ---------------------------------------------------------------------------

export const KEEP = 500;

/** One answer per line, tab-separated: input, math view, result, approximation. */
export function historyText(saved) {
  const clean = (s) => s.replace(/[\t\n\r]/g, ' ');
  return saved.map((s) => `${clean(s.expr)}\t${clean(s.input)}\t${clean(s.result)}\t${clean(s.approx)}\n`).join('');
}

export function parseHistory(text) {
  const out = [];
  for (const line of text.split('\n')) {
    const f = line.replace(/\r$/, '').split('\t');
    if (f.length < 3 || f[0].trim() === '') continue;
    out.push({ expr: f[0], input: f[1], result: f[2], approx: f.slice(3).join('\t') });
  }
  return out.slice(-KEEP);
}
