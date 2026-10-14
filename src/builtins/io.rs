//! Data files: csvread/csvwrite, dlmread/dlmwrite, readmatrix/writematrix, xlsread, importdata,
//! readtable. Everything goes through a grid of cell texts (shared with the data editor):
//! delimited text (separator sniffed: tab, semicolon, comma, else whitespace; "quoted" fields;
//! decimal commas unless commas separate) or a workbook sheet through calamine (.xlsx .xlsm .xlsb
//! .xls .ods; dates read as their Excel serial number). Fully blank rows are dropped.
//! readmatrix and friends skip leading rows without a number (titles, column names); the last of
//! them names the columns. Text and blank cells are NaN (csvread/dlmread: 0, like Octave).
//! readtable returns a struct of columns, our stand-in for tables: field names made valid like
//! matlab.lang.makeValidName, numeric column vectors, or a cell of strings for text columns.
//! ponytail: struct fields list alphabetically, not in column order; a table type needs Value support.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{R, is_opt, nargs, num, real_arg};
use crate::lang::Interp;
use crate::value::{Builtin, Matrix, Value};

/// Rows of cell texts (rows may differ in length).
pub type Grid = Vec<Vec<String>>;

pub fn lookup(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "csvread" => |_, a, _| {
            nargs("csvread", a, 1, 3)?;
            raw("csvread", &a[0], Some(','), a.get(1), a.get(2))
        },
        "dlmread" => |_, a, _| {
            nargs("dlmread", a, 1, 4)?;
            let sep = a.get(1).map(|v| sep_arg("dlmread", v)).transpose()?.flatten();
            raw("dlmread", &a[0], sep, a.get(2), a.get(3))
        },
        "csvwrite" => |_, a, _| {
            nargs("csvwrite", a, 2, 2)?;
            write_num("csvwrite", &a[0], &a[1], ',')
        },
        "dlmwrite" => |_, a, _| {
            nargs("dlmwrite", a, 2, 4)?;
            // dlmwrite(f, M, sep) or dlmwrite(f, M, 'delimiter', sep)
            let s = if a.get(2).is_some_and(|k| is_opt(k, "delimiter")) { a.get(3) } else { a.get(2) };
            let sep = s.map(|v| sep_arg("dlmwrite", v)).transpose()?.flatten();
            write_num("dlmwrite", &a[0], &a[1], sep.unwrap_or(','))
        },
        "readmatrix" => |_, a, _| {
            nargs("readmatrix", a, 1, 5)?;
            num(to_matrix(&split_header(with_opts("readmatrix", a)?).1, f64::NAN))
        },
        "writematrix" => writematrix,
        "xlsread" => |_, a, _| {
            nargs("xlsread", a, 1, 2)?;
            let (head, body) = split_header(load("xlsread", &a[0], a.get(1), None)?);
            Ok(vec![Value::Num(to_matrix(&body, f64::NAN)), strs(head.last().map_or(&[][..], |h| h))])
        },
        "importdata" => importdata,
        "readtable" => readtable,
        _ => return None,
    };
    Some(f)
}

/// Cells of `path`: a workbook sheet (by name, default the first) or delimited text (`sep` None
/// sniffs it, ' ' is runs of whitespace). Errors name the file.
pub fn read_cells(path: &Path, sheet: Option<&str>, sep: Option<char>) -> Result<Grid, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut g = if is_book(path) {
        use calamine::{Data, Reader};
        let mut wb = calamine::open_workbook_auto(path).map_err(|e| at(&e))?;
        let names = wb.sheet_names();
        let name = match sheet {
            Some(s) => names.iter().find(|n| n.eq_ignore_ascii_case(s)).ok_or_else(|| at(&format!("no sheet named '{s}' (has {})", names.join(", "))))?,
            None => names.first().ok_or_else(|| at(&"workbook has no sheets"))?,
        };
        let range = wb.worksheet_range(name).map_err(|e| at(&e))?;
        let text = |d: &Data| match d {
            Data::Int(i) => i.to_string(),
            Data::Float(x) => fmt_num(*x),
            Data::DateTime(t) => fmt_num(t.as_f64()),
            Data::DateTimeIso(s) => iso_serial(s).map_or_else(|| s.clone(), fmt_num),
            Data::Bool(b) => (*b as u8).to_string(),
            Data::String(s) | Data::DurationIso(s) => s.trim().to_string(),
            Data::Error(_) | Data::Empty => String::new(),
        };
        range.rows().map(|r| r.iter().map(text).collect()).collect()
    } else {
        parse_text(&String::from_utf8_lossy(&std::fs::read(path).map_err(|e| at(&e))?), sep)
    };
    // trailing blank cells carry nothing; blank rows go too
    for r in &mut g {
        r.truncate(r.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1));
    }
    g.retain(|r| !r.is_empty());
    Ok(g)
}

/// Sheet names of a workbook, in order.
pub fn sheet_names(path: &Path) -> Result<Vec<String>, String> {
    use calamine::Reader;
    Ok(calamine::open_workbook_auto(path).map_err(|e| format!("{}: {e}", path.display()))?.sheet_names())
}

/// .xlsx .xlsm .xlsb .xls .ods (by extension); anything else reads as delimited text.
pub fn is_book(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "xlsx" | "xlsm" | "xlsb" | "xls" | "ods")
}

/// ISO date[Ttime] (ODS date cells) -> Excel serial day number (1970-01-01 is 25569).
fn iso_serial(s: &str) -> Option<f64> {
    let (date, time) = s.split_once('T').unwrap_or((s, ""));
    let mut p = date.splitn(3, '-').map(|x| x.parse::<i64>().ok());
    let (y, m, d) = (p.next()??, p.next()??, p.next()??);
    // days from civil (H. Hinnant)
    let y = if m <= 2 { y - 1 } else { y };
    let (era, mp) = (y.div_euclid(400), (m + 9) % 12);
    let yoe = y - era * 400;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + (153 * mp + 2) / 5 + d - 1;
    let secs: f64 = time.split(':').zip([3600.0, 60.0, 1.0]).map(|(x, k)| x.parse::<f64>().unwrap_or(0.0) * k).sum();
    Some((era * 146097 + doe - 719468 + 25569) as f64 + secs / 86400.0)
}

/// Delimited text -> cells. `sep` None sniffs it; ' ' splits on runs of whitespace. With a
/// separator other than ',' a decimal comma ("1,5", LibreOffice in many locales) becomes a point.
pub fn parse_text(text: &str, sep: Option<char>) -> Grid {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let sep = sep.unwrap_or_else(|| sniff(text));
    let mut g: Grid = if sep == ' ' {
        text.lines().map(|l| l.split_whitespace().map(String::from).collect()).collect()
    } else {
        split_quoted(text, sep)
    };
    if sep != ',' {
        for c in g.iter_mut().flatten() {
            if to_num(c, false).is_none() && to_num(c, true).is_some() {
                *c = c.replace(',', ".");
            }
        }
    }
    g
}

/// Tab, semicolon or comma, whichever (in that order) splits most of the first 20 lines; else
/// whitespace.
fn sniff(text: &str) -> char {
    let head = text.lines().take(20).collect::<Vec<_>>().join("\n");
    let splits = |sep| {
        let g = split_quoted(&head, sep);
        let rows: Vec<_> = g.iter().filter(|r| r.iter().any(|c| !c.is_empty())).collect();
        2 * rows.iter().filter(|r| r.len() > 1).count() > rows.len()
    };
    ['\t', ';', ','].into_iter().find(|&s| splits(s)).unwrap_or(' ')
}

/// RFC 4180-ish: "a,b" and "say ""hi""" are one field, quoted fields may span lines. Cells trimmed.
fn split_quoted(text: &str, sep: char) -> Grid {
    let (mut g, mut row, mut cell, mut quoted) = (vec![], vec![], String::new(), false);
    let mut it = text.chars().peekable();
    while let Some(ch) = it.next() {
        match ch {
            '"' if quoted && it.peek() == Some(&'"') => {
                cell.push('"');
                it.next();
            }
            '"' if quoted => quoted = false,
            '"' if cell.trim().is_empty() => {
                cell.clear();
                quoted = true;
            }
            c if quoted => cell.push(c),
            '\n' => {
                row.push(cell.trim().to_string());
                g.push(std::mem::take(&mut row));
                cell.clear();
            }
            c if c == sep => {
                row.push(cell.trim().to_string());
                cell.clear();
            }
            c => cell.push(c),
        }
    }
    if !cell.trim().is_empty() || !row.is_empty() {
        row.push(cell.trim().to_string());
        g.push(row);
    }
    g
}

/// Cell text -> number ("1.5", "-2e3", "Inf", "NaN"); `comma` also takes a decimal comma.
/// None for blanks and text.
pub fn to_num(s: &str, comma: bool) -> Option<f64> {
    let s = s.trim();
    s.parse().ok().or_else(|| if comma && s.contains(',') { s.replace(',', ".").parse().ok() } else { None })
}

/// Number -> cell text: shortest round-trip form, exponent when huge or tiny, NaN/Inf like Octave.
pub fn fmt_num(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        (if x > 0.0 { "Inf" } else { "-Inf" }).into()
    } else if x != 0.0 && (x.abs() >= 1e15 || x.abs() < 1e-5) {
        format!("{x:e}")
    } else {
        format!("{x}")
    }
}

/// Leading rows without any number (titles, column names) and the data rows after them.
// ponytail: a header of numeric names ("2019,2020") reads as data; add 'NumHeaderLines' if it bites.
pub fn split_header(mut g: Grid) -> (Grid, Grid) {
    let n = g.iter().position(|r| r.iter().any(|c| to_num(c, false).is_some())).unwrap_or(g.len());
    let body = g.split_off(n);
    (g, body)
}

/// Rows -> matrix as wide as the widest row; blanks, text and missing cells are `fill`.
pub fn to_matrix(rows: &[Vec<String>], fill: f64) -> Matrix {
    let (r, c) = (rows.len(), rows.iter().map(Vec::len).max().unwrap_or(0));
    Matrix::new(r, c, (0..r * c).map(|k| rows[k % r].get(k / r).and_then(|s| to_num(s, false)).unwrap_or(fill)).collect())
}

/// Delimited file from cells; fields holding the separator, quotes or newlines get quoted.
pub fn write_cells(path: &Path, rows: &[Vec<String>], sep: char) -> Result<(), String> {
    let quote = |s: &String| if s.contains([sep, '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.clone() };
    let text: String = rows.iter().map(|r| r.iter().map(quote).collect::<Vec<_>>().join(&sep.to_string()) + "\n").collect();
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Column header -> identifier like matlab.lang.makeValidName: whitespace dropped with the next
/// letter capitalized, other invalid characters '_', a leading non-letter gets an 'x' prefix.
pub fn valid_name(s: &str) -> String {
    let (mut out, mut up) = (String::new(), false);
    for c in s.trim().chars() {
        if c.is_whitespace() {
            up = true;
            continue;
        }
        let c = if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' };
        out.push(if up { c.to_ascii_uppercase() } else { c });
        up = false;
    }
    if !out.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.insert(0, 'x');
    }
    out
}

fn strs(cells: &[String]) -> Value {
    Value::Cell(cells.iter().map(|s| Value::Str(s.clone())).collect())
}

fn path_arg(name: &str, v: &Value) -> Result<PathBuf, String> {
    v.as_str().map(PathBuf::from).map_err(|_| format!("{name}: FILE must be a string"))
}

/// Separator argument: one character, "\t" spelled out, a name, or "" (sniff).
fn sep_arg(name: &str, v: &Value) -> Result<Option<char>, String> {
    let s = v.as_str().map_err(|_| format!("{name}: SEP must be a string"))?;
    Ok(match s.to_lowercase().as_str() {
        "" => None,
        "\\t" | "tab" => Some('\t'),
        "space" | "\\s" => Some(' '),
        "comma" => Some(','),
        "semi" | "semicolon" => Some(';'),
        _ if s.chars().count() == 1 => s.chars().next(),
        _ => return Err(format!("{name}: unknown delimiter '{s}'")),
    })
}

/// FILE's cells; `sheet` is a name or a 1-based number.
fn load(name: &str, file: &Value, sheet: Option<&Value>, sep: Option<char>) -> Result<Grid, String> {
    let path = path_arg(name, file)?;
    let sheet = match sheet {
        None => None,
        Some(Value::Str(s)) => Some(s.clone()),
        Some(v) => {
            let k = v.as_usize().map_err(|_| format!("{name}: SHEET must be a name or a number"))?;
            let names = sheet_names(&path).map_err(|e| format!("{name}: {e}"))?;
            let n = names.len();
            Some(names.into_iter().nth(k.wrapping_sub(1)).ok_or_else(|| format!("{name}: {}: no sheet {k} (it has {n})", path.display()))?)
        }
    };
    read_cells(&path, sheet.as_deref(), sep).map_err(|e| format!("{name}: {e}"))
}

/// FILE[, 'Sheet', name|number][, 'Delimiter', d]
fn with_opts(name: &str, a: &[Value]) -> Result<Grid, String> {
    let (mut sheet, mut sep) = (None, None);
    for p in a[1..].chunks(2) {
        match p {
            [k, v] if is_opt(k, "sheet") => sheet = Some(v),
            [k, v] if is_opt(k, "delimiter") => sep = sep_arg(name, v)?,
            _ => return Err(format!("{name}: options are 'Sheet', name|number and 'Delimiter', d")),
        }
    }
    load(name, &a[0], sheet, sep)
}

/// csvread / dlmread: every row from R0, every column from C0 (0-based); text and blanks 0.
fn raw(name: &str, file: &Value, sep: Option<char>, r0: Option<&Value>, c0: Option<&Value>) -> R {
    let off = |v: Option<&Value>| v.map_or(Ok(0), |v| v.as_usize().map_err(|_| format!("{name}: R0 and C0 must be non-negative integers")));
    let (r0, c0) = (off(r0)?, off(c0)?);
    let g = load(name, file, None, sep)?;
    let rows: Grid = g.into_iter().skip(r0).map(|r| r.into_iter().skip(c0).collect()).collect();
    num(to_matrix(&rows, 0.0))
}

fn write_num(name: &str, file: &Value, m: &Value, sep: char) -> R {
    let path = path_arg(name, file)?;
    let m = real_arg(name, m)?;
    let rows: Grid = (0..m.rows).map(|i| (0..m.cols).map(|j| fmt_num(m.get(i, j))).collect()).collect();
    write_cells(&path, &rows, sep).map_err(|e| format!("{name}: {e}"))?;
    Ok(vec![])
}

/// writematrix(M, FILE[, 'Delimiter', d]): .tsv tab-separated, others comma.
fn writematrix(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("writematrix", a, 2, 4)?;
    let path = path_arg("writematrix", &a[1])?;
    if is_book(&path) {
        return Err(format!("writematrix: {}: writing spreadsheets is not supported, save as .csv (opens in any spreadsheet)", path.display()));
    }
    let tsv = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("tsv"));
    let sep = match &a[2..] {
        [] => None,
        [k, v] if is_opt(k, "delimiter") => sep_arg("writematrix", v)?,
        _ => return Err("writematrix: the only option is 'Delimiter', d".into()),
    };
    write_num("writematrix", &a[1], &a[0], sep.unwrap_or(if tsv { '\t' } else { ',' }))
}

/// importdata(FILE[, SEP]): the matrix, or with header rows a struct of data, textdata (header
/// cells) and colheaders (the last header row).
fn importdata(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("importdata", a, 1, 2)?;
    let sep = a.get(1).map(|v| sep_arg("importdata", v)).transpose()?.flatten();
    let (head, body) = split_header(load("importdata", &a[0], None, sep)?);
    let data = to_matrix(&body, f64::NAN);
    let Some(names) = head.last() else { return num(data) };
    let mut s = BTreeMap::new();
    s.insert("data".to_string(), Value::Num(data));
    s.insert("textdata".to_string(), strs(&head.concat()));
    s.insert("colheaders".to_string(), strs(names));
    Ok(vec![Value::Struct(s)])
}

/// readtable(FILE[, opts]): struct of columns named by the header (Var1, Var2, ... without one).
/// A column is text when most of its filled cells aren't numbers.
fn readtable(_: &mut Interp, a: &[Value], _: usize) -> R {
    nargs("readtable", a, 1, 5)?;
    let (mut head, mut body) = split_header(with_opts("readtable", a)?);
    if body.is_empty() && head.len() > 1 {
        body = head.split_off(1); // all text: the first row names the columns
    }
    let names = head.pop().unwrap_or_default();
    let width = body.iter().map(Vec::len).chain([names.len()]).max().unwrap_or(0);
    let mut s = BTreeMap::new();
    for j in 0..width {
        let col: Vec<&str> = body.iter().map(|r| r.get(j).map_or("", String::as_str)).collect();
        let nums = col.iter().filter(|c| to_num(c, false).is_some()).count();
        let text = col.iter().filter(|c| !c.is_empty()).count() - nums;
        let v = if text > nums {
            Value::Cell(col.iter().map(|&c| Value::from(c)).collect())
        } else {
            Value::Num(Matrix::col(col.iter().map(|c| to_num(c, false).unwrap_or(f64::NAN)).collect()))
        };
        let base = match names.get(j) {
            Some(n) if !n.is_empty() => valid_name(n),
            _ => format!("Var{}", j + 1),
        };
        let mut key = base.clone();
        for k in 1.. {
            if !s.contains_key(&key) {
                break;
            }
            key = format!("{base}_{k}");
        }
        s.insert(key, v);
    }
    Ok(vec![Value::Struct(s)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(rows: &[&[&str]]) -> Grid {
        rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("qcalcp-io-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    fn fixture(name: &str) -> String {
        format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    fn run(it: &mut Interp, src: &str) -> Value {
        it.eval(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        it.vars["r"].clone()
    }

    #[test]
    fn parsing() {
        // separators, quotes, BOM, CRLF, blank lines
        assert_eq!(parse_text("\u{feff}a,b\r\n1, 2\r\n\r\n3,4\r\n", None), g(&[&["a", "b"], &["1", "2"], &[""], &["3", "4"]]));
        assert_eq!(parse_text("x\ty\n1,5\t2\n", None), g(&[&["x", "y"], &["1.5", "2"]]));
        assert_eq!(parse_text("x;y\n1,5;-2,25\n", None), g(&[&["x", "y"], &["1.5", "-2.25"]]));
        assert_eq!(parse_text("1  2\n 3 4\n", None), g(&[&["1", "2"], &["3", "4"]]));
        assert_eq!(parse_text("\"a,b\",\"say \"\"hi\"\"\"\n\"two\nlines\",1,5", None), g(&[&["a,b", "say \"hi\""], &["two\nlines", "1", "5"]]));
        // a title line doesn't stop the sniffing; a comma file keeps "1,5" as text
        assert_eq!(parse_text("Run 3\na,b\n1,2\n3,4\n", None)[2], ["1", "2"]);
        assert_eq!(to_num("1,5", false), None);
        assert!(to_num("NaN", false).unwrap().is_nan());
        assert_eq!(to_num(" -Inf ", false), Some(f64::NEG_INFINITY));
        assert_eq!(to_num("1e3", false), Some(1000.0));
        assert_eq!(to_num("abc", true), None);
        assert_eq!(iso_serial("2024-01-15"), Some(45306.0));
        assert_eq!(iso_serial("1900-03-01T12:00:00"), Some(61.5));

        let (head, body) = split_header(g(&[&["t", "v"], &["0", ""], &["1", "x"], &["2"]]));
        assert_eq!(head, g(&[&["t", "v"]]));
        let m = to_matrix(&body, f64::NAN);
        assert_eq!((m.rows, m.cols, m.get(2, 0)), (3, 2, 2.0));
        assert!(m.get(0, 1).is_nan() && m.get(1, 1).is_nan() && m.get(2, 1).is_nan());

        assert_eq!(valid_name("time s"), "timeS");
        assert_eq!(valid_name("Time (s)"), "Time_s_");
        assert_eq!(valid_name("1st"), "x1st");
        assert_eq!(valid_name("ok_1"), "ok_1");
        assert_eq!([fmt_num(f64::NAN), fmt_num(-f64::INFINITY), fmt_num(0.25), fmt_num(1e300), fmt_num(1.5e-7)], ["NaN", "-Inf", "0.25", "1e300", "1.5e-7"]);
    }

    #[test]
    fn text_files() {
        let mut it = Interp::new();
        let (a, h, w) = (tmp("a.csv"), tmp("h.csv"), tmp("w.tsv"));
        let (p, hp, t) = (a.display(), h.display(), w.display());
        let Value::Num(m) = run(&mut it, &format!("csvwrite('{p}', [1 2; NaN 4.5]); r = csvread('{p}');")) else { panic!() };
        assert_eq!((m.get(0, 1), m.get(1, 1)), (2.0, 4.5));
        assert!(m.get(1, 0).is_nan());
        std::fs::write(&h, "name,x,y\nfoo,1,2\nbar,3,\n").unwrap();
        // csvread like Octave: no header skipping, text and blanks 0, 0-based offsets
        assert_eq!(run(&mut it, &format!("r = csvread('{hp}', 1, 1);")).matrix().unwrap(), Matrix::from_rows(&[vec![1., 2.], vec![3., 0.]]));
        assert_eq!(run(&mut it, &format!("r = dlmread('{hp}', ',');")).matrix().unwrap().get(0, 1), 0.0);
        let m = run(&mut it, &format!("r = readmatrix('{hp}');")).matrix().unwrap();
        assert_eq!((m.rows, m.cols, m.get(1, 1)), (2, 3, 3.0));
        assert!(m.get(0, 0).is_nan() && m.get(1, 2).is_nan());
        let Value::Struct(s) = run(&mut it, &format!("r = readtable('{hp}');")) else { panic!() };
        assert!(matches!(&s["name"], Value::Cell(c) if c.len() == 2 && c[1].as_str() == Ok("bar")));
        assert_eq!(s["x"].matrix().unwrap(), Matrix::col(vec![1., 3.]));
        let Value::Struct(s) = run(&mut it, &format!("r = importdata('{hp}');")) else { panic!() };
        assert!(matches!(&s["colheaders"], Value::Cell(c) if c.len() == 3));
        assert_eq!(s["data"].matrix().unwrap().rows, 2);
        // importdata without a header: the matrix
        assert_eq!(run(&mut it, &format!("r = importdata('{p}');")).size(), (2, 2));

        run(&mut it, &format!("writematrix([1 2 3], '{t}'); r = readmatrix('{t}', 'Delimiter', '\\t');"));
        assert_eq!(std::fs::read_to_string(&w).unwrap(), "1\t2\t3\n");
        assert_eq!(it.vars["r"].matrix().unwrap(), Matrix::row(vec![1., 2., 3.]));
        run(&mut it, &format!("dlmwrite('{t}', [1 2], 'delimiter', ';'); r = 0;"));
        assert_eq!(std::fs::read_to_string(&w).unwrap(), "1;2\n");
        let e = it.eval(&format!("writematrix(1, '{}')", tmp("x.xlsx").display())).unwrap_err();
        assert!(e.contains("x.xlsx") && e.contains(".csv"), "{e}");
        let e = it.eval("readmatrix('/nonexistent/q.csv')").unwrap_err();
        assert!(e.starts_with("readmatrix: /nonexistent/q.csv:"), "{e}");
        assert!(it.eval(&format!("readmatrix('{hp}', 'Bogus', 1)")).is_err());

        // a cell holding the separator or quotes round-trips
        let q = tmp("q.csv");
        write_cells(&q, &g(&[&["a,b", "c\"d"], &["1", "2"]]), ',').unwrap();
        assert_eq!(read_cells(&q, None, None).unwrap(), g(&[&["a,b", "c\"d"], &["1", "2"]]));
        let _ = std::fs::remove_dir_all(q.parent().unwrap());
    }

    #[test]
    fn workbooks() {
        // tests/data/book.*: sheet "Data" = time s | volts | when (dates) | tag, 3 rows with a blank
        // volts cell; sheet "Second" = 7 8. Made with LibreOffice from a hand-written flat ODS.
        for f in ["book.xlsx", "book.ods", "book.xls"] {
            let p = PathBuf::from(fixture(f));
            assert_eq!(sheet_names(&p).unwrap(), ["Data", "Second"], "{f}");
            let (head, body) = split_header(read_cells(&p, None, None).unwrap());
            assert_eq!(head, g(&[&["time s", "volts", "when", "tag"]]), "{f}");
            let m = to_matrix(&body, f64::NAN);
            assert_eq!((m.rows, m.cols), (3, 4), "{f}");
            assert_eq!((m.get(1, 0), m.get(2, 1)), (0.5, -2.25), "{f}");
            assert!(m.get(1, 1).is_nan() && m.get(0, 3).is_nan(), "{f}");
            assert_eq!(m.get(0, 2), 45306.0, "{f}: 2024-01-15 as an Excel serial");
            assert_eq!(read_cells(&p, Some("second"), None).unwrap(), g(&[&["7", "8"]]), "{f}");
        }
        let mut it = Interp::new();
        let p = fixture("book.xlsx");
        assert_eq!(run(&mut it, &format!("r = readmatrix('{p}', 'Sheet', 2);")).matrix().unwrap(), Matrix::row(vec![7., 8.]));
        run(&mut it, &format!("[r, txt] = xlsread('{p}');"));
        assert!(matches!(&it.vars["txt"], Value::Cell(c) if c.len() == 4 && c[0].as_str() == Ok("time s")));
        let Value::Struct(s) = run(&mut it, &format!("r = readtable('{p}');")) else { panic!() };
        assert_eq!(s.keys().collect::<Vec<_>>(), ["tag", "timeS", "volts", "when"]);
        let e = it.eval(&format!("readmatrix('{p}', 'Sheet', 'Nope')")).unwrap_err();
        assert!(e.contains("book.xlsx") && e.contains("Nope"), "{e}");
        assert!(it.eval(&format!("readmatrix('{p}', 'Sheet', 3)")).unwrap_err().contains("no sheet 3"));
    }
}
