//! Functions popover: classroom functions by topic, searchable; a tap inserts the template
//! (□ marks a slot) into the calculator entry or the graph row being edited.
use gtk::prelude::*;
use gtk::glib;
use gtk4 as gtk;
use std::rc::Rc;

/// (topic, [(template, description, example)]). Examples are in degrees, like a fresh Simple mode.
pub const GROUPS: &[(&str, &[(&str, &str, &str)])] = &[
    (
        "Powers and roots",
        &[
            ("□^□", "Power: a to the b", "2^5 = 32"),
            ("□²", "Square", "3² = 9"),
            ("√(□)", "Square root", "√(16) = 4"),
            ("∛(□)", "Cube root", "∛(27) = 3"),
            ("root(□, □)", "n-th root: root(a, n)", "root(16, 4) = 2"),
            ("abs(□)", "Absolute value (distance from 0)", "abs(-7) = 7"),
            ("□!", "Factorial: n × (n − 1) × … × 1", "5! = 120"),
            ("%", "Percent", "20% × 50 = 10"),
        ],
    ),
    (
        "Exponentials and logarithms",
        &[
            ("e^□", "e to a power", "e^2 ≈ 7.389056099"),
            ("ln(□)", "Natural logarithm (base e)", "ln(e^3) = 3"),
            ("log10(□)", "Logarithm base 10", "log10(1000) = 3"),
            ("log(□, □)", "Logarithm with a base: log(a, base)", "log(8, 2) = 3"),
        ],
    ),
    (
        "Trigonometry",
        &[
            ("sin(□)", "Sine", "sin(30) = 1/2"),
            ("cos(□)", "Cosine", "cos(60) = 1/2"),
            ("tan(□)", "Tangent", "tan(45) = 1"),
            ("asin(□)", "Inverse sine (sin⁻¹)", "asin(1/2) = 30"),
            ("acos(□)", "Inverse cosine (cos⁻¹)", "acos(1/2) = 60"),
            ("atan(□)", "Inverse tangent (tan⁻¹)", "atan(1) = 45"),
            ("sec(□)", "Secant: 1 / cos", "sec(60) = 2"),
            ("csc(□)", "Cosecant: 1 / sin", "csc(30) = 2"),
            ("cot(□)", "Cotangent: 1 / tan", "cot(45) = 1"),
            ("sinh(□)", "Hyperbolic sine", "sinh(0) = 0"),
            ("cosh(□)", "Hyperbolic cosine", "cosh(0) = 1"),
            ("tanh(□)", "Hyperbolic tangent", "tanh(0) = 0"),
            ("°", "Degrees, whatever DEG / RAD says", "sin(90°) = 1"),
        ],
    ),
    (
        "Rounding and whole numbers",
        &[
            ("round(□)", "Round to the nearest whole number", "round(2.5) = 3"),
            ("floor(□)", "Round down", "floor(2.7) = 2"),
            ("ceil(□)", "Round up", "ceil(2.1) = 3"),
            ("trunc(□)", "Drop the decimals", "trunc(-2.7) = −2"),
            ("sgn(□)", "Sign: −1, 0 or 1", "sgn(-3) = −1"),
            ("mod(□, □)", "Remainder of a ÷ b", "mod(17, 5) = 2"),
            ("gcd(□, □)", "Greatest common divisor", "gcd(12, 18) = 6"),
            ("lcm(□, □)", "Least common multiple", "lcm(4, 6) = 12"),
            ("binomial(□, □)", "Combinations nCr: choose r of n", "binomial(5, 2) = 10"),
            ("perm(□, □)", "Permutations nPr: ordered choices", "perm(5, 2) = 20"),
        ],
    ),
    (
        "Statistics",
        &[
            ("mean(□, □)", "Average of the values", "mean(2, 4, 9) = 5"),
            ("median(□, □)", "Middle value", "median(3, 1, 7) = 3"),
            ("max(□, □)", "Largest value", "max(3, 8, 5) = 8"),
            ("min(□, □)", "Smallest value", "min(3, 8, 5) = 3"),
        ],
    ),
    (
        "Constants",
        &[
            ("π", "Pi: a circle's circumference ÷ diameter", "π ≈ 3.141592654"),
            ("e", "Euler's number", "e ≈ 2.718281828"),
            ("golden", "Golden ratio φ", "golden ≈ 1.618033989"),
            ("x", "The variable of a graph", "y = 2x + 1"),
        ],
    ),
];

fn text(s: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(s).xalign(0.0).wrap(true).build();
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}

/// The popover; `pick` gets the tapped template.
pub fn popover(pick: impl Fn(&'static str) + 'static) -> gtk::Popover {
    let search = gtk::SearchEntry::builder().placeholder_text("Search functions").build();
    let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["simple-catalog"]).build();
    // per topic: its heading row and its (row, template, search text)
    let mut groups: Vec<(gtk::ListBoxRow, Vec<(gtk::ListBoxRow, &'static str, String)>)> = vec![];
    for (topic, items) in GROUPS {
        let head = gtk::ListBoxRow::builder().activatable(false).selectable(false).child(&text(topic, "catalog-topic")).build();
        list.append(&head);
        let mut rows = vec![];
        for (template, desc, example) in *items {
            let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let name = text(template, "catalog-name");
            name.set_wrap(false);
            top.append(&name);
            let d = text(desc, "dim-label");
            d.set_hexpand(true);
            top.append(&d);
            body.append(&top);
            body.append(&text(example, "catalog-example"));
            let row = gtk::ListBoxRow::builder().child(&body).tooltip_text("Tap to insert").build();
            list.append(&row);
            rows.push((row, *template, format!("{topic} {template} {desc}").to_lowercase()));
        }
        groups.push((head, rows));
    }
    let scroll = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).min_content_height(420).build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_size_request(420, -1);
    content.append(&search);
    content.append(&scroll);
    let pop = gtk::Popover::builder().child(&content).build();
    let groups = Rc::new(groups);
    let pick = Rc::new(pick);

    let (g, p, picked) = (groups.clone(), pop.downgrade(), pick.clone());
    list.connect_row_activated(move |_, row| {
        let Some(t) = g.iter().flat_map(|(_, rows)| rows).find(|(r, _, _)| r == row).map(|(_, t, _)| *t) else { return };
        if let Some(p) = p.upgrade() {
            p.popdown();
        }
        // after the popover gives focus back
        let picked = picked.clone();
        glib::idle_add_local_once(move || picked(t));
    });
    let g = groups.clone();
    search.connect_search_changed(move |s| {
        let q = s.text().to_lowercase();
        for (head, rows) in g.iter() {
            let mut any = false;
            for (row, _, hay) in rows {
                let hit = q.split_whitespace().all(|w| hay.contains(w));
                row.set_visible(hit);
                any |= hit;
            }
            head.set_visible(any);
        }
    });
    // Enter takes the first match
    let (g, l) = (groups.clone(), list.downgrade());
    search.connect_activate(move |_| {
        if let (Some((row, _, _)), Some(l)) = (g.iter().flat_map(|(_, rows)| rows).find(|(r, _, _)| r.is_visible()), l.upgrade()) {
            l.emit_by_name::<()>("row-activated", &[row]);
        }
    });
    let (s, adj) = (search.clone(), scroll.vadjustment());
    pop.connect_show(move |_| {
        s.set_text("");
        adj.set_value(0.0);
    });
    pop
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{gui::keypad::SLOT, qalc};

    #[test]
    fn templates_and_examples_calculate() {
        let o = qalc::Options { angle: "deg".into(), ..Default::default() };
        for (topic, items) in GROUPS {
            for (template, _, example) in *items {
                if *template != "x" && *template != "%" && *template != "°" {
                    let ok = ["2", "1/2"].iter().any(|v| qalc::classroom_eval(&template.replace(SLOT, v), &o).is_ok());
                    assert!(ok, "{topic}: {template}");
                }
                let Some((input, want, exact)) = example.split_once(" = ").map(|(a, b)| (a, b, true)).or_else(|| example.split_once(" ≈ ").map(|(a, b)| (a, b, false))) else {
                    panic!("{example}: no = or ≈")
                };
                if input.starts_with("y") {
                    continue;
                }
                let a = qalc::classroom_eval(input, &o).unwrap_or_else(|e| panic!("{example}: {e}"));
                let got = if exact { a.result } else { a.approx };
                assert_eq!(got, want, "{example}");
            }
        }
    }
}
