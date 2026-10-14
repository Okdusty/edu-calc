//! Octave language: lexer, parser, evaluator, value display.
//! Public items in this file are CONTRACT (other modules call them); internals are free.
mod display;
mod eval;
mod fns;
mod index;
mod lexer;
mod parser;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::value::{Builtin, Func, Value};

/// Anonymous function `@(x) ...`. Fields beyond these are free to add.
#[derive(Clone, Debug)]
pub struct Lambda {
    pub params: Vec<String>,
    /// Octave source, e.g. "@(x) x .^ 2"
    pub src: String,
    def: Rc<parser::LambdaDef>,
    /// workspace variables referenced by the body, captured by value at creation
    caps: Rc<HashMap<String, Value>>,
}

pub struct Interp {
    /// workspace (top-level scope)
    pub vars: HashMap<String, Value>,
    pub plots: crate::plot::Session,
    out: String,
    /// functions defined in scripts or loaded from `name.m`
    funcs: HashMap<String, Rc<parser::Function>>,
    /// call depth (recursion limit)
    depth: usize,
    /// `end` context per open index: (rows, cols, arg position, arg count)
    end_ctx: Vec<(usize, usize, usize, usize)>,
    /// line of the innermost failing statement
    err_line: Option<usize>,
    /// names with no `name.m` in the current directory (cleared on each `eval`)
    no_mfile: HashSet<String>,
    /// a top-level statement is running (nested evals don't log it again)
    logging: bool,
    /// plots version left by the last change that isn't logged (figure switch, clf, close)
    unlogged: u64,
}

impl Interp {
    pub fn new() -> Self {
        display::set_long(false);
        Interp {
            vars: HashMap::new(),
            plots: Default::default(),
            out: String::new(),
            funcs: HashMap::new(),
            depth: 0,
            end_ctx: vec![],
            err_line: None,
            no_mfile: HashSet::new(),
            logging: false,
            unlogged: 0,
        }
    }
    /// Run Octave source (script or REPL line). Printed text accumulates; read it with
    /// `take_output`, also after an Err.
    pub fn eval(&mut self, src: &str) -> Result<(), String> {
        self.err_line = None;
        self.no_mfile.clear();
        self.run_src(src).map_err(|e| match self.err_line.take() {
            Some(l) if !e.contains("near line") => format!("{e} near line {l}"),
            _ => e,
        })
    }
    /// Keep the plot change just made out of the figure's Octave log (`Figure::code`) unless the
    /// statement changes plots again: switching figures, clf, close.
    pub(crate) fn skip_log(&mut self) {
        self.unlogged = self.plots.version;
    }
    /// Names of functions defined in scripts or loaded from `name.m` (for completion).
    pub fn func_names(&self) -> Vec<String> {
        self.funcs.keys().cloned().collect()
    }
    pub fn take_output(&mut self) -> String {
        std::mem::take(&mut self.out)
    }
    /// Append to the output buffer (disp/printf/auto-display go through here).
    pub fn print(&mut self, s: &str) {
        self.out.push_str(s);
    }
    /// Call a function handle (Value::Func) or a function by name (Value::Str).
    /// A string that isn't a name, like "x^2 + y", becomes a vectorized function of its free variables.
    pub fn call(&mut self, f: &Value, args: &[Value], nargout: usize) -> Result<Vec<Value>, String> {
        match f {
            Value::Func(h) => match &**h {
                Func::Named(n) => self.call_named(n, args.to_vec(), nargout),
                Func::Anon(l) => self.call_lambda(l, args, nargout),
            },
            Value::Str(s) if is_ident(s) => self.call_named(s, args.to_vec(), nargout),
            Value::Str(s) => {
                let f = self.str_lambda(s)?;
                self.call(&f, args, nargout)
            }
            v => Err(format!("invalid use of a {} value as a function", v.type_name())),
        }
    }
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Octave-style display of `name = value` (what the REPL prints without `;`).
pub fn display(name: &str, v: &Value) -> String {
    display::display(name, v)
}

/// Interpreter-level builtins (eval, feval, str2func, func2str, exist, clear, who, ...).
pub fn builtin(name: &str) -> Option<Builtin> {
    fns::lookup(name)
}
