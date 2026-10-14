use super::Interp;
use crate::value::Value;

/// Run a snippet, return its output plus "error: ..." if it failed.
fn run(src: &str) -> String {
    let mut it = Interp::new();
    let r = it.eval(src);
    let mut out = it.take_output();
    if let Err(e) = r {
        out += &format!("error: {e}\n");
    }
    out
}

fn mat(rows: &[&str]) -> String {
    format!("ans =\n\n{}\n", rows.iter().map(|r| format!("{r}\n")).collect::<String>())
}

#[test]
fn precedence() {
    assert_eq!(run("-2^2"), "ans = -4\n");
    assert_eq!(run("2^-1"), "ans = 0.5000\n");
    assert_eq!(run("1 + 2 * 3"), "ans = 7\n");
    assert_eq!(run("a = 2; -a'"), "ans = -2\n");
    assert_eq!(run("!0 == 1"), "ans = 1\n");
    assert_eq!(run("1 | 0 & 0"), "ans = 1\n");
    assert_eq!(run("1:3 + 1"), mat(&["   1   2   3   4"]));
    assert_eq!(run("[1 2 3]'"), mat(&["   1", "   2", "   3"]));
    assert_eq!(run("2 * 3 ^ 2"), "ans = 18\n");
    assert_eq!(run("0 && undefined_fn()"), "ans = 0\n");
    assert_eq!(run("1 || undefined_fn()"), "ans = 1\n");
    assert_eq!(run("3 + 4; ans * 2"), "ans = 14\n");
}

#[test]
fn ranges() {
    assert_eq!(run("x = 0:0.1:1; k = 0; for v = x, k = k + 1; end, k"), "k = 11\n");
    assert_eq!(run("x = 0:0.1:1; x(end)"), "ans = 1\n");
    assert_eq!(run("5:-2:1"), mat(&["   5   3   1"]));
    assert_eq!(run("x = 3:1"), "x = [](1x0)\n");
    assert_eq!(run("0:0.25:0.5"), mat(&["        0   0.2500   0.5000"]));
}

#[test]
fn concatenation() {
    assert_eq!(run("[1 -2]"), mat(&["   1  -2"]));
    assert_eq!(run("[1 - 2]"), "ans = -1\n");
    assert_eq!(run("a = [1 2]; [a' a']"), mat(&["   1   1", "   2   2"]));
    assert_eq!(run("x = 5; [x (1)]"), mat(&["   5   1"]));
    assert_eq!(run("['ab', 'cd']"), "ans = abcd\n");
    assert_eq!(run("['ab' 'cd']"), "ans = abcd\n");
    assert_eq!(run("x = ['a' 66]"), "x = aB\n");
    assert_eq!(run("[1 2; 3 4]"), mat(&["   1   2", "   3   4"]));
    assert_eq!(run("[[1 2]; [3 4]]"), mat(&["   1   2", "   3   4"]));
    assert_eq!(run("[1, 2\n 3, 4]"), mat(&["   1   2", "   3   4"]));
    assert_eq!(run("[]"), "ans = [](0x0)\n");
    assert_eq!(run("[[], 1, []]"), "ans = 1\n");
    assert!(run("[1 2; 3]").contains("vertical dimensions mismatch (1x2 vs 1x1)"));
    assert_eq!(run("[1 .5]"), mat(&["   1.0000   0.5000"]));
}

#[test]
fn indexing() {
    let a = "A = [1 2 3; 4 5 6];";
    assert_eq!(run(&format!("{a} A(2, 3)")), "ans = 6\n");
    assert_eq!(run(&format!("{a} A(end, :)")), mat(&["   4   5   6"]));
    assert_eq!(run(&format!("{a} A(:, 2)'")), mat(&["   2   5"]));
    assert_eq!(run(&format!("{a} A(:)'")), mat(&["   1   4   2   5   3   6"]));
    assert_eq!(run(&format!("{a} A(end)")), "ans = 6\n");
    assert_eq!(run(&format!("{a} A([1 2], [1 3])")), mat(&["   1   3", "   4   6"]));
    assert_eq!(run(&format!("{a} A(A > 4)'")), mat(&["   5   6"]));
    let x = "x = 10:10:50;";
    assert_eq!(run(&format!("{x} x([1 end])")), mat(&["   10   50"]));
    assert_eq!(run(&format!("{x} x(end-1)")), "ans = 40\n");
    assert_eq!(run(&format!("{x} x(x > 25)")), mat(&["   30   40   50"]));
    assert_eq!(run(&format!("{x} y = [2 4]; x(y(end))")), "ans = 40\n");
    assert_eq!(run(&format!("{x} x([1; 2])")), mat(&["   10   20"]));
    assert_eq!(run("v = [3 -1]; v(:)"), mat(&["   3", "  -1"]));
    assert!(run(&format!("{x} x(0)")).contains("index (0): out of bound; value 0 out of bound 5"));
    assert!(run(&format!("{x} x(6)")).contains("index (6): out of bound 5"));
    assert!(run(&format!("{x} x(1.5)")).contains("subscripts must be"));
    assert_eq!(run("s = 'hello'; s(end:-1:1)"), "ans = olleh\n");
    assert_eq!(run("s = 'hello'; s([1 1])"), "ans = hh\n");
    assert_eq!(run("c = [1 2; 3 4]; c(2, :)(1)"), "ans = 3\n");
}

#[test]
fn assignment() {
    assert_eq!(run("x = []; x(3) = 1"), "x =\n\n   0   0   1\n\n");
    assert_eq!(run("x = 1:3; x(end+1) = 4; x"), "x =\n\n   1   2   3   4\n\n");
    assert_eq!(run("A = [1 2; 3 4]; A(3, 3) = 9"), "A =\n\n   1   2   0\n   3   4   0\n   0   0   9\n\n");
    assert_eq!(run("x = 1:5; x(2) = []; x"), "x =\n\n   1   3   4   5\n\n");
    assert_eq!(run("x = 1:5; x([1 end]) = []; x"), "x =\n\n   2   3   4\n\n");
    assert_eq!(run("A = [1 2; 3 4]; A(1, :) = []"), "A =\n\n   3   4\n\n");
    assert_eq!(run("A = [1 2; 3 4]; A(:, 1) = []"), "A =\n\n   2\n   4\n\n");
    assert_eq!(run("y = [-1 2 -3]; y(y < 0) = 0"), "y =\n\n   0   2   0\n\n");
    assert_eq!(run("x = 1:3; x(:) = 7"), "x =\n\n   7   7   7\n\n");
    assert_eq!(run("z(2, 3) = 1"), "z =\n\n   0   0   0\n   0   0   1\n\n");
    assert_eq!(run("v(3) = 2;\nv"), "v =\n\n   0   0   2\n\n");
    assert_eq!(run("c = (1:3)'; c(5) = 1; c'"), mat(&["   1   2   3   0   1"]));
    assert!(run("x = 1:3; x([1 2]) = [1 2 3]").contains("nonconformant"));
    assert!(run("A = [1 2; 3 4]; A(1, 1) = []").contains("null assignment"));
    assert_eq!(run("s = 'abc'; s(2) = 'X'"), "s = aXc\n");
    assert_eq!(run("x = 5; x(2, 2) = 1; x"), "x =\n\n   5   0\n   0   1\n\n");
    assert_eq!(run("q(2).a = 1"), "error: struct arrays are not supported near line 1\n");
}

#[test]
fn control_flow() {
    assert_eq!(run("for i = 1:3, disp(i), end"), "1\n2\n3\n");
    assert_eq!(run("for col = [1 2; 3 4], disp(col'), end"), "   1   3\n   2   4\n");
    assert_eq!(run("for c = {1, 'a'}, disp(class(c)), end"), "cell\ncell\n");
    assert_eq!(run("for i = [], disp(i), end"), "");
    assert_eq!(run("k = 0; while k < 5, k = k + 1; if k == 3, break; end, end, k"), "k = 3\n");
    assert_eq!(run("s = 0; for i = 1:5, if i == 2, continue, end, s = s + i; end, s"), "s = 13\n");
    assert_eq!(run("i = 0; do i = i + 1; until i >= 3\ni"), "i = 3\n");
    let ifs = "if x > 10, disp('big'), elseif x > 3, disp('mid'), else, disp('small'), end";
    assert_eq!(run(&format!("x = 5; {ifs}")), "mid\n");
    assert_eq!(run(&format!("x = 50; {ifs}")), "big\n");
    assert_eq!(run(&format!("x = 1; {ifs}")), "small\n");
    assert_eq!(run("switch 'abc'\n case {'x', 'abc'}\n disp('hit')\n otherwise\n disp('miss')\nend"), "hit\n");
    assert_eq!(run("switch 3, case 1, disp('one'), case 3, disp('three'), end"), "three\n");
    assert_eq!(run("switch 'q', case 'a', disp(1), otherwise, disp(2), endswitch"), "2\n");
    assert_eq!(run("try, error('boom'), catch err, disp(err.message), end"), "boom\n");
    assert_eq!(run("try\n error('My:id', 'msg')\ncatch e\n disp(e.message)\nend_try_catch"), "msg\n");
    assert_eq!(run("try, x = [1 2](3); catch e, disp(e.message), end"), "index (3): out of bound 2 (dimensions are 1x2)\n");
    assert_eq!(run("try, undefined_thing; catch, disp('caught'), end"), "caught\n");
    assert_eq!(run("x = 1; if x, y = 2, endif"), "y = 2\n");
    assert_eq!(run("for k = 1:3\n  for j = 1:3\n    if j == 2, break, end\n  end\nend\n[k j]"), mat(&["   3   2"]));
}

#[test]
fn functions() {
    let sp = "function [s, p] = sp(a, b)\n  s = a + b;\n  p = a * b;\nend\n";
    assert_eq!(run(&format!("{sp}[x, y] = sp(2, 3)")), "x = 5\ny = 6\n");
    assert_eq!(run(&format!("{sp}sp(2, 3)")), "ans = 5\n");
    assert_eq!(run(&format!("{sp}[~, q] = sp(2, 3)")), "q = 6\n");
    assert!(run(&format!("{sp}sp(1, 2, 3)")).contains("sp: function called with too many inputs"));
    assert!(run(&format!("{sp}[a, b, c] = sp(1, 2)")).contains("too many outputs"));
    let f = "function r = f(a, b)\n  if nargin < 2\n    b = 10;\n  end\n  r = a + b;\nendfunction\n";
    assert_eq!(run(&format!("{f}disp(f(1)); disp(f(1, 2))")), "11\n3\n");
    let fact = "function r = fact(n)\n  if n <= 1\n    r = 1;\n  else\n    r = n * fact(n - 1);\n  end\nend\n";
    assert_eq!(run(&format!("{fact}fact(5)")), "ans = 120\n");
    // debug builds use ~10 KB of stack per call level; test threads only get 2 MB
    let deep = std::thread::Builder::new().stack_size(32 << 20).spawn(|| run("function r = deep(n)\n r = deep(n + 1);\nend\ndeep(1)"));
    assert!(deep.unwrap().join().unwrap().contains("max_recursion_depth exceeded"));
    // script-ending function without `end`, called before its definition
    assert_eq!(run("x = g(2)\nfunction r = g(a)\n  r = a * 2;\n"), "x = 4\n");
    assert_eq!(run("function r = h(x)\n r = 1;\n if x, return, end\n r = 2;\nend\nh(1)"), "ans = 1\n");
    assert_eq!(run("function n = cnt(varargin)\n n = 0;\n for k = 1:nargin, n = n + varargin{k}; end\nend\ncnt(1, 2, 3)"), "ans = 6\n");
    assert_eq!(run("function show(x)\n disp(x)\nend\nshow(7)"), "7\n");
    assert_eq!(run("function noret()\nend\nnoret\nx = 1"), "x = 1\n");
    assert!(run("function r = nothing()\nend\ny = nothing()").contains("output 'r' undefined"));
    assert_eq!(run("function varargout = vo()\n varargout{1} = 1; varargout{2} = 2;\nend\n[a, b] = vo();\n[a b]"), mat(&["   1   2"]));
    // function scope doesn't see the caller's variables
    assert!(run("function r = leak()\n r = secret;\nend\nsecret = 1; leak()").contains("'secret' undefined"));
}

#[test]
fn anonymous_functions() {
    assert_eq!(run("a = 2; f = @(x) a * x; a = 100; f(3)"), "ans = 6\n");
    assert_eq!(run("f = @(x, y) x + y; f(1, 2)"), "ans = 3\n");
    assert_eq!(run("g = @() 42; g()"), "ans = 42\n");
    let sq = "function r = sq(x)\n r = x^2;\nend\n";
    assert_eq!(run(&format!("{sq}h = @sq; h(4)")), "ans = 16\n");
    assert_eq!(run(&format!("{sq}feval(@sq, 3)")), "ans = 9\n");
    assert_eq!(run(&format!("{sq}feval('sq', 3)")), "ans = 9\n");
    assert_eq!(run("f = @(x) x.^2"), "f =\n\n@(x) x.^2\n\n");
    assert_eq!(run("f = @(x) x.^2; func2str(f)"), "ans = @(x) x.^2\n");
    assert_eq!(run("h = @sin"), "h = @sin\n");
    assert_eq!(run("g = str2func('@(x) x + 1'); g(2)"), "ans = 3\n");
    assert_eq!(run("c = {@(x) x + 1, @(x) x * 2}; f2 = c{2}; f2(5)"), "ans = 10\n");
    assert_eq!(run("k = 3; mk = @(a) @(b) a + b + k; add = mk(1); add(10)"), "ans = 14\n");
    assert_eq!(run("s.f = @(x) x + 1; s.f(1)"), "ans = 2\n");
    assert_eq!(run("f = @(x) x(end); f([4 5 6])"), "ans = 6\n");
    assert!(run("f = @(x) x; f(1, 2)").contains("too many inputs"));

    let mut it = Interp::new();
    let r = it.call(&Value::from("x^2 + y"), &[Value::num(2.), Value::num(3.)], 1).unwrap();
    assert_eq!(r[0].scalar().unwrap(), 7.0);
    let r = it.call(&Value::from("t^2"), &[Value::from(vec![1., 2., 3.])], 1).unwrap();
    assert_eq!(r[0].matrix().unwrap().data, vec![1., 4., 9.]);
    it.eval("f = @(x) 2 * x;").unwrap();
    let f = it.vars["f"].clone();
    assert_eq!(it.call(&f, &[Value::num(4.)], 1).unwrap()[0].scalar().unwrap(), 8.0);
}

#[test]
fn cells() {
    let c = "c = {1, 'two', [3 4]};";
    assert_eq!(run(&format!("{c} class(c)")), "ans = cell\n");
    assert_eq!(run(&format!("{c} c{{2}}")), "ans = two\n");
    assert_eq!(run(&format!("{c} x = c(2); class(x)")), "ans = cell\n");
    assert_eq!(run(&format!("{c} c{{end+1}} = 5; c{{4}}")), "ans = 5\n");
    assert_eq!(run(&format!("{c} c(2) = []; c{{2}}")), mat(&["   3   4"]));
    assert_eq!(run(&format!("{c} [a, b] = c{{1:2}}")), "a = 1\nb = two\n");
    assert_eq!(run(&format!("{c} c{{3}}(2)")), "ans = 4\n");
    assert_eq!(run("c = {1, 'abc'}"), "c =\n{\n  [1,1] = 1\n  [1,2] = abc\n}\n\n");
    assert_eq!(run("c = {}"), "c = {}(0x0)\n");
    assert_eq!(run("q{2} = 'x'; class(q{1})"), "ans = double\n");
    assert_eq!(run("c = {1, 2}; d = [c, {3}]; d{3}"), "ans = 3\n");
    assert_eq!(run("c = {[1 2]}; c{1}(end+1) = 3; c{1}"), mat(&["   1   2   3"]));
}

#[test]
fn structs() {
    assert_eq!(run("s.a = 1; s.b.c = 'x'; s.b.c"), "ans = x\n");
    let s = "s = struct('name', 'bob', 'age', 42);";
    assert_eq!(run(&format!("{s} s.age")), "ans = 42\n");
    assert_eq!(run(&format!("{s} isfield(s, 'name')")), "ans = 1\n");
    assert_eq!(run(&format!("{s} s.('age')")), "ans = 42\n");
    assert_eq!(run(&format!("{s} s = rmfield(s, 'age'); isfield(s, 'age')")), "ans = 0\n");
    assert_eq!(run(&format!("{s} f = fieldnames(s); f{{1}}")), "ans = age\n");
    assert_eq!(run("t.x = 1"), "t =\n\n  scalar structure containing the fields:\n\n    x = 1\n\n");
    assert_eq!(run("s.v(3) = 7; s.v"), mat(&["   0   0   7"]));
    assert_eq!(run("n = 'k'; s.(n) = 5; s.k"), "ans = 5\n");
    assert!(run("s.a = 1; s.zz").contains("invalid use of undefined value"));
    assert!(run("x = 1; x.f").contains("cannot be indexed with ."));
}

#[test]
fn command_syntax_and_format() {
    assert_eq!(run("format long\npi\nformat short\npi"), "ans = 3.141592653589793\nans = 3.1416\n");
    assert_eq!(run("format long; x = 0.1\nformat"), "x = 0.100000000000000\n");
    assert_eq!(run("x = 1; clear x; exist('x')"), "ans = 0\n");
    assert_eq!(run("pkg load statistics"), "");
    assert_eq!(run("disp hello"), "hello\n");
    assert_eq!(run("disp 'a b'"), "a b\n");
    assert_eq!(run("x = 3; x"), "x = 3\n");
}

#[test]
fn display_formatting() {
    assert_eq!(run("x = 5"), "x = 5\n");
    assert_eq!(run("x = pi"), "x = 3.1416\n");
    assert_eq!(run("x = -0.5"), "x = -0.5000\n");
    assert_eq!(run("x = 1e10"), "x = 1.0000e+10\n");
    assert_eq!(run("s = 'hello'"), "s = hello\n");
    assert_eq!(run("b = true"), "b = 1\n");
    assert_eq!(run("e = []"), "e = [](0x0)\n");
    assert_eq!(run("A = [1.5 2.25]"), "A =\n\n   1.5000   2.2500\n\n");
    assert_eq!(run("L = [1 2] > 1"), "L =\n\n  0  1\n\n");
    assert_eq!(run("x = NaN"), "x = NaN\n");
    assert_eq!(run("x = -Inf"), "x = -Inf\n");
    assert_eq!(run("x = [1 -2; 30 4]"), "x =\n\n    1   -2\n   30    4\n\n");
    assert_eq!(run("NaN(1, 2)"), mat(&["   NaN   NaN"]));
    assert_eq!(run("true(2)"), mat(&["  1  1", "  1  1"]));
    assert_eq!(run("x = 'it''s'"), "x = it's\n");
    assert_eq!(run("disp(\"a\\tb\")"), "a\tb\n");
    assert_eq!(run("class(1 == 1)"), "ans = logical\n");
    assert_eq!(run("isa(1, 'numeric')"), "ans = 1\n");
}

#[test]
fn syntax_and_errors() {
    assert_eq!(run("x = 1 % comment\n# another\n%{\nblock\n%}\ny = 2"), "x = 1\ny = 2\n");
    assert_eq!(run("x = 1 + ...\n 2"), "x = 3\n");
    assert_eq!(run("x = [1, 2, ... comment\n 3]"), "x =\n\n   1   2   3\n\n");
    assert_eq!(run("x = 1;\ny = undefined_thing + 1;"), "error: 'undefined_thing' undefined near line 2\n");
    assert!(run("x = (1 + ").starts_with("error: parse error near line 1"));
    assert!(run("if 1\n x = 1;\n").contains("missing"));
    assert_eq!(run("x = 1, y = 2;"), "x = 1\n");
    assert_eq!(run("eval('x = 3;'); x"), "x = 3\n");
    assert_eq!(run("eval('error(''bad'')', 'disp(''caught'')')"), "caught\n");
    assert_eq!(run("y = eval('1 + 2')"), "y = 3\n");
    assert!(run("error('Octave:some-id', 'custom')").contains("error: custom"));
    assert_eq!(run("warning('careful')"), "warning: careful\n");
    assert_eq!(run("x = 3; exist('x')"), "ans = 1\n");
    assert_eq!(run("isvarname('a1')"), "ans = 1\n");
    assert_eq!(run("isvarname('1a')"), "ans = 0\n");
    assert_eq!(run("a = 1; b = 2; who"), "Variables visible from the current scope:\n\na  b\n\n");
}

#[test]
fn more_syntax() {
    assert_eq!(run("x = 5; x += 2"), "x = 7\n");
    assert_eq!(run("x = [1 2]; x(2) *= 10; x"), "x =\n\n    1   20\n\n");
    assert_eq!(run("if 0, disp a, else disp b, end"), "b\n");
    assert_eq!(run("function r = five\n r = 5;\nend\nfive + 1"), "ans = 6\n");
    assert_eq!(run("c = {1, [2 3]}; [c{:}]"), mat(&["   1   2   3"]));
    assert_eq!(run("x = []; for i = 1:1000, x(end+1) = i; end, x(end)"), "ans = 1000\n");
    assert_eq!(run("who"), "");
    assert_eq!(run("b = [true false true]; b(2) = []; class(b)"), "ans = logical\n");
    assert_eq!(run("s = ''; s(end+1) = 'a'; s(end+1) = 'b'"), "s = ab\n");
}

#[test]
fn complex_literals_and_ops() {
    assert_eq!(run("z = 3 + 4i"), "z = 3 + 4i\n");
    assert_eq!(run("z = 2.5e-3i + 3I + 2J + 1j"), "z = 0 + 6.0025i\n");
    assert_eq!(run("1i * 1i == -1"), "ans = 1\n");
    assert_eq!(run("class(1i * 1i), iscomplex(1i * 1i)"), "ans = double\nans = 0\n");
    assert_eq!(run("(-8)^(1/3)"), "ans = 1.0000 + 1.7321i\n");
    assert_eq!(run("(-1)^0.5"), "ans = 0 + 1i\n");
    assert_eq!(run("[-4 4] .^ 0.5"), mat(&["   0 + 2i   2 + 0i"]));
    assert_eq!(run("[1 2i]'"), mat(&["   1 - 0i", "   0 - 2i"]));
    assert_eq!(run("[1 2i].'"), mat(&["   1 + 0i", "   0 + 2i"]));
    assert_eq!(run("[1+1i 2] * [1; 1i]"), "ans = 1 + 3i\n");
    assert_eq!(run("1i < 2, 2i > -1, 3+4i == 3+4i"), "ans = 1\nans = 1\nans = 1\n");
    assert_eq!(run("for i = 1:3, end, i"), "i = 3\n");
    assert_eq!(run("i = 5; z = 2i + i"), "z = 5 + 2i\n");
    assert_eq!(run("j"), "ans = 0 + 1i\n");
    assert_eq!(run("isreal(1i), isnumeric(1i), isreal(2), complex(1, 2)"), "ans = 0\nans = 1\nans = 1\nans = 1 + 2i\n");
    assert!(run("1:2i").contains("error: expected real value, got complex"));
    assert_eq!(run("k = 0; for z = [1i 2 3i], k = k + z; end, k"), "k = 2 + 4i\n");
    assert_eq!(run("if 1i, disp yes, end"), "yes\n");
}

#[test]
fn complex_indexing() {
    assert_eq!(run("x = [1 2 3]; x(2) = 1i"), "x =\n\n   1 + 0i   0 + 1i   3 + 0i\n\n");
    assert_eq!(run("x = [1 2 3]; x(2) = 1i; x(2) = 5"), "x =\n\n   1   5   3\n\n");
    assert_eq!(run("z = [1i 2 3i 4]; z(end)"), "ans = 4\n");
    assert_eq!(run("z = [1i 2 3i 4]; z([1 3]) = []"), "z =\n\n   2   4\n\n");
    assert_eq!(run("z = [1i 2; 3 4i]; z(:, 1)"), mat(&["   0 + 1i", "   3 + 0i"]));
    assert_eq!(run("z = [1i 2; 3 4i]; z(z ~= 2 & z ~= 3)"), mat(&["   0 + 1i", "   0 + 4i"]));
    assert_eq!(run("z = [1i 2]; z(2, 3) = 1"), "z =\n\n   0 + 1i   2 + 0i   0 + 0i\n   0 + 0i   0 + 0i   1 + 0i\n\n");
    assert_eq!(run("v = []; v(2) = 2i"), "v =\n\n   0 + 0i   0 + 2i\n\n");
    assert_eq!(run("[1i; 2]"), mat(&["   0 + 1i", "   2 + 0i"]));
    assert!(run("[1i, {2}]").contains("ans =\n{"));
}

#[test]
fn complex_display() {
    assert_eq!(run("z = 0.5 - 1.25i"), "z = 0.5000 - 1.2500i\n");
    assert_eq!(run("z = -2i"), "z = 0 - 2i\n");
    assert_eq!(run("z = 1e10 + 2e-3i"), "z = 1.0000e+10 + 2.0000e-03i\n");
    assert_eq!(run("z = [1+2i 3-4i; 1i 2]"), "z =\n\n   1 + 2i   3 - 4i\n   0 + 1i   2 + 0i\n\n");
    assert_eq!(run("[0.5+1i, 2-0.5i]"), mat(&["   0.5000 + 1.0000i   2.0000 - 0.5000i"]));
    assert_eq!(run("[100+1i; 2]"), mat(&["   100 +   1i", "     2 +   0i"]));
    assert_eq!(run("[NaN+1i, Inf-2i]"), mat(&["   NaN +   1i   Inf -   2i"]));
    assert_eq!(run("[1e10+1i, 1]"), mat(&["   1.0000e+10 + 1.0000e+00i   1.0000e+00 +          0i"]));
    assert_eq!(run("format long; pi + 1i"), "ans = 3.141592653589793 + 1.000000000000000i\n");
    assert_eq!(run("disp(3 - 4i); disp([1i 2])"), "3 - 4i\n   0 + 1i   2 + 0i\n");
    assert_eq!(run("c = {1i}"), "c =\n{\n  [1,1] = 0 + 1i\n}\n\n");
    assert!(run("s.z = 2 - 1i").contains("    z = 2 - 1i\n"));
}
