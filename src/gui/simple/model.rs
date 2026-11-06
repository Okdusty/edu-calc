//! Classroom input and chart transformations, independent of GTK widgets.
use crate::builtins::io;
use crate::plot::{Data, Figure, LineStyle, Marker, Series, cursor};
use crate::qalc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimpleTab {
    Calculator,
    Graph,
    Data,
}

impl SimpleTab {
    pub fn name(self) -> &'static str {
        match self {
            Self::Calculator => "Calculator",
            Self::Graph => "Graph",
            Self::Data => "Data",
        }
    }
}

/// Accept a bare expression or y = expression, never an implicit equation/script.
pub fn expression(text: &str) -> Result<String, String> {
    let text = text.trim();
    let text = if let Some((left, right)) = text.split_once('=') {
        if left.trim() != "y" {
            return Err("Use y = a function of x, for example y = x².".into());
        }
        right.trim()
    } else {
        text
    };
    if text.is_empty() {
        return Err("Enter a function of x.".into());
    }
    if text.contains(['=', '<', '>', ';', ':', '[', ']', '□']) {
        return Err("Complete a function of x. Equations, inequalities and scripts belong in Advanced.".into());
    }
    syntax(text)?;
    Ok(text.into())
}

const BINARY: &[char] = &['*', '/', '^', '×', '÷', '·', '∙'];

/// Bracket, operator, comma and decimal point mistakes, explained. qalc would quietly close
/// brackets or drop a stray operator and graph something else.
fn syntax(text: &str) -> Result<(), String> {
    let op = |c: char| BINARY.contains(&c) || matches!(c, '+' | '-' | '−');
    let cs: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    // the function name in front of cs[i] == '('
    let name = |i: usize| cs[..i].iter().rev().take_while(|c| c.is_alphanumeric()).collect::<Vec<_>>().into_iter().rev().collect::<String>();
    let mut open = vec![];
    for (i, &c) in cs.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| cs[j]);
        let next = cs.get(i + 1).copied();
        match c {
            '(' => open.push(i),
            ')' => {
                let Some(start) = open.pop() else {
                    return Err("There is a “)” without a “(” before it. Remove it, or add the missing “(”.".into());
                };
                if start + 1 == i {
                    let f = name(start);
                    let example = if f.is_empty() { "(x + 1)".into() } else { format!("{f}(x)") };
                    return Err(format!("“{f}()” has nothing inside the brackets. Put a value in, for example {example}."));
                }
            }
            ',' if open.is_empty() => {
                return Err("A comma only separates the values of a function, as in log(x, 2). For decimals use a point: 2.5".into());
            }
            ',' if matches!(prev, Some('(' | ',')) || matches!(next, None | Some(')' | ',')) => {
                return Err("A value is missing next to a comma. Write one on each side, as in log(x, 2).".into());
            }
            _ if op(c) => {
                match next {
                    None => return Err(format!("“{c}” at the end needs something after it, for example x {c} 1.")),
                    Some(n @ (')' | ',')) => return Err(format!("“{c}” right before “{n}” needs something after it.")),
                    // x**2 is qalc's power too
                    Some(n) if BINARY.contains(&n) && (c, n) != ('*', '*') => {
                        return Err(format!("“{c}{n}” is two operators in a row. Remove one, or put a number between them."));
                    }
                    _ => {}
                }
                if BINARY.contains(&c) && prev != Some('*') {
                    match prev {
                        None => return Err(format!("“{c}” at the start needs something before it, for example 2 {c} x.")),
                        Some(p @ ('(' | ',')) => return Err(format!("“{c}” right after “{p}” needs something before it.")),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(&start) = open.first() {
        let from = start - name(start).chars().count();
        let rest: String = cs[from..].iter().collect();
        let n = open.len();
        let what = if n == 1 { "a closing “)”".to_string() } else { format!("{n} closing “)”") };
        return Err(format!("“{rest}” is missing {what}."));
    }
    let number = cs.split(|c| !(c.is_ascii_digit() || *c == '.')).find(|run| run.iter().filter(|c| **c == '.').count() > 1);
    if let Some(run) = number {
        return Err(format!("“{}” has more than one decimal point.", run.iter().collect::<String>()));
    }
    Ok(())
}

/// A qalc error for the Simple graph, in a student's words.
pub fn explain(text: &str, err: &str) -> String {
    if err == qalc::UNITS_ERROR {
        return match unknown_word(text) {
            Some(w) => format!("EduCalc doesn't know “{w}”. Use x as the variable, with functions such as sin, sqrt or ln."),
            None => err.into(),
        };
    }
    let quoted = || err.split('"').nth(1).filter(|q| !q.is_empty());
    if err.starts_with("Misplaced operator") {
        let op = quoted().unwrap_or("an operator");
        return format!("“{op}” is in a place where it doesn't fit. Check what comes before and after it.");
    }
    if err.contains("is not a valid variable/function/unit") {
        return format!("EduCalc doesn't understand “{}” in a function.", quoted().unwrap_or(err));
    }
    if let Some(f) = err.starts_with("You need at least").then(|| err.split("in function ").nth(1)).flatten() {
        let f = f.trim_end_matches('.').trim_end_matches("()");
        let sig = qalc::catalog().iter().find(|i| i.kind == qalc::ItemKind::Function && i.name == f).map(|i| i.signature.clone());
        return match sig {
            Some(sig) => format!("{f} needs more values inside its brackets: {sig}"),
            None => format!("{f} needs more values inside its brackets."),
        };
    }
    if err.ends_with("timed out") {
        return "This function takes too long to calculate. Try a simpler one.".into();
    }
    err.into()
}

/// The first word that isn't x, π, e or known function names run together (sinx is sin x).
fn unknown_word(text: &str) -> Option<String> {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        let f = qalc::catalog().iter().filter(|i| i.kind == qalc::ItemKind::Function).map(|i| i.name.clone());
        f.chain(["x", "pi", "π", "e"].map(String::from)).collect()
    });
    let known = |w: &[char]| {
        // ok[j]: w[..j] splits into names
        let mut ok = vec![false; w.len() + 1];
        ok[0] = true;
        for j in 1..=w.len() {
            ok[j] = (0..j).any(|i| ok[i] && names.contains(&w[i..j].iter().collect::<String>()));
        }
        ok[w.len()]
    };
    let cs: Vec<char> = text.chars().collect();
    cs.split(|c| !c.is_alphabetic()).find(|w| !w.is_empty() && !known(w)).map(|w| w.iter().collect())
}

pub fn graph_figure() -> Figure {
    let mut f = Figure::new(1);
    let a = f.axes();
    a.grid = true;
    a.origin_axes = true;
    a.equal = true;
    a.xlim = Some((-10.0, 10.0));
    a.ylim = Some((-6.0, 6.0));
    a.xlabel = "x".into();
    a.ylabel = "y".into();
    f
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chart {
    Scatter,
    Line,
    Bar,
    Histogram,
}
impl Chart {
    pub fn from_index(i: u32) -> Self {
        match i {
            1 => Self::Line,
            2 => Self::Bar,
            3 => Self::Histogram,
            _ => Self::Scatter,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub skipped: usize,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub std: Option<f64>,
    pub sum: Option<f64>,
    /// quartiles by linear interpolation, like the calculator's quartile() and Excel's QUARTILE.INC
    pub q1: Option<f64>,
    pub q3: Option<f64>,
    /// the population's standard deviation; `std` is the sample's
    pub pop_std: Option<f64>,
    /// the most frequent values; empty when none repeats
    pub modes: Vec<f64>,
}

fn number(s: &str) -> Option<f64> {
    io::to_num(s.trim(), true).filter(|n| n.is_finite())
}

/// The complete, finite X/Y pairs; no x column = the row number.
pub fn pairs(grid: &[Vec<String>], xcol: Option<usize>, ycol: usize) -> Vec<(f64, f64)> {
    grid.iter().skip(1).enumerate().filter_map(|(i, row)| {
        let x = match xcol {
            Some(c) => number(row.get(c)?),
            None => Some(i as f64 + 1.0),
        }?;
        let y = number(row.get(ycol)?)?;
        Some((x, y))
    }).collect()
}

/// Pearson's r; None when either list doesn't vary. Scaled first, so large values don't overflow.
pub fn pearson(pairs: &[(f64, f64)]) -> Option<f64> {
    let n = pairs.len() as f64;
    let (mx, my) = (pairs.iter().map(|p| p.0 / n).sum::<f64>(), pairs.iter().map(|p| p.1 / n).sum::<f64>());
    let (sx, sy) = pairs.iter().fold((0f64, 0f64), |(a, b), p| (a.max((p.0 - mx).abs()), b.max((p.1 - my).abs())));
    if pairs.len() < 2 || sx == 0.0 || sy == 0.0 {
        return None;
    }
    let sum = |f: &dyn Fn(&(f64, f64)) -> f64| pairs.iter().map(f).sum::<f64>();
    let xy = sum(&|p| (p.0 - mx) / sx * ((p.1 - my) / sy));
    let (xx, yy) = (sum(&|p| ((p.0 - mx) / sx).powi(2)), sum(&|p| ((p.1 - my) / sy).powi(2)));
    Some((xy / (xx * yy).sqrt()).clamp(-1.0, 1.0)).filter(|r| r.is_finite())
}

/// Ranks from 1, ties sharing their average.
fn ranks(v: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..v.len()).collect();
    order.sort_by(|a, b| v[*a].total_cmp(&v[*b]));
    let mut out = vec![0.0; v.len()];
    let mut i = 0;
    while i < order.len() {
        let j = (i..order.len()).take_while(|k| v[order[*k]] == v[order[i]]).last().unwrap_or(i);
        for k in i..=j {
            out[order[k]] = (i + j) as f64 / 2.0 + 1.0;
        }
        i = j + 1;
    }
    out
}

/// Spearman's rank correlation: Pearson's r of the ranks.
pub fn spearman(pairs: &[(f64, f64)]) -> Option<f64> {
    let (x, y): (Vec<f64>, Vec<f64>) = pairs.iter().copied().unzip();
    pearson(&ranks(&x).into_iter().zip(ranks(&y)).collect::<Vec<_>>())
}

/// The population's covariance, like the calculator's cov(); the sample's is n / (n − 1) of it.
pub fn covariance(pairs: &[(f64, f64)]) -> Option<f64> {
    let n = pairs.len() as f64;
    let (mx, my) = (pairs.iter().map(|p| p.0 / n).sum::<f64>(), pairs.iter().map(|p| p.1 / n).sum::<f64>());
    (!pairs.is_empty()).then(|| pairs.iter().map(|p| (p.0 - mx) * (p.1 - my) / n).sum())
}

/// Curve through a table's points, by the Data tab's Fit menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitKind {
    None,
    Linear,
    Quadratic,
    Exponential,
    Power,
    Logarithmic,
}
impl FitKind {
    pub const NAMES: [&str; 6] = ["No fit", "Linear", "Quadratic", "Exponential", "Power", "Logarithmic"];
    pub fn from_index(i: u32) -> Self {
        match i {
            0 => Self::None,
            2 => Self::Quadratic,
            3 => Self::Exponential,
            4 => Self::Power,
            5 => Self::Logarithmic,
            _ => Self::Linear,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    kind: FitKind,
    /// Linear: m, b. Quadratic: a, b, c. Exponential and Power: a, b. Logarithmic: a, b.
    coef: Vec<f64>,
    /// 1 - SSres / SStot in y itself, also for the curves fitted through logarithms; None when y doesn't vary
    pub r2: Option<f64>,
}

impl Fit {
    pub fn at(&self, x: f64) -> f64 {
        let c = &self.coef;
        match self.kind {
            FitKind::Linear => c[0] * x + c[1],
            FitKind::Quadratic => (c[0] * x + c[1]) * x + c[2],
            FitKind::Exponential => c[0] * (c[1] * x).exp(),
            FitKind::Power => c[0] * x.powf(c[1]),
            FitKind::Logarithmic => c[0] + c[1] * x.ln(),
            FitKind::None => f64::NAN,
        }
    }

    pub fn equation(&self) -> String {
        let num = |v: f64| cursor::num(v).replace('-', "−");
        // " + 2" or " − 2"
        let then = |v: f64| format!(" {} {}", if v < 0.0 { "−" } else { "+" }, num(v.abs()));
        let c = &self.coef;
        match self.kind {
            FitKind::Linear => format!("y = {}x{}", num(c[0]), then(c[1])),
            FitKind::Quadratic => format!("y = {}x²{}x{}", num(c[0]), then(c[1]), then(c[2])),
            FitKind::Exponential => format!("y = {}·e^({}x)", num(c[0]), num(c[1])),
            FitKind::Power => format!("y = {}·x^{}", num(c[0]), num(c[1])),
            FitKind::Logarithmic => format!("y = {}{}·ln(x)", num(c[0]), then(c[1])),
            FitKind::None => String::new(),
        }
    }
}

/// Least squares of `kind` over the points; a reason when it doesn't apply. Exponential, power and
/// logarithmic go through logarithms, so they refuse points those can't take instead of dropping them.
pub fn fit(points: &[(f64, f64)], kind: FitKind) -> Result<Fit, String> {
    let line = |p: &[(f64, f64)]| least_squares(p).map_err(String::from);
    let need = if kind == FitKind::Quadratic { 3 } else { 2 };
    if points.len() < need {
        return Err(format!("it needs at least {need} complete rows"));
    }
    let positive = |ok: bool, what: &str| if ok { Ok(()) } else { Err(format!("every {what} must be above 0")) };
    let coef = match kind {
        FitKind::None => return Err("no fit picked".into()),
        FitKind::Linear => {
            let (m, b) = line(points)?;
            vec![m, b]
        }
        FitKind::Quadratic => {
            // in u = (x - mean) / spread, which keeps the normal equations well conditioned
            let n = points.len() as f64;
            let mx = points.iter().map(|p| p.0 / n).sum::<f64>();
            let sx = points.iter().map(|p| (p.0 - mx).abs()).fold(0.0, f64::max);
            let mut xs: Vec<f64> = points.iter().map(|p| p.0).collect();
            xs.sort_by(f64::total_cmp);
            xs.dedup();
            if xs.len() < 3 {
                return Err("it needs at least 3 different x values".into());
            }
            let mut a = nalgebra::Matrix3::zeros();
            let mut v = nalgebra::Vector3::zeros();
            for p in points {
                let u = (p.0 - mx) / sx;
                let row = nalgebra::Vector3::new(u * u, u, 1.0);
                a += row * row.transpose();
                v += row * p.1;
            }
            let s = a.lu().solve(&v).ok_or("the points don't determine a curve")?;
            let (qa, qb) = (s[0] / (sx * sx), s[1] / sx);
            vec![qa, qb - 2.0 * qa * mx, s[2] - qb * mx + qa * mx * mx]
        }
        FitKind::Exponential => {
            positive(points.iter().all(|p| p.1 > 0.0), "y")?;
            let (b, ln_a) = line(&points.iter().map(|p| (p.0, p.1.ln())).collect::<Vec<_>>())?;
            vec![ln_a.exp(), b]
        }
        FitKind::Power => {
            positive(points.iter().all(|p| p.0 > 0.0), "x")?;
            positive(points.iter().all(|p| p.1 > 0.0), "y")?;
            let (b, ln_a) = line(&points.iter().map(|p| (p.0.ln(), p.1.ln())).collect::<Vec<_>>())?;
            vec![ln_a.exp(), b]
        }
        FitKind::Logarithmic => {
            positive(points.iter().all(|p| p.0 > 0.0), "x")?;
            let (b, a) = line(&points.iter().map(|p| (p.0.ln(), p.1)).collect::<Vec<_>>())?;
            vec![a, b]
        }
    };
    if coef.iter().any(|c| !c.is_finite()) {
        return Err("the values exceed the numeric range".into());
    }
    let mut f = Fit { kind, coef, r2: None };
    let n = points.len() as f64;
    let my = points.iter().map(|p| p.1 / n).sum::<f64>();
    let total = points.iter().map(|p| (p.1 - my).powi(2)).sum::<f64>();
    let rest = points.iter().map(|p| (p.1 - f.at(p.0)).powi(2)).sum::<f64>();
    f.r2 = (total > 0.0).then_some((1.0 - rest / total).max(0.0));
    Ok(f)
}

/// A calculator answer as a table cell: its decimal form when it has one, with a plain minus.
pub fn answer_number(result: &str, approx: &str) -> String {
    (if approx.is_empty() { result } else { approx }).replace('−', "-")
}

/// The table's columns for the calculator.
#[derive(Debug, Default, PartialEq)]
pub struct TableColumns {
    /// (name, its numeric cells) for columns with a valid name and some numbers
    pub named: Vec<(String, Vec<f64>)>,
    /// columns with numbers whose heading can't be a variable name
    pub unnamed: Vec<String>,
    /// a row has a number in one named column and not in another: skipping blanks shifts the lists
    pub misaligned: bool,
}

/// What the calculator can see of the table. A first column of a repeated name wins.
pub fn table_columns(grid: &[Vec<String>]) -> TableColumns {
    let mut out = TableColumns::default();
    let names = grid.first().cloned().unwrap_or_default();
    let rows = grid.get(1..).unwrap_or_default();
    let mut used = vec![];
    for (c, name) in names.iter().enumerate() {
        let values: Vec<f64> = rows.iter().filter_map(|r| r.get(c).and_then(|s| number(s))).collect();
        if values.is_empty() {
            continue;
        }
        let name = name.trim();
        if name.is_empty() || io::valid_name(name) != name {
            out.unnamed.push(if name.is_empty() { format!("Column {}", c + 1) } else { name.to_string() });
        } else if out.named.iter().all(|(n, _)| n != name) {
            out.named.push((name.to_string(), values));
            used.push(c);
        }
    }
    out.misaligned = rows.iter().any(|r| {
        let filled = used.iter().filter(|c| r.get(**c).and_then(|s| number(s)).is_some()).count();
        filled > 0 && filled < used.len()
    });
    out
}

/// Ordinary least squares over complete, finite X/Y pairs.
pub fn least_squares(pairs: &[(f64, f64)]) -> Result<(f64, f64), &'static str> {
    if pairs.len() < 2 {
        return Err("need at least 2 valid X/Y pairs");
    }
    let n = pairs.len() as f64;
    let mx = pairs.iter().map(|(x, _)| x / n).sum::<f64>();
    let my = pairs.iter().map(|(_, y)| y / n).sum::<f64>();
    // Scale the centered values before squaring to avoid overflow.
    let sx = pairs.iter().map(|(x, _)| (x - mx).abs()).fold(0.0, f64::max);
    let sy = pairs.iter().map(|(_, y)| (y - my).abs()).fold(0.0, f64::max);
    if sx == 0.0 {
        return Err("X values must differ");
    }
    let slope = if sy == 0.0 { 0.0 } else {
        let xx = pairs.iter().map(|(x, _)| ((x - mx) / sx).powi(2)).sum::<f64>();
        let xy = pairs.iter().map(|(x, y)| ((x - mx) / sx) * ((y - my) / sy)).sum::<f64>();
        (xy / xx) * (sy / sx)
    };
    let intercept = (-slope).mul_add(mx, my);
    if !slope.is_finite() || !intercept.is_finite() || !sx.is_finite() || !sy.is_finite() {
        return Err("values exceed numeric range");
    }
    Ok((slope, intercept))
}

/// Linear interpolation between the sorted values at fraction `p` of the way through.
fn quantile(sorted: &[f64], p: f64) -> Option<f64> {
    let h = (sorted.len().checked_sub(1)? as f64) * p;
    let (lo, hi) = (h.floor() as usize, h.ceil() as usize);
    Some(sorted[lo] + (sorted[hi] - sorted[lo]) * (h - lo as f64))
}

pub fn summary(values: impl Iterator<Item = Option<f64>>) -> Summary {
    let all: Vec<_> = values.collect();
    let mut v: Vec<_> = all.iter().flatten().copied().filter(|n| n.is_finite()).collect();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let mean = (n > 0).then(|| v.iter().map(|x| x / n as f64).sum::<f64>());
    let spread = |div: usize| {
        let mean = mean.unwrap();
        (v.iter().map(|x| (x - mean).powi(2) / div as f64).sum::<f64>()).sqrt()
    };
    let std = (n > 1).then(|| spread(n - 1));
    let runs = v.chunk_by(|a, b| a == b).map(|run| (run[0], run.len()));
    let most = runs.clone().map(|(_, k)| k).max().unwrap_or(0);
    let modes = if most > 1 { runs.filter(|(_, k)| *k == most).map(|(x, _)| x).collect() } else { vec![] };
    Summary {
        count: n,
        skipped: all.len() - n,
        mean,
        median: (n > 0).then(|| if n % 2 == 1 { v[n / 2] } else { v[n / 2 - 1] / 2.0 + v[n / 2] / 2.0 }),
        min: v.first().copied(),
        max: v.last().copied(),
        std,
        sum: (n > 0).then(|| v.iter().sum()),
        q1: quantile(&v, 0.25),
        q3: quantile(&v, 0.75),
        pop_std: (n > 0).then(|| spread(n)),
        modes,
    }
}

pub fn data_chart(grid: &[Vec<String>], chart: Chart, xcol: Option<usize>, ycol: usize) -> (Figure, Summary, usize) {
    let names = grid.first().cloned().unwrap_or_default();
    let mut rows = grid.get(1..).unwrap_or_default();
    rows = &rows[..rows.iter().rposition(|r| r.iter().any(|s| !s.trim().is_empty())).map_or(0, |i| i + 1)];
    let at = |r: &Vec<String>, c: usize| r.get(c).and_then(|s| number(s));
    let stats = summary(rows.iter().map(|r| at(r, ycol)));
    let mut fig = Figure::new(1);
    let a = fig.axes();
    a.grid = true;
    a.origin_axes = true;
    a.xlabel = xcol.and_then(|c| names.get(c)).cloned().unwrap_or_else(|| "Row number".into());
    a.ylabel = names.get(ycol).cloned().unwrap_or_else(|| "Value".into());
    let mut skipped = 0;
    let series = if chart == Chart::Histogram {
        let values: Vec<_> = rows.iter().filter_map(|r| at(r, ycol)).collect();
        skipped = rows.len() - values.len();
        a.xlabel = a.ylabel.clone();
        a.ylabel = "Frequency".into();
        let lo = values.iter().copied().reduce(f64::min).unwrap_or(0.0);
        let hi = values.iter().copied().reduce(f64::max).unwrap_or(1.0);
        let bins = (values.len() as f64).sqrt().ceil().clamp(1.0, 40.0) as usize;
        let width = if hi > lo { (hi - lo) / bins as f64 } else { 1.0 };
        let start = if hi > lo { lo } else { lo - 0.5 };
        let x: Vec<_> = (0..bins).map(|i| start + (i as f64 + 0.5) * width).collect();
        let mut y = vec![0.0; bins];
        for v in values {
            y[(((v - start) / width).floor().max(0.0) as usize).min(bins - 1)] += 1.0;
        }
        Series::new(Data::Bar { x, y, width: width * 0.95 })
    } else {
        let (mut xs, mut ys) = (vec![], vec![]);
        for (i, row) in rows.iter().enumerate() {
            let x = xcol.map_or(Some(i as f64 + 1.0), |c| at(row, c));
            match (x, at(row, ycol)) {
                (Some(x), Some(y)) => {
                    xs.push(x);
                    ys.push(y);
                }
                _ => {
                    skipped += 1;
                    if chart == Chart::Line {
                        xs.push(f64::NAN);
                        ys.push(f64::NAN);
                    }
                }
            }
        }
        if chart == Chart::Bar {
            let mut sorted: Vec<_> = xs.iter().copied().filter(|x| x.is_finite()).collect();
            sorted.sort_by(f64::total_cmp);
            let width = sorted.windows(2).map(|v| v[1] - v[0]).filter(|d| *d > 0.0).reduce(f64::min).unwrap_or(1.0) * 0.8;
            Series::new(Data::Bar { x: xs, y: ys, width })
        } else {
            let mut s = Series::new(Data::Line { x: xs, y: ys });
            s.marker = Marker::Circle;
            s.line = if chart == Chart::Scatter { LineStyle::None } else { LineStyle::Solid };
            s
        }
    };
    a.series.push(series);
    (fig, stats, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classroom_expressions() {
        assert_eq!(expression(" y = 2x + 1 ").unwrap(), "2x + 1");
        assert_eq!(expression("x²").unwrap(), "x²");
        for s in ["x=2", "x²+y²=1", "y=x<2", "y=", "sin(□)", "y=x; quit"] {
            assert!(expression(s).is_err(), "{s}");
        }
        for s in ["-x + 1", "x**2", "2^-x", "log(x, 2)", "sin(cos(x))", "x! / 2", "e^(-x²)", "(x + 1)(x − 1)", "3.5x", "√(x)"] {
            assert_eq!(expression(s), Ok(s.into()), "{s}");
        }
    }
    #[test]
    fn syntax_mistakes_are_explained() {
        let err = |s: &str| expression(s).unwrap_err();
        assert!(err("2x +").contains("“+” at the end"), "{}", err("2x +"));
        assert!(err("* x").contains("“*” at the start"));
        assert!(err("x +* 2").contains("“+*” is two operators"));
        assert!(err("(^x)").contains("right after “(”"));
        assert!(err("sin(x +)").contains("right before “)”"));
        assert_eq!(err("sin(x"), "“sin(x” is missing a closing “)”.");
        assert_eq!(err("sin(cos(x"), "“sin(cos(x” is missing 2 closing “)”.");
        assert!(err("x + 1)").contains("without a “(”"));
        assert!(err("sin()").contains("“sin()” has nothing inside"));
        assert!(err("()").contains("for example (x + 1)"));
        assert!(err("2,5x").contains("For decimals use a point"));
        assert!(err("log(x,)").contains("missing next to a comma"));
        assert!(err("1.2.3x").contains("“1.2.3”"));
        assert!(err("x..2").contains("“..2” has more than one decimal point"));
    }
    #[test]
    fn qalc_errors_are_explained() {
        assert!(explain("abc", qalc::UNITS_ERROR).contains("“abc”"));
        assert!(explain("2foo(x)", qalc::UNITS_ERROR).contains("“foo”"));
        // run-together functions are fine, so nothing to name
        assert_eq!(explain("sinx + 30°", qalc::UNITS_ERROR), qalc::UNITS_ERROR);
        assert!(explain("x+", "Misplaced operator(s) \"+\" ignored").starts_with("“+” is in a place"));
        assert!(explain("#", "\"#\" is not a valid variable/function/unit.").contains("“#”"));
        let e = explain("log()", "You need at least 1 argument (Value) in function log().");
        assert!(e.starts_with("log needs more values"), "{e}");
        assert!(explain("x", "plot sampling timed out").contains("too long"));
        assert_eq!(explain("x", "something else"), "something else");
    }
    /// Writes web/test/parity-model.json: what the desktop computes for the Data and Graph tabs, which
    /// the web port must match (web/test/parity-model.test.mjs, parity.mjs). After a change here:
    /// `cargo test export_model_parity -- --ignored`.
    #[test]
    #[ignore = "writes web/test/parity-model.json"]
    fn export_model_parity() {
        use crate::gui::canvas;
        use crate::gui::keypad;
        use crate::plot::{analysis, mathtext, render};
        fn j(s: &str) -> String {
            let mut o = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => o += "\\\"",
                    '\\' => o += "\\\\",
                    '\n' => o += "\\n",
                    '\r' => o += "\\r",
                    '\t' => o += "\\t",
                    c if (c as u32) < 0x20 => o += &format!("\\u{:04x}", c as u32),
                    c => o.push(c),
                }
            }
            o + "\""
        }
        fn n(v: f64) -> String { if v.is_finite() { format!("{v:e}") } else { "null".into() } }
        fn on(v: Option<f64>) -> String { v.map_or("null".into(), n) }
        fn arr(v: Vec<String>) -> String { format!("[{}]", v.join(",")) }
        fn nums(v: &[f64]) -> String { arr(v.iter().map(|x| n(*x)).collect()) }
        fn strs<S: AsRef<str>>(v: &[S]) -> String { arr(v.iter().map(|s| j(s.as_ref())).collect()) }
        fn grid(g: &[Vec<String>]) -> String { arr(g.iter().map(|r| strs(r)).collect()) }
        fn points(v: &[[f64; 2]]) -> String { arr(v.iter().map(|p| nums(p)).collect()) }
        let g = |rows: &[&[&str]]| -> Vec<Vec<String>> { rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect() };
        let mut out = vec![];

        let values = [0.0, -0.0, 1.23456, 0.944, -2.5, 1234567.0, 0.00001234, 100.0, 999.95, 9999.5, 99999.5, 0.0001, 0.00009999, 1e300, 1e-300, 3.14159265, 2.5e-5, 123456.7, 999999.5, 1000000.0, -0.02, 2.02, 1.0 / 3.0, 7.05, 0.9991049325];
        out.push(format!("\"num\":{}", arr(values.iter().map(|v| format!("[{},{}]", n(*v), j(&cursor::num(*v)))).collect())));
        out.push(format!("\"fmtNum\":{}", arr([1.5, 0.1, 1e15, 1e-6, 1234567.0, 100.0, -3.25, 2.5e-5, 1e21].iter().map(|v| format!("[{},{}]", n(*v), j(&io::fmt_num(*v)))).collect())));
        let cells = ["1.5", " -2e3 ", "1,5", "1,5,2", "Inf", "-inf", "NaN", "", "abc", "1e", ".5", "5.", "+7", "0x10", "1 2", "infinity"];
        out.push(format!("\"toNum\":{}", arr(cells.iter().map(|s| format!("[{},{},{}]", j(s), io::to_num(s, false).map_or("\"none\"".to_string(), |v| if v.is_nan() { "\"nan\"".into() } else if v.is_infinite() { format!("\"{v}\"") } else { n(v) }), io::to_num(s, true).map_or("\"none\"".to_string(), |v| if v.is_nan() { "\"nan\"".into() } else if v.is_infinite() { format!("\"{v}\"") } else { n(v) }))).collect())));
        out.push(format!("\"validName\":{}", arr(["X", "Height (cm)", "2nd", "a b c", "  y ", "ä", "_x", "v_0"].iter().map(|s| format!("[{},{}]", j(s), j(&io::valid_name(s)))).collect())));
        let texts = ["a,b\n1,2\n", "1\t2,5\n3\t4\n", "1;2,5\n3;4", "1 2\n3 4\n", "\"a,b\",2\n\"say \"\"hi\"\"\",3\n", "\u{feff}x,y\n1,2", "Title\nday;temp\n1;20,5\n2;22\n", "1,5\n2,5\n", "a\n\nb\n", "x y\n1.5 2,5\n", "1\r\n2\r\n", "1,2\r\n3,4\r\n", ""];
        out.push(format!("\"parseText\":{}", arr(texts.iter().map(|s| { let p = io::parse_text(s, None); let (h, b) = io::split_header(p.clone()); format!("[{},{},{},{}]", j(s), grid(&p), grid(&h), grid(&b)) }).collect())));

        let tpls = [("sin(□)", ""), ("7", "□"), ("sin(□)", "x+1"), ("□²", "x+1"), ("√(□)", "2"), ("1/□", "(a+b)"), ("□^□", "x"), ("□^□", "2x+1"), ("integrate(□, □, □, x)", "x²"), ("sin(□)", "□"), ("abs(□)", "□+1"), (" to ", ""), ("root(□, □)", "27"), ("mean(□)", "X"), ("□!", "x.5"), ("log(□, □)", "a_b")];
        out.push(format!("\"expand\":{}", arr(tpls.iter().map(|(t, s)| { let (o, k) = keypad::expand(t, s); format!("[{},{},{},{}]", j(t), j(s), j(&o), k.map_or("null".into(), |k| k.to_string())) }).collect())));

        let inputs = [" y = 2x + 1 ", "x²", "x=2", "x²+y²=1", "y=x<2", "y=", "sin(□)", "y=x; quit", "-x + 1", "x**2", "2^-x", "log(x, 2)", "sin(cos(x))", "x! / 2", "e^(-x²)", "(x + 1)(x − 1)", "3.5x", "√(x)", "2x +", "* x", "x +* 2", "(^x)", "sin(x +)", "sin(x", "sin(cos(x", "x + 1)", "sin()", "()", "2,5x", "log(x,)", "1.2.3x", "x..2", "y = ", "=", "x;", "[1,2]", "x:2", "1 + (2", "2 × ÷ 3", "÷ 2", "x −", "(,x)", "log(,x)"];
        out.push(format!("\"expression\":{}", arr(inputs.iter().map(|s| match expression(s) { Ok(e) => format!("[{},{},null]", j(s), j(&e)), Err(e) => format!("[{},null,{}]", j(s), j(&e)) }).collect())));
        // whole graph rows through the engine, errors in a student's words
        let o = qalc::Options { angle: "deg".into(), ..Default::default() };
        let rows = ["foo(x)", "abc", "x+t", "x+y", "2x+", "sin(", "sin()", "#", "x²+", "log()", "sinx + 30°", "t", "a·x", "m", "x km", "1/x", "y = sin(x)", "binomial(x)", "root(x)", "gcd(1)", "3 m + x", "x + 1 s"];
        out.push(format!("\"graphRows\":{}", arr(rows.iter().map(|s| {
            let r = expression(s).and_then(|e| qalc::classroom_curve(&e, &o, -5.0, 5.0).map_err(|err| explain(&e, &err)));
            match r { Ok((x, _)) => format!("[{},{},null]", j(s), x.len()), Err(e) => format!("[{},null,{}]", j(s), j(&e)) }
        }).collect())));

        let ticks: Vec<String> = [(-10.0, 10.0, 10), (0.0, 1.0, 4), (-6.0, 6.0, 15), (-13.5, 13.5, 16), (0.0, 12.2, 7), (1e6, 2e6, 5), (0.00001, 0.00002, 4), (-1.0, 1.0, 11), (2.1, 12.2, 9), (1e-7, 1e-6, 5), (0.0, 1e7, 6), (5.0, 5.0, 3), (-0.3, 6.3, 12)]
            .iter()
            .map(|&(a, b, k)| { let (t, _) = render::ticks((a, b), false, k); format!("[{},{},{},{},{}]", n(a), n(b), k, nums(&t.iter().map(|p| p.0).collect::<Vec<_>>()), strs(&t.iter().map(|p| p.1.clone()).collect::<Vec<_>>())) })
            .collect();
        out.push(format!("\"ticks\":{}", arr(ticks)));
        let eq: Vec<String> = [((-10.0, 10.0), (-6.0, 6.0), 500.0, 600.0), ((-10.0, 10.0), (-6.0, 6.0), 1200.0, 300.0), ((0.0, 1.0), (0.0, 100.0), 300.0, 300.0), ((-1.0, 1.0), (1.0, -1.0), 400.0, 200.0)]
            .iter()
            .map(|&(x, y, w, h)| { let (a, b) = render::equal_ranges(x, y, w, h); format!("[{},{},{},{},{},{}]", nums(&[x.0, x.1]), nums(&[y.0, y.1]), n(w), n(h), nums(&[a.0, a.1]), nums(&[b.0, b.1])) })
            .collect();
        out.push(format!("\"equalRanges\":{}", arr(eq)));
        let adj: Vec<String> = [((0.0, 10.0), 0.1, 1.0, 0.0), ((0.0, 10.0), 0.0, 0.5, 0.5), ((0.0, 10.0), 0.0, 2.0, 0.0), ((-3.0, 7.0), -0.25, 1.25, 0.3), ((0.0, 10.0), f64::INFINITY, 1.0, 0.0), ((0.0, 10.0), 0.0, 0.0, 0.5)]
            .iter()
            .map(|&(r, s, f, a)| { let o = canvas::adjust(r, s, f, a, false); format!("[{},{},{},{},{}]", nums(&[r.0, r.1]), if s.is_finite() { n(s) } else { "\"inf\"".into() }, n(f), n(a), nums(&[o.0, o.1])) })
            .collect();
        out.push(format!("\"adjust\":{}", arr(adj)));
        let marks = ["x^2 + 1", "x^{10}", "v_0 = a_{ij}", "e^{-t/\\tau_0}", "e^(−x / 2)", "x^\\alpha", "\\int_0^\\infty \\alpha\\Omega", "x^2^3", "x_{}^2", "\\bf{bold}\\it x\\_1 \\{a\\}", "\\foo √x²", "a^", "{x", "x^{2", "\\", "", "Height (cm)", "T_{max}", "m/s^2", "\\Delta t", "x_1^2", "\\sum_i"];
        out.push(format!("\"mathRuns\":{}", arr(marks.iter().map(|s| format!("[{},{}]", j(s), arr(mathtext::parse(s).iter().map(|r| format!("[{},{},{},{}]", j(&r.text), n(r.scale), n(r.shift), r.stack)).collect()))).collect())));

        // Data tab
        let many: Vec<Vec<String>> = std::iter::once(vec!["t".to_string(), "v".to_string()]).chain((1..=30).map(|i| vec![i.to_string(), format!("{}", (i * i % 17) as f64 + 0.5 * i as f64)])).collect();
        let grids = [
            g(&[&["X", "Y"], &["1", "2.1"], &["2", "3.9"], &["3", "6.2"], &["4", "7.8"], &["5", "10.1"], &["6", "12.2"], &["", ""]]),
            g(&[&["X", "Y"], &["1", "1,5"], &["2", "oops"], &["3", "5,5"], &["NaN", "7"], &["", ""]]),
            g(&[&["X", "Y"], &["1", "3"], &["2", "3"], &["3", "3"]]),
            g(&[&["X", "Y"], &["1", "4"], &["", ""]]),
            many,
            g(&[&["a", "b"], &["-2", "4"], &["-1", "1"], &["0", "0"], &["1", "1"], &["2", "4"]]),
            g(&[&["a", "b"], &["1", "1"], &["2", "8"], &["3", "27"], &["4", "64"], &["5", "125"]]),
            g(&[&["a", "b"], &["1", "2"], &["2", "2"], &["2", "3"], &["3", "3"], &["3", "3"]]),
            g(&[&["big", "bigger"], &["1e200", "2e200"], &["2e200", "4e200"], &["3e200", "6e200"]]),
            g(&[&["Height (cm)", "", "m", "A", "A"], &["1", "2", "3", "4", "5"], &["2", "", "4", "x", "6"]]),
            g(&[&["X", "Y"], &["", ""], &["", ""]]),
            g(&[&["X", "Y"], &["1", "2"], &["", "3"], &["4", ""]]),
            g(&[&["x"], &["1"], &["1"], &["1"], &["2"]]),
        ];
        let mut data = vec![];
        for gr in &grids {
            let width = gr[0].len();
            let table = table_columns(gr);
            let named = arr(table.named.iter().map(|(n1, v)| format!("[{},{}]", j(n1), nums(v))).collect());
            let mut views = vec![];
            for x in std::iter::once(None).chain((0..width).map(Some)) {
                for y in 0..width {
                    let ps = pairs(gr, x, y);
                    let mut charts = vec![];
                    for c in 0..4u32 {
                        let (mut f, s, skipped) = data_chart(gr, Chart::from_index(c), x, y);
                        let a = f.axes();
                        let series = match &a.series[0].data {
                            Data::Line { x, y } => format!("{{\"kind\":\"line\",\"x\":{},\"y\":{},\"line\":{},\"marker\":{}}}", nums(x), nums(y), a.series[0].line != LineStyle::None, a.series[0].marker != Marker::None),
                            Data::Bar { x, y, width } => format!("{{\"kind\":\"bar\",\"x\":{},\"y\":{},\"width\":{}}}", nums(x), nums(y), n(*width)),
                            _ => panic!("chart kind"),
                        };
                        charts.push(format!(
                            "{{\"xlabel\":{},\"ylabel\":{},\"skipped\":{skipped},\"series\":{series},\"count\":{},\"statSkipped\":{},\"mean\":{},\"median\":{},\"min\":{},\"max\":{},\"std\":{},\"sum\":{},\"q1\":{},\"q3\":{},\"popStd\":{},\"modes\":{}}}",
                            j(&a.xlabel), j(&a.ylabel), s.count, s.skipped, on(s.mean), on(s.median), on(s.min), on(s.max), on(s.std), on(s.sum), on(s.q1), on(s.q3), on(s.pop_std), nums(&s.modes)
                        ));
                    }
                    let fits: Vec<String> = (1..=5).map(|k| match fit(&ps, FitKind::from_index(k)) {
                        Ok(f) => format!("{{\"equation\":{},\"r2\":{},\"at\":{}}}", j(&f.equation()), on(f.r2), nums(&ps.iter().map(|p| f.at(p.0)).collect::<Vec<_>>())),
                        Err(e) => format!("{{\"error\":{}}}", j(&e)),
                    }).collect();
                    let ls = match least_squares(&ps) { Ok((m, b)) => nums(&[m, b]), Err(e) => j(e) };
                    views.push(format!(
                        "{{\"x\":{},\"y\":{y},\"pairs\":{},\"pearson\":{},\"spearman\":{},\"cov\":{},\"leastSquares\":{ls},\"fits\":{},\"charts\":{}}}",
                        x.map_or("null".into(), |v| v.to_string()), arr(ps.iter().map(|p| nums(&[p.0, p.1])).collect()), on(pearson(&ps)), on(spearman(&ps)), on(covariance(&ps)), arr(fits), arr(charts)
                    ));
                }
            }
            data.push(format!("{{\"grid\":{},\"table\":{{\"named\":{named},\"unnamed\":{},\"misaligned\":{}}},\"views\":{}}}", grid(gr), strs(&table.unnamed), table.misaligned, arr(views)));
        }
        out.push(format!("\"data\":{}", arr(data)));

        // key points of real engine samples
        let curves = ["x²", "(x-1)(x-3)", "x³ - 3x", "sin(x)", "cos(x)", "tan(x)", "1/x", "ln(x)", "sqrt(x)", "e^x", "abs(x)", "floor(x)", "2x + 1", "3", "sin(x)/x", "x^4 - 4x^2", "1/(x^2 - 1)", "(x-1)^3 + 2", "cos(x - 1) + 0.3(x - 1)^2"];
        let rad = qalc::Options::default();
        let mut keys = vec![];
        for e in curves {
            let (x0, y0) = qalc::classroom_curve(e, &rad, -6.0, 6.0).unwrap();
            let (x, y): (Vec<f64>, Vec<f64>) = x0.iter().zip(&y0).step_by(4).map(|(a, b)| (*a, *b)).unzip();
            let k = analysis::analyze(&x, &y);
            let (nx, ny) = (arr(x.iter().map(|v| n(*v)).collect()), arr(y.iter().map(|v| if v.is_infinite() { format!("\"{v}\"") } else { n(*v) }).collect()));
            let body = match k {
                None => "null".to_string(),
                Some(k) => format!("{{\"max\":{},\"min\":{},\"roots\":{},\"inflection\":{},\"yInt\":{},\"vertex\":{},\"symmetry\":{}}}", points(&k.max), points(&k.min), points(&k.roots), points(&k.inflection), k.y_int.map_or("null".into(), |p: [f64; 2]| nums(&p)), k.vertex.map_or("null".into(), |p: [f64; 2]| nums(&p)), j(&k.symmetry.text())),
            };
            let (bx, by) = crate::plot::builtins::break_jumps(&x, &y);
            keys.push(format!("{{\"expr\":{},\"x\":{nx},\"y\":{ny},\"breaks\":{},\"points\":{},\"keys\":{body}}}", j(e), bx.iter().filter(|v| v.is_nan()).count(), by.len()));
        }
        out.push(format!("\"keyPoints\":{}", arr(keys)));

        // spreadsheets, read as the Data tab reads them
        let root = env!("CARGO_MANIFEST_DIR");
        let mut books = vec![];
        for f in ["tests/data/book.xlsx", "tests/data/book.xls", "tests/data/book.ods", "demo/weather.csv"] {
            let p = std::path::PathBuf::from(root).join(f);
            let names = if io::is_book(&p) { io::sheet_names(&p).unwrap() } else { vec![] };
            let mut sheets = vec![];
            for s in if names.is_empty() { vec![None] } else { names.iter().map(|n| Some(n.as_str())).collect() } {
                let cells = io::read_cells(&p, s, None).unwrap();
                sheets.push(format!("[{},{}]", s.map_or("null".to_string(), j), grid(&cells)));
            }
            books.push(format!("{{\"file\":{},\"sheets\":{}}}", j(f), arr(sheets)));
        }
        out.push(format!("\"books\":{}", arr(books)));

        std::fs::write(format!("{root}/web/test/parity-model.json"), format!("{{\n{}\n}}\n", out.join(",\n"))).unwrap();
    }

    fn linear_regression(grid: &[Vec<String>], xcol: Option<usize>, ycol: usize) -> Result<(f64, f64), &'static str> {
        least_squares(&pairs(grid, xcol, ycol))
    }
    #[test]
    fn answers_become_cells() {
        assert_eq!(answer_number("5/4", "1.25"), "1.25");
        assert_eq!(answer_number("−3", ""), "-3");
        assert_eq!(io::to_num(&answer_number("−1/2", "−0.5"), true), Some(-0.5));
    }
    #[test]
    fn more_statistics_match_the_calculator() {
        let o = qalc::Options::default();
        let values = [1.0, 2.0, 2.0, 3.0, 4.0, 7.0, 9.0, 12.0];
        let s = summary(values.iter().map(|v| Some(*v)));
        let list = "[1, 2, 2, 3, 4, 7, 9, 12]";
        let calc = |e: String| {
            let a = qalc::classroom_eval(&e, &o).unwrap();
            let shown = if a.approx.is_empty() { a.result } else { a.approx };
            shown.replace('−', "-").parse::<f64>().unwrap_or_else(|_| panic!("{e}: {shown}"))
        };
        for (got, f) in [(s.sum, "total"), (s.mean, "mean"), (s.median, "median"), (s.std, "stdev"), (s.pop_std, "stdevp")] {
            assert!((got.unwrap() - calc(format!("{f}({list})"))).abs() < 1e-8, "{f}");
        }
        assert!((s.q1.unwrap() - calc(format!("quartile({list}, 1)"))).abs() < 1e-9);
        assert!((s.q3.unwrap() - calc(format!("quartile({list}, 3)"))).abs() < 1e-9);
        assert_eq!(s.modes, [2.0]);
        assert_eq!(summary([Some(1.0), Some(2.0)].into_iter()).modes, Vec::<f64>::new());
        assert_eq!(summary([Some(1.0), Some(1.0), Some(2.0), Some(2.0)].into_iter()).modes, [1.0, 2.0]);
        assert_eq!(summary([None].into_iter()).sum, None);

        let p = [(1.0, 2.0), (2.0, 4.0), (3.0, 5.0), (4.0, 9.0)];
        let (x, y) = ("[1, 2, 3, 4]", "[2, 4, 5, 9]");
        assert!((pearson(&p).unwrap() - calc(format!("pearson({x}, {y})"))).abs() < 1e-8);
        assert!((spearman(&p).unwrap() - calc(format!("spearman({x}, {y})"))).abs() < 1e-9);
        assert!((covariance(&p).unwrap() - calc(format!("cov({x}, {y})"))).abs() < 1e-9);
        let line = fit(&p, FitKind::Linear).unwrap();
        assert!((line.at(0.0) - calc(format!("intercept({x}, {y})"))).abs() < 1e-9);
        assert!((line.at(1.0) - line.at(0.0) - calc(format!("slope({x}, {y})"))).abs() < 1e-9);
    }
    #[test]
    fn correlation_edges() {
        assert_eq!(pearson(&[(1.0, 2.0), (2.0, 4.0), (3.0, 6.0)]), Some(1.0));
        assert_eq!(pearson(&[(1.0, 3.0), (2.0, 2.0), (3.0, 1.0)]), Some(-1.0));
        assert_eq!(pearson(&[(1.0, 2.0), (1.0, 3.0)]), None, "x doesn't vary");
        assert_eq!(pearson(&[(1.0, 2.0), (2.0, 2.0)]), None, "y doesn't vary");
        assert_eq!(pearson(&[(1.0, 2.0)]), None);
        assert_eq!(pearson(&[]), None);
        assert_eq!(pearson(&[(1e200, 2e200), (2e200, 4e200), (3e200, 6e200)]), Some(1.0));
        // monotonic but not straight, with a tie
        assert_eq!(spearman(&[(1.0, 1.0), (2.0, 8.0), (3.0, 27.0), (4.0, 64.0)]), Some(1.0));
        assert_eq!(ranks(&[10.0, 20.0, 20.0, 5.0]), [2.0, 3.5, 3.5, 1.0]);
    }
    #[test]
    fn fits_recover_their_curves() {
        let on = |f: &dyn Fn(f64) -> f64| (1..=6).map(|i| (i as f64, f(i as f64))).collect::<Vec<_>>();
        for (kind, curve, eq) in [
            (FitKind::Linear, on(&|x| 2.0 * x - 0.5), "y = 2x − 0.5"),
            (FitKind::Quadratic, on(&|x| 0.5 * x * x - 3.0 * x + 1.0), "y = 0.5x² − 3x + 1"),
            (FitKind::Exponential, on(&|x| 1.5 * (0.5 * x).exp()), "y = 1.5·e^(0.5x)"),
            (FitKind::Power, on(&|x| 3.0 * x.powf(2.0)), "y = 3·x^2"),
            (FitKind::Logarithmic, on(&|x| 2.0 + 4.0 * x.ln()), "y = 2 + 4·ln(x)"),
        ] {
            let f = fit(&curve, kind).unwrap();
            assert_eq!(f.equation(), eq);
            assert!((f.r2.unwrap() - 1.0).abs() < 1e-9, "{kind:?}");
            for (x, y) in &curve {
                assert!((f.at(*x) - y).abs() < 1e-6, "{kind:?} at {x}");
            }
        }
        // a poor fit says so, in y itself
        let r2 = fit(&[(1.0, 1.0), (2.0, 5.0), (3.0, 1.0), (4.0, 5.0)], FitKind::Linear).unwrap().r2.unwrap();
        assert!(r2 < 0.3, "{r2}");
        assert_eq!(fit(&[(1.0, 2.0), (2.0, 2.0)], FitKind::Linear).unwrap().r2, None);
    }
    #[test]
    fn fits_say_why_not() {
        let two = [(1.0, 1.0), (2.0, 2.0)];
        assert!(fit(&two[..1], FitKind::Linear).unwrap_err().contains("at least 2"));
        assert!(fit(&two, FitKind::Quadratic).unwrap_err().contains("at least 3"));
        assert!(fit(&[(1.0, 1.0), (1.0, 2.0), (2.0, 3.0)], FitKind::Quadratic).unwrap_err().contains("3 different"));
        assert!(fit(&[(1.0, 1.0), (1.0, 2.0)], FitKind::Linear).unwrap_err().contains("must differ"));
        assert!(fit(&[(1.0, -1.0), (2.0, 2.0)], FitKind::Exponential).unwrap_err().contains("every y"));
        assert!(fit(&[(0.0, 1.0), (2.0, 2.0)], FitKind::Power).unwrap_err().contains("every x"));
        assert!(fit(&[(-1.0, 1.0), (2.0, 2.0)], FitKind::Logarithmic).unwrap_err().contains("every x"));
    }
    #[test]
    fn table_columns_for_the_calculator() {
        let grid = |rows: &[&[&str]]| rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect::<Vec<_>>()).collect::<Vec<_>>();
        let t = table_columns(&grid(&[&["X", "Y"], &["1", "2,5"], &["2", "4"], &["", ""]]));
        assert_eq!(t.named, [("X".to_string(), vec![1.0, 2.0]), ("Y".to_string(), vec![2.5, 4.0])]);
        assert!(!t.misaligned && t.unnamed.is_empty());
        // only X has data so far: nothing to line up with
        let t = table_columns(&grid(&[&["X", "Y"], &["1", ""], &["2", ""]]));
        assert_eq!(t.named.len(), 1);
        assert!(!t.misaligned);
        // blanks on different rows would pair the wrong values
        let t = table_columns(&grid(&[&["X", "Y"], &["1", "2"], &["", "3"], &["4", ""]]));
        assert!(t.misaligned);
        // text counts as blank; bad headings are reported, repeats ignored
        let t = table_columns(&grid(&[&["Height (cm)", "", "A", "A"], &["1", "2", "3", "4"]]));
        assert_eq!(t.unnamed, ["Height (cm)", "Column 2"]);
        assert_eq!(t.named, [("A".to_string(), vec![3.0])]);
        assert_eq!(table_columns(&[]), TableColumns::default());
    }
    #[test]
    fn statistics_and_missing_values() {
        let s = summary([Some(1.0), Some(2.0), None, Some(3.0), Some(f64::INFINITY)].into_iter());
        assert_eq!((s.count, s.skipped, s.mean, s.median, s.std), (3, 2, Some(2.0), Some(2.0), Some(1.0)));
        assert_eq!(summary([Some(2.0)].into_iter()).std, None);
        assert_eq!(summary([None].into_iter()).mean, None);
    }
    #[test]
    fn regression_uses_complete_pairs_and_selected_columns() {
        let g: Vec<Vec<String>> = [["X", "Y"], ["1", "1,5"], ["2", "oops"], ["3", "5,5"], ["NaN", "7"], ["", ""]]
            .map(|r| r.map(String::from).to_vec()).to_vec();
        assert_eq!(linear_regression(&g, Some(0), 1), Ok((2.0, -0.5)));
        assert_eq!(linear_regression(&g[..4], None, 1), Ok((2.0, -0.5)));
        assert_eq!(linear_regression(&g, Some(1), 0), Ok((0.5, 0.25)));
    }
    #[test]
    fn regression_handles_degenerate_and_large_values() {
        let grid = |pairs: [[&str; 2]; 3]| pairs.map(|r| r.map(String::from).to_vec()).to_vec();
        assert_eq!(linear_regression(&grid([["X", "Y"], ["1", "3"], ["2", "3"]]), Some(0), 1), Ok((0.0, 3.0)));
        assert!(linear_regression(&grid([["X", "Y"], ["1", "3"], ["1", "4"]]), Some(0), 1).unwrap_err().contains("differ"));
        assert!(linear_regression(&grid([["X", "Y"], ["1", "3"], ["2", "inf"]]), Some(0), 1).is_err());
        assert_eq!(linear_regression(&grid([["X", "Y"], ["1e200", "2e200"], ["2e200", "4e200"]]), Some(0), 1), Ok((2.0, 0.0)));
    }
    #[test]
    fn chart_data_and_histogram_counts() {
        let g: Vec<Vec<String>> = [["X", "Y"], ["1", "1,5"], ["2", "oops"], ["3", "3"], ["", ""]].map(|r| r.map(String::from).to_vec()).to_vec();
        let (f, s, skipped) = data_chart(&g, Chart::Line, Some(0), 1);
        assert_eq!((s.count, skipped), (2, 1));
        let Data::Line { y, .. } = &f.axes[0].series[0].data else { panic!() };
        assert_eq!(y[0], 1.5);
        assert!(y[1].is_nan());
        assert_eq!(y[2], 3.0);
        for chart in [Chart::Scatter, Chart::Bar, Chart::Histogram] {
            let (f, _, skipped) = data_chart(&g, chart, None, 1);
            assert_eq!(skipped, 1);
            if chart == Chart::Histogram {
                let Data::Bar { y, .. } = &f.axes[0].series[0].data else { panic!() };
                assert_eq!(y.iter().sum::<f64>(), 2.0);
            }
        }
    }
}
