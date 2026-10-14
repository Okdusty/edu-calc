//! On-screen keypad: one mode's keys (General, Programming, Algebra, Custom pages) and the number
//! pad, from keypad.ini (DEFAULT_KEYPAD when missing or bad, or still the previous version's
//! default), and the □ placeholder logic shared by the keypad and the calculator entry.
use std::rc::Rc;

use gtk4 as gtk;
use gtk::glib;
use gtk::prelude::*;

/// Placeholder in templates: the selection is wrapped into the first one, Tab moves between them.
pub const SLOT: char = '□';

pub const DEFAULT_KEYPAD: &str = "\
# qcalcp on-screen keypad. Delete this file to get the defaults back.
# Modes (keypad menu in the header): [General] [Programming] [Algebra] [Custom], with the
# [Number pad] beside them. Every group with another name is one more Custom page (a switcher shows
# when there are several). A missing General / Programming / Algebra / Number pad uses the built-in one.
#   columns=N                 buttons per row (1-12)
#   keys=label|template;...   keys in rows of N; an empty key leaves a gap; write \\; for a literal ;
# template: the text inserted. A bare label inserts the label itself. □ marks a slot: a selection is
#   wrapped into the first one and the next one gets selected (Tab / Shift+Tab move between them).
# actions: @back @clear @left @right @enter (evaluate) @ans
#          @base:N  type numbers and show results in base N (2, 8, 10, 16); A-F need 16
# Templates are calculator (libqalculate) syntax; in Octave mode × ÷ − π √ ∛ ² ³ become ASCII.
# A bad entry makes qcalcp use these defaults and say why in the console.
# style.css: .keypad  .keypad button  .keypad .op (operators, actions)  .keypad .fn (functions)
#            .keypad .general (group name, lowercase, - for other chars: .number-pad)

[General]
columns=5
keys=sin|sin(□);cos|cos(□);tan|tan(□);π;e;\
asin|asin(□);acos|acos(□);atan|atan(□);i;!;\
ln|ln(□);log₁₀|log10(□);√□|√(□);∛□|∛(□);abs|abs(□);\
□²|□²;□^□|□^□;1/□|1/□;EE|E;%;\
x;y;to| to ;mod| mod ;=

[Programming]
columns=6
keys=BIN|@base:2;OCT|@base:8;DEC|@base:10;HEX|@base:16;<<;>>;\
A;B;C;D;E;F;\
AND|&;OR||;XOR| xor ;NOT|~;cmp|bitcmp(□);rot|bitrot(□, □);\
&&;‖|||;!;mod| mod ;div|//;log₂|log2(□)

[Algebra]
columns=5
keys=d/dx|diff(□, x);∫ dx|integrate(□, , , x);∫ₐᵇ|integrate(□, □, □, x);Σ|sum(□, □, □, n);lim|limit(□, □, x);\
solve|solve(□ = □, x);extrema|extrema(□);plot|plot(□, □, □);∞;=;\
2×2|[□ □\\; □ □];3×3|[□ □ □\\; □ □ □\\; □ □ □];det|det(□);A⁻¹|inverse(□);Aᵀ|transpose(□);\
rank|rk(□);[;];\\;;x

[Custom]
columns=6
keys=to| to ;→ a/b| to fraction;→ hex| to hex;→ bin| to bin;→ oct| to oct;°;\
m;km;mi;ft;in;kg;\
lb;s;min;h;°C;°F;\
K;J;W;V;A;Ω;\
Hz;Pa;bar;L;N;eV

[Greek]
columns=7
keys=α;β;γ;δ;ε;θ;λ;\
μ;π;σ;τ;φ;ω;Δ;\
x;y;z;a;b;c;n;\
t;:=| := ;=;,;␣| ;←|@left;→|@right

[Number pad]
columns=5
keys=AC|@clear;(;);^;⌫|@back;\
7;8;9;÷;←|@left;\
4;5;6;×;→|@right;\
1;2;3;−;ans|@ans;\
0;.;,;+;=|@enter
";

/// Keypad modes (qcalcp.ini keypad_mode, keypad menu) and their keypad.ini group; "custom" takes
/// every group that isn't another mode's. "numpad" is the number pad alone.
pub const MODES: [(&str, &str); 5] = [("general", "General"), ("programming", "Programming"), ("algebra", "Algebra"), ("custom", "Custom"), ("numpad", "Number pad")];

/// The previous version's default keypad.ini: a file still equal to it is replaced by DEFAULT_KEYPAD.
const OLD_DEFAULT: &str = "\
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
    /// @base:N: input and result base (handled by the GUI, not the text widget)
    Base(u32),
}

#[derive(Clone, Debug, PartialEq)]
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
        a if a.starts_with("@base:") => Act::Base(a[6..].parse().ok().filter(|b| (2..=36).contains(b)).ok_or(format!("'{item}': want @base:2 .. @base:36"))?),
        a if a.starts_with('@') => return Err(format!("'{item}': unknown action {a}")),
        _ if tpl.is_empty() => return Err(format!("'{item}': empty template")),
        _ => Act::Insert(tpl.to_string()), // untrimmed: " to " and a space key need their spaces
    };
    if label.is_empty() {
        return Err(format!("'{item}': empty label"));
    }
    Ok(Some((label.to_string(), act)))
}

/// Pages from `path`, writing the defaults there on first run (and over the previous version's
/// untouched defaults); warnings for the console.
pub fn load(path: &std::path::Path) -> (Vec<Page>, String) {
    let defaults = || parse(DEFAULT_KEYPAD).unwrap_or_default();
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim() == OLD_DEFAULT.trim() => {
            let w = std::fs::write(path, DEFAULT_KEYPAD).err().map_or(String::new(), |e| format!("{}: {e}\n", path.display()));
            (defaults(), w)
        }
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

/// Each mode's pages, in MODES order: the group named like the mode (the default file's when
/// missing); Custom gets every group that isn't another mode's (the default Custom pages when none).
pub fn arrange(pages: &[Page]) -> Vec<Vec<Page>> {
    let def = parse(DEFAULT_KEYPAD).unwrap_or_default();
    let named = |p: &Page| MODES.iter().any(|&(m, g)| m != "custom" && g == p.name);
    let block = |ps: &[Page], mode: &str, group: &str| -> Vec<Page> {
        ps.iter().filter(|p| if mode == "custom" { !named(p) } else { p.name == group }).cloned().collect()
    };
    MODES.iter().map(|&(m, g)| Some(block(pages, m, g)).filter(|b| !b.is_empty()).unwrap_or_else(|| block(&def, m, g))).collect()
}

/// On-screen keypad: one mode block at a time (stack) and the number pad beside or below it.
pub struct Keypad {
    pub root: gtk::Box,
    modes: gtk::Stack,
    numpad: gtk::Box,
    keys: Vec<(gtk::Button, Act)>,
    /// A-F of the Programming block, usable with hexadecimal input
    hex: Vec<gtk::Button>,
}

impl Keypad {
    pub fn new(pages: &[Page]) -> Keypad {
        let (mut keys, mut hex) = (vec![], vec![]);
        let modes = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).build();
        let numpad = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for (&(mode, _), pages) in MODES.iter().zip(arrange(pages)) {
            let first = keys.len();
            let w = block(&pages, &mut keys);
            if mode == "programming" {
                let digit = |a: &Act| matches!(a, Act::Insert(t) if matches!(t.as_str(), "A" | "B" | "C" | "D" | "E" | "F"));
                hex = keys[first..].iter().filter(|(_, a)| digit(a)).map(|(b, _)| b.clone()).collect();
            }
            if mode == "numpad" {
                numpad.append(&w);
            } else {
                modes.add_named(&w, Some(mode));
            }
        }
        // hexpand(false): the buttons' hexpand would take the calculator's width when beside it
        let root = gtk::Box::builder().spacing(8).hexpand(false).css_classes(["keypad"]).build();
        root.append(&modes);
        root.append(&numpad);
        Keypad { root, modes, numpad, keys, hex }
    }

    /// `press` gets each key's action.
    pub fn connect(&self, press: impl Fn(&Act) + 'static) {
        let press = Rc::new(press);
        for (b, act) in &self.keys {
            let (press, act) = (press.clone(), act.clone());
            b.connect_clicked(move |_| press(&act));
        }
    }

    /// `mode`'s block ("numpad": the number pad alone), plus the number pad when `numpad`.
    pub fn show(&self, mode: &str, numpad: bool, side_by_side: bool) {
        self.modes.set_visible(mode != "numpad");
        if self.modes.child_by_name(mode).is_some() {
            self.modes.set_visible_child_name(mode);
        }
        self.numpad.set_visible(numpad || mode == "numpad");
        self.root.set_orientation(if side_by_side { gtk::Orientation::Horizontal } else { gtk::Orientation::Vertical });
    }

    /// Width the mode's keys and the number pad need side by side.
    pub fn side_by_side_width(&self) -> i32 {
        let min = |w: &gtk::Widget| w.measure(gtk::Orientation::Horizontal, -1).0;
        min(self.modes.upcast_ref()) + self.root.spacing() + min(self.numpad.upcast_ref())
    }

    /// Check the @base key of the input base; A-F only for hexadecimal input.
    pub fn set_base(&self, base: u32) {
        for (b, act) in &self.keys {
            if let (Act::Base(n), Some(t)) = (act, b.downcast_ref::<gtk::ToggleButton>()) {
                t.set_active(*n == base);
            }
        }
        self.hex.iter().for_each(|b| b.set_sensitive(base >= 16));
    }
}

/// A mode block: its page's grid, or a page switcher over a stack of grids.
fn block(pages: &[Page], keys: &mut Vec<(gtk::Button, Act)>) -> gtk::Widget {
    if let [p] = pages {
        return grid(p, keys).upcast();
    }
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::SlideLeftRight).build();
    for p in pages {
        stack.add_titled(&grid(p, keys), None, &p.name);
    }
    let switcher = gtk::StackSwitcher::builder().stack(&stack).halign(gtk::Align::Center).build();
    let top = gtk::ScrolledWindow::builder().child(&switcher).vscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).build();
    let b = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
    b.append(&top);
    b.append(&stack);
    b.upcast()
}

/// A page's buttons; @base keys are toggles showing the active base.
fn grid(p: &Page, keys: &mut Vec<(gtk::Button, Act)>) -> gtk::Grid {
    let grid = gtk::Grid::builder().row_homogeneous(true).column_homogeneous(true).row_spacing(4).column_spacing(4).build();
    grid.add_css_class(&p.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-"));
    for (i, k) in p.keys.iter().enumerate() {
        let Some((label, act)) = k else { continue };
        let b: gtk::Button = match act {
            Act::Base(_) => gtk::ToggleButton::with_label(label).upcast(),
            _ => gtk::Button::with_label(label),
        };
        // focus stays in the text widget, so its selection and cursor survive the tap
        b.set_focus_on_click(false);
        b.set_hexpand(true);
        b.set_vexpand(true);
        if let Some(c) = class(act) {
            b.add_css_class(c);
        }
        match act {
            Act::Enter => b.add_css_class("suggested-action"),
            Act::Clear => b.add_css_class("destructive-action"),
            _ => {}
        }
        grid.attach(&b, (i % p.columns) as i32, (i / p.columns) as i32, 1, 1);
        keys.push((b, act.clone()));
    }
    grid
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
                Act::Base(_) => {}
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
                Act::Base(_) => {}
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
        assert_eq!(names, ["General", "Programming", "Algebra", "Custom", "Greek", "Number pad"]);
        let find = |p: &str, l: &str| pages.iter().find(|g| g.name == p)?.keys.iter().flatten().find(|(k, _)| k == l).map(|(_, a)| a.clone());
        assert_eq!(find("Number pad", "7"), Some(Act::Insert("7".into())));
        assert_eq!(find("Number pad", "⌫"), Some(Act::Back));
        assert_eq!(find("Number pad", "="), Some(Act::Enter));
        assert_eq!(find("Number pad", "ans"), Some(Act::Insert("ans".into())));
        assert_eq!(find("Algebra", "2×2"), Some(Act::Insert("[□ □; □ □]".into()))); // \; is a literal ;
        assert_eq!(find("Algebra", ";"), Some(Act::Insert(";".into())));
        assert_eq!(find("General", "to"), Some(Act::Insert(" to ".into())));
        assert_eq!(find("Programming", "HEX"), Some(Act::Base(16)));
        assert_eq!(find("Programming", "OR"), Some(Act::Insert("|".into())));
        assert_eq!(find("Programming", "‖"), Some(Act::Insert("||".into())));
        assert_eq!(find("Programming", "XOR"), Some(Act::Insert(" xor ".into())));
        assert!(pages.iter().all(|p| p.keys.len() <= p.columns * 6));

        let p = parse("[Mine]\ncolumns=2\nkeys=a;;b|x(□);  c ;@enter;b3|@base:3\n").unwrap();
        let k = |l: &str, a: Act| Some((l.to_string(), a));
        assert_eq!(p[0].keys, [k("a", Act::Insert("a".into())), None, k("b", Act::Insert("x(□)".into())), k("c", Act::Insert("c".into())), k("@enter", Act::Enter), k("b3", Act::Base(3))]);
        for bad in ["[P]\ncolumns=0\nkeys=a", "[P]\ncolumns=x\nkeys=a", "[P]\ncolumns=3", "[P]\nkeys=a|@nope", "[P]\nkeys=|x", "[P]\nkeys=a|", "", "keys=a", "not ini [", "[P]\nkeys=a|@base:1", "[P]\nkeys=a|@base:x"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(class(&Act::Insert("sin(□)".into())), Some("fn"));
        assert_eq!(class(&Act::Insert("×".into())), Some("op"));
        assert_eq!(class(&Act::Back), Some("op"));
        assert_eq!(class(&Act::Insert("km".into())), None);
    }

    #[test]
    fn modes_and_migration() {
        let names = |b: Vec<Vec<Page>>| b.iter().map(|ps| ps.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(",")).collect::<Vec<_>>();
        assert_eq!(names(arrange(&parse(DEFAULT_KEYPAD).unwrap())), ["General", "Programming", "Algebra", "Custom,Greek", "Number pad"]);
        // an edited old-style file: its pages become Custom pages, the modes come from the defaults
        let old = parse(OLD_DEFAULT).unwrap();
        assert_eq!(names(arrange(&old)), ["General", "Programming", "Algebra", "Basic,Scientific,Calculus,Matrix,Convert,Letters", "Number pad"]);
        let a = arrange(&parse("[Number pad]\ncolumns=3\nkeys=1;2;3\n[General]\nkeys=x\n").unwrap());
        assert_eq!((a[0][0].keys.len(), a[4][0].columns), (1, 3)); // the user's
        assert_eq!(names(a)[3], "Custom,Greek"); // no other groups: the default Custom pages

        // the untouched previous default is replaced; an edited file is kept
        let path = std::env::temp_dir().join(format!("qcalcp-keypad-{}.ini", std::process::id()));
        std::fs::write(&path, OLD_DEFAULT).unwrap();
        let (p, w) = load(&path);
        assert_eq!((p.len(), w.as_str()), (6, ""));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_KEYPAD);
        let edited = OLD_DEFAULT.replace("columns=7", "columns=6");
        std::fs::write(&path, &edited).unwrap();
        assert_eq!(load(&path).0[0].name, "Basic");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
        let _ = std::fs::remove_file(&path);
    }
}
