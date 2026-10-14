//! AST and recursive-descent parser with Octave precedence.
use std::rc::Rc;

use super::lexer::{KEYWORDS, Tok, Token, lex};
use crate::value::{BinOp, UnOp};

#[derive(Clone, Debug)]
pub enum Expr {
    Num(f64),
    /// imaginary literal `3i`
    Imag(f64),
    Str(String),
    Id(String),
    /// bare `:` inside an index
    Colon,
    /// `end` inside an index
    End,
    Bin(BinOp, Box<Expr>, Box<Expr>),
    AndAnd(Box<Expr>, Box<Expr>),
    OrOr(Box<Expr>, Box<Expr>),
    Un(UnOp, Box<Expr>),
    /// start, step, stop
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    Index(Box<Expr>, Vec<Expr>),
    Brace(Box<Expr>, Vec<Expr>),
    Field(Box<Expr>, String),
    DynField(Box<Expr>, Box<Expr>),
    Matrix(Vec<Vec<Expr>>),
    Cell(Vec<Vec<Expr>>),
    Handle(String),
    Lambda(Rc<LambdaDef>),
}

#[derive(Debug)]
pub struct LambdaDef {
    pub params: Vec<String>,
    pub body: Expr,
    pub src: String,
}

#[derive(Debug)]
pub enum Acc {
    Paren(Vec<Expr>),
    Brace(Vec<Expr>),
    Field(String),
    Dyn(Expr),
}

/// Assignment target; name "~" discards.
#[derive(Debug)]
pub struct LValue {
    pub name: String,
    pub path: Vec<Acc>,
}

#[derive(Debug)]
pub struct Stmt {
    pub kind: Kind,
    pub line: usize,
}

/// The bool on Expr/Assign: display the result (no trailing `;`).
#[derive(Debug)]
pub enum Kind {
    Expr(Expr, bool),
    Assign(Vec<LValue>, Expr, bool),
    If(Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
    For(String, Expr, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    DoUntil(Vec<Stmt>, Expr),
    Switch(Expr, Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
    Try(Vec<Stmt>, Option<String>, Vec<Stmt>),
    Break,
    Continue,
    Return,
}

#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<String>,
    pub outs: Vec<String>,
    pub body: Vec<Stmt>,
}

pub struct Program {
    pub body: Vec<Stmt>,
    pub funcs: Vec<Function>,
}

pub fn parse(src: &str, is_var: &dyn Fn(&str) -> bool) -> Result<Program, String> {
    let mut p = P { t: lex(src, is_var)?, i: 0, src, idx: 0, funcs: vec![] };
    let body = p.block(&[])?;
    Ok(Program { body, funcs: p.funcs })
}

/// One expression (str2func, eval with outputs).
pub fn parse_expr(src: &str) -> Result<Expr, String> {
    let mut p = P { t: lex(src, &|_| true)?, i: 0, src, idx: 0, funcs: vec![] };
    let e = p.expr()?;
    while p.at_sep() {
        p.i += 1;
    }
    if !matches!(p.tok(), Tok::Eof) {
        return Err(p.err(&format!("unexpected {}", describe(p.tok()))));
    }
    Ok(e)
}

struct P<'a> {
    t: Vec<Token>,
    i: usize,
    src: &'a str,
    /// > 0 inside index parens/braces, where `end` and a bare `:` mean something
    idx: usize,
    funcs: Vec<Function>,
}

use BinOp::*;
const LEVELS: [&[(&str, BinOp)]; 5] = [
    &[("|", Or)],
    &[("&", And)],
    &[("==", Eq), ("~=", Ne), ("!=", Ne), ("<", Lt), ("<=", Le), (">", Gt), (">=", Ge)],
    &[("+", Add), ("-", Sub)],
    &[("*", Mul), ("/", Div), ("\\", LDiv), (".*", EMul), ("./", EDiv), (".\\", ELDiv)],
];

fn describe(t: &Tok) -> String {
    match t {
        Tok::Op(o) => format!("'{o}'"),
        Tok::Ident(s) | Tok::Cmd(s, _) => format!("'{s}'"),
        Tok::Num(x) => format!("'{x}'"),
        Tok::Imag(x) => format!("'{x}i'"),
        Tok::Str(s) => format!("'{s}'"),
        Tok::Newline => "newline".into(),
        Tok::Eof => "end of input".into(),
    }
}

/// `a.b(2){3}` -> name `a` + accessor path; None if not assignable.
fn lvalue(mut e: Expr) -> Option<LValue> {
    let mut path = vec![];
    loop {
        e = match e {
            Expr::Id(name) => {
                path.reverse();
                return Some(LValue { name, path });
            }
            Expr::Index(b, a) => {
                path.push(Acc::Paren(a));
                *b
            }
            Expr::Brace(b, a) => {
                path.push(Acc::Brace(a));
                *b
            }
            Expr::Field(b, f) => {
                path.push(Acc::Field(f));
                *b
            }
            Expr::DynField(b, f) => {
                path.push(Acc::Dyn(*f));
                *b
            }
            _ => return None,
        }
    }
}

impl P<'_> {
    fn tok(&self) -> &Tok {
        &self.t[self.i].tok
    }
    fn line(&self) -> usize {
        self.t[self.i].line
    }
    fn err(&self, m: &str) -> String {
        format!("parse error near line {}: {m}", self.line())
    }
    fn is_op(&self, o: &str) -> bool {
        matches!(self.tok(), Tok::Op(x) if *x == o)
    }
    fn eat(&mut self, o: &str) -> bool {
        let y = self.is_op(o);
        if y {
            self.i += 1;
        }
        y
    }
    fn expect(&mut self, o: &str) -> Result<(), String> {
        if self.eat(o) { Ok(()) } else { Err(self.err(&format!("'{o}' expected, found {}", describe(self.tok())))) }
    }
    fn kw(&self) -> Option<&'static str> {
        match self.tok() {
            Tok::Ident(s) => KEYWORDS.iter().find(|k| **k == s).copied(),
            _ => None,
        }
    }
    fn ident(&mut self) -> Result<String, String> {
        match self.tok() {
            Tok::Ident(s) if self.kw().is_none() => {
                let s = s.clone();
                self.i += 1;
                Ok(s)
            }
            t => Err(self.err(&format!("identifier expected, found {}", describe(t)))),
        }
    }
    fn at_sep(&self) -> bool {
        matches!(self.tok(), Tok::Newline | Tok::Op(";" | ","))
    }

    /// Statements up to (not including) one of the `terms` keywords.
    fn block(&mut self, terms: &[&str]) -> Result<Vec<Stmt>, String> {
        let mut out = vec![];
        loop {
            while self.at_sep() {
                self.i += 1;
            }
            match self.kw() {
                Some(k) if terms.contains(&k) => return Ok(out),
                Some("function") => {
                    self.function()?;
                    continue;
                }
                _ => {}
            }
            if matches!(self.tok(), Tok::Eof) {
                if terms.is_empty() || terms.contains(&"function") {
                    return Ok(out);
                }
                return Err(self.err(&format!("'{}' missing", terms[terms.len() - 1])));
            }
            let line = self.line();
            let kind = self.stmt()?;
            out.push(Stmt { kind, line });
        }
    }

    /// Consume the block-closing keyword.
    fn close(&mut self) {
        self.i += 1;
    }

    fn stmt(&mut self) -> Result<Kind, String> {
        let Some(k) = self.kw() else { return self.simple() };
        self.i += 1;
        Ok(match k {
            "if" => {
                let mut arms = vec![];
                let mut els = vec![];
                let terms = ["elseif", "else", "end", "endif"];
                let c = self.expr()?;
                arms.push((c, self.block(&terms)?));
                loop {
                    match self.kw() {
                        Some("elseif") => {
                            self.i += 1;
                            let c = self.expr()?;
                            arms.push((c, self.block(&terms)?));
                        }
                        Some("else") => {
                            self.i += 1;
                            els = self.block(&["end", "endif"])?;
                        }
                        _ => break,
                    }
                }
                self.close();
                Kind::If(arms, els)
            }
            "for" | "parfor" => {
                let paren = self.eat("(");
                let var = self.ident()?;
                self.expect("=")?;
                let e = self.expr()?;
                if paren {
                    self.expect(")")?;
                }
                let body = self.block(&["end", "endfor", "endparfor"])?;
                self.close();
                Kind::For(var, e, body)
            }
            "while" => {
                let c = self.expr()?;
                let body = self.block(&["end", "endwhile"])?;
                self.close();
                Kind::While(c, body)
            }
            "do" => {
                let body = self.block(&["until"])?;
                self.close();
                Kind::DoUntil(body, self.expr()?)
            }
            "switch" => {
                let subj = self.expr()?;
                let (mut cases, mut other) = (vec![], vec![]);
                let terms = ["case", "otherwise", "end", "endswitch"];
                loop {
                    while self.at_sep() {
                        self.i += 1;
                    }
                    match self.kw() {
                        Some("case") => {
                            self.i += 1;
                            let v = self.expr()?;
                            cases.push((v, self.block(&terms)?));
                        }
                        Some("otherwise") => {
                            self.i += 1;
                            other = self.block(&terms)?;
                        }
                        Some("end" | "endswitch") => break,
                        _ => return Err(self.err("'case' expected")),
                    }
                }
                self.close();
                Kind::Switch(subj, cases, other)
            }
            "try" => {
                let body = self.block(&["catch", "end", "end_try_catch"])?;
                let (mut var, mut handler) = (None, vec![]);
                if self.kw() == Some("catch") {
                    self.i += 1;
                    if let Tok::Ident(s) = self.tok()
                        && self.kw().is_none()
                    {
                        var = Some(s.clone());
                        self.i += 1;
                    }
                    handler = self.block(&["end", "end_try_catch"])?;
                }
                self.close();
                Kind::Try(body, var, handler)
            }
            "break" => Kind::Break,
            "continue" => Kind::Continue,
            "return" => Kind::Return,
            k => {
                self.i -= 1;
                return Err(self.err(&format!("unexpected '{k}'")));
            }
        })
    }

    fn simple(&mut self) -> Result<Kind, String> {
        if let Tok::Cmd(name, words) = self.tok().clone() {
            self.i += 1;
            let call = Expr::Index(Box::new(Expr::Id(name)), words.into_iter().map(Expr::Str).collect());
            return Ok(Kind::Expr(call, self.end_stmt()?));
        }
        if self.is_op("[") {
            let (i, idx) = (self.i, self.idx);
            match self.multi_lhs() {
                Some(lhs) => {
                    let rhs = self.expr()?;
                    return Ok(Kind::Assign(lhs, rhs, self.end_stmt()?));
                }
                None => (self.i, self.idx) = (i, idx),
            }
        }
        let e = self.expr()?;
        for (o, op) in [("+=", Add), ("-=", Sub), ("*=", Mul), ("/=", Div)] {
            if self.eat(o) {
                let lv = lvalue(e.clone()).ok_or_else(|| self.err("invalid assignment target"))?;
                let rhs = Expr::Bin(op, Box::new(e), Box::new(self.expr()?));
                return Ok(Kind::Assign(vec![lv], rhs, self.end_stmt()?));
            }
        }
        if self.eat("=") {
            let lv = lvalue(e).ok_or_else(|| self.err("invalid assignment target"))?;
            let rhs = self.expr()?;
            return Ok(Kind::Assign(vec![lv], rhs, self.end_stmt()?));
        }
        Ok(Kind::Expr(e, self.end_stmt()?))
    }

    /// `[a, b, ~] =`; None (position not restored) if this isn't one.
    fn multi_lhs(&mut self) -> Option<Vec<LValue>> {
        self.i += 1;
        let mut out = vec![];
        loop {
            if self.eat("]") {
                break;
            }
            if self.eat(",") {
                continue;
            }
            if self.eat("~") || self.eat("!") {
                out.push(LValue { name: "~".into(), path: vec![] });
                continue;
            }
            let e = self.postfix().ok()?;
            out.push(lvalue(e)?);
        }
        if self.eat("=") { Some(out) } else { None }
    }

    /// Statement terminator; returns whether the result is displayed.
    fn end_stmt(&mut self) -> Result<bool, String> {
        match self.tok() {
            Tok::Op(";") => {
                self.i += 1;
                Ok(false)
            }
            Tok::Op(",") | Tok::Newline => {
                self.i += 1;
                Ok(true)
            }
            Tok::Eof => Ok(true),
            _ if self.kw().is_some() => Ok(true),
            t => Err(self.err(&format!("unexpected {}", describe(t)))),
        }
    }

    fn function(&mut self) -> Result<(), String> {
        self.i += 1;
        let mut outs = vec![];
        if self.eat("[") {
            while !self.eat("]") {
                if !self.eat(",") {
                    outs.push(self.ident()?);
                }
            }
            self.expect("=")?;
        }
        let mut name = self.ident()?;
        if outs.is_empty() && self.eat("=") {
            outs.push(name);
            name = self.ident()?;
        }
        let mut params = vec![];
        if self.eat("(") {
            while !self.eat(")") {
                if self.eat(",") {
                    continue;
                }
                if self.eat("~") {
                    params.push("~".into());
                    continue;
                }
                params.push(self.ident()?);
            }
        }
        let body = self.block(&["end", "endfunction", "function"])?;
        if matches!(self.kw(), Some("end" | "endfunction")) {
            self.i += 1;
        }
        self.funcs.push(Function { name, params, outs, body });
        Ok(())
    }

    pub fn expr(&mut self) -> Result<Expr, String> {
        let mut l = self.andand()?;
        while self.eat("||") {
            l = Expr::OrOr(Box::new(l), Box::new(self.andand()?));
        }
        Ok(l)
    }

    fn andand(&mut self) -> Result<Expr, String> {
        let mut l = self.bin(0)?;
        while self.eat("&&") {
            l = Expr::AndAnd(Box::new(l), Box::new(self.bin(0)?));
        }
        Ok(l)
    }

    /// `|` < `&` < comparisons < range < `+ -` < `* /` < unary
    fn bin(&mut self, lv: usize) -> Result<Expr, String> {
        let next = |p: &mut Self| match lv {
            2 => p.range(),
            4 => p.unary(false),
            _ => p.bin(lv + 1),
        };
        let mut l = next(self)?;
        'outer: loop {
            for &(o, op) in LEVELS[lv] {
                if self.eat(o) {
                    l = Expr::Bin(op, Box::new(l), Box::new(next(self)?));
                    continue 'outer;
                }
            }
            return Ok(l);
        }
    }

    fn range(&mut self) -> Result<Expr, String> {
        let a = self.bin(3)?;
        if !self.eat(":") {
            return Ok(a);
        }
        let b = self.bin(3)?;
        if self.eat(":") {
            let c = self.bin(3)?;
            return Ok(Expr::Range(Box::new(a), Some(Box::new(b)), Box::new(c)));
        }
        Ok(Expr::Range(Box::new(a), None, Box::new(b)))
    }

    /// Prefix `+ - ! ~`, binding looser than `^` (so -2^2 == -4) except in an exponent (2^-1).
    fn unary(&mut self, exponent: bool) -> Result<Expr, String> {
        let op = match self.tok() {
            Tok::Op("-") => UnOp::Neg,
            Tok::Op("+") => UnOp::Plus,
            Tok::Op("!" | "~") => UnOp::Not,
            _ if exponent => return self.postfix(),
            _ => return self.power(),
        };
        self.i += 1;
        Ok(Expr::Un(op, Box::new(self.unary(exponent)?)))
    }

    /// `^ .^ ' .'`, same precedence, left to right.
    fn power(&mut self) -> Result<Expr, String> {
        let mut l = self.postfix()?;
        loop {
            l = match self.tok() {
                Tok::Op(o @ ("^" | ".^")) => {
                    let op = if *o == "^" { Pow } else { EPow };
                    self.i += 1;
                    Expr::Bin(op, Box::new(l), Box::new(self.unary(true)?))
                }
                Tok::Op(o @ ("'" | ".'")) => {
                    let op = if *o == "'" { UnOp::CTranspose } else { UnOp::Transpose };
                    self.i += 1;
                    Expr::Un(op, Box::new(l))
                }
                _ => return Ok(l),
            }
        }
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        loop {
            e = if self.eat("(") {
                Expr::Index(Box::new(e), self.args(")")?)
            } else if self.eat("{") {
                Expr::Brace(Box::new(e), self.args("}")?)
            } else if self.is_op(".") {
                self.i += 1;
                match self.tok().clone() {
                    Tok::Ident(n) => {
                        self.i += 1;
                        Expr::Field(Box::new(e), n)
                    }
                    Tok::Op("(") => {
                        self.i += 1;
                        let f = self.expr()?;
                        self.expect(")")?;
                        Expr::DynField(Box::new(e), Box::new(f))
                    }
                    t => return Err(self.err(&format!("field name expected, found {}", describe(&t)))),
                }
            } else {
                return Ok(e);
            };
        }
    }

    fn args(&mut self, close: &str) -> Result<Vec<Expr>, String> {
        self.idx += 1;
        let mut v = vec![];
        if !self.eat(close) {
            loop {
                let lone = self.is_op(":") && matches!(&self.t[self.i + 1].tok, Tok::Op(o) if *o == "," || *o == close);
                if lone {
                    self.i += 1;
                    v.push(Expr::Colon);
                } else {
                    v.push(self.expr()?);
                }
                if self.eat(close) {
                    break;
                }
                self.expect(",")?;
            }
        }
        self.idx -= 1;
        Ok(v)
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let t = self.t[self.i].clone();
        if matches!(t.tok, Tok::Eof) {
            return Err(self.err("unexpected end of input"));
        }
        self.i += 1;
        Ok(match t.tok {
            Tok::Num(x) => Expr::Num(x),
            Tok::Imag(x) => Expr::Imag(x),
            Tok::Str(s) => Expr::Str(s),
            Tok::Ident(s) if s == "end" && self.idx > 0 => Expr::End,
            Tok::Ident(s) if !KEYWORDS.contains(&s.as_str()) => Expr::Id(s),
            Tok::Op("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                e
            }
            Tok::Op("[") => Expr::Matrix(self.rows("]")?),
            Tok::Op("{") => Expr::Cell(self.rows("}")?),
            Tok::Op("@") => self.handle(t.pos)?,
            Tok::Op(":") if self.idx > 0 => Expr::Colon,
            tok => {
                self.i -= 1;
                return Err(self.err(&format!("unexpected {}", describe(&tok))));
            }
        })
    }

    /// Matrix / cell literal rows (lexer already split elements with `,` and rows with `;`).
    fn rows(&mut self, close: &str) -> Result<Vec<Vec<Expr>>, String> {
        let (mut rows, mut row) = (vec![], vec![]);
        loop {
            if self.eat(close) {
                break;
            }
            if self.eat(";") {
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                }
                continue;
            }
            if self.eat(",") {
                continue;
            }
            row.push(self.expr()?);
        }
        if !row.is_empty() {
            rows.push(row);
        }
        Ok(rows)
    }

    /// After `@`: `@name` or `@(params) body`.
    fn handle(&mut self, start: usize) -> Result<Expr, String> {
        if !self.eat("(") {
            return Ok(Expr::Handle(self.ident()?));
        }
        let mut params = vec![];
        while !self.eat(")") {
            match self.tok().clone() {
                Tok::Ident(s) => params.push(s),
                Tok::Op("~" | "!") => params.push("~".into()),
                Tok::Op(",") => {}
                t => return Err(self.err(&format!("unexpected {} in parameter list", describe(&t)))),
            }
            self.i += 1;
        }
        let saved = std::mem::replace(&mut self.idx, 0);
        let body = self.expr();
        self.idx = saved;
        let body = body?;
        let src = self.src[start..self.t[self.i - 1].end].to_string();
        Ok(Expr::Lambda(Rc::new(LambdaDef { params, body, src })))
    }
}
