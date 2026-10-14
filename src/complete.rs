//! Completion shared by the GUI and the TUI: the identifier being typed -> matching functions,
//! variables, units with a short description. Math (calculator) side from `qalc::catalog`, Octave
//! side from `doc` plus workspace variables and user functions. CONTRACT: completion agent implements.
//!
//! Ranking: exact name, then prefix matches (case-sensitive, then not), then substring matches
//! ("grate" -> integrate). Within each: workspace before builtins before units, the COMMON names
//! in their order, then shorter, then alphabetical.
use std::collections::HashSet;

use crate::doc;
use crate::qalc::{self, CatalogItem, ItemKind};

/// Which language the text is in.
#[allow(dead_code)] // the GUI and TUI completion popups use it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Math,
    Octave,
}

#[allow(dead_code)] // the GUI and TUI completion popups use it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Function,
    Variable,
    Unit,
    Keyword,
}

#[allow(dead_code)] // the GUI and TUI completion popups use it
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    /// the name as listed ("integrate")
    pub name: String,
    /// replaces the typed prefix. Math functions: `name(□, □)`, one □ per required argument
    /// (`name()` with none). Octave functions: `name(□)`, or `name()` when they take no arguments.
    /// Names: the name. The GUI selects the first □; the TUI drops the □ and puts the cursor there.
    pub insert: String,
    /// "integrate(f, a, b, x)"; may equal name
    pub signature: String,
    /// one short line ("definite or indefinite integral"); may be empty
    pub desc: String,
    pub kind: Kind,
}

/// Units listed besides an exact match (libqalculate has hundreds).
const MAX_UNITS: usize = 3;

/// Ranked first, in this order, among matches of the same tier and group.
const COMMON: &[&str] = &[
    "sin", "cos", "tan", "sqrt", "exp", "log", "ln", "abs", "integrate", "integral", "diff", "solve", "limit", "sum", "plot", "fplot",
    "linspace", "zeros", "ones", "rand", "mean", "std", "max", "min", "size", "length", "disp", "printf", "sprintf", "find", "sort",
    "inv", "det", "eig", "factor", "simplify", "expand", "floor", "round", "mod", "figure", "hold", "legend", "title",
];

/// Completions for the identifier that ends at char offset `cursor` of `text`: the char offset where
/// that prefix starts, and the matches (best first, at most `limit`). None when the cursor isn't
/// at the end of an identifier, the identifier has fewer than 2 chars, the cursor is inside an
/// Octave string or comment, or nothing matches. `vars`: workspace (name, summary like "1x3 double").
#[allow(dead_code)] // the GUI and TUI completion popups use it
pub fn complete(text: &str, cursor: usize, lang: Lang, vars: &[(String, String)], limit: usize) -> Option<(usize, Vec<Completion>)> {
    let before: Vec<char> = text.chars().take(cursor).collect();
    if before.len() < cursor {
        return None;
    }
    // Octave names are ASCII; the calculator also takes Greek and other letters
    let ident = |c: char| c == '_' || if lang == Lang::Math { c.is_alphanumeric() } else { c.is_ascii_alphanumeric() };
    let mut start = cursor;
    while start > 0 && ident(before[start - 1]) {
        start -= 1;
    }
    // a name starts with a letter or _: "2pi" completes "pi"
    while start < cursor && !(before[start].is_alphabetic() || before[start] == '_') {
        start += 1;
    }
    if cursor - start < 2 {
        return None;
    }
    // struct fields (s.na) aren't known here
    if lang == Lang::Octave && (start > 0 && before[start - 1] == '.' || in_string_or_comment(&before[..start])) {
        return None;
    }
    let prefix: String = before[start..].iter().collect();
    let hits = match lang {
        Lang::Math => math(&prefix, qalc::catalog(), vars, limit),
        Lang::Octave => octave(&prefix, vars, limit),
    };
    (!hits.is_empty()).then_some((start, hits))
}

/// Is an Octave string or comment open at the end of `s` (text up to the name)? Only the last line counts.
// ponytail: no %{ block comments %} or `...` continuations, add if completion pops up inside them
fn in_string_or_comment(s: &[char]) -> bool {
    let line = s.iter().rposition(|&c| c == '\n').map_or(s, |k| &s[k + 1..]);
    let mut quote = None;
    let mut k = 0;
    while k < line.len() {
        let c = line[k];
        match quote {
            // a doubled quote is an escaped one
            Some(q) if c == q => match line.get(k + 1) {
                Some(&n) if n == q => k += 1,
                _ => quote = None,
            },
            Some('"') if c == '\\' => k += 1,
            Some(_) => {}
            None => match c {
                '%' | '#' => return true,
                '"' => quote = Some('"'),
                // right after a value ' is the transpose operator: a', x(1)', [1 2]', a.'
                '\'' if !(k > 0 && (line[k - 1].is_ascii_alphanumeric() || "_)]}'.".contains(line[k - 1]))) => quote = Some('\''),
                _ => {}
            },
        }
        k += 1;
    }
    quote.is_some()
}

/// Octave: workspace variables and user functions, documented builtins, keywords.
fn octave(prefix: &str, vars: &[(String, String)], limit: usize) -> Vec<Completion> {
    let lower = prefix.to_lowercase();
    let vars = vars.iter().map(|(name, summary)| {
        let c = if summary == "function" {
            Completion { name: name.clone(), insert: format!("{name}(□)"), signature: name.clone(), desc: "user function".into(), kind: Kind::Function }
        } else {
            variable(name, summary)
        };
        (0, c)
    });
    let docs = doc::all().iter().filter(|d| tier(d.0, prefix, &lower).is_some()).map(|&(name, sig, desc)| {
        (1, Completion { name: name.into(), insert: call(name, sig), signature: sig.into(), desc: desc.into(), kind: Kind::Function })
    });
    let kws = doc::KEYWORDS.iter().map(|&(name, sig, desc)| {
        (1, Completion { name: name.into(), insert: name.into(), signature: sig.into(), desc: desc.into(), kind: Kind::Keyword })
    });
    rank(prefix, vars.chain(docs).chain(kws), limit)
}

/// Calculator: numeric workspace variables (the calculator sees those), then libqalculate's
/// functions and variables, then its units.
fn math(prefix: &str, catalog: &[CatalogItem], vars: &[(String, String)], limit: usize) -> Vec<Completion> {
    let lower = prefix.to_lowercase();
    let vars = vars.iter().filter(|(_, s)| s.ends_with("double") || s.ends_with("logical")).map(|(n, s)| (0, variable(n, s)));
    let items = catalog.iter().filter(|c| tier(&c.name, prefix, &lower).is_some()).map(|c| {
        let kind = match c.kind {
            ItemKind::Function => Kind::Function,
            ItemKind::Variable => Kind::Variable,
            ItemKind::Unit => Kind::Unit,
        };
        let group = if kind == Kind::Unit { 2 } else { 1 };
        (group, Completion { name: c.name.clone(), insert: c.insert.clone(), signature: c.signature.clone(), desc: c.desc.clone(), kind })
    });
    rank(prefix, vars.chain(items), limit)
}

fn variable(name: &str, summary: &str) -> Completion {
    Completion { name: name.into(), insert: name.into(), signature: name.into(), desc: summary.into(), kind: Kind::Variable }
}

/// Octave insert from the signature's first form: `name(□)` for a call, `name()` for one without
/// arguments, else the bare name (`pi[(n, m)]`, `hold [on|off]`).
fn call(name: &str, sig: &str) -> String {
    let first = sig.split(" | ").next().unwrap_or(sig);
    match first.find(&format!("{name}(")) {
        Some(k) if first[k + name.len() + 1..].starts_with(')') => format!("{name}()"),
        Some(_) => format!("{name}(□)"),
        None => name.to_string(),
    }
}

/// 0 exact, 1 prefix, 2 prefix ignoring case, 3 substring ignoring case; None: no match.
fn tier(name: &str, prefix: &str, lower: &str) -> Option<u8> {
    if name.starts_with(prefix) {
        return Some((name.len() != prefix.len()) as u8);
    }
    // ponytail: lowercases every other name per keystroke (~2 ms per 6000 names in a debug build),
    // keep lowercased names next to the catalog if typing lags
    let n = name.to_lowercase();
    if n.starts_with(lower) {
        Some(2)
    } else if n.contains(lower) {
        Some(3)
    } else {
        None
    }
}

/// Best first by (tier, group, COMMON position, length, name); one entry per name; at most
/// MAX_UNITS units besides an exact match.
fn rank(prefix: &str, cands: impl Iterator<Item = (u8, Completion)>, limit: usize) -> Vec<Completion> {
    let lower = prefix.to_lowercase();
    let mut hits: Vec<_> = cands
        .filter_map(|(group, c)| {
            let common = COMMON.iter().position(|&n| n == c.name).unwrap_or(COMMON.len());
            Some(((tier(&c.name, prefix, &lower)?, group, common, c.name.chars().count()), c))
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let (mut seen, mut units) = (HashSet::new(), 0);
    hits.into_iter()
        .filter(|(key, c)| {
            seen.insert(c.name.clone())
                && (c.kind != Kind::Unit || key.0 == 0 || {
                    units += 1;
                    units <= MAX_UNITS
                })
        })
        .map(|(_, c)| c)
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oct(text: &str, vars: &[(&str, &str)]) -> Option<(usize, Vec<Completion>)> {
        let vars: Vec<(String, String)> = vars.iter().map(|(n, s)| (n.to_string(), s.to_string())).collect();
        complete(text, text.chars().count(), Lang::Octave, &vars, 10)
    }

    fn names(r: &Option<(usize, Vec<Completion>)>) -> Vec<&str> {
        r.as_ref().map_or(vec![], |(_, v)| v.iter().map(|c| c.name.as_str()).collect())
    }

    fn item(name: &str, kind: ItemKind, insert: &str) -> CatalogItem {
        CatalogItem { name: name.into(), kind, insert: insert.into(), signature: name.into(), desc: String::new() }
    }

    #[test]
    fn octave_names() {
        let (start, c) = oct("y = lins", &[]).unwrap();
        assert_eq!(start, 4);
        assert_eq!((c[0].insert.as_str(), c[0].signature.as_str(), c[0].kind), ("linspace(□)", "linspace(a, b[, n])", Kind::Function));
        assert!(!c[0].desc.is_empty());
        // workspace first; user functions get a call
        let r = oct("lin", &[("lin_fit", "1x3 double"), ("linf", "function")]);
        assert_eq!(names(&r)[..3], ["linf", "lin_fit", "linspace"]);
        assert_eq!(r.unwrap().1[0].insert, "linf(□)");
        // exact match first; substring matches after prefix matches; case-insensitive prefix
        assert_eq!(names(&oct("sum", &[("summary", "1x1 double")]))[..2], ["sum", "summary"]);
        let plot = oct("plot", &[]);
        let plot = names(&plot);
        assert_eq!(plot[..2], ["plot", "plot3"]);
        assert!(plot.contains(&"fplot"));
        assert_eq!(names(&oct("Lins", &[]))[0], "linspace");
        // keywords, bare inserts, no-argument calls
        assert_eq!(names(&oct("whi", &[]))[0], "while");
        assert_eq!(oct("hol", &[]).unwrap().1[0].insert, "hold");
        assert_eq!(oct("pi", &[]).unwrap().1[0].insert, "pi");
        assert_eq!(oct("pyadd", &[]).unwrap().1[0].insert, "pyaddons()");
        assert_eq!(oct("dec2b", &[]).unwrap().1[0].insert, "dec2bin(□)");
        assert!(oct("xyzzyq", &[]).is_none());
    }

    #[test]
    fn where_not_to_complete() {
        assert!(oct("s", &[]).is_none());
        assert!(oct("x = 'lins", &[]).is_none());
        assert!(oct("printf(\"%d lins", &[]).is_none());
        assert!(oct("printf(\"\\\" lins", &[]).is_none());
        assert!(oct("x = 1 % lins", &[]).is_none());
        assert!(oct("# lins", &[]).is_none());
        assert!(oct("s.lins", &[]).is_none());
        assert!(oct("x = 'it''s' + lins", &[]).is_some());
        assert!(oct("y = a' * lins", &[]).is_some());
        assert!(oct("y = [1 2]'; lins", &[]).is_some());
        assert!(oct("disp('%') ; lins", &[]).is_some());
        assert!(oct("x = 'abc\nlins", &[]).is_some()); // strings end with their line
        assert_eq!(oct("2lins", &[]).unwrap().0, 1);
        // cursor mid-word: the part before it; out of range or no room: None
        let r = complete("linspace", 4, Lang::Octave, &[], 5).unwrap();
        assert_eq!((r.0, r.1[0].name.as_str()), (0, "linspace"));
        assert!(complete("lins", 9, Lang::Octave, &[], 5).is_none());
        assert!(complete("lins", 4, Lang::Octave, &[], 0).is_none());
        // unicode: no panics, Greek names in calculator mode only
        assert!(oct("αβ", &[]).is_none());
        assert!(oct("é 'ü' lins", &[]).is_some());
        assert_eq!(complete("x = αβ", 6, Lang::Math, &[("αβγ".into(), "1x1 double".into())], 5).unwrap().0, 4);
        assert!(complete("", 0, Lang::Math, &[], 5).is_none());
    }

    #[test]
    fn math_ranking() {
        let cat = vec![
            item("integer", ItemKind::Unit, "integer"),
            item("int", ItemKind::Function, "int(□)"),
            item("integral", ItemKind::Function, "integral(□)"),
            item("integrate", ItemKind::Function, "integrate(□, □, □)"),
            item("inch", ItemKind::Unit, "inch"),
            item("in", ItemKind::Unit, "in"),
            item("inHg", ItemKind::Unit, "inHg"),
            item("inH2O", ItemKind::Unit, "inH2O"),
            item("inf", ItemKind::Variable, "inf"),
            item("sin", ItemKind::Function, "sin(□)"),
        ];
        let vars = vec![("inv_x".to_string(), "1x1 double".to_string()), ("info".to_string(), "1x4 char".to_string())];
        let r = math("integ", &cat, &vars, 10);
        assert_eq!((r[0].name.as_str(), r[0].insert.as_str()), ("integrate", "integrate(□, □, □)"));
        assert_eq!(r.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["integrate", "integral", "integer"]);
        // exact unit first; numeric variables, then functions/variables, then at most 3 more units
        let r: Vec<_> = math("in", &cat, &vars, 20).into_iter().map(|c| c.name).collect();
        assert_eq!(r, ["in", "inv_x", "integrate", "integral", "inf", "int", "inHg", "inch", "inH2O", "sin"]);
        assert_eq!(math("grate", &cat, &[], 5)[0].name, "integrate");
        assert_eq!(math("in", &cat, &vars, 2).len(), 2);
    }
}
