//! On-screen keypad: pages from keypad.ini (DEFAULT_KEYPAD when missing or bad), and the □
//! placeholder logic shared by the keypad and the calculator entry.
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;

/// Placeholder in templates: the selection is wrapped into the first one, Tab moves between them.
pub const SLOT: char = '□';

pub const DEFAULT_KEYPAD: &str = "\
# qcalcp on-screen keypad. Delete this file to get the defaults back.
# One [group] per page, in display order:
#   columns=N                 buttons per row (1-12)
#   keys=label|template;...   keys in rows of N; an empty key leaves a gap; write \\; for a literal ;
# template: the text inserted. A bare label inserts the label itself. □ marks a slot: a selection is
#   wrapped into the first one and the next one gets selected (Tab / Shift+Tab move between them).
# actions: @back @clear @left @right @enter (evaluate) @ans
# Templates are calculator (libqalculate) syntax; in Octave mode × ÷ − π √ ∛ ² ³ become ASCII.
# A bad entry makes qcalcp use these defaults and say why in the console.
# style.css: .keypad  .keypad button  .keypad .op (operators, actions)  .keypad .fn (functions)
#            .keypad .basic (page name, lowercase)

[Basic]
columns=5
keys=AC|@clear;(;);^;⌫|@back;7;8;9;÷;←|@left;4;5;6;×;→|@right;1;2;3;−;ans|@ans;0;.;,;+;=|@enter

[Scientific]
columns=5
keys=sin|sin(□);cos|cos(□);tan|tan(□);π;e;\
asin|asin(□);acos|acos(□);atan|atan(□);i;!;\
ln|ln(□);log₁₀|log10(□);√□|√(□);∛□|∛(□);abs|abs(□);\
□²|□²;□^□|□^□;1/□|1/□;EE|E;%;\
(;);,;←|@left;⌫|@back

[Calculus]
columns=4
keys=d/dx|diff(□, x);∫ dx|integrate(□, , , x);∫ₐᵇ|integrate(□, □, □, x);Σ|sum(□, □, □, n);\
lim|limit(□, □, x);solve|solve(□ = □, x);plot|plot(□, □, □);∞;\
x;y;n;=;\
(;);,;⌫|@back

[Matrix]
columns=4
keys=2×2|[□ □\\; □ □];3×3|[□ □ □\\; □ □ □\\; □ □ □];det|det(□);inverse|inverse(□);\
transpose|transpose(□);rank|rk(□);×;^;\
[;];\\;;␣| ;\
(;);,;⌫|@back

[Convert]
columns=6
keys=to| to ;→ a/b| to fraction;→ hex| to hex;→ bin| to bin;→ oct| to oct;⌫|@back;\
m;km;mi;ft;in;kg;\
lb;s;min;h;°C;°F;\
K;J;W;V;A;Ω;\
Hz;Pa;bar;←|@left;→|@right;=|@enter

[Letters]
columns=7
keys=x;y;z;a;b;c;n;\
t;α;β;γ;δ;θ;λ;\
μ;π;σ;φ;ω;Δ;Ω;\
:=| := ;=;,;(;);←|@left;⌫|@back
";

/// What a key does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Insert(String),
    Back,
    Clear,
    Left,
    Right,
    Enter,
}

#[derive(Debug, PartialEq)]
pub struct Page {
    pub name: String,
    pub columns: usize,
    /// row-major; None = gap
    pub keys: Vec<Option<(String, Act)>>,
}

/// keypad.ini text -> pages; Err names the first bad entry.
pub fn parse(text: &str) -> Result<Vec<Page>, String> {
    let kf = glib::KeyFile::new();
    kf.load_from_data(text, glib::KeyFileFlags::NONE).map_err(|e| e.to_string())?;
    let mut pages = vec![];
    for g in kf.groups().iter() {
        let g = g.as_str();
        let columns = match kf.string(g, "columns") {
            Ok(c) => c.trim().parse().ok().filter(|c| (1..=12).contains(c)).ok_or(format!("[{g}] columns={c}: want 1-12"))?,
            Err(_) => 5,
        };
        let list = kf.string_list(g, "keys").map_err(|_| format!("[{g}]: no keys="))?;
        let keys = list.iter().map(|k| key(k.as_str()).map_err(|e| format!("[{g}] {e}"))).collect::<Result<Vec<_>, _>>()?;
        pages.push(Page { name: g.to_string(), columns, keys });
    }
    if pages.is_empty() {
        return Err("no pages".into());
    }
    Ok(pages)
}

fn key(item: &str) -> Result<Option<(String, Act)>, String> {
    if item.trim().is_empty() {
        return Ok(None);
    }
    let (label, tpl) = match item.split_once('|') {
        Some((l, t)) => (l.trim(), t),
        None => (item.trim(), item.trim()),
    };
    let act = match tpl.trim() {
        "@back" => Act::Back,
        "@clear" => Act::Clear,
        "@left" => Act::Left,
        "@right" => Act::Right,
        "@enter" => Act::Enter,
        "@ans" => Act::Insert("ans".into()),
        a if a.starts_with('@') => return Err(format!("'{item}': unknown action {a}")),
        _ if tpl.is_empty() => return Err(format!("'{item}': empty template")),
        _ => Act::Insert(tpl.to_string()), // untrimmed: " to " and a space key need their spaces
    };
    if label.is_empty() {
        return Err(format!("'{item}': empty label"));
    }
    Ok(Some((label.to_string(), act)))
}

/// Pages from `path`, writing the defaults there on first run; warnings for the console.
pub fn load(path: &std::path::Path) -> (Vec<Page>, String) {
    let defaults = || parse(DEFAULT_KEYPAD).unwrap_or_default();
    match std::fs::read_to_string(path) {
        Ok(t) => match parse(&t) {
            Ok(p) => (p, String::new()),
            Err(e) => (defaults(), format!("{}: {e}; using the default keypad\n", path.display())),
        },
        Err(_) => {
            let w = std::fs::write(path, DEFAULT_KEYPAD).err().map_or(String::new(), |e| format!("{}: {e}\n", path.display()));
            (defaults(), w)
        }
    }
}

/// Text to insert for `tpl` with the selected text `sel` wrapped into its first □ (in parentheses
/// where it needs them), plus the char offset of the □ to select afterwards, if any is left.
pub fn expand(tpl: &str, sel: &str) -> (String, Option<usize>) {
    let slot_at = |s: &str, from: usize| s[from..].find(SLOT).map(|j| s[..from + j].chars().count());
    let wrap = !sel.is_empty() && sel.chars().any(|c| c != SLOT);
    match tpl.find(SLOT) {
        Some(i) if wrap => {
            let arg = tpl[..i].ends_with(['(', '[', ',', ';', ' ']);
            let atom = sel.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_') || (sel.starts_with('(') && sel.ends_with(')'));
            let sel = if arg || atom { sel.to_string() } else { format!("({sel})") };
            let out = format!("{}{sel}{}", &tpl[..i], &tpl[i + SLOT.len_utf8()..]);
            let next = slot_at(&out, i + sel.len()).or_else(|| slot_at(&out, 0));
            (out, next)
        }
        _ => (tpl.to_string(), slot_at(tpl, 0)),
    }
}

/// Char offsets of every □ in `text`.
pub fn slots(text: &str) -> Vec<usize> {
    text.chars().enumerate().filter(|(_, c)| *c == SLOT).map(|(i, _)| i).collect()
}

/// The □ after the selection `(start, end)` (before it when `back`), wrapping around.
pub fn next_slot(text: &str, (start, end): (usize, usize), back: bool) -> Option<usize> {
    let s = slots(text);
    if back { s.iter().rev().find(|&&i| i < start).or(s.last()) } else { s.iter().find(|&&i| i >= end).or(s.first()) }.copied()
}

/// Keypad text for an Octave target: the calculator's math symbols in ASCII.
// ponytail: symbols only; qalc-only names (ln, integrate, °C ...) go in as is
pub fn octave(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '×' => out.push('*'),
            '÷' => out.push('/'),
            '−' => out.push('-'),
            'π' => out.push_str("pi"),
            '√' => out.push_str("sqrt"),
            '∛' => out.push_str("cbrt"),
            '²' => out.push_str("^2"),
            '³' => out.push_str("^3"),
            c => out.push(c),
        }
    }
    out
}

/// .op for operators and actions, .fn for function/conversion templates; digits, letters, units plain.
fn class(act: &Act) -> Option<&'static str> {
    match act {
        Act::Insert(t) if t.contains(['(', SLOT, ' ']) => Some("fn"),
        Act::Insert(t) if t.chars().any(|c| c.is_alphanumeric() || c == '.') => None,
        _ => Some("op"),
    }
}

/// Keypad widget: page switcher over a stack of button grids; `press` gets each key's action.
pub fn widget(pages: &[Page], press: impl Fn(&Act) + 'static) -> gtk::Box {
    let press = Rc::new(press);
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::SlideLeftRight).build();
    for p in pages {
        let grid = gtk::Grid::builder().row_homogeneous(true).column_homogeneous(true).row_spacing(4).column_spacing(4).build();
        grid.add_css_class(&p.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-"));
        for (i, k) in p.keys.iter().enumerate() {
            let Some((label, act)) = k else { continue };
            // focus stays in the text widget, so its selection and cursor survive the tap
            let b = gtk::Button::builder().label(label).focus_on_click(false).hexpand(true).vexpand(true).build();
            if let Some(c) = class(act) {
                b.add_css_class(c);
            }
            match act {
                Act::Enter => b.add_css_class("suggested-action"),
                Act::Clear => b.add_css_class("destructive-action"),
                _ => {}
            }
            let (press, act) = (press.clone(), act.clone());
            b.connect_clicked(move |_| press(&act));
            grid.attach(&b, (i % p.columns) as i32, (i / p.columns) as i32, 1, 1);
        }
        stack.add_titled(&grid, None, &p.name);
    }
    let switcher = gtk::StackSwitcher::builder().stack(&stack).halign(gtk::Align::Center).build();
    let top = gtk::ScrolledWindow::builder().child(&switcher).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).build();
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).css_classes(["keypad"]).build();
    b.append(&top);
    b.append(&stack);
    b
}

/// A text widget the keypad types into. `octave`: REPL entry or script (ASCII operators).
pub enum Target<'a> {
    Entry(&'a gtk::Entry, bool),
    Text(&'a gtk::TextView),
}

pub fn press(t: Target, act: &Act) {
    match t {
        Target::Entry(e, oct) => {
            let pos = e.position();
            let (a, b) = e.selection_bounds().unwrap_or((pos, pos));
            match act {
                Act::Insert(tpl) => {
                    let tpl = if oct { octave(tpl) } else { tpl.clone() };
                    let (text, slot) = expand(&tpl, &e.chars(a, b));
                    e.delete_text(a, b);
                    let mut p = a;
                    e.insert_text(&text, &mut p);
                    match slot {
                        Some(k) => e.select_region(a + k as i32, a + k as i32 + 1),
                        None => e.set_position(p),
                    }
                }
                Act::Back if a < b => e.delete_text(a, b),
                Act::Back => e.delete_text((pos - 1).max(0), pos),
                Act::Clear => e.set_text(""),
                Act::Left => e.set_position((pos - 1).max(0)),
                Act::Right => e.set_position((pos + 1).min(e.text().chars().count() as i32)),
                Act::Enter => e.emit_activate(),
            }
        }
        Target::Text(v) => {
            let buf = v.buffer();
            let cur = buf.iter_at_mark(&buf.get_insert());
            let (mut s, mut e) = buf.selection_bounds().unwrap_or((cur, cur));
            match act {
                Act::Insert(tpl) => {
                    let (text, slot) = expand(&octave(tpl), &buf.text(&s, &e, false));
                    let off = s.offset();
                    buf.begin_user_action();
                    buf.delete(&mut s, &mut e);
                    buf.insert(&mut s, &text);
                    buf.end_user_action();
                    match slot {
                        Some(k) => buf.select_range(&buf.iter_at_offset(off + k as i32), &buf.iter_at_offset(off + k as i32 + 1)),
                        None => buf.place_cursor(&s),
                    }
                }
                Act::Back => {
                    if !buf.delete_selection(true, true) {
                        buf.backspace(&mut s, true, true);
                    }
                }
                Act::Clear => {} // never wipe a script from a calculator key
                Act::Left => {
                    s.backward_char();
                    buf.place_cursor(&s);
                }
                Act::Right => {
                    e.forward_char();
                    buf.place_cursor(&e);
                }
                Act::Enter => {
                    buf.delete_selection(true, true);
                    buf.insert_at_cursor("\n");
                }
            }
            v.scroll_mark_onscreen(&buf.get_insert());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        assert_eq!(expand("sin(□)", ""), ("sin(□)".into(), Some(4)));
        assert_eq!(expand("7", "□"), ("7".into(), None)); // typing over a slot
        assert_eq!(expand("sin(□)", "x+1"), ("sin(x+1)".into(), None));
        assert_eq!(expand("□²", "x+1"), ("(x+1)²".into(), None));
        assert_eq!(expand("√(□)", "2"), ("√(2)".into(), None));
        assert_eq!(expand("1/□", "(a+b)"), ("1/(a+b)".into(), None));
        assert_eq!(expand("□^□", "x"), ("x^□".into(), Some(2)));
        assert_eq!(expand("□^□", "2x+1"), ("(2x+1)^□".into(), Some(7)));
        assert_eq!(expand("integrate(□, □, □, x)", "x²"), ("integrate(x², □, □, x)".into(), Some(14)));
        assert_eq!(expand("sin(□)", "□"), ("sin(□)".into(), Some(4))); // a bare slot isn't wrapped
        assert_eq!(expand("abs(□)", "□+1"), ("abs(□+1)".into(), Some(4))); // slot inside the selection
        assert_eq!(expand(" to ", ""), (" to ".into(), None));
        assert_eq!(octave("2×π÷√(□)−x²"), "2*pi/sqrt(□)-x^2");
    }

    #[test]
    fn placeholder_navigation() {
        let t = "integrate(□, □, □, x)";
        assert_eq!(slots(t), [10, 13, 16]);
        assert_eq!(next_slot(t, (0, 0), false), Some(10));
        assert_eq!(next_slot(t, (10, 11), false), Some(13));
        assert_eq!(next_slot(t, (16, 17), false), Some(10)); // wraps
        assert_eq!(next_slot(t, (13, 14), true), Some(10));
        assert_eq!(next_slot(t, (10, 11), true), Some(16)); // wraps back
        assert_eq!(next_slot(t, (21, 21), true), Some(16));
        assert_eq!(next_slot("x^2", (0, 0), false), None);
        assert_eq!(next_slot("√(□)", (0, 0), false), Some(2)); // char offsets, not bytes
    }

    #[test]
    fn keypad_file() {
        let pages = parse(DEFAULT_KEYPAD).unwrap();
        let names: Vec<&str> = pages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Basic", "Scientific", "Calculus", "Matrix", "Convert", "Letters"]);
        let find = |p: usize, l: &str| pages[p].keys.iter().flatten().find(|(k, _)| k == l).map(|(_, a)| a.clone());
        assert_eq!(find(0, "7"), Some(Act::Insert("7".into())));
        assert_eq!(find(0, "⌫"), Some(Act::Back));
        assert_eq!(find(0, "="), Some(Act::Enter));
        assert_eq!(find(0, "ans"), Some(Act::Insert("ans".into())));
        assert_eq!(find(3, "2×2"), Some(Act::Insert("[□ □; □ □]".into()))); // \; is a literal ;
        assert_eq!(find(4, "to"), Some(Act::Insert(" to ".into())));
        assert!(pages.iter().all(|p| p.keys.len() <= p.columns * 6));

        let p = parse("[Mine]\ncolumns=2\nkeys=a;;b|x(□);  c ;@enter\n").unwrap();
        assert_eq!(p[0].keys, [Some(("a".into(), Act::Insert("a".into()))), None, Some(("b".into(), Act::Insert("x(□)".into()))), Some(("c".into(), Act::Insert("c".into()))), Some(("@enter".into(), Act::Enter))]);
        for bad in ["[P]\ncolumns=0\nkeys=a", "[P]\ncolumns=x\nkeys=a", "[P]\ncolumns=3", "[P]\nkeys=a|@nope", "[P]\nkeys=|x", "[P]\nkeys=a|", "", "keys=a", "not ini ["] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(class(&Act::Insert("sin(□)".into())), Some("fn"));
        assert_eq!(class(&Act::Insert("×".into())), Some("op"));
        assert_eq!(class(&Act::Back), Some("op"));
        assert_eq!(class(&Act::Insert("km".into())), None);
    }
}
