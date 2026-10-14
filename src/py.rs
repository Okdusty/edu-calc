//! Python addons: ~/.config/qcalcp/addons/*.py register functions callable from Octave code.
//! Addons `import qcalcp` (embedded module, source below); its `functions` dict is the registry.
use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

use pyo3::IntoPyObjectExt;
use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyBool, PyComplex, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

use crate::lang::Interp;
use crate::value::{Builtin, C64, CMatrix, Mat, Value};

const API_SRC: &CStr = cr#"
"""qcalcp addon API: register Python functions callable from Octave code."""
import contextlib, io

version = ""  # set from Rust
functions = {}  # Octave name -> callable
_globals = vars(__import__("__main__"))  # shared by pyexec / pyeval / pyrun
_out = ""


def register(name, f):
    """Make f callable from Octave code as name(...)."""
    if not callable(f):
        raise TypeError(f"qcalcp.register: {name!r} is not callable")
    functions[name] = f
    return f


def function(f):
    """Decorator: @function registers f as f.__name__, @function("name") as name."""
    if isinstance(f, str):
        return lambda g: register(f, g)
    return register(f.__name__, f)


def _run(src, mode, filename):
    """Run src in the shared globals; stdout lands in _out, even on error."""
    global _out
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            return eval(compile(src, filename, mode), _globals)
    finally:
        _out = buf.getvalue()
"#;

static API: PyOnceLock<Py<PyModule>> = PyOnceLock::new();

/// The `qcalcp` module, created (and put in sys.modules) on first use.
fn api(py: Python<'_>) -> PyResult<&Bound<'_, PyModule>> {
    API.get_or_try_init(py, || {
        let m = PyModule::from_code(py, API_SRC, c"<qcalcp>", c"qcalcp")?;
        m.setattr("version", env!("CARGO_PKG_VERSION"))?;
        Ok::<_, PyErr>(m.unbind())
    })
    .map(|m| m.bind(py))
}

fn functions(py: Python<'_>) -> PyResult<Bound<'_, PyDict>> {
    Ok(api(py)?.getattr("functions")?.cast_into()?)
}

fn names(py: Python<'_>) -> PyResult<Vec<String>> {
    functions(py)?.keys().extract()
}

/// Load every *.py in `dir`; returns names of registered functions.
pub fn load_addons(dir: &Path) -> Result<Vec<String>, String> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "py")).collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    if files.is_empty() {
        return Ok(vec![]);
    }
    files.sort();
    Python::attach(|py| {
        let setup = || -> PyResult<()> {
            api(py)?;
            let path = py.import("sys")?.getattr("path")?;
            let d = dir.to_string_lossy();
            if !path.contains(&*d)? {
                path.call_method1("append", (d,))?;
            }
            Ok(())
        };
        setup().map_err(|e| format!("python: {}", describe(py, &e)))?;
        let errs: Vec<String> = files
            .iter()
            .filter_map(|f| load_file(py, f).err().map(|e| format!("{}: {}", f.file_name().unwrap_or_default().display(), describe(py, &e))))
            .collect();
        if errs.is_empty() { names(py).map_err(|e| describe(py, &e)) } else { Err(errs.join("; ")) }
    })
}

/// Execute a file as a module named after its stem (lands in sys.modules, like an import).
fn load_file(py: Python<'_>, path: &Path) -> PyResult<()> {
    let src = CString::new(std::fs::read_to_string(path)?)?;
    let file = CString::new(path.to_string_lossy().into_owned())?;
    let stem = CString::new(path.file_stem().unwrap_or_default().to_string_lossy().into_owned())?;
    PyModule::from_code(py, &src, &file, &stem).map(drop)
}

/// Is `name` an addon function?
pub fn has(name: &str) -> bool {
    // try_attach: never start Python just to answer "no"
    Python::try_attach(|py| API.get(py).is_some() && functions(py).and_then(|f| f.contains(name)).unwrap_or(false)).unwrap_or(false)
}

pub fn call(name: &str, args: &[Value], nargout: usize) -> Result<Vec<Value>, String> {
    if !has(name) {
        return Err(format!("python addon '{name}' not found"));
    }
    with_py(|py| {
        let f = functions(py)?.as_any().get_item(name)?;
        let args = args.iter().map(|a| to_py(py, a)).collect::<PyResult<Vec<_>>>()?;
        outputs(&f.call1(PyTuple::new(py, args)?)?, nargout)
    })
}

/// Octave-side Python builtins (pyeval, pyexec, ...).
pub fn builtin(name: &str) -> Option<Builtin> {
    match name {
        "pyeval" => Some(|i, a, n| run(i, str_arg(a, "pyeval")?, "eval", "<pyeval>", n)),
        "pyexec" => Some(|i, a, n| run(i, str_arg(a, "pyexec")?, "exec", "<pyexec>", n)),
        "pyrun" => Some(|i, a, n| {
            let path = str_arg(a, "pyrun")?;
            let src = std::fs::read_to_string(path).map_err(|e| format!("pyrun: {path}: {e}"))?;
            run(i, &src, "exec", path, n)
        }),
        "pyaddons" => Some(|_, _, _| Ok(vec![Value::Cell(with_py(names)?.into_iter().map(Value::Str).collect())])),
        _ => None,
    }
}

fn str_arg<'a>(args: &'a [Value], f: &str) -> Result<&'a str, String> {
    match args {
        [Value::Str(s)] => Ok(s),
        _ => Err(format!("{f}: expected one string argument")),
    }
}

/// pyeval / pyexec / pyrun: `src` in the shared globals; Python's stdout goes to the interp output.
fn run(interp: &mut Interp, src: &str, mode: &str, file: &str, nargout: usize) -> Result<Vec<Value>, String> {
    let (r, out) = Python::attach(|py| {
        let r = api(py).and_then(|m| m.call_method1("_run", (src, mode, file))).and_then(|r| outputs(&r, nargout));
        let out: String = api(py).and_then(|m| m.getattr("_out")?.extract()).unwrap_or_default();
        (r.map_err(|e| format!("python: {}", describe(py, &e))), out)
    });
    interp.print(&out);
    r
}

/// Attach to Python; errors become "python: ExcType: msg (File ..., line N, in f)".
fn with_py<T>(f: impl for<'py> FnOnce(Python<'py>) -> PyResult<T>) -> Result<T, String> {
    Python::attach(|py| f(py).map_err(|e| format!("python: {}", describe(py, &e))))
}

/// "ExcType: msg", plus the innermost traceback frame when there is one.
fn describe(py: Python<'_>, e: &PyErr) -> String {
    let ty = e.get_type(py).name().map_or("Exception".to_string(), |n| n.to_string());
    let mut s = format!("{ty}: {}", e.value(py));
    let tb = e.traceback(py).and_then(|tb| tb.format().ok()).unwrap_or_default();
    if let Some(at) = tb.lines().rev().map(str::trim).find(|l| l.starts_with("File ")) {
        s += &format!(" ({at})");
    }
    s
}

/// Python result -> outputs: a tuple spreads (`[a, b] = f(x)`), None at nargout 0 is no output.
fn outputs(r: &Bound<'_, PyAny>, nargout: usize) -> PyResult<Vec<Value>> {
    if r.is_none() && nargout == 0 {
        return Ok(vec![]);
    }
    match r.cast::<PyTuple>() {
        Ok(t) if !t.is_empty() => t.iter().take(nargout.max(1)).map(|o| to_value(&o)).collect(),
        _ => Ok(vec![to_value(r)?]),
    }
}

fn to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    match v {
        Value::Num(m) => array(py, m, |x| x),
        Value::Bool(m) => array(py, m, |x| x != 0.0),
        Value::Str(s) => s.into_bound_py_any(py),
        Value::Complex(m) => array(py, m, |z| PyComplex::from_doubles(py, z.re, z.im)),
        Value::Cell(c) => c.iter().map(|v| to_py(py, v)).collect::<PyResult<Vec<_>>>()?.into_bound_py_any(py),
        Value::Struct(s) => {
            let d = PyDict::new(py);
            for (k, v) in s {
                d.set_item(k, to_py(py, v)?)?;
            }
            Ok(d.into_any())
        }
        // ponytail: handles can't cross; wrap them in a Python callable once Python can call back into Interp
        Value::Func(_) => Err(PyTypeError::new_err("function handles can't be passed to Python")),
    }
}

/// Scalar -> number, vector (or empty) -> flat list, matrix -> list of row lists.
// ponytail: column vectors flatten like rows, orientation is lost on the way back
fn array<'py, E: Copy + Default, T: IntoPyObject<'py>>(py: Python<'py>, m: &Mat<E>, f: impl Fn(E) -> T) -> PyResult<Bound<'py, PyAny>> {
    if m.is_scalar() {
        return f(m.data[0]).into_bound_py_any(py);
    }
    if m.rows <= 1 || m.cols == 1 {
        return m.data.iter().map(|&x| f(x)).collect::<Vec<_>>().into_bound_py_any(py);
    }
    (0..m.rows).map(|r| (0..m.cols).map(|c| f(m.get(r, c))).collect::<Vec<_>>()).collect::<Vec<_>>().into_bound_py_any(py)
}

/// numpy arrays/scalars and anything else with `.tolist()` -> plain Python objects.
fn plain<'py>(o: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
    if o.hasattr("tolist")? { o.call_method0("tolist") } else { Ok(o.clone()) }
}

fn to_value(o: &Bound<'_, PyAny>) -> PyResult<Value> {
    let o = &plain(o)?;
    if o.is_none() {
        return Ok(Value::empty());
    }
    if let Ok(b) = o.cast::<PyBool>() {
        return Ok(Value::bool(b.is_true()));
    }
    if o.is_instance_of::<PyInt>() || o.is_instance_of::<PyFloat>() {
        return Ok(Value::num(o.extract()?));
    }
    if let Ok(z) = o.cast::<PyComplex>() {
        return Ok(Value::complex(CMatrix::scalar(C64::new(z.real(), z.imag()))));
    }
    if let Ok(s) = o.cast::<PyString>() {
        return Ok(Value::Str(s.to_str()?.to_string()));
    }
    if let Ok(d) = o.cast::<PyDict>() {
        return d.iter().map(|(k, v)| Ok((k.str()?.to_string(), to_value(&v)?))).collect::<PyResult<_>>().map(Value::Struct);
    }
    if o.is_instance_of::<PyList>() || o.is_instance_of::<PyTuple>() {
        return seq(o.try_iter()?.map(|x| plain(&x?)).collect::<PyResult<_>>()?);
    }
    Err(PyTypeError::new_err(format!("can't convert {} to an Octave value", o.get_type().name()?)))
}

/// list/tuple -> row vector (all numbers), matrix (all equal-length numeric rows), else cell.
fn seq(items: Vec<Bound<'_, PyAny>>) -> PyResult<Value> {
    if items.is_empty() {
        return Ok(Value::empty());
    }
    let nested = items.iter().all(|o| o.is_instance_of::<PyList>() || o.is_instance_of::<PyTuple>());
    let vals = items.iter().map(to_value).collect::<PyResult<Vec<_>>>()?;
    // built complex, narrowed back to real below when no element has an imaginary part
    let rows: Option<Vec<CMatrix>> = vals
        .iter()
        .map(|v| match v {
            Value::Num(_) | Value::Bool(_) | Value::Complex(_) if v.size().0 == 1 && (nested || v.size().1 == 1) => v.cmatrix().ok(),
            _ => None,
        })
        .collect();
    let m = match rows {
        Some(r) if !nested => CMatrix::row(r.iter().map(|m| m.data[0]).collect()),
        Some(r) if r.iter().all(|m| m.cols == r[0].cols) => CMatrix::from_rows(&r.iter().map(|m| m.data.clone()).collect::<Vec<_>>()),
        _ => return Ok(Value::Cell(vals)),
    };
    Ok(if vals.iter().all(|v| matches!(v, Value::Bool(_))) { Value::Bool(m.re()) } else { Value::complex(m) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Matrix;

    /// Fresh temp dir with the given files.
    fn addon_dir(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("qcalcp-py-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (name, src) in files {
            std::fs::write(d.join(name), src).unwrap();
        }
        d
    }

    fn one(name: &str, args: &[Value]) -> Value {
        call(name, args, 1).unwrap().remove(0)
    }

    fn mat(v: &Value) -> Matrix {
        v.matrix().unwrap()
    }

    const ROUNDTRIP: &str = r#"
import qcalcp

@qcalcp.function
def t_double(x):
    return x * 2

@qcalcp.function("t_inc")
def _inc(v):
    return [a + 1 for a in v]

qcalcp.register("t_same", lambda x: x)

@qcalcp.function
def t_kinds(*args):
    return " ".join(type(a).__name__ for a in args)

@qcalcp.function
def t_mask(v):
    return [a > 1 for a in v]

@qcalcp.function
def t_minmax(v):
    return min(v), max(v)

@qcalcp.function
def t_dict():
    return {"a": 1, "b": "x", "c": [1, "y"]}

@qcalcp.function
def t_raise():
    raise ValueError("boom")

@qcalcp.function
def t_none():
    pass

@qcalcp.function
def t_version():
    return qcalcp.version
"#;

    #[test]
    fn roundtrip() {
        let d = addon_dir("roundtrip", &[("rt.py", ROUNDTRIP)]);
        let names = load_addons(&d).unwrap();
        assert!(["t_double", "t_inc", "t_same"].iter().all(|n| names.contains(&n.to_string())), "{names:?}");
        assert!(has("t_double") && !has("t_nope"));

        assert_eq!(one("t_double", &[Value::num(21.)]).scalar().unwrap(), 42.);
        assert_eq!(mat(&one("t_inc", &[vec![1., 2.].into()])), Matrix::row(vec![2., 3.]));
        let m = Matrix::from_rows(&[vec![1., 2., 3.], vec![4., 5., 6.]]);
        assert_eq!(mat(&one("t_same", &[Value::Num(m.clone())])), m);
        assert_eq!(one("t_same", &["hi".into()]).as_str().unwrap(), "hi");
        assert!(matches!(one("t_same", &[Value::bool(true)]), Value::Bool(b) if b.data == [1.]));
        let mask = Value::Bool(Matrix::from_rows(&[vec![1., 0.], vec![0., 1.]]));
        assert!(matches!(one("t_same", &[mask]), Value::Bool(b) if b.rows == 2 && b.data == [1., 0., 0., 1.]));
        assert!(matches!(one("t_same", &[Value::empty()]), Value::Num(e) if e.is_empty()));
        let z = Value::complex(CMatrix::from_rows(&[vec![C64::new(1., 2.), C64::new(0., -1.)], vec![C64::new(3., 0.), C64::new(4., 5.)]]));
        assert!(matches!((one("t_same", &[z.clone()]), z), (Value::Complex(a), Value::Complex(b)) if a == b));
        assert!(matches!(one("t_same", &[Value::complex(CMatrix::scalar(C64::new(0., 1.)))]), Value::Complex(a) if a.data == [C64::new(0., 1.)]));
        assert_eq!(one("t_kinds", &[Value::complex(CMatrix::scalar(C64::new(0., 1.)))]).as_str().unwrap(), "complex");

        let st = Value::Struct([("k".to_string(), Value::num(1.))].into());
        let cell = Value::Cell(vec![Value::num(1.), "a".into()]);
        let kinds = [Value::num(1.), vec![1., 2.].into(), Value::Num(m), "s".into(), Value::bool(false), cell.clone(), st];
        assert_eq!(one("t_kinds", &kinds).as_str().unwrap(), "float list list str bool list dict");
        assert!(matches!(one("t_same", &[cell]), Value::Cell(c) if c.len() == 2));
        assert!(matches!(one("t_mask", &[vec![1., 2.].into()]), Value::Bool(b) if b.data == [0., 1.]));

        let out = call("t_minmax", &[vec![3., 1., 2.].into()], 2).unwrap();
        assert_eq!((out[0].scalar().unwrap(), out[1].scalar().unwrap()), (1., 3.));
        assert_eq!(call("t_minmax", &[vec![3., 1.].into()], 0).unwrap().len(), 1);

        let Value::Struct(s) = one("t_dict", &[]) else { panic!("expected struct") };
        assert_eq!(s["a"].scalar().unwrap(), 1.);
        assert_eq!(s["b"].as_str().unwrap(), "x");
        assert!(matches!(&s["c"], Value::Cell(c) if c.len() == 2));

        let e = call("t_raise", &[], 1).unwrap_err();
        assert!(e.starts_with("python: ValueError: boom") && e.contains("rt.py"), "{e}");
        let handle = Value::Func(std::rc::Rc::new(crate::value::Func::Named("sin".into())));
        assert!(call("t_same", &[handle], 1).unwrap_err().contains("TypeError"));
        assert!(call("t_nope", &[], 1).unwrap_err().contains("not found"));
        assert!(call("t_none", &[], 0).unwrap().is_empty());
        assert_eq!(one("t_version", &[]).as_str().unwrap(), env!("CARGO_PKG_VERSION"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn broken_file_does_not_block_others() {
        let good = "import qcalcp\nqcalcp.register('t_good', abs)\n";
        let d = addon_dir("broken", &[("a_bad.py", "def oops(:\n"), ("b_good.py", good)]);
        let e = load_addons(&d).unwrap_err();
        assert!(e.starts_with("a_bad.py: SyntaxError"), "{e}");
        assert_eq!(one("t_good", &[Value::num(-2.)]).scalar().unwrap(), 2.);
        assert!(load_addons(&d.join("missing")).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn python_to_value_shapes() {
        let v = |src: &str| Python::attach(|py| to_value(&py.eval(&CString::new(src).unwrap(), None, None)?));
        assert_eq!(mat(&v("[[1], [2]]").unwrap()), Matrix::col(vec![1., 2.]));
        assert!(matches!(v("[1, 'a']").unwrap(), Value::Cell(c) if c.len() == 2));
        assert!(matches!(v("[[1, 2], [3]]").unwrap(), Value::Cell(c) if c.len() == 2));
        assert!(matches!(v("None").unwrap(), Value::Num(e) if e.is_empty()));
        assert!(matches!(v("[True, False]").unwrap(), Value::Bool(_)));
        assert!(matches!(v("[True, 2]").unwrap(), Value::Num(_)));
        assert!(matches!(v("1j").unwrap(), Value::Complex(z) if z.data == [C64::new(0., 1.)]));
        assert!(matches!(v("[1, 2j]").unwrap(), Value::Complex(z) if z.data == [C64::new(1., 0.), C64::new(0., 2.)]));
        assert!(matches!(v("[[1j, 2], [3, 4]]").unwrap(), Value::Complex(z) if z.rows == 2 && z.get(0, 1) == C64::new(2., 0.)));
        assert!(matches!(v("[1+0j, 2]").unwrap(), Value::Num(m) if m.data == [1., 2.])); // narrows
    }

    #[test]
    fn pyexec_pyeval() {
        let mut i = Interp::new();
        let (exec, eval) = (builtin("pyexec").unwrap(), builtin("pyeval").unwrap());
        assert!(exec(&mut i, &["t_a = 3\nprint('hi', t_a)".into()], 0).unwrap().is_empty());
        assert_eq!(i.take_output(), "hi 3\n");
        assert_eq!(eval(&mut i, &["t_a * 2".into()], 1).unwrap()[0].scalar().unwrap(), 6.);
        assert_eq!(eval(&mut i, &["(1, 'x')".into()], 2).unwrap()[1].as_str().unwrap(), "x");
        let e = exec(&mut i, &["print('partial')\n1/0".into()], 0).unwrap_err();
        assert!(e.starts_with("python: ZeroDivisionError") && e.contains("<pyexec>"), "{e}");
        assert_eq!(i.take_output(), "partial\n");
        assert!(eval(&mut i, &[Value::num(1.)], 1).is_err());
        assert!(builtin("pyrun").unwrap()(&mut i, &["/nonexistent/x.py".into()], 0).is_err());
        assert!(matches!(&builtin("pyaddons").unwrap()(&mut i, &[], 1).unwrap()[0], Value::Cell(_)));
    }

    #[test]
    fn example_addon() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/addons");
        let names = load_addons(&d).unwrap();
        for f in ["sinc", "minmax", "movavg", "projectile", "hypotn"] {
            assert!(names.contains(&f.to_string()), "{f} in {names:?}");
        }
        let s = mat(&one("sinc", &[vec![0., 0.5].into()]));
        assert!(s.data[0] == 1. && (s.data[1] - 2. / std::f64::consts::PI).abs() < 1e-12);
        let mm = call("minmax", &[vec![3., -1., 2.].into()], 2).unwrap();
        assert_eq!((mm[0].scalar().unwrap(), mm[1].scalar().unwrap()), (-1., 3.));
        assert_eq!(mat(&one("movavg", &[vec![1., 2., 3., 4.].into(), Value::num(2.)])), Matrix::row(vec![1.5, 2.5, 3.5]));
        let p = call("projectile", &[Value::num(10.), Value::num(45.)], 2).unwrap();
        assert!((p[0].scalar().unwrap() - 100. / 9.81).abs() < 1e-9);
        assert!((p[1].scalar().unwrap() - 25. / 9.81).abs() < 1e-9);
        assert_eq!(one("hypotn", &[Value::num(3.), Value::num(4.), Value::num(12.)]).scalar().unwrap(), 13.);
    }
}
