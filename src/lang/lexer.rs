//! Tokenizer. Inside `[]` / `{}` whitespace separates elements, so the lexer inserts `,` there
//! and turns newlines into `;`.

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    /// imaginary literal: `3i` -> Imag(3.0)
    Imag(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
    /// command syntax at statement start: `hold on` -> Cmd("hold", ["on"])
    Cmd(String, Vec<String>),
    Newline,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    /// byte span in the source
    pub pos: usize,
    pub end: usize,
}

pub const KEYWORDS: &[&str] = &[
    "if",
    "elseif",
    "else",
    "end",
    "endif",
    "for",
    "parfor",
    "endfor",
    "endparfor",
    "while",
    "endwhile",
    "do",
    "until",
    "switch",
    "case",
    "otherwise",
    "endswitch",
    "break",
    "continue",
    "return",
    "function",
    "endfunction",
    "try",
    "catch",
    "end_try_catch",
    "unwind_protect",
    "unwind_protect_cleanup",
    "end_unwind_protect",
    "global",
    "persistent",
];

const OPS2: &[&str] = &["==", "~=", "!=", "<=", ">=", "&&", "||", ".*", "./", ".\\", ".^", ".'", "+=", "-=", "*=", "/="];
const OPS1: &[&str] = &["+", "-", "*", "/", "\\", "^", "<", ">", "=", "&", "|", "!", "~", "(", ")", "[", "]", "{", "}", ",", ";", ":", ".", "@"];

struct Lx<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
    line: usize,
    toks: Vec<Token>,
    /// open brackets: b'(' b'[' b'{', or b'@' for an anonymous function's parameter list
    stack: Vec<u8>,
    /// last token closed `@(...)`: the body follows, so no element-splitting comma
    after_params: bool,
}

/// `is_var` decides command syntax: `name word` is a command only if `name` isn't a variable.
pub fn lex(src: &str, is_var: &dyn Fn(&str) -> bool) -> Result<Vec<Token>, String> {
    let mut lx = Lx { s: src, b: src.as_bytes(), i: 0, line: 1, toks: vec![], stack: vec![], after_params: false };
    while lx.i < lx.b.len() {
        lx.next(is_var)?;
    }
    let n = src.len();
    lx.toks.push(Token { tok: Tok::Eof, line: lx.line, pos: n, end: n });
    Ok(lx.toks)
}

impl Lx<'_> {
    fn at(&self, k: usize) -> u8 {
        self.b.get(self.i + k).copied().unwrap_or(0)
    }
    fn err(&self, m: &str) -> String {
        format!("parse error near line {}: {m}", self.line)
    }
    fn push(&mut self, tok: Tok, pos: usize) {
        self.toks.push(Token { tok, line: self.line, pos, end: self.i });
        self.after_params = false;
    }
    fn last(&self) -> Option<&Tok> {
        self.toks.last().map(|t| &t.tok)
    }
    fn ends_value(&self) -> bool {
        !self.after_params
            && match self.last() {
                Some(Tok::Ident(s)) => s == "end" || !KEYWORDS.contains(&s.as_str()),
                Some(Tok::Num(_) | Tok::Imag(_) | Tok::Str(_)) => true,
                Some(Tok::Op(o)) => matches!(*o, ")" | "]" | "}" | "'" | ".'"),
                _ => false,
            }
    }
    fn starts_value(&self) -> bool {
        let (c, d) = (self.at(0), self.at(1));
        c.is_ascii_alphanumeric()
            || matches!(c, b'_' | b'\'' | b'"' | b'(' | b'[' | b'{' | b'@')
            || (c == b'.' && d.is_ascii_digit())
            || (matches!(c, b'+' | b'-') && !matches!(d, b' ' | b'\t' | b'=' | b'\r' | b'\n'))
            || (matches!(c, b'!' | b'~') && d != b'=')
    }
    fn stmt_start(&self) -> bool {
        self.stack.is_empty()
            && match self.last() {
                None | Some(Tok::Newline | Tok::Op(";" | ",")) => true,
                Some(Tok::Ident(k)) => matches!(k.as_str(), "else" | "do" | "try" | "otherwise"),
                _ => false,
            }
    }

    fn next(&mut self, is_var: &dyn Fn(&str) -> bool) -> Result<(), String> {
        let mut space = false;
        loop {
            match self.at(0) {
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'.' if self.b[self.i..].starts_with(b"...") => {
                    while !matches!(self.at(0), b'\n' | 0) {
                        self.i += 1;
                    }
                    if self.at(0) == b'\n' {
                        self.i += 1;
                        self.line += 1;
                    }
                }
                _ => break,
            }
            space = true;
        }
        let p = self.i;
        if p >= self.b.len() {
            return Ok(());
        }
        let c = self.at(0);
        match c {
            b'%' | b'#' => {
                if self.at(1) == b'{' && self.alone_on_line(p, 2) {
                    return self.block_comment();
                }
                while !matches!(self.at(0), b'\n' | 0) {
                    self.i += 1;
                }
                return Ok(());
            }
            b'\n' => {
                self.i += 1;
                match self.stack.last() {
                    Some(b'[' | b'{') => self.push(Tok::Op(";"), p),
                    Some(_) => {}
                    None => self.push(Tok::Newline, p),
                }
                self.line += 1;
                return Ok(());
            }
            _ => {}
        }
        if space && matches!(self.stack.last(), Some(b'[' | b'{')) && self.ends_value() && self.starts_value() {
            self.toks.push(Token { tok: Tok::Op(","), line: self.line, pos: p, end: p });
        }
        if c.is_ascii_digit() || (c == b'.' && self.at(1).is_ascii_digit()) {
            return self.number();
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let at_start = self.stmt_start();
            while self.at(0).is_ascii_alphanumeric() || self.at(0) == b'_' {
                self.i += 1;
            }
            let name = self.s[p..self.i].to_string();
            if at_start
                && !KEYWORDS.contains(&name.as_str())
                && !is_var(&name)
                && let Some(words) = self.command_words()?
            {
                self.push(Tok::Cmd(name, words), p);
                return Ok(());
            }
            self.push(Tok::Ident(name), p);
            return Ok(());
        }
        if c == b'"' || (c == b'\'' && !self.ends_value()) {
            return self.string(c);
        }
        if c == b'\'' {
            self.i += 1;
            self.push(Tok::Op("'"), p);
            return Ok(());
        }
        let Some(&op) = OPS2.iter().chain(OPS1).find(|o| self.b[p..].starts_with(o.as_bytes())) else {
            let ch = self.s[p..].chars().next().unwrap_or('?');
            return Err(self.err(&format!("invalid character '{ch}'")));
        };
        self.i += op.len();
        match op {
            "(" => self.stack.push(if matches!(self.last(), Some(Tok::Op("@"))) { b'@' } else { b'(' }),
            "[" | "{" => self.stack.push(op.as_bytes()[0]),
            ")" | "]" | "}" => {
                let closed = self.stack.pop();
                self.push(Tok::Op(op), p);
                self.after_params = closed == Some(b'@');
                return Ok(());
            }
            _ => {}
        }
        self.push(Tok::Op(op), p);
        Ok(())
    }

    fn number(&mut self) -> Result<(), String> {
        let p = self.i;
        while self.at(0).is_ascii_digit() {
            self.i += 1;
        }
        // `3.*x` is 3 .* x, `1...` is a continuation
        if self.at(0) == b'.' && !matches!(self.at(1), b'*' | b'/' | b'\\' | b'^' | b'\'') && !self.b[self.i..].starts_with(b"...") {
            self.i += 1;
            while self.at(0).is_ascii_digit() {
                self.i += 1;
            }
        }
        if matches!(self.at(0), b'e' | b'E') {
            let k = if matches!(self.at(1), b'+' | b'-') { 2 } else { 1 };
            if self.at(k).is_ascii_digit() {
                self.i += k;
                while self.at(0).is_ascii_digit() {
                    self.i += 1;
                }
            }
        }
        let v = self.s[p..self.i].parse::<f64>().map_err(|_| self.err("invalid number"))?;
        // `3i`, `2.5e-3j`, `1I`: imaginary unless the suffix starts a longer word
        if matches!(self.at(0), b'i' | b'j' | b'I' | b'J') && !(self.at(1).is_ascii_alphanumeric() || self.at(1) == b'_') {
            self.i += 1;
            self.push(Tok::Imag(v), p);
            return Ok(());
        }
        self.push(Tok::Num(v), p);
        Ok(())
    }

    /// 'single' ('' escapes a quote) or "double" (backslash escapes).
    fn string(&mut self, q: u8) -> Result<(), String> {
        let p = self.i;
        self.i += 1;
        let mut out = vec![];
        loop {
            let c = self.at(0);
            match c {
                _ if self.i >= self.b.len() || c == b'\n' => return Err(self.err("unterminated character string constant")),
                _ if c == q && self.at(1) == q => {
                    out.push(q);
                    self.i += 2;
                }
                _ if c == q => {
                    self.i += 1;
                    break;
                }
                b'\\' if q == b'"' => {
                    let e = self.at(1);
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'a' => out.push(7),
                        b'0' => out.push(0),
                        b'\\' | b'"' | b'\'' => out.push(e),
                        _ => out.extend([b'\\', e]),
                    }
                    self.i += 2;
                }
                _ => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
        self.push(Tok::Str(String::from_utf8_lossy(&out).into_owned()), p);
        Ok(())
    }

    /// `%{` / `%}` count only when alone on their line.
    fn alone_on_line(&self, p: usize, len: usize) -> bool {
        let before = self.s[..p].rsplit('\n').next().unwrap_or("");
        let after = self.s[p + len..].split('\n').next().unwrap_or("");
        before.trim().is_empty() && after.trim().is_empty()
    }

    fn block_comment(&mut self) -> Result<(), String> {
        let start = self.line;
        let mut depth = 0;
        loop {
            let rest = &self.s[self.i..];
            if rest.is_empty() {
                return Err(format!("parse error near line {start}: unterminated block comment"));
            }
            let line = rest.split('\n').next().unwrap_or("");
            match line.trim() {
                "%{" | "#{" => depth += 1,
                "%}" | "#}" => depth -= 1,
                _ => {}
            }
            self.i += line.len();
            if depth == 0 {
                return Ok(()); // the '\n' becomes a statement separator
            }
            if self.i < self.b.len() {
                self.i += 1;
                self.line += 1;
            }
        }
    }

    /// Words of a command-syntax call (`self.i` is just past the name), or None if this isn't one.
    fn command_words(&mut self) -> Result<Option<Vec<String>>, String> {
        let b = self.b;
        let mut j = self.i;
        if !matches!(b.get(j), Some(b' ' | b'\t')) {
            return Ok(None);
        }
        while matches!(b.get(j), Some(b' ' | b'\t')) {
            j += 1;
        }
        let (c, d) = (b.get(j).copied().unwrap_or(0), b.get(j + 1).copied().unwrap_or(0));
        if !(c.is_ascii_alphanumeric() || matches!(c, b'_' | b'\'' | b'"') || (c == b'-' && d.is_ascii_alphabetic())) {
            return Ok(None);
        }
        let mut words = vec![];
        loop {
            while matches!(b.get(j), Some(b' ' | b'\t' | b'\r')) {
                j += 1;
            }
            if matches!(b.get(j), None | Some(b'\n' | b';' | b',' | b'%' | b'#')) {
                break;
            }
            let mut w = vec![];
            while let Some(&c) = b.get(j) {
                if matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b';' | b',') {
                    break;
                }
                j += 1;
                if c != b'\'' && c != b'"' {
                    w.push(c);
                    continue;
                }
                loop {
                    match b.get(j) {
                        None | Some(b'\n') => return Err(self.err("unterminated character string constant")),
                        Some(&x) if x == c && b.get(j + 1) == Some(&c) => {
                            w.push(c);
                            j += 2;
                        }
                        Some(&x) if x == c => {
                            j += 1;
                            break;
                        }
                        Some(&x) => {
                            w.push(x);
                            j += 1;
                        }
                    }
                }
            }
            words.push(String::from_utf8_lossy(&w).into_owned());
        }
        self.i = j;
        Ok(Some(words))
    }
}
