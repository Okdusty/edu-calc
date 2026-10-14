//! Tree-walking evaluator.
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use super::display;
use super::index::{self, AccV};
use super::parser::{Acc, Expr, Function, Kind, LValue, LambdaDef, Stmt, parse, parse_expr};
use super::{Interp, Lambda};
use crate::builtins::ops::{binop, unop};
use crate::value::{BinOp, C64, CMatrix, Func, Mat, Value};

const MAX_DEPTH: usize = 256;

pub enum Flow {
    Next,
    Break,
    Continue,
    Return,
}

/// Every identifier in an expression (lambda captures, free variables of string functions).
pub fn walk_ids(e: &Expr, f: &mut dyn FnMut(&str)) {
    match e {
        Expr::Id(n) => f(n),
        Expr::Bin(_, a, b) | Expr::AndAnd(a, b) | Expr::OrOr(a, b) | Expr::DynField(a, b) => {
            walk_ids(a, f);
            walk_ids(b, f);
        }
        Expr::Un(_, a) | Expr::Field(a, _) => walk_ids(a, f),
        Expr::Range(a, s, b) => {
            walk_ids(a, f);
            if let Some(s) = s {
                walk_ids(s, f);
            }
            walk_ids(b, f);
        }
        Expr::Index(b, args) | Expr::Brace(b, args) => {
            walk_ids(b, f);
            args.iter().for_each(|a| walk_ids(a, f));
        }
        Expr::Matrix(rows) | Expr::Cell(rows) => rows.iter().flatten().for_each(|a| walk_ids(a, f)),
        Expr::Lambda(d) => walk_ids(&d.body, f),
        Expr::Num(_) | Expr::Imag(_) | Expr::Str(_) | Expr::Colon | Expr::End | Expr::Handle(_) => {}
    }
}

/// Bind call arguments to parameter names (`varargin` collects the rest).
fn bind(scope: &mut HashMap<String, Value>, params: &[String], args: Vec<Value>, name: &str) -> Result<(), String> {
    let va = params.last().is_some_and(|p| p == "varargin");
    let fixed = params.len() - va as usize;
    if args.len() > fixed && !va {
        return Err(format!("{name}: function called with too many inputs"));
    }
    let mut it = args.into_iter();
    for p in &params[..fixed] {
        match it.next() {
            Some(v) if p != "~" => drop(scope.insert(p.clone(), v)),
            Some(_) => {}
            None => break,
        }
    }
    if va {
        scope.insert("varargin".into(), Value::Cell(it.collect()));
    }
    Ok(())
}

fn field(v: Value, f: &str) -> Result<Value, String> {
    match v {
        Value::Struct(mut s) => s.remove(f).ok_or_else(|| format!("invalid use of undefined value (no field '{f}')")),
        v => Err(format!("{} value cannot be indexed with .", v.type_name())),
    }
}

fn case_eq(v: &Value, c: &Value) -> bool {
    match (v, c) {
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::Str(_), _) | (_, Value::Str(_)) => false,
        _ => matches!((v.scalar(), c.scalar()), (Ok(a), Ok(b)) if a == b),
    }
}

/// `for` iterates columns.
fn ncols(v: &Value) -> usize {
    match v {
        Value::Num(m) | Value::Bool(m) if m.is_empty() => 0,
        Value::Num(m) | Value::Bool(m) => m.cols,
        Value::Complex(m) => m.cols,
        Value::Str(s) => s.chars().count(),
        Value::Cell(c) => c.len(),
        _ => 1,
    }
}

fn column(v: &Value, k: usize) -> Value {
    fn col<T: Copy + Default>(m: &Mat<T>, k: usize) -> Mat<T> {
        Mat::col(m.data[k * m.rows..(k + 1) * m.rows].to_vec())
    }
    match v {
        Value::Num(m) => Value::Num(col(m, k)),
        Value::Bool(m) => Value::Bool(col(m, k)),
        Value::Complex(m) => Value::complex(col(m, k)),
        Value::Str(s) => Value::Str(s.chars().nth(k).map(String::from).unwrap_or_default()),
        Value::Cell(c) => Value::Cell(vec![c[k].clone()]),
        v => v.clone(),
    }
}

impl Interp {
    /// Parse and run in the current scope; functions defined in `src` are registered first.
    pub(super) fn run_src(&mut self, src: &str) -> Result<(), String> {
        let vars = &self.vars;
        let prog = parse(src, &|n| vars.contains_key(n))?;
        for f in prog.funcs {
            self.funcs.insert(f.name.clone(), Rc::new(f));
        }
        self.exec_block(&prog.body).map(drop)
    }

    /// `eval(src)`: statements, or an expression when outputs are wanted.
    pub(super) fn eval_str(&mut self, src: &str, nargout: usize) -> Result<Vec<Value>, String> {
        if nargout == 0 {
            return self.run_src(src).map(|_| vec![]);
        }
        let e = parse_expr(src)?;
        self.eval_multi(&e, nargout)
    }

    fn exec_block(&mut self, body: &[Stmt]) -> Result<Flow, String> {
        for s in body {
            self.err_line = None;
            match self.exec(s) {
                Ok(Flow::Next) => {}
                Ok(f) => return Ok(f),
                Err(e) => {
                    self.err_line.get_or_insert(s.line);
                    return Err(e);
                }
            }
        }
        Ok(Flow::Next)
    }

    /// Loop body; Some(flow) leaves the loop.
    fn exec_loop_body(&mut self, body: &[Stmt]) -> Result<Option<Flow>, String> {
        Ok(match self.exec_block(body)? {
            Flow::Break => Some(Flow::Next),
            Flow::Return => Some(Flow::Return),
            _ => None,
        })
    }

    // ponytail: exec/ev/eval_multi only dispatch; each case gets its own method so debug-build
    // stack frames stay small enough for recursion depth 256 on an 8 MB stack.
    fn exec(&mut self, s: &Stmt) -> Result<Flow, String> {
        match &s.kind {
            Kind::Expr(e, show) => self.exec_expr(e, *show),
            Kind::Assign(lhs, rhs, show) => self.exec_assign(lhs, rhs, *show),
            Kind::If(arms, els) => self.exec_if(arms, els),
            Kind::For(var, e, body) => self.exec_for(var, e, body),
            Kind::While(c, body) => self.exec_while(c, body),
            Kind::DoUntil(body, c) => self.exec_do(body, c),
            Kind::Switch(subj, cases, other) => self.exec_switch(subj, cases, other),
            Kind::Try(body, var, handler) => self.exec_try(body, var.as_deref(), handler),
            Kind::Break => Ok(Flow::Break),
            Kind::Continue => Ok(Flow::Continue),
            Kind::Return => Ok(Flow::Return),
        }
    }

    fn exec_expr(&mut self, e: &Expr, show: bool) -> Result<Flow, String> {
        if let Expr::Id(n) = e
            && let Some(v) = self.vars.get(n)
        {
            if show {
                let t = display(n, v);
                self.print(&t);
            }
            return Ok(Flow::Next);
        }
        if let Some(v) = self.eval_multi(e, 0)?.into_iter().next() {
            if show {
                self.print(&display("ans", &v));
            }
            self.vars.insert("ans".into(), v);
        }
        Ok(Flow::Next)
    }

    fn exec_assign(&mut self, lhs: &[LValue], rhs: &Expr, show: bool) -> Result<Flow, String> {
        if let [lv] = lhs {
            let v = self.ev(rhs)?;
            self.assign(lv, v)?;
        } else {
            let mut vals = self.eval_multi(rhs, lhs.len())?.into_iter();
            for (k, lv) in lhs.iter().enumerate() {
                let v = vals.next().ok_or_else(|| format!("element number {} undefined in return list", k + 1))?;
                if lv.name != "~" {
                    self.assign(lv, v)?;
                }
            }
        }
        if show {
            for lv in lhs.iter().filter(|l| l.name != "~") {
                if let Some(v) = self.vars.get(&lv.name) {
                    let t = display(&lv.name, v);
                    self.print(&t);
                }
            }
        }
        Ok(Flow::Next)
    }

    fn exec_if(&mut self, arms: &[(Expr, Vec<Stmt>)], els: &[Stmt]) -> Result<Flow, String> {
        for (c, b) in arms {
            if self.ev(c)?.truthy()? {
                return self.exec_block(b);
            }
        }
        self.exec_block(els)
    }

    fn exec_for(&mut self, var: &str, e: &Expr, body: &[Stmt]) -> Result<Flow, String> {
        let v = self.ev(e)?;
        for k in 0..ncols(&v) {
            self.vars.insert(var.to_string(), column(&v, k));
            if let Some(f) = self.exec_loop_body(body)? {
                return Ok(f);
            }
        }
        Ok(Flow::Next)
    }

    fn exec_while(&mut self, c: &Expr, body: &[Stmt]) -> Result<Flow, String> {
        while self.ev(c)?.truthy()? {
            if let Some(f) = self.exec_loop_body(body)? {
                return Ok(f);
            }
        }
        Ok(Flow::Next)
    }

    fn exec_do(&mut self, body: &[Stmt], c: &Expr) -> Result<Flow, String> {
        loop {
            if let Some(f) = self.exec_loop_body(body)? {
                return Ok(f);
            }
            if self.ev(c)?.truthy()? {
                return Ok(Flow::Next);
            }
        }
    }

    fn exec_switch(&mut self, subj: &Expr, cases: &[(Expr, Vec<Stmt>)], other: &[Stmt]) -> Result<Flow, String> {
        let v = self.ev(subj)?;
        for (c, b) in cases {
            let hit = match self.ev(c)? {
                Value::Cell(opts) => opts.iter().any(|o| case_eq(&v, o)),
                c => case_eq(&v, &c),
            };
            if hit {
                return self.exec_block(b);
            }
        }
        self.exec_block(other)
    }

    fn exec_try(&mut self, body: &[Stmt], var: Option<&str>, handler: &[Stmt]) -> Result<Flow, String> {
        match self.exec_block(body) {
            Ok(f) => Ok(f),
            Err(e) => {
                self.err_line = None;
                if let Some(n) = var {
                    let err = BTreeMap::from([("message".to_string(), Value::Str(e)), ("identifier".to_string(), Value::Str(String::new()))]);
                    self.vars.insert(n.to_string(), Value::Struct(err));
                }
                self.exec_block(handler)
            }
        }
    }

    /// Evaluate to exactly one value.
    pub(super) fn ev(&mut self, e: &Expr) -> Result<Value, String> {
        match e {
            Expr::Num(x) => Ok(Value::num(*x)),
            Expr::Bin(op, a, b) => self.ev_bin(*op, a, b),
            Expr::Id(_) | Expr::Index(..) | Expr::Brace(..) | Expr::Field(..) | Expr::DynField(..) => self.ev_one(e),
            Expr::Matrix(rows) => self.ev_matrix(rows),
            _ => self.ev_other(e),
        }
    }

    fn ev_bin(&mut self, op: BinOp, a: &Expr, b: &Expr) -> Result<Value, String> {
        let a = self.ev(a)?;
        let b = self.ev(b)?;
        binop(op, &a, &b)
    }

    fn ev_one(&mut self, e: &Expr) -> Result<Value, String> {
        let v = self.eval_multi(e, 1)?;
        v.into_iter().next().ok_or_else(|| "value on right hand side of assignment is undefined (function returned nothing)".into())
    }

    fn ev_matrix(&mut self, rows: &[Vec<Expr>]) -> Result<Value, String> {
        let mut vals = vec![];
        for row in rows {
            let mut r = vec![];
            for e in row {
                r.extend(self.eval_cs(e)?);
            }
            vals.push(r);
        }
        index::concat(vals)
    }

    fn ev_other(&mut self, e: &Expr) -> Result<Value, String> {
        Ok(match e {
            Expr::Imag(x) => Value::complex(CMatrix::scalar(C64::new(0.0, *x))),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Colon => Value::Str(":".into()),
            Expr::End => {
                let &(r, c, k, n) = self.end_ctx.last().ok_or("'end': nonconformant arguments (used outside an index)")?;
                Value::num(match (n, k) {
                    (1, _) => r * c,
                    (_, 0) => r,
                    (_, 1) => c,
                    _ => 1,
                } as f64)
            }
            Expr::Un(op, a) => unop(*op, &self.ev(a)?)?,
            Expr::AndAnd(a, b) => Value::bool(self.ev(a)?.truthy()? && self.ev(b)?.truthy()?),
            Expr::OrOr(a, b) => Value::bool(self.ev(a)?.truthy()? || self.ev(b)?.truthy()?),
            Expr::Range(a, s, b) => {
                let a = self.bound(a)?;
                let s = match s {
                    Some(s) => self.bound(s)?,
                    None => Some(1.0),
                };
                let b = self.bound(b)?;
                match (a, s, b) {
                    (Some(a), Some(s), Some(b)) => Value::Num(index::range(a, s, b)?),
                    _ => Value::Num(Mat::zeros(1, 0)),
                }
            }
            Expr::Cell(rows) => {
                // ponytail: 1-D cells, so rows are flattened in order
                let mut out = vec![];
                for e in rows.iter().flatten() {
                    out.extend(self.eval_cs(e)?);
                }
                Value::Cell(out)
            }
            Expr::Handle(n) => Value::Func(Rc::new(Func::Named(n.clone()))),
            Expr::Lambda(def) => self.lambda(def),
            e => return self.ev(e),
        })
    }

    /// First element of a range bound (Octave uses the first element of a matrix).
    fn bound(&mut self, e: &Expr) -> Result<Option<f64>, String> {
        Ok(self.ev(e)?.matrix()?.data.first().copied())
    }

    /// Evaluate with `nargout` outputs wanted; calls and `c{:}` can yield several values (or none).
    pub(super) fn eval_multi(&mut self, e: &Expr, nargout: usize) -> Result<Vec<Value>, String> {
        match e {
            Expr::Id(n) => match self.vars.get(n) {
                Some(v) => Ok(vec![v.clone()]),
                None => self.call_named(n, vec![], nargout),
            },
            Expr::Index(base, args) => self.ev_index(base, args, nargout),
            Expr::Brace(..) | Expr::Field(..) | Expr::DynField(..) => self.ev_access(e),
            _ => Ok(vec![self.ev(e)?]),
        }
    }

    /// `name(args)`: call, index a variable, or call the handle it holds.
    fn ev_index(&mut self, base: &Expr, args: &[Expr], nargout: usize) -> Result<Vec<Value>, String> {
        if let Expr::Id(n) = base {
            match self.vars.get(n) {
                None => return self.call_with(n, args, nargout),
                Some(Value::Func(_)) => {}
                Some(_) => return self.subscript(base, args, |v, idx| Ok(vec![index::index(v, idx)?])),
            }
        }
        let v = self.ev(base)?;
        if matches!(v, Value::Func(_)) {
            let a = self.eval_args(args)?;
            return self.call(&v, &a, nargout);
        }
        let idx = self.eval_index_args(v.size(), args)?;
        Ok(vec![index::index(&v, &idx)?])
    }

    fn call_with(&mut self, name: &str, args: &[Expr], nargout: usize) -> Result<Vec<Value>, String> {
        let a = self.eval_args(args)?;
        self.call_named(name, a, nargout)
    }

    /// `c{...}`, `s.f`, `s.(name)`
    fn ev_access(&mut self, e: &Expr) -> Result<Vec<Value>, String> {
        match e {
            Expr::Brace(base, args) => self.subscript(base, args, |v, idx| match v {
                Value::Cell(c) => Ok(index::cell_positions(c.len(), idx)?.into_iter().map(|k| c[k].clone()).collect()),
                v => Err(format!("'{{' undefined for arguments of type '{}'; matrix cannot be indexed with {{", v.type_name())),
            }),
            Expr::Field(base, f) => Ok(vec![field(self.ev(base)?, f)?]),
            Expr::DynField(base, f) => {
                let s = self.ev(base)?;
                let f = self.ev(f)?;
                Ok(vec![field(s, f.as_str()?)?])
            }
            e => self.eval_multi(e, 1),
        }
    }

    /// Subscripts evaluated against `base` (for `end`) without cloning a variable base.
    fn subscript<T>(&mut self, base: &Expr, args: &[Expr], f: impl FnOnce(&Value, &[Value]) -> Result<T, String>) -> Result<T, String> {
        if let Expr::Id(n) = base
            && let Some(v) = self.vars.get(n)
        {
            let idx = self.eval_index_args(v.size(), args)?;
            let v = self.vars.get(n).ok_or_else(|| format!("'{n}' undefined"))?;
            return f(v, &idx);
        }
        let v = self.ev(base)?;
        let idx = self.eval_index_args(v.size(), args)?;
        f(&v, &idx)
    }

    /// Index arguments; `end` in argument k of n refers to `dims`.
    fn eval_index_args(&mut self, (r, c): (usize, usize), args: &[Expr]) -> Result<Vec<Value>, String> {
        let mut out = vec![];
        for (k, a) in args.iter().enumerate() {
            self.end_ctx.push((r, c, k, args.len()));
            let v = self.ev(a);
            self.end_ctx.pop();
            out.push(v?);
        }
        Ok(out)
    }

    /// Function-call arguments; `c{:}` expands to several.
    fn eval_args(&mut self, args: &[Expr]) -> Result<Vec<Value>, String> {
        let mut out = vec![];
        for a in args {
            out.extend(self.eval_cs(a)?);
        }
        Ok(out)
    }

    fn eval_cs(&mut self, e: &Expr) -> Result<Vec<Value>, String> {
        match e {
            Expr::Brace(..) => self.eval_multi(e, 1),
            _ => Ok(vec![self.ev(e)?]),
        }
    }

    /// `@(x) ...`: captures referenced workspace variables by value.
    fn lambda(&self, def: &Rc<LambdaDef>) -> Value {
        let mut caps = HashMap::new();
        walk_ids(&def.body, &mut |n| {
            if !def.params.iter().any(|p| p == n)
                && let Some(v) = self.vars.get(n)
            {
                caps.insert(n.to_string(), v.clone());
            }
        });
        let l = Lambda { params: def.params.clone(), src: def.src.clone(), def: def.clone(), caps: Rc::new(caps) };
        Value::Func(Rc::new(Func::Anon(l)))
    }

    /// Run `f` in a fresh variable scope.
    fn scoped<T>(&mut self, scope: HashMap<String, Value>, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        if self.depth >= MAX_DEPTH {
            return Err("max_recursion_depth exceeded".into());
        }
        let vars = std::mem::replace(&mut self.vars, scope);
        let ends = std::mem::take(&mut self.end_ctx);
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        self.vars = vars;
        self.end_ctx = ends;
        r
    }

    pub(super) fn call_lambda(&mut self, l: &Lambda, args: &[Value], nargout: usize) -> Result<Vec<Value>, String> {
        let mut scope = (*l.caps).clone();
        bind(&mut scope, &l.params, args.to_vec(), "@<anonymous>")?;
        self.scoped(scope, |me| me.eval_multi(&l.def.body, nargout))
    }

    fn call_user(&mut self, f: Rc<Function>, args: Vec<Value>, nargout: usize) -> Result<Vec<Value>, String> {
        let va_out = f.outs.last().is_some_and(|o| o == "varargout");
        let nout = f.outs.len() - va_out as usize;
        if nargout > nout && !va_out {
            return Err(format!("{}: function called with too many outputs", f.name));
        }
        let mut scope = HashMap::new();
        scope.insert("nargin".to_string(), Value::num(args.len() as f64));
        scope.insert("nargout".to_string(), Value::num(nargout as f64));
        bind(&mut scope, &f.params, args, &f.name)?;
        self.scoped(scope, |me| {
            me.exec_block(&f.body)?;
            let mut out = vec![];
            for (k, o) in f.outs[..nout].iter().enumerate() {
                match me.vars.remove(o) {
                    Some(v) => out.push(v),
                    None if k < nargout => return Err(format!("{}: output '{o}' undefined", f.name)),
                    None => break,
                }
            }
            if va_out
                && out.len() == nout
                && let Some(Value::Cell(c)) = me.vars.remove("varargout")
            {
                out.extend(c);
            }
            if out.len() < nargout {
                return Err(format!("{}: function called with too many outputs", f.name));
            }
            Ok(out)
        })
    }

    /// Function by name: script/user functions, `name.m` in the current directory, builtins, Python addons.
    pub(super) fn call_named(&mut self, name: &str, args: Vec<Value>, nargout: usize) -> Result<Vec<Value>, String> {
        if let Some(f) = self.funcs.get(name).cloned() {
            return self.call_user(f, args, nargout);
        }
        if !self.no_mfile.contains(name) && self.mfile(name)? {
            return match self.funcs.get(name).cloned() {
                Some(f) => self.call_user(f, args, nargout),
                None => Ok(vec![]), // it was a script
            };
        }
        if let Some(b) = crate::resolve_builtin(name) {
            return b(self, &args, nargout);
        }
        if crate::py::has(name) {
            return crate::py::call(name, &args, nargout);
        }
        Err(format!("'{name}' undefined"))
    }

    /// `name.m` in the current directory: a function file is registered, a script runs right here
    /// in the caller's workspace. False if there is no such file.
    fn mfile(&mut self, name: &str) -> Result<bool, String> {
        let path = format!("{name}.m");
        let Ok(src) = std::fs::read_to_string(&path) else {
            self.no_mfile.insert(name.to_string());
            return Ok(false);
        };
        let prog = parse(&src, &|_| false).map_err(|e| format!("{path}: {e}"))?;
        if prog.funcs.is_empty() {
            self.exec_block(&prog.body)?;
        }
        for (k, f) in prog.funcs.into_iter().enumerate() {
            let f = Rc::new(f);
            if k == 0 {
                self.funcs.insert(name.to_string(), f.clone());
            }
            self.funcs.entry(f.name.clone()).or_insert(f);
        }
        Ok(true)
    }
    pub(super) fn is_function(&self, n: &str) -> bool {
        self.funcs.contains_key(n) || crate::resolve_builtin(n).is_some() || crate::py::has(n)
    }

    /// `fplot('x^2 + y')`-style string: vectorized anonymous function of its free variables (sorted).
    pub(super) fn str_lambda(&mut self, s: &str) -> Result<Value, String> {
        let mut v = String::new();
        for c in s.chars() {
            if matches!(c, '*' | '/' | '^') && !v.ends_with('.') {
                v.push('.');
            }
            v.push(c);
        }
        let mut names: Vec<String> = vec![];
        walk_ids(&parse_expr(&v)?, &mut |n| {
            if !names.iter().any(|x| x == n) && !self.is_function(n) && !self.vars.contains_key(n) {
                names.push(n.to_string());
            }
        });
        names.sort();
        if names.is_empty() {
            names.push("x".into());
        }
        let e = parse_expr(&format!("@({}) {v}", names.join(", ")))?;
        self.ev(&e)
    }

    fn assign(&mut self, lv: &LValue, rhs: Value) -> Result<(), String> {
        if lv.path.is_empty() {
            self.vars.insert(lv.name.clone(), rhs);
            return Ok(());
        }
        let mut path = vec![];
        for acc in &lv.path {
            let dims = self.vars.get(&lv.name).and_then(|v| index::peek(v, &path)).map_or((0, 0), Value::size);
            path.push(match acc {
                Acc::Field(f) => AccV::Field(f.clone()),
                Acc::Dyn(e) => AccV::Field(self.ev(e)?.as_str()?.to_string()),
                Acc::Paren(a) => AccV::Paren(self.eval_index_args(dims, a)?),
                Acc::Brace(a) => AccV::Brace(self.eval_index_args(dims, a)?),
            });
        }
        // ponytail: an error halfway through a nested path can leave created fields behind
        let existed = self.vars.contains_key(&lv.name);
        let mut v = self.vars.remove(&lv.name).unwrap_or_else(Value::empty);
        let r = index::assign_path(&mut v, &path, rhs);
        if existed || r.is_ok() {
            self.vars.insert(lv.name.clone(), v);
        }
        r
    }
}
