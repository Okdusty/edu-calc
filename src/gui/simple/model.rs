//! Classroom input and chart transformations, independent of GTK widgets.
use crate::builtins::io;
use crate::plot::{Data, Figure, LineStyle, Marker, Series};
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
}

fn number(s: &str) -> Option<f64> {
    io::to_num(s.trim(), true).filter(|n| n.is_finite())
}

/// Ordinary least squares over complete, finite X/Y pairs.
pub fn linear_regression(grid: &[Vec<String>], xcol: Option<usize>, ycol: usize) -> Result<(f64, f64), &'static str> {
    let pairs: Vec<_> = grid.iter().skip(1).enumerate().filter_map(|(i, row)| {
        let x = match xcol {
            Some(c) => number(row.get(c)?),
            None => Some(i as f64 + 1.0),
        }?;
        let y = number(row.get(ycol)?)?;
        Some((x, y))
    }).collect();
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

pub fn summary(values: impl Iterator<Item = Option<f64>>) -> Summary {
    let all: Vec<_> = values.collect();
    let mut v: Vec<_> = all.iter().flatten().copied().filter(|n| n.is_finite()).collect();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let mean = (n > 0).then(|| v.iter().map(|x| x / n as f64).sum::<f64>());
    let std = (n > 1).then(|| {
        let mean = mean.unwrap();
        (v.iter().map(|x| (x - mean).powi(2) / (n - 1) as f64).sum::<f64>()).sqrt()
    });
    Summary {
        count: n,
        skipped: all.len() - n,
        mean,
        median: (n > 0).then(|| if n % 2 == 1 { v[n / 2] } else { v[n / 2 - 1] / 2.0 + v[n / 2] / 2.0 }),
        min: v.first().copied(),
        max: v.last().copied(),
        std,
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
