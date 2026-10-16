//! Classroom input and chart transformations, independent of GTK widgets.
use crate::builtins::io;
use crate::plot::{Data, Figure, LineStyle, Marker, Series};

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
    Ok(text.into())
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
    }
    #[test]
    fn statistics_and_missing_values() {
        let s = summary([Some(1.0), Some(2.0), None, Some(3.0), Some(f64::INFINITY)].into_iter());
        assert_eq!((s.count, s.skipped, s.mean, s.median, s.std), (3, 2, Some(2.0), Some(2.0), Some(1.0)));
        assert_eq!(summary([Some(2.0)].into_iter()).std, None);
        assert_eq!(summary([None].into_iter()).mean, None);
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
