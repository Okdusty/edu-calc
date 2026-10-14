//! One-line documentation of Octave-side builtins (signature + description), for completion and
//! `help NAME`. CONTRACT: completion agent implements.
//! Signatures are Octave style: `[]` optional, `|` alternatives. Completion inserts follow the
//! first form: `name(□)` when it is a call, the bare name for `pi`, `hold on|off` and the like.
use crate::lang::Interp;
use crate::value::{Builtin, Value};

/// (name, signature, description)
pub type Doc = (&'static str, &'static str, &'static str);

/// Every documented builtin, sorted by name.
pub fn all() -> &'static [Doc] {
    DOCS
}

/// Documentation of one builtin.
pub fn doc(name: &str) -> Option<&'static Doc> {
    DOCS.binary_search_by(|d| d.0.cmp(name)).ok().map(|k| &DOCS[k])
}

/// `help`, `help NAME`, `lookfor WORD`.
pub fn builtin(name: &str) -> Option<Builtin> {
    let f: Builtin = match name {
        "help" => |it, a, n| {
            let s = match words("help", a)?.as_str() {
                "" => OVERVIEW.to_string(),
                w => help(it, w),
            };
            out(it, s, n)
        },
        "lookfor" => |it, a, n| match words("lookfor", a)?.as_str() {
            "" => Err("Invalid call to lookfor".into()),
            w => out(it, lookfor(w), n),
        },
        _ => return None,
    };
    Some(f)
}

/// String arguments joined by spaces (`help foo`, `lookfor bit shift`).
fn words(f: &str, a: &[Value]) -> Result<String, String> {
    let w: Result<Vec<&str>, _> = a.iter().map(|v| v.as_str().map_err(|_| format!("{f}: expected a name"))).collect();
    Ok(w?.join(" ").trim().to_string())
}

/// Printed, or returned as text when asked for (`s = help('sin')`).
fn out(it: &mut Interp, s: String, nargout: usize) -> Result<Vec<Value>, String> {
    if nargout > 0 {
        return Ok(vec![Value::Str(s)]);
    }
    it.print(&s);
    Ok(vec![])
}

fn help(it: &Interp, w: &str) -> String {
    if let Some((_, sig, desc)) = doc(w).or_else(|| KEYWORDS.iter().find(|k| k.0 == w)) {
        return format!("{sig}\n    {desc}\n");
    }
    if it.vars.contains_key(w) {
        return format!("'{w}' is a variable\n");
    }
    if it.func_names().iter().any(|f| f == w) {
        return format!("'{w}' is a user-defined function\n");
    }
    if crate::py::has(w) {
        return format!("'{w}' is a Python addon function\n");
    }
    let near = close(w);
    if near.is_empty() {
        return format!("help: '{w}' not found; try lookfor {w}\n");
    }
    format!("help: '{w}' not found; did you mean {}?\n", near.join(", "))
}

/// Up to 5 documented names within 2 edits of `w` or containing it, closest first.
fn close(w: &str) -> Vec<&'static str> {
    let lw = w.to_lowercase();
    let mut near: Vec<(usize, &str)> = DOCS
        .iter()
        .map(|d| (if d.0.to_lowercase().contains(&lw) { 1 } else { edits(&lw, &d.0.to_lowercase()) }, d.0))
        .filter(|&(k, _)| k <= 2)
        .collect();
    near.sort();
    near.into_iter().take(5).map(|(_, n)| n).collect()
}

/// Levenshtein distance.
fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = (diag + (ca != cb) as usize).min(up + 1).min(row[j] + 1);
            diag = up;
        }
    }
    row[b.len()]
}

/// Names and descriptions containing `w`, ignoring case.
fn lookfor(w: &str) -> String {
    let w = w.to_lowercase();
    let hits: Vec<&Doc> = DOCS.iter().chain(KEYWORDS).filter(|d| d.0.to_lowercase().contains(&w) || d.2.to_lowercase().contains(&w)).collect();
    if hits.is_empty() {
        return format!("lookfor: nothing found for '{w}'\n");
    }
    let width = hits.iter().map(|d| d.0.len()).max().unwrap_or(0);
    hits.iter().map(|d| format!("{:<width$}  {}\n", d.0, d.2)).collect()
}

const OVERVIEW: &str = "\
help NAME      signature and description of a function or keyword
lookfor WORD   functions whose name or description mentions WORD
While typing, suggestions pop up with the same docs; Tab or Enter accepts one.

  elementary   sin cos tan exp log sqrt abs round mod factorial
  matrices     zeros ones eye linspace reshape size find sort repmat
  linear alg.  inv det eig svd lu qr norm rank linsolve
  statistics   sum mean median std var max min hist quantile
  calculus     diff integral trapz fzero fminsearch ode45 polyfit
  plotting     plot fplot surf contour subplot hold legend keypoints
  bits, logic  dec2bin bin2dec dec2hex bitand bitor bitxor bitshift xor truthtable
  files        csvread csvwrite readmatrix writematrix importdata readtable
  strings      sprintf printf num2str strsplit strrep strcmp
  calculator   qalc qalcfn qalc2oct pretty
";

/// Statement keywords the parser accepts (completion and `help for`).
pub const KEYWORDS: &[Doc] = &[
    ("break", "break", "leave the innermost loop"),
    ("case", "case value | case {a, b}", "branch of a switch"),
    ("catch", "catch [err]", "error handler of a try block"),
    ("continue", "continue", "next iteration of the innermost loop"),
    ("do", "do ... until cond", "loop that runs at least once"),
    ("else", "else", "branch taken when no if/elseif condition holds"),
    ("elseif", "elseif cond", "further condition of an if"),
    ("end", "end", "close a block; inside an index, the last index"),
    ("end_try_catch", "end_try_catch", "close a try block"),
    ("endfor", "endfor", "close a for loop"),
    ("endfunction", "endfunction", "close a function definition"),
    ("endif", "endif", "close an if block"),
    ("endparfor", "endparfor", "close a parfor loop"),
    ("endswitch", "endswitch", "close a switch block"),
    ("endwhile", "endwhile", "close a while loop"),
    ("for", "for var = range ... end", "loop over the columns of range"),
    ("function", "function [out] = name(args) ... end", "define a function"),
    ("if", "if cond ... elseif cond ... else ... end", "run code when cond is true"),
    ("otherwise", "otherwise", "default branch of a switch"),
    ("parfor", "parfor var = range ... end", "loop like for (runs serially)"),
    ("return", "return", "leave the current function or script"),
    ("switch", "switch expr ... case v ... otherwise ... end", "branch on a value"),
    ("try", "try ... catch err ... end", "run code and catch its errors"),
    ("until", "until cond", "end condition of a do loop"),
    ("while", "while cond ... end", "loop while cond is true"),
];

// ponytail: hand-written, the test below keeps it in step with the lookup tables
const DOCS: &[Doc] = &[
    ("I", "I", "imaginary unit √-1"),
    ("Inf", "Inf[(n, m)]", "positive infinity"),
    ("J", "J", "imaginary unit √-1"),
    ("NA", "NA[(n, m)]", "missing value (NaN here)"),
    ("NaN", "NaN[(n, m)]", "not-a-number"),
    ("abs", "abs(x)", "absolute value, or magnitude of a complex number"),
    ("acos", "acos(x)", "inverse cosine, in radians"),
    ("acosd", "acosd(x)", "inverse cosine, in degrees"),
    ("acosh", "acosh(x)", "inverse hyperbolic cosine"),
    ("all", "all(x[, dim])", "true if every element is nonzero"),
    ("and", "and(a, b)", "logical and a & b"),
    ("angle", "angle(z)", "phase angle of a complex number"),
    ("any", "any(x[, dim])", "true if any element is nonzero"),
    ("area", "area([x, ]y)", "area filled down to zero"),
    ("arg", "arg(z)", "phase angle of a complex number"),
    ("arrayfun", "arrayfun(f, A, ...)", "apply f to each element"),
    ("asin", "asin(x)", "inverse sine, in radians"),
    ("asind", "asind(x)", "inverse sine, in degrees"),
    ("asinh", "asinh(x)", "inverse hyperbolic sine"),
    ("atan", "atan(x)", "inverse tangent, in radians"),
    ("atan2", "atan2(y, x)", "four-quadrant inverse tangent of y/x"),
    ("atand", "atand(x)", "inverse tangent, in degrees"),
    ("atanh", "atanh(x)", "inverse hyperbolic tangent"),
    ("axis", "axis([x0 x1 y0 y1]) | axis equal|tight|auto", "axis limits and scaling"),
    ("bar", "bar([x, ]y[, width])", "bar chart; matrix columns are grouped"),
    ("base2dec", "base2dec(s, base)", "string in base 2..36 to decimal"),
    ("bin2dec", "bin2dec(s)", "binary string to decimal"),
    ("binopdf", "binopdf(k, n, p)", "binomial probability P(X = k)"),
    ("bitand", "bitand(a, b)", "bitwise AND"),
    ("bitcmp", "bitcmp(a[, bits])", "bitwise NOT within a bit width"),
    ("bitor", "bitor(a, b)", "bitwise OR"),
    ("bitshift", "bitshift(a, n)", "shift bits left (n > 0) or right (n < 0)"),
    ("bitxor", "bitxor(a, b)", "bitwise XOR"),
    ("blanks", "blanks(n)", "string of n spaces"),
    ("cat", "cat(dim, A, B, ...)", "concatenate along dimension dim"),
    ("cbrt", "cbrt(x)", "real cube root"),
    ("ceil", "ceil(x)", "round toward +Inf"),
    ("cell", "cell(n[, m])", "cell array of empty matrices"),
    ("cellfun", "cellfun(f, C, ...)", "apply f to each cell"),
    ("chol", "chol(A)", "Cholesky factor R with R'·R = A"),
    ("circshift", "circshift(x, n[, dim])", "rotate elements by n positions"),
    ("class", "class(x)", "class name: double, char, logical, cell, struct…"),
    ("clc", "clc", "clear the command window"),
    ("clear", "clear [name ... | all]", "remove workspace variables (x* wildcards ok)"),
    ("clearvars", "clearvars [name ...]", "remove workspace variables (x* wildcards ok)"),
    ("clf", "clf", "clear the current figure"),
    ("close", "close [all | n]", "close the current figure, all, or figure n"),
    ("colon", "colon(a, b) | colon(a, step, b)", "range a:b or a:step:b"),
    ("colorbar", "colorbar", "accepted; no colorbar is drawn"),
    ("colormap", "colormap(name)", "colormap: viridis, jet, hot, gray, cool, parula, turbo"),
    ("columns", "columns(x)", "number of columns"),
    ("compass", "compass(u, v)", "arrows from the origin (phasors)"),
    ("complex", "complex(re[, im])", "complex number re + im·i"),
    ("cond", "cond(A)", "condition number (2-norm)"),
    ("conj", "conj(z)", "complex conjugate"),
    ("contour", "contour([X, Y, ]Z[, n])", "contour lines"),
    ("contourf", "contourf([X, Y, ]Z[, n])", "filled contours"),
    ("conv", "conv(a, b)", "convolution, polynomial product"),
    ("corr", "corr(x[, y])", "correlation coefficients between columns"),
    ("corrcoef", "corrcoef(x[, y])", "correlation coefficient matrix"),
    ("cos", "cos(x)", "cosine (radians)"),
    ("cosd", "cosd(x)", "cosine of x in degrees"),
    ("cosh", "cosh(x)", "hyperbolic cosine"),
    ("cot", "cot(x)", "cotangent 1/tan(x)"),
    ("cov", "cov(x[, y])", "covariance (matrix)"),
    ("cross", "cross(a, b)", "cross product of 3-vectors"),
    ("csc", "csc(x)", "cosecant 1/sin(x)"),
    ("csvread", "csvread(file[, r0, c0])", "numeric matrix from a CSV file"),
    ("csvwrite", "csvwrite(file, M)", "write a matrix as CSV"),
    ("cummax", "cummax(x[, dim])", "running maximum"),
    ("cummin", "cummin(x[, dim])", "running minimum"),
    ("cumprod", "cumprod(x[, dim])", "cumulative product"),
    ("cumsum", "cumsum(x[, dim])", "cumulative sum"),
    ("cumtrapz", "cumtrapz([x, ]y)", "cumulative trapezoidal integral"),
    ("curl", "[cz, cav] = curl([X, Y, ]U, V)", "curl of a 2-D vector field"),
    ("cylinder", "cylinder([r[, n]])", "cylinder surface"),
    ("datatip", "datatip(x, y[, z])", "pin a data tip; datatip clear removes them"),
    ("db", "db(x[, 'power'])", "decibels 20·log10|x| (10·log10 for power)"),
    ("db2mag", "db2mag(d)", "decibels to magnitude, 10^(d/20)"),
    ("dblquad", "dblquad(f, xa, xb, ya, yb)", "double integral of f(x, y)"),
    ("deal", "[a, b, ...] = deal(x, ...)", "copy the inputs to the outputs"),
    ("dec2base", "dec2base(d, base[, len])", "decimal to a string in base 2..36"),
    ("dec2bin", "dec2bin(d[, len])", "decimal to binary string"),
    ("dec2hex", "dec2hex(d[, len])", "decimal to hexadecimal string"),
    ("deconv", "[q, r] = deconv(y, a)", "polynomial division"),
    ("deg2rad", "deg2rad(d)", "degrees to radians"),
    ("del2", "del2(M[, h])", "discrete Laplacian / 4"),
    ("det", "det(A)", "determinant"),
    ("diag", "diag(v[, k])", "diagonal matrix from v, or the diagonal of a matrix"),
    ("diff", "diff(x[, k[, dim]])", "differences of neighbors (k-th order)"),
    ("disp", "disp(x)", "print a value without its name"),
    ("display", "display(x)", "print a value without its name"),
    ("divergence", "divergence([X, Y, ]U, V)", "divergence of a 2-D vector field"),
    ("dlmread", "dlmread(file[, sep[, r0, c0]])", "numeric matrix from a delimited text file"),
    ("dlmwrite", "dlmwrite(file, M[, sep])", "write a matrix with a delimiter"),
    ("dot", "dot(a, b)", "dot product"),
    ("drawnow", "drawnow", "accepted; figures update by themselves"),
    ("e", "e[(n, m)]", "Euler's number 2.71828…"),
    ("eig", "[V, D] = eig(A)", "eigenvalues and eigenvectors"),
    ("eigplot", "eigplot(A)", "2×2 A: unit circle, its image, the eigenvectors"),
    ("eps", "eps[(x)]", "machine epsilon, or the spacing of doubles at x"),
    ("eq", "eq(a, b)", "a == b"),
    ("erf", "erf(x)", "error function"),
    ("erfc", "erfc(x)", "complementary error function 1 - erf(x)"),
    ("erfinv", "erfinv(y)", "inverse error function"),
    ("error", "error([id, ]fmt, ...)", "raise an error with a formatted message"),
    ("errorbar", "errorbar([x, ]y, e[, fmt])", "plot with error bars"),
    ("eval", "eval(try[, catch])", "run Octave code from a string"),
    ("exist", "exist(name)", "1 variable, 2 file, 5 builtin, 103 function, 0 none"),
    ("exp", "exp(x)", "exponential e^x"),
    ("expm", "expm(A)", "matrix exponential"),
    ("expm1", "expm1(x)", "e^x - 1, accurate for small x"),
    ("exppdf", "exppdf(x, mu)", "exponential density with mean mu"),
    ("eye", "eye(n[, m])", "identity matrix"),
    ("ezcontour", "ezcontour(f[, [a b]])", "contour lines of f(x, y)"),
    ("ezmesh", "ezmesh(f[, [a b]])", "wireframe of z = f(x, y)"),
    ("ezplot", "ezplot(f[, [a b]])", "plot y = f(x) from a handle or expression"),
    ("ezsurf", "ezsurf(f[, [a b]])", "surface z = f(x, y)"),
    ("factor", "[p, k] = factor(n)", "prime factorization of n"),
    ("factorial", "factorial(n)", "n!"),
    ("false", "false[(n, m)]", "logical 0, or a matrix of them"),
    ("fcontour", "fcontour(f[, [a b]])", "contour lines of f(x, y)"),
    ("feather", "feather(u, v)", "arrows along a baseline"),
    ("feval", "feval(f, args...)", "call a function by handle or name"),
    ("fft", "fft(x[, n])", "discrete Fourier transform"),
    ("fft2", "fft2(A)", "2-D discrete Fourier transform"),
    ("fftshift", "fftshift(X)", "move the zero frequency to the center"),
    ("fieldnames", "fieldnames(s)", "cell of a struct's field names"),
    ("figure", "figure[(n)]", "new figure, or select figure n"),
    ("fill", "fill(x, y[, color])", "filled polygon"),
    ("filter", "filter(b, a, x)", "1-D digital filter (FIR/IIR)"),
    ("fimplicit", "fimplicit(f[, [a b]])", "implicit curve f(x, y) = 0"),
    ("find", "find(x[, n])", "indices of nonzero elements"),
    ("fix", "fix(x)", "round toward zero"),
    ("flintmax", "flintmax", "largest exact consecutive integer, 2^53"),
    ("flip", "flip(x[, dim])", "reverse the order along a dimension"),
    ("fliplr", "fliplr(A)", "flip columns left to right"),
    ("flipud", "flipud(A)", "flip rows upside down"),
    ("floor", "floor(x)", "round toward -Inf"),
    ("fmesh", "fmesh(f[, [a b]])", "wireframe of z = f(x, y)"),
    ("fminbnd", "[x, fx] = fminbnd(f, a, b)", "minimum of f on [a, b]"),
    ("fminsearch", "[x, fx] = fminsearch(f, x0)", "minimum of f near x0 (Nelder-Mead)"),
    ("fminunc", "[x, fval] = fminunc(f, x0[, opts])", "unconstrained minimum of f (BFGS)"),
    ("format", "format short|long", "number display precision"),
    ("fplot", "fplot(f[, [a b]][, fmt])", "plot y = f(x) from a handle or expression"),
    ("fprintf", "fprintf([fid, ]fmt, ...)", "print formatted text"),
    ("fsolve", "[x, fval, info] = fsolve(f, x0[, opts])", "solve the nonlinear system f(x) = 0"),
    ("fsurf", "fsurf(f[, [a b]])", "surface z = f(x, y)"),
    ("func2str", "func2str(f)", "source text of a function handle"),
    ("fzero", "[x, fx] = fzero(f, x0)", "root of f near x0 or inside a bracket [a b]"),
    ("gamma", "gamma(x)", "gamma function Γ(x)"),
    ("gammaln", "gammaln(x)", "log of the gamma function"),
    ("gcd", "gcd(a, b, ...)", "greatest common divisor"),
    ("ge", "ge(a, b)", "a >= b"),
    ("gradient", "gradient(y[, h])", "numeric derivative of samples"),
    ("grid", "grid [on|off]", "toggle grid lines"),
    ("gt", "gt(a, b)", "a > b"),
    ("help", "help [name]", "documentation of a function or keyword"),
    ("hess", "[P, H] = hess(A)", "Hessenberg decomposition"),
    ("hex2dec", "hex2dec(s)", "hexadecimal string to decimal"),
    ("hist", "[n, c] = hist(y[, bins])", "histogram (10 bins by default)"),
    ("histc", "histc(x, edges)", "counts of x in the bins given by edges"),
    ("histogram", "histogram(y[, bins])", "histogram (10 bins by default)"),
    ("hold", "hold [on|off]", "keep the current plots when adding new ones"),
    ("horzcat", "horzcat(A, B, ...)", "concatenate side by side, [A, B]"),
    ("hypot", "hypot(x, y)", "√(x² + y²) without overflow"),
    ("i", "i", "imaginary unit √-1"),
    ("ifft", "ifft(X[, n])", "inverse discrete Fourier transform"),
    ("ifft2", "ifft2(A)", "inverse 2-D discrete Fourier transform"),
    ("ifftshift", "ifftshift(X)", "undo fftshift"),
    ("imag", "imag(z)", "imaginary part"),
    ("image", "image([x, y, ]C)", "matrix as a scaled color image"),
    ("imagesc", "imagesc([x, y, ]C)", "matrix as a scaled color image"),
    ("importdata", "importdata(file)", "load data from a text or spreadsheet file"),
    ("inf", "inf[(n, m)]", "positive infinity"),
    ("int2str", "int2str(x)", "rounded number to string"),
    ("integral", "integral(f, a, b)", "definite integral of f from a to b (adaptive)"),
    ("integral2", "integral2(f, xa, xb, ya, yb)", "double integral of f(x, y); limits may be functions"),
    ("integral3", "integral3(f, xa, xb, ya, yb, za, zb)", "triple integral of f(x, y, z)"),
    ("interp1", "interp1(x, y, xi[, method])", "1-D interpolation (linear, nearest, spline …)"),
    ("intplot", "intplot(f, a, b)", "f with its integral from a to b shaded"),
    ("inv", "inv(A)", "matrix inverse"),
    ("inverse", "inverse(A)", "matrix inverse"),
    ("iqr", "iqr(x)", "interquartile range"),
    ("is_function_handle", "is_function_handle(x)", "true if x is a function handle"),
    ("isa", "isa(x, cls)", "true if x is of class cls ('numeric', 'float' ok)"),
    ("isbool", "isbool(x)", "true if x is logical"),
    ("iscell", "iscell(x)", "true if x is a cell array"),
    ("ischar", "ischar(x)", "true if x is a string"),
    ("iscolumn", "iscolumn(x)", "true if x is n×1"),
    ("iscomplex", "iscomplex(x)", "true if x is complex"),
    ("isempty", "isempty(x)", "true if x has no elements"),
    ("isequal", "isequal(a, b, ...)", "true if all arguments are equal"),
    ("isfield", "isfield(s, name)", "true if struct s has field name"),
    ("isfinite", "isfinite(x)", "true where x is finite"),
    ("ishold", "ishold", "true if hold is on"),
    ("isinf", "isinf(x)", "true where x is ±Inf"),
    ("islogical", "islogical(x)", "true if x is logical"),
    ("ismatrix", "ismatrix(x)", "true if x is a 2-D array"),
    ("isnan", "isnan(x)", "true where x is NaN"),
    ("isnumeric", "isnumeric(x)", "true if x is a real or complex number array"),
    ("isprime", "isprime(n)", "true where n is prime"),
    ("isreal", "isreal(x)", "true if x has no complex storage"),
    ("isrow", "isrow(x)", "true if x is 1×n"),
    ("isscalar", "isscalar(x)", "true if x is 1×1"),
    ("issquare", "issquare(x)", "true if x is n×n"),
    ("isstruct", "isstruct(x)", "true if x is a struct"),
    ("isvarname", "isvarname(s)", "true if s is a valid variable name"),
    ("isvector", "isvector(x)", "true if x is 1×n or n×1"),
    ("j", "j", "imaginary unit √-1"),
    ("jacobian", "jacobian(f, x0)", "numeric Jacobian matrix of f at x0"),
    ("keypoints", "keypoints [on|off] | p = keypoints([x, y])", "mark extrema, roots, vertex, inflections, symmetry"),
    ("kron", "kron(A, B)", "Kronecker product"),
    ("kurtosis", "kurtosis(x)", "sample kurtosis"),
    ("lcm", "lcm(a, b, ...)", "least common multiple"),
    ("ldivide", "ldivide(a, b)", "element-wise left division a .\\ b"),
    ("le", "le(a, b)", "a <= b"),
    ("legend", "legend(str1, str2, ...)", "legend; also 'show', 'off', 'toggle'"),
    ("length", "length(x)", "size of the longest dimension"),
    ("lgamma", "lgamma(x)", "log of the gamma function"),
    ("linsolve", "linsolve(A, b)", "solve the linear system A·x = b"),
    ("linspace", "linspace(a, b[, n])", "n evenly spaced points from a to b (default 100)"),
    ("log", "log(x)", "natural logarithm"),
    ("log10", "log10(x)", "base-10 logarithm"),
    ("log1p", "log1p(x)", "log(1 + x), accurate for small x"),
    ("log2", "log2(x)", "base-2 logarithm"),
    ("loglog", "loglog([x, ]y[, fmt])", "plot with both axes logarithmic"),
    ("logm", "logm(A)", "matrix logarithm"),
    ("logspace", "logspace(a, b[, n])", "n points from 10^a to 10^b (default 50)"),
    ("lookfor", "lookfor word", "search function names and descriptions"),
    ("lower", "lower(s)", "lowercase"),
    ("lt", "lt(a, b)", "a < b"),
    ("lu", "[L, U, P] = lu(A)", "LU decomposition with pivoting"),
    ("mad", "mad(x[, opt[, dim]])", "mean (opt 0) or median (opt 1) absolute deviation"),
    ("mag2db", "mag2db(x)", "magnitude to decibels, 20·log10(x)"),
    ("magic", "magic(n)", "n×n magic square"),
    ("mat2str", "mat2str(A[, n])", "matrix as an Octave literal string"),
    ("matview", "matview(A)", "matrix heatmap with the values written in"),
    ("max", "[m, i] = max(x[, [], dim]) | max(a, b)", "largest element and its index, or element-wise max"),
    ("mean", "mean(x[, dim])", "arithmetic mean"),
    ("median", "median(x[, dim])", "median"),
    ("mesh", "mesh([X, Y, ]Z)", "wireframe 3-D surface"),
    ("meshgrid", "[X, Y] = meshgrid(x[, y])", "2-D grid coordinates from two vectors"),
    ("min", "[m, i] = min(x[, [], dim]) | min(a, b)", "smallest element and its index, or element-wise min"),
    ("minus", "minus(a, b)", "a - b"),
    ("mldivide", "mldivide(A, b)", "solve A·x = b, A \\ b"),
    ("mod", "mod(x, y)", "modulo, result has the sign of y"),
    ("mode", "mode(x[, dim])", "most frequent value"),
    ("more", "more on|off", "accepted and ignored"),
    ("mpower", "mpower(A, p)", "matrix power A ^ p"),
    ("mrdivide", "mrdivide(a, b)", "matrix right division a / b"),
    ("mtimes", "mtimes(a, b)", "matrix product a * b"),
    ("nan", "nan[(n, m)]", "not-a-number"),
    ("nchoosek", "nchoosek(n, k)", "binomial coefficient, or k-combinations of vector n"),
    ("ndgrid", "[X, Y] = ndgrid(x[, y])", "grid coordinates, x running down the rows"),
    ("ndims", "ndims(x)", "number of dimensions"),
    ("ne", "ne(a, b)", "a != b"),
    ("nextpow2", "nextpow2(n)", "exponent of the next power of 2 ≥ |n|"),
    ("nnz", "nnz(x)", "number of nonzero elements"),
    ("norm", "norm(A[, p])", "vector or matrix norm (2, 1, Inf, 'fro')"),
    ("normcdf", "normcdf(x[, mu, sigma])", "normal cumulative distribution"),
    ("norminv", "norminv(p[, mu, sigma])", "inverse normal CDF (quantile)"),
    ("normpdf", "normpdf(x[, mu, sigma])", "normal probability density"),
    ("not", "not(a)", "logical not !a"),
    ("nthroot", "nthroot(x, n)", "real n-th root"),
    ("null", "null(A)", "orthonormal basis of the null space"),
    ("num2cell", "num2cell(A)", "cell array with one element per cell"),
    ("num2str", "num2str(x[, prec | fmt])", "number to string"),
    ("numel", "numel(x)", "number of elements"),
    ("ode15s", "[t, y] = ode15s(f, tspan, y0)", "solve the ODE y' = f(t, y) (ode45's solver here)"),
    ("ode23", "[t, y] = ode23(f, tspan, y0)", "solve the ODE y' = f(t, y) (Runge-Kutta 2/3)"),
    ("ode23s", "[t, y] = ode23s(f, tspan, y0)", "solve the ODE y' = f(t, y) (ode45's solver here)"),
    ("ode45", "[t, y] = ode45(f, tspan, y0)", "solve the ODE y' = f(t, y) (Runge-Kutta 4/5)"),
    ("odeget", "odeget(opts, name[, default])", "read one ODE option"),
    ("odeset", "odeset('Name', value, ...)", "ODE options struct (RelTol, AbsTol, MaxStep …)"),
    ("ones", "ones(n[, m])", "matrix of ones"),
    ("or", "or(a, b)", "logical or a | b"),
    ("orth", "orth(A)", "orthonormal basis of the range"),
    ("pcolor", "pcolor([X, Y, ]Z)", "pseudocolor plot"),
    ("peaks", "peaks([n])", "sample surface (plots it, or returns Z)"),
    ("physconst", "physconst(name)", "physical constant by name ('LightSpeed', 'Planck', …)"),
    ("pi", "pi[(n, m)]", "π = 3.14159…; with sizes, a matrix of π"),
    ("pinv", "pinv(A)", "Moore-Penrose pseudoinverse"),
    ("pkg", "pkg load NAME", "accepted and ignored (everything is built in)"),
    ("plot", "plot([x, ]y[, fmt], ...)", "2-D line plot"),
    ("plot3", "plot3(x, y, z[, fmt])", "3-D line plot"),
    ("plus", "plus(a, b)", "a + b"),
    ("poisspdf", "poisspdf(k, lambda)", "Poisson probability P(X = k)"),
    ("polar", "polar(theta, rho[, fmt])", "polar plot"),
    ("polarplot", "polarplot(theta, rho[, fmt])", "polar plot"),
    ("poly", "poly(r)", "polynomial with roots r, or characteristic polynomial"),
    ("polyder", "polyder(p)", "derivative of polynomial p"),
    ("polyfit", "polyfit(x, y, n)", "least-squares polynomial of degree n"),
    ("polyint", "polyint(p[, k])", "antiderivative of polynomial p (constant k)"),
    ("polyval", "polyval(p, x)", "evaluate polynomial p at x"),
    ("power", "power(x, y)", "element-wise power x.^y"),
    ("prctile", "prctile(x, p[, dim])", "percentiles at p (0..100)"),
    ("pretty", "pretty(expr)", "expression in math notation (x^2 → x²)"),
    ("primes", "primes(n)", "primes up to n"),
    ("print", "print('file.png')", "save the current figure (png or svg)"),
    ("printf", "printf(fmt, ...)", "print formatted text"),
    ("prod", "prod(x[, dim])", "product of elements"),
    ("pyaddons", "pyaddons()", "names of the loaded Python addon functions"),
    ("pyeval", "pyeval(expr)", "value of a Python expression"),
    ("pyexec", "pyexec(code)", "run Python statements"),
    ("pyrun", "pyrun(file)", "run a Python file"),
    ("qalc", "qalc(expr)", "evaluate with the calculator (libqalculate)"),
    ("qalc2oct", "qalc2oct(expr)", "calculator result as Octave code"),
    ("qalcfn", "qalcfn(expr)", "calculator result as a function handle"),
    ("qr", "[Q, R] = qr(A)", "QR decomposition"),
    ("quad", "quad(f, a, b)", "definite integral of f from a to b (adaptive)"),
    ("quadgk", "quadgk(f, a, b)", "definite integral of f from a to b (Gauss-Kronrod)"),
    ("quadl", "quadl(f, a, b)", "definite integral of f from a to b (adaptive)"),
    ("quantile", "quantile(x, p[, dim])", "quantiles at probabilities p"),
    ("quiver", "quiver([x, y, ]u, v)", "vector field arrows"),
    ("rad2deg", "rad2deg(r)", "radians to degrees"),
    ("rand", "rand([n[, m]])", "uniform random numbers in (0, 1)"),
    ("randi", "randi(imax[, n[, m]])", "random integers in 1..imax (or [lo hi])"),
    ("randn", "randn([n[, m]])", "normal random numbers (mean 0, std 1)"),
    ("range", "range(x[, dim])", "max - min"),
    ("rank", "rank(A)", "matrix rank"),
    ("rdivide", "rdivide(a, b)", "element-wise division a ./ b"),
    ("readmatrix", "readmatrix(file[, 'Sheet', s])", "matrix from csv, tsv, txt, xlsx, xls or ods"),
    ("readtable", "readtable(file)", "struct of named columns from a file"),
    ("real", "real(z)", "real part"),
    ("reallog", "reallog(x)", "natural logarithm, error for negative x"),
    ("realmax", "realmax", "largest finite double"),
    ("realmin", "realmin", "smallest normalized positive double"),
    ("realpow", "realpow(x, y)", "x.^y, error if the result is complex"),
    ("realsqrt", "realsqrt(x)", "square root, error for negative x"),
    ("rem", "rem(x, y)", "remainder, result has the sign of x"),
    ("repmat", "repmat(A, m[, n])", "tile A m-by-n times"),
    ("reshape", "reshape(A, m, n)", "same elements in an m×n shape"),
    ("rethrow", "rethrow(err)", "raise a caught error struct again"),
    ("riemann", "riemann(f, a, b, n[, 'left'|'right'|'mid'|'trap'])", "Riemann sum strips against the integral"),
    ("rmfield", "rmfield(s, name)", "struct without field name"),
    ("rng", "rng(seed)", "seed the random number generator"),
    ("roots", "roots(p)", "roots of polynomial p"),
    ("rot90", "rot90(A[, k])", "rotate a matrix by k·90° counterclockwise"),
    ("round", "round(x)", "round to the nearest integer"),
    ("rows", "rows(x)", "number of rows"),
    ("rref", "rref(A)", "reduced row echelon form"),
    ("saveas", "saveas(h, 'file.png')", "save the current figure (png or svg)"),
    ("scatter", "scatter(x, y[, s[, c]])", "scatter plot"),
    ("scatter3", "scatter3(x, y, z[, s[, c]])", "3-D scatter plot"),
    ("schur", "[U, S] = schur(A)", "Schur decomposition"),
    ("sec", "sec(x)", "secant 1/cos(x)"),
    ("semilogx", "semilogx([x, ]y[, fmt])", "plot with a logarithmic x axis"),
    ("semilogy", "semilogy([x, ]y[, fmt])", "plot with a logarithmic y axis"),
    ("shg", "shg", "accepted; figures show by themselves"),
    ("sign", "sign(x)", "signum: -1, 0 or 1"),
    ("sin", "sin(x)", "sine (radians)"),
    ("sind", "sind(x)", "sine of x in degrees"),
    ("sinh", "sinh(x)", "hyperbolic sine"),
    ("size", "size(x[, dim])", "[rows, cols], or the size along dim"),
    ("skewness", "skewness(x)", "sample skewness"),
    ("slopefield", "slopefield(f, [x0 x1], [y0 y1][, y0s])", "direction field of y' = f(x, y), with solutions"),
    ("sort", "[y, i] = sort(x[, dim][, 'descend'])", "sort ascending (or descending), with indices"),
    ("sphere", "sphere([n])", "unit sphere surface"),
    ("sprintf", "sprintf(fmt, ...)", "formatted string"),
    ("sqrt", "sqrt(x)", "square root"),
    ("sqrtm", "sqrtm(A)", "matrix square root"),
    ("stairs", "stairs([x, ]y)", "staircase plot"),
    ("std", "std(x[, opt[, dim]])", "standard deviation"),
    ("stem", "stem([x, ]y)", "stem plot of discrete data"),
    ("str2double", "str2double(s)", "string to number (NaN if invalid)"),
    ("str2func", "str2func(s)", "function handle from a name or '@(x) …'"),
    ("str2num", "str2num(s)", "evaluate a string as a number or matrix"),
    ("strcat", "strcat(s1, s2, ...)", "concatenate strings"),
    ("strcmp", "strcmp(a, b)", "true if the strings are equal"),
    ("strcmpi", "strcmpi(a, b)", "true if the strings are equal ignoring case"),
    ("strfind", "strfind(s, pat)", "start indices of pat in s"),
    ("strjoin", "strjoin(c[, delim])", "join a cell of strings"),
    ("strrep", "strrep(s, from, to)", "replace every occurrence of a substring"),
    ("strsplit", "strsplit(s[, delim])", "split a string into a cell"),
    ("strtrim", "strtrim(s)", "remove leading and trailing whitespace"),
    ("struct", "struct('field', value, ...)", "struct from field/value pairs"),
    ("subplot", "subplot(r, c, i)", "select tile i of an r×c grid"),
    ("sum", "sum(x[, dim])", "sum of elements (per column for matrices)"),
    ("surf", "surf([X, Y, ]Z)", "shaded 3-D surface"),
    ("svd", "[U, S, V] = svd(A)", "singular value decomposition"),
    ("tan", "tan(x)", "tangent (radians)"),
    ("tand", "tand(x)", "tangent of x in degrees"),
    ("tangentplot", "tangentplot(f, x0[, [a b]])", "f with its tangent line at x0"),
    ("tanh", "tanh(x)", "hyperbolic tangent"),
    ("text", "text(x, y, str)", "text at data coordinates"),
    ("tic", "tic", "start the stopwatch timer"),
    ("times", "times(a, b)", "element-wise product a .* b"),
    ("title", "title(str)", "axes title"),
    ("toc", "toc[(id)]", "seconds since tic"),
    ("tolower", "tolower(s)", "lowercase"),
    ("toupper", "toupper(s)", "uppercase"),
    ("trace", "trace(A)", "sum of the diagonal"),
    ("transpose", "transpose(A)", "transpose A.' (no conjugation)"),
    ("trapz", "trapz([x, ]y)", "trapezoidal integral of samples"),
    ("tril", "tril(A[, k])", "lower triangular part"),
    ("triplequad", "triplequad(f, xa, xb, ya, yb, za, zb)", "triple integral of f(x, y, z)"),
    ("triu", "triu(A[, k])", "upper triangular part"),
    ("true", "true[(n, m)]", "logical 1, or a matrix of them"),
    ("truthtable", "truthtable(expr)", "truth table of a logic expression"),
    ("uminus", "uminus(a)", "negation -a"),
    ("unique", "[u, i, j] = unique(x)", "sorted unique values"),
    ("unitconv", "unitconv(x, from, to)", "convert units ('km', 'mi', 'psi', 'C', 'F', …)"),
    ("unwrap", "unwrap(p)", "remove 2π jumps from phase angles"),
    ("upper", "upper(s)", "uppercase"),
    ("vander", "vander(c[, n])", "Vandermonde matrix"),
    ("var", "var(x[, opt[, dim]])", "variance"),
    ("vecnorm", "vecnorm(A[, p[, dim]])", "norm of each column"),
    ("vertcat", "vertcat(A, B, ...)", "stack vertically, [A; B]"),
    ("view", "view(az, el)", "3-D view angles in degrees; view(2), view(3)"),
    ("warning", "warning([id, ]fmt, ...)", "print a formatted warning"),
    ("who", "who", "list workspace variable names"),
    ("whos", "whos", "list workspace variables with size and class"),
    ("writematrix", "writematrix(M, file)", "write a matrix to a file (format from extension)"),
    ("xlabel", "xlabel(str)", "x-axis label"),
    ("xlim", "xlim([lo hi])", "x-axis limits (no argument: current ones)"),
    ("xlsread", "xlsread(file[, sheet])", "numeric data from a spreadsheet"),
    ("xor", "xor(a, b)", "logical exclusive or"),
    ("ylabel", "ylabel(str)", "y-axis label"),
    ("ylim", "ylim([lo hi])", "y-axis limits (no argument: current ones)"),
    ("zeros", "zeros(n[, m])", "matrix of zeros"),
    ("zlabel", "zlabel(str)", "z-axis label"),
    ("zlim", "zlim([lo hi])", "z-axis limits (no argument: current ones)"),
    ("zscore", "zscore(x)", "standardize: (x - mean) / std"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Top-level `"name" | "alias" =>` arms of every `pub fn lookup(` / `pub fn builtin(` in `src`.
    fn arm_names(src: &str) -> Vec<&str> {
        let (mut names, mut inside) = (vec![], false);
        for line in src.lines() {
            if line.starts_with("pub fn lookup(") || line.starts_with("pub fn builtin(") {
                inside = true;
            } else if line == "}" {
                inside = false;
            } else if let Some(arm) = line.strip_prefix("        ").filter(|r| inside && (r.starts_with('"') || r.starts_with("| \""))) {
                let pat = arm.split("=>").next().unwrap_or("");
                names.extend(pat.split('|').map(|p| p.trim().trim_matches('"')).filter(|p| !p.is_empty()));
            }
        }
        names
    }

    #[test]
    fn every_builtin_documented() {
        let srcs = [
            include_str!("lang/fns.rs"),
            include_str!("builtins/math.rs"),
            include_str!("builtins/stats.rs"),
            include_str!("builtins/linalg.rs"),
            include_str!("builtins/numeric.rs"),
            include_str!("builtins/fft.rs"),
            include_str!("builtins/strings.rs"),
            include_str!("builtins/calculus.rs"),
            include_str!("builtins/ops.rs"),
            include_str!("builtins/bits.rs"),
            include_str!("builtins/io.rs"),
            include_str!("plot/builtins.rs"),
            include_str!("plot/calculus.rs"),
            include_str!("qalc/mod.rs"),
            include_str!("py.rs"),
            include_str!("doc.rs"),
        ];
        let names: Vec<&str> = srcs.iter().flat_map(|s| arm_names(s)).collect();
        assert!(names.len() > 380, "arm scan broke: {} names", names.len());
        let missing: Vec<&&str> = names.iter().filter(|n| doc(n).is_none()).collect();
        assert!(missing.is_empty(), "undocumented builtins: {missing:?}");
        assert!(arm_names("pub fn lookup(n: &str) {\n        \"a\" | \"b\"\n        | \"c\" => 1,\n            \"opt\" => 2,\n}\n") == ["a", "b", "c"]);
    }

    #[test]
    fn table_sorted_and_short() {
        assert!(DOCS.windows(2).all(|w| w[0].0 < w[1].0), "DOCS must be sorted without duplicates");
        for (name, sig, desc) in DOCS.iter().chain(KEYWORDS) {
            assert!(sig.contains(name) && !desc.is_empty() && desc.chars().count() <= 60, "{name}");
        }
        assert_eq!(doc("linspace").unwrap().1, "linspace(a, b[, n])");
        assert!(doc("nope").is_none());
        // every name in the overview is documented
        for w in OVERVIEW.lines().filter(|l| l.starts_with("  ")).flat_map(|l| l[15..].split_whitespace()) {
            assert!(doc(w).is_some(), "overview: {w}");
        }
    }

    #[test]
    fn help_and_lookfor() {
        let mut it = Interp::new();
        let run = |it: &mut Interp, f: &str, a: &[&str]| {
            let a: Vec<Value> = a.iter().map(|s| Value::Str(s.to_string())).collect();
            builtin(f).unwrap()(it, &a, 0).map(|_| it.take_output())
        };
        assert!(run(&mut it, "help", &[]).unwrap().contains("lookfor WORD"));
        assert_eq!(run(&mut it, "help", &["linspace"]).unwrap(), "linspace(a, b[, n])\n    n evenly spaced points from a to b (default 100)\n");
        assert!(run(&mut it, "help", &["for"]).unwrap().contains("loop over"));
        assert!(run(&mut it, "help", &["lnspace"]).unwrap().contains("did you mean linspace"));
        assert!(run(&mut it, "help", &["zzqqxx"]).unwrap().contains("not found"));
        assert!(run(&mut it, "help", &["ü→"]).unwrap().contains("not found"));
        let l = run(&mut it, "lookfor", &["Fourier"]).unwrap();
        assert!(l.contains("fft ") && l.contains("ifft2"), "{l}");
        assert!(run(&mut it, "lookfor", &["qqqzz"]).unwrap().contains("nothing found"));
        assert!(run(&mut it, "lookfor", &[]).is_err());
        let s = builtin("help").unwrap()(&mut it, &[Value::Str("sin".into())], 1).unwrap();
        assert!(matches!(&s[..], [Value::Str(t)] if t.starts_with("sin(x)")));
        assert!(builtin("sin").is_none());
    }
}
