//! Octave's default "tex" text markup -> runs of plain text, each with a size and a baseline shift.
//! `x^2`, `x^{10}`, `v_0`, `a_{ij}`, `e^(-t)` (the parens drop, for qalc's math view), `\alpha`,
//! `\int`, `\{` ... Unknown commands stay literal; Unicode passes through.

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    /// font size relative to the base size
    pub scale: f64,
    /// baseline shift up, in base line heights (negative = down)
    pub shift: f64,
    /// starts where the previous run started (a subscript under a superscript: `x_1^2`)
    pub stack: bool,
}

pub fn parse(s: &str) -> Vec<Run> {
    let c: Vec<char> = s.chars().collect();
    let mut out = vec![];
    walk(&c, 1.0, 0.0, &mut out);
    out
}

fn walk(c: &[char], scale: f64, shift: f64, out: &mut Vec<Run>) {
    // the script kind ('^' or '_') just finished, so the other kind stacks on it
    let mut last = None;
    let mut i = 0;
    while i < c.len() {
        let script = last.take();
        match c[i] {
            '{' => match close(c, i) {
                Some(j) => {
                    walk(&c[i + 1..j], scale, shift, out);
                    i = j + 1;
                }
                None => {
                    put(out, "{", scale, shift);
                    i += 1;
                }
            },
            ch @ ('^' | '_') if i + 1 < c.len() => {
                let up = if ch == '^' { 0.42 } else { -0.2 };
                let (sc, sh) = ((scale * 0.75).max(0.55), shift + up * scale);
                i += 1;
                // the script: {group}, (group), \command or one char
                let j = match c[i] {
                    '{' | '(' => close(c, i),
                    '\\' => Some(i + command(&c[i..]) - 1),
                    _ => None,
                };
                let first = out.len();
                match j {
                    Some(j) if c[i] != '\\' => walk(&c[i + 1..j], sc, sh, out),
                    _ => walk(&c[i..=j.unwrap_or(i)], sc, sh, out),
                }
                i = j.unwrap_or(i) + 1;
                if let Some(r) = out.get_mut(first) {
                    if script.is_some_and(|k| k != ch) { r.stack = true } else { last = Some(ch) }
                }
            }
            '\\' => {
                let n = command(&c[i..]);
                let name: String = c[i + 1..i + n].iter().collect();
                match symbol(&name) {
                    Some(sym) => put(out, sym, scale, shift),
                    None if n == 2 && !c[i + 1].is_ascii_alphabetic() => put(out, &name, scale, shift), // \{ \_ \\ ...
                    None if matches!(name.as_str(), "it" | "bf" | "rm" | "sl") => {}
                    None => put(out, &format!("\\{name}"), scale, shift),
                }
                i += n;
            }
            ch => {
                put(out, ch.encode_utf8(&mut [0; 4]), scale, shift);
                i += 1;
            }
        }
    }
}

/// Append text, merged into the last run when it has the same size and baseline.
fn put(out: &mut Vec<Run>, s: &str, scale: f64, shift: f64) {
    // big operators a size up and centered; in scripts '-' is a minus
    let (scale, shift) = if matches!(s, "∫" | "∮" | "∑" | "∏") { (scale * 1.3, shift - 0.1 * scale) } else { (scale, shift) };
    let s = if scale < 1.0 { s.replace('-', "\u{2212}") } else { s.to_string() };
    match out.last_mut() {
        Some(r) if r.scale == scale && r.shift == shift => r.text.push_str(&s),
        _ => out.push(Run { text: s, scale, shift, stack: false }),
    }
}

/// Index of the bracket closing the one at `c[i]`.
fn close(c: &[char], i: usize) -> Option<usize> {
    let (open, shut) = (c[i], if c[i] == '(' { ')' } else { '}' });
    let mut depth = 0;
    for (k, &ch) in c.iter().enumerate().skip(i) {
        depth += (ch == open) as i32 - (ch == shut) as i32;
        if depth == 0 {
            return Some(k);
        }
    }
    None
}

/// Length of the command starting with the backslash at `c[0]`: `\name` or `\x`.
fn command(c: &[char]) -> usize {
    match c[1..].iter().take_while(|c| c.is_ascii_alphabetic()).count() {
        0 => c.len().min(2),
        n => n + 1,
    }
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "varphi" => "ϕ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "int" => "∫",
        "oint" => "∮",
        "sum" => "∑",
        "prod" => "∏",
        "sqrt" | "surd" => "√",
        "infty" => "∞",
        "partial" => "∂",
        "nabla" => "∇",
        "pm" => "±",
        "mp" => "∓",
        "times" => "×",
        "cdot" => "·",
        "div" => "÷",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "propto" => "∝",
        "sim" => "∼",
        "deg" => "°",
        "circ" => "∘",
        "rightarrow" | "to" => "→",
        "leftarrow" => "←",
        "leftrightarrow" => "↔",
        "Rightarrow" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" => "⇔",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "in" => "∈",
        "forall" => "∀",
        "exists" => "∃",
        "prime" => "′",
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(text: &str, scale: f64, shift: f64) -> Run {
        Run { text: text.into(), scale, shift, stack: false }
    }
    const S: f64 = 0.75;

    #[test]
    fn markup() {
        assert_eq!(parse("x^2 + 1"), [r("x", 1.0, 0.0), r("2", S, 0.42), r(" + 1", 1.0, 0.0)]);
        assert_eq!(parse("x^{10}"), [r("x", 1.0, 0.0), r("10", S, 0.42)]);
        assert_eq!(parse("v_0 = a_{ij}"), [r("v", 1.0, 0.0), r("0", S, -0.2), r(" = a", 1.0, 0.0), r("ij", S, -0.2)]);
        // nesting: smaller and higher again, braces and parens are grouping only, '-' in scripts is a minus
        let e = parse("e^{-t/\\tau_0}");
        assert_eq!(e[1], r("\u{2212}t/τ", S, 0.42));
        assert_eq!((e[2].text.as_str(), e[2].scale), ("0", 0.5625));
        assert!((e[2].shift - (0.42 - 0.2 * S)).abs() < 1e-12);
        assert_eq!(parse("e^(−x / 2)")[1], r("−x / 2", S, 0.42));
        assert_eq!(parse("x^\\alpha")[1], r("α", S, 0.42));
        // symbols, big operators, stacked limits, escapes, font switches, unknown commands, plain Unicode
        let i = parse("\\int_0^\\infty \\alpha\\Omega");
        assert_eq!(i[0], r("∫", 1.3, -0.1));
        assert_eq!((i[1].clone(), i[2].stack), (r("0", S, -0.2), true));
        assert_eq!((i[2].text.as_str(), &i[3]), ("∞", &r(" αΩ", 1.0, 0.0)));
        assert!(!parse("x^2^3").iter().chain(&parse("x_{}^2")).any(|r| r.stack));
        assert_eq!(parse("\\bf{bold}\\it x\\_1 \\{a\\}"), [r("bold x_1 {a}", 1.0, 0.0)]);
        assert_eq!(parse("\\foo √x²"), [r("\\foo √x²", 1.0, 0.0)]);
        // malformed input stays literal instead of failing
        assert_eq!(parse("a^"), [r("a^", 1.0, 0.0)]);
        assert_eq!(parse("{x"), [r("{x", 1.0, 0.0)]);
        assert_eq!(parse("x^{2"), [r("x", 1.0, 0.0), r("{", S, 0.42), r("2", 1.0, 0.0)]);
        assert_eq!(parse("\\"), [r("\\", 1.0, 0.0)]);
        assert!(parse("").is_empty());
    }
}
