// C ABI over libqalculate for src/qalc/mod.rs. One global Calculator; Rust serializes all calls.
// Returned strings are malloc'd UTF-8, released with qs_free. No C++ exception leaves this file.
#include <libqalculate/qalculate.h>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <utility>
#include <vector>

using std::string;
using std::vector;

// Exported by libqalculate but not in its headers: qalc's "a = 5" -> "a := 5" when that is an assignment.
// ponytail: internal symbol, re-implement the name check here if a libqalculate release drops it
bool transform_expression_for_equals_save(string &str, const ParseOptions &po);

static const char US = '\x1f', RS = '\x1e', GS = '\x1d';

static EvaluationOptions eo;
static PrintOptions po;
static KnownVariable *ans_var;
static MathFunction *eig_fn;

static char *dup(const string &s) {
    char *p = (char *)malloc(s.size() + 1);
    if (p) memcpy(p, s.c_str(), s.size() + 1);
    return p;
}

// Queued libqalculate messages, RS-separated; *err set when one is an error.
static string messages(bool *err) {
    string s;
    for (CalculatorMessage *m = CALCULATOR->message(); m; m = CALCULATOR->nextMessage()) {
        if (m->type() == MESSAGE_ERROR && err) *err = true;
        if (!s.empty()) s += RS;
        s += m->message();
    }
    return s;
}

// Error reply; also ends a timer an exception skipped past.
static char *failed(const string &msg) {
    try {
        if (CALCULATOR && CALCULATOR->isControlled()) CALCULATOR->stopControl();
    } catch (...) {
    }
    return dup(msg);
}

static void init() {
    if (CALCULATOR) return;
    new Calculator(true);              // ignore locale: '.' is the decimal point whatever LANG says
    CALCULATOR->loadExchangeRates();   // cached rates file only; nothing in this shim fetches
    CALCULATOR->loadGlobalDefinitions();
    CALCULATOR->loadLocalDefinitions();
    CALCULATOR->setExchangeRatesWarningEnabled(false);
    CALCULATOR->setPrecision(10);
    // qalc has no eigenvalue function: roots of the characteristic polynomial, exact when solve manages
    // ponytail: a repeated eigenvalue is listed once (solve lists distinct roots); Octave's eig() counts them
    eig_fn = CALCULATOR->addFunction(new UserFunction("Matrices", "eigenvalues", "solve(det(\\x - y*identity(\\x)) = 0, y)"));
    eig_fn->addName("eig");
    // qalc's defaults (src/qalc.cc load_preferences)
    eo.approximation = APPROXIMATION_TRY_EXACT;
    eo.structuring = STRUCTURING_SIMPLIFY;
    eo.parse_options.unknowns_enabled = false;
    eo.parse_options.read_precision = DONT_READ_PRECISION;
    eo.parse_options.angle_unit = ANGLE_UNIT_RADIANS;
    eo.assume_denominators_nonzero = true;
    eo.auto_post_conversion = POST_CONVERSION_OPTIMAL;
    po.use_unicode_signs = true;
    po.multiplication_sign = MULTIPLICATION_SIGN_X;
    po.division_sign = DIVISION_SIGN_SLASH;
    po.base_display = BASE_DISPLAY_NORMAL;
    po.exp_display = EXP_UPPERCASE_E;
    po.interval_display = INTERVAL_DISPLAY_SIGNIFICANT_DIGITS;
    po.spell_out_logical_operators = true;
    po.use_max_decimals = false;
    po.restrict_fraction_length = true;
    ans_var = (KnownVariable *)CALCULATOR->addVariable(new KnownVariable("Temporary", "ans", m_undefined));
    messages(nullptr);
    // libqalculate's static destructors hang while its calculation thread still runs
    atexit([] {
        try {
            CALCULATOR->terminateThreads();
        } catch (...) {
        }
    });
}

static string show(const MathStructure &m, PrintOptions p, bool *approx = nullptr) {
    bool a = false;
    p.is_approximate = &a;
    MathStructure c(m);
    c.removeDefaultAngleUnit(eo);
    c.format(p);
    c.removeDefaultAngleUnit(eo);
    string s = c.print(p);
    if (approx) *approx = a || m.isApproximate();
    return s;
}

// How the input was understood, in math notation: formatted as typed, printed like a result
// (so 1/3 reads "1/3", not "1 / 3").
static string show_parsed(const MathStructure &m) {
    PrintOptions p = po;
    p.preserve_format = true;
    p.show_ending_zeroes = false;
    p.restrict_to_parent_precision = false;
    p.interval_display = INTERVAL_DISPLAY_PLUSMINUS;
    MathStructure c(m);
    c.removeDefaultAngleUnit(eo);
    c.format(p);
    c.removeDefaultAngleUnit(eo);
    p.preserve_format = false;
    return c.print(p);
}

// ---- Octave code ----

static string ostr(const MathStructure &m, bool &ok);

// m as an Octave operand: parenthesized unless atomic.
static string opar(const MathStructure &m, bool &ok) {
    string s = ostr(m, ok);
    bool atom = m.isVariable() || m.isSymbolic() || m.isFunction() || m.isVector() ||
                (m.isNumber() && !m.number().isNegative() && m.number().isReal());
    return atom ? s : "(" + s + ")";
}

static string onum(const Number &n, bool &ok) {
    if (n.hasImaginaryPart()) {
        string re = n.hasRealPart() ? onum(n.realPart(), ok) + " + " : "";
        string im = n.imaginaryPart().isOne() ? "i" : onum(n.imaginaryPart(), ok) + "*i";
        return re.empty() && im == "i" ? im : "(" + re + im + ")";
    }
    if (n.isPlusInfinity()) return "Inf";
    if (n.isMinusInfinity()) return "-Inf";
    if (n.isRational() && !n.isApproximate()) {
        PrintOptions p;
        p.min_exp = EXP_NONE;
        p.number_fraction_format = FRACTION_DECIMAL;
        if (n.isInteger()) return n.print(p);
        return "(" + n.numerator().print(p) + "/" + n.denominator().print(p) + ")";
    }
    char buf[40];
    snprintf(buf, sizeof buf, "%.17g", n.floatValue());
    return buf;
}

static const char *ofunc(const string &q) {
    static const char *const map[][2] = {
        {"ln", "log"}, {"sgn", "sign"}, {"trunc", "fix"}, {"re", "real"}, {"im", "imag"}, {"arg", "angle"},
        {"root", "nthroot"}, {"sin", "sin"}, {"cos", "cos"}, {"tan", "tan"}, {"arcsin", "asin"}, {"arccos", "acos"},
        {"arctan", "atan"}, {"sinh", "sinh"}, {"cosh", "cosh"}, {"tanh", "tanh"}, {"arsinh", "asinh"},
        {"arcosh", "acosh"}, {"artanh", "atanh"}, {"sec", "sec"}, {"csc", "csc"}, {"cot", "cot"}, {"exp", "exp"},
        {"log10", "log10"}, {"log2", "log2"}, {"sqrt", "sqrt"}, {"cbrt", "cbrt"}, {"abs", "abs"},
        {"floor", "floor"}, {"ceil", "ceil"}, {"round", "round"}, {"gamma", "gamma"}, {"erf", "erf"},
        {"erfc", "erfc"}, {"factorial", "factorial"}, {"conj", "conj"}, {"mod", "mod"}, {"rem", "rem"},
        {"atan2", "atan2"}, {"hypot", "hypot"}, {"max", "max"}, {"min", "min"},
    };
    for (auto &e : map)
        if (q == e[0]) return e[1];
    return nullptr;
}

// Vectorized Octave for a format()ted structure; ok = false when not expressible (units, dates, ...).
static string ostr(const MathStructure &m, bool &ok) {
    switch (m.type()) {
    case STRUCT_NUMBER: return onum(m.number(), ok);
    case STRUCT_SYMBOLIC: return m.symbol();
    case STRUCT_VARIABLE: {
        Variable *v = m.variable();
        if (v == CALCULATOR->v_pi) return "pi";
        if (v == CALCULATOR->v_e) return "e";
        if (!v->isKnown()) return v->referenceName();
        const MathStructure &val = ((KnownVariable *)v)->get();
        if (val.isNumber()) return onum(val.number(), ok);
        ok = false;
        return "";
    }
    case STRUCT_FUNCTION: {
        string name = m.function()->referenceName();
        string args;
        for (size_t i = 0; i < m.size(); i++) args += (i ? ", " : "") + ostr(m[i], ok);
        if (name == "log" && m.size() == 2) return "(log(" + ostr(m[0], ok) + ")./log(" + ostr(m[1], ok) + "))";
        const char *f = name == "log" ? "log" : ofunc(name);
        if (!f) ok = false;
        return string(f ? f : "") + "(" + args + ")";
    }
    case STRUCT_ADDITION: {
        string s;
        for (size_t i = 0; i < m.size(); i++) {
            const MathStructure &c = m[i];
            if (c.isVariable() && c.variable() == CALCULATOR->v_C) continue; // integration constant
            if (c.isNegate()) s += (s.empty() ? "-" : " - ") + opar(c[0], ok);
            else s += (s.empty() ? "" : " + ") + ostr(c, ok);
        }
        return s.empty() ? "0" : s;
    }
    case STRUCT_MULTIPLICATION: {
        string s;
        for (size_t i = 0; i < m.size(); i++) s += (i ? ".*" : "") + opar(m[i], ok);
        return s;
    }
    case STRUCT_DIVISION: return opar(m[0], ok) + "./" + opar(m[1], ok);
    case STRUCT_INVERSE: return "1./" + opar(m[0], ok);
    case STRUCT_NEGATE: return "-" + opar(m[0], ok);
    case STRUCT_POWER: return opar(m[0], ok) + ".^" + opar(m[1], ok);
    case STRUCT_VECTOR: {
        string s = "[";
        for (size_t i = 0; i < m.size(); i++) {
            if (m[i].isVector()) {
                for (size_t j = 0; j < m[i].size(); j++) s += (j ? ", " : "") + ostr(m[i][j], ok);
                if (i + 1 < m.size()) s += "; ";
            } else {
                s += (i ? ", " : "") + ostr(m[i], ok);
            }
        }
        return s + "]";
    }
    default: ok = false; return "";
    }
}

static string octave(const MathStructure &m) {
    PrintOptions p;
    p.number_fraction_format = FRACTION_FRACTIONAL;
    p.halfexp_to_sqrt = true;
    p.short_multiplication = false;
    MathStructure c(m);
    c.removeDefaultAngleUnit(eo);
    c.format(p);
    bool ok = true;
    string s = ostr(c, ok);
    return ok ? s : "";
}

// ---- numbers and unknowns ----

static void unknowns(const MathStructure &m, vector<string> &out) {
    string n;
    if (m.isSymbolic()) n = m.symbol();
    else if (m.isVariable() && !m.variable()->isKnown() && m.variable() != CALCULATOR->v_C) n = m.variable()->referenceName();
    if (!n.empty() && std::find(out.begin(), out.end(), n) == out.end()) out.push_back(n);
    for (size_t i = 0; i < m.size(); i++) unknowns(m[i], out);
}

static bool unitish(const MathStructure &m) {
    return m.isUnit() || (m.isPower() && m[0].isUnit());
}

static double fval(const Number &n) {
    if (n.isPlusInfinity()) return INFINITY;
    if (n.isMinusInfinity()) return -INFINITY;
    return n.floatValue();
}

// Scalar value; number x units counts as its magnitude in the shown unit.
static bool numval(const MathStructure &m, double &re, double &im) {
    if (m.isNumber()) {
        re = fval(m.number().realPart());
        im = fval(m.number().imaginaryPart());
        return true;
    }
    if (!m.isMultiplication()) return false;
    bool got = false;
    for (size_t i = 0; i < m.size(); i++) {
        if (m[i].isNumber() && !got) got = numval(m[i], re, im);
        else if (!unitish(m[i])) return false;
    }
    return got;
}

// "rows cols re im re im ..." row-major, or "" when not numeric.
static string value(const MathStructure &m) {
    vector<const MathStructure *> el;
    size_t r = 1, c = 1;
    if (m.isMatrix()) {
        r = m.size();
        c = m[0].size();
        for (size_t i = 0; i < r; i++)
            for (size_t j = 0; j < c; j++) el.push_back(&m[i][j]);
    } else if (m.isVector()) {
        c = m.size();
        for (size_t j = 0; j < c; j++) el.push_back(&m[j]);
    } else {
        el.push_back(&m);
    }
    string s = std::to_string(r) + " " + std::to_string(c);
    char buf[64];
    for (auto e : el) {
        double re, im;
        if (!numval(*e, re, im)) return "";
        snprintf(buf, sizeof buf, " %.17g %.17g", re, im);
        s += buf;
    }
    return s;
}

// ---- parsed input: side effects, argument order ----

static bool unknown(const MathStructure &m) {
    return m.isSymbolic() || (m.isVariable() && !m.variable()->isKnown());
}

static bool undef(const MathStructure &m) {
    return m.isVariable() && m.variable() == CALCULATOR->v_undef;
}

// Functions that change something outside the calculation: variables, files, gnuplot, the shell.
static bool side_effects(const MathStructure &m) {
    if (m.isFunction()) {
        int id = m.function()->id();
        if (id == FUNCTION_ID_SAVE || id == FUNCTION_ID_PLOT || id == FUNCTION_ID_EXPORT || id == FUNCTION_ID_COMMAND) return true;
    }
    for (size_t i = 0; i < m.size(); i++)
        if (side_effects(m[i])) return true;
    return false;
}

// integrate(f, x, a, b) (Octave/SymPy order) -> qalc's integrate(f, a, b, x). Only where qalc
// itself would fail: its 4th argument must be the (unknown) variable.
static bool reorder_integrals(MathStructure &m) {
    bool any = false;
    for (size_t i = 0; i < m.size(); i++) any |= reorder_integrals(m[i]);
    if (m.isFunction() && m.function()->id() == FUNCTION_ID_INTEGRATE && m.size() >= 4 && unknown(m[1]) && !undef(m[2]) &&
        !undef(m[3]) && !unknown(m[3])) {
        MathStructure x(m[1]);
        m[1].set(m[2]);
        m[2].set(m[3]);
        m[3].set(x);
        any = true;
    }
    return any;
}

// ---- visuals: Octave code that draws an answer ----

static bool ident(const string &s) {
    return !s.empty() && (isalpha((unsigned char)s[0]) || s[0] == '_') &&
           std::all_of(s.begin(), s.end(), [](char c) { return isalnum((unsigned char)c) || c == '_'; });
}

// Variable to plot f over: v when it is an unknown, else f's only unknown ("x" when it has none);
// "" when f has other unknowns too.
static string plot_var(const MathStructure &f, const MathStructure *v) {
    vector<string> u;
    unknowns(f, u);
    string x = v && unknown(*v) ? (v->isSymbolic() ? v->symbol() : v->variable()->referenceName()) : u.empty() ? "x" : u[0];
    return ident(x) && (u.empty() || (u.size() == 1 && u[0] == x)) ? x : "";
}

// Math view as an Octave string literal. qalc quotes symbols ('t'); labels drop those quotes
// like pretty() does, which also leaves nothing to escape.
static string olabel(string s) {
    s.erase(std::remove(s.begin(), s.end(), '\''), s.end());
    return "'" + s + "'";
}

static string fmt(double d) {
    char b[32];
    snprintf(b, sizeof b, "%.10g", d);
    return b;
}

// f and F (its integral or derivative, as Octave code and as shown) on one plot.
static string pair_plot(const string &x, const MathStructure &f, const string &F, const string &shown) {
    string fo = octave(f), h = "fplot(@(" + x + ") ";
    if (x.empty() || fo.empty() || F.empty()) return "";
    return "figure; " + h + fo + ", [-5 5]); hold on; " + h + F + ", [-5 5]); hold off; legend(" + olabel(show_parsed(f)) + ", " +
           olabel(shown) + ")";
}

// Octave code that visualizes the answer, from the parsed input p, exact result m (as Octave
// code `oct`, shown as `result`), approximate result ma and its unknowns fv; "" when nothing fits.
static string visual(const MathStructure &p, const MathStructure &m, const MathStructure &ma, const string &oct, const string &result,
                     const vector<string> &fv) {
    int id = p.isFunction() ? p.function()->id() : 0;
    string s;
    if (id == FUNCTION_ID_INTEGRATE && p.size() >= 4 && !undef(p[1]) && !undef(p[2])) {
        string x = plot_var(p[0], &p[3]), f = octave(p[0]), a = octave(p[1]), b = octave(p[2]);
        if (!x.empty() && !f.empty() && !a.empty() && !b.empty()) return "intplot(@(" + x + ") " + f + ", " + a + ", " + b + ")";
    } else if (id == FUNCTION_ID_INTEGRATE && p.size() >= 4) {
        if (!(s = pair_plot(plot_var(p[0], &p[3]), p[0], oct, result)).empty()) return s;
    } else if (id == FUNCTION_ID_DIFFERENTIATE && p.size() >= 2) {
        if (!(s = pair_plot(plot_var(p[0], &p[1]), p[0], oct, result)).empty()) return s;
    } else if (id == FUNCTION_ID_SOLVE && p.size() >= 1 && (!p[0].isComparison() || p[0].comparisonType() == COMPARISON_EQUALS)) {
        // lhs - rhs over the real roots, which get marked
        MathStructure g(p[0].isComparison() ? p[0][0] : p[0]);
        if (p[0].isComparison() && !p[0][1].isZero()) g.subtract(p[0][1]);
        string x = plot_var(g, p.size() >= 2 ? &p[1] : nullptr), go = octave(g);
        if (!x.empty() && !go.empty()) {
            vector<double> r;
            for (size_t i = 0; i < (ma.isVector() ? ma.size() : 1); i++) {
                const MathStructure &z = ma.isVector() ? ma[i] : ma;
                if (z.isNumber() && z.number().isReal()) r.push_back(z.number().floatValue());
            }
            string h = "figure; fplot(@(" + x + ") " + go;
            if (r.empty()) return h + ", [-5 5]); grid on";
            string rs;
            for (double v : r) rs += (rs.empty() ? "" : " ") + fmt(v);
            if (r.size() > 1) rs = "[" + rs + "]";
            auto [lo, hi] = std::minmax_element(r.begin(), r.end());
            return h + ", [" + fmt(*lo - 2) + " " + fmt(*hi + 2) + "]); hold on; plot(" + rs + ", 0*" + rs + ", 'o'); hold off; grid on";
        }
    } else if (p.isFunction() && p.function() == eig_fn && p.size() == 1) {
        MathStructure a(p[0]);
        a.eval(eo);
        bool real2 = a.isMatrix() && a.size() == 2 && a[0].size() == 2;
        for (size_t k = 0; real2 && k < 4; k++) real2 = a[k / 2][k % 2].isNumber() && a[k / 2][k % 2].number().isReal();
        string o = real2 ? octave(a) : "";
        if (!o.empty()) return "eigplot(" + o + ")";
    }
    if (oct.empty()) return "";
    if (m.isMatrix()) return "matview(" + oct + ")";
    if (fv.size() == 1 && ident(fv[0])) return "fplot(@(" + fv[0] + ") " + oct + ", [-5 5])";
    if (fv.size() == 2 && ident(fv[0]) && ident(fv[1])) return "fsurf(@(" + fv[0] + ", " + fv[1] + ") " + oct + ", [-5 5 -5 5])";
    return "";
}

// ---- entry points ----

extern "C" void qs_free(char *s) { free(s); }

// Fields (US-separated): error, input, result, approx, alts (RS between, GS label|text), value,
// octave, free vars (RS), messages (RS), assigned name, visual. Failure = just the error text
// (RS-separated). msecs bounds the whole call. mode: 0 plain, 1 sets ans, 2 preview (no side
// effects at all: anything that would assign, plot, export or run a command is refused).
extern "C" char *qs_calc(const char *expr, int msecs, int mode) {
    try {
        bool set_ans = mode == 1, pure = mode == 2;
        auto t0 = std::chrono::steady_clock::now();
        // what is left of the time budget, never <= 0 (that would mean unlimited)
        auto left = [&] {
            long used = std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t0).count();
            return (int)std::max(1L, (long)msecs - used);
        };
        init();
        CALCULATOR->clearMessages();
        EvaluationOptions e = eo;
        PrintOptions p = po;
        string str = CALCULATOR->unlocalizeExpression(expr, e.parse_options), to, conv;
        Number cbase;
        int binpref = -1;
        bool angle = false, factors = false, pfe = false, cal = false, bases = false;
        if (CALCULATOR->separateToExpression(str, to, e, true)) {
            conv = CALCULATOR->parseToExpression(to, e, p, &cbase, &binpref, &angle, &factors, &pfe, &cal, &bases);
            // "5 km to mi" is 3.106855961 mi, not 3 mi + 188 yd + ... ("to mixed" still asks for that)
            if (!conv.empty()) e.mixed_units_conversion = MIXED_UNITS_CONVERSION_NONE;
        }
        bool user_frac = p.number_fraction_format != po.number_fraction_format || p.restrict_fraction_length != po.restrict_fraction_length;
        // "factor ..." / "expand ..." prefixes, like qalc
        size_t sp = str.find(' ');
        if (sp != string::npos && (str.compare(0, sp, "factor") == 0 || str.compare(0, sp, "factorize") == 0)) {
            factors = true;
            str.erase(0, sp + 1);
        } else if (sp != string::npos && str.compare(0, sp, "expand") == 0) {
            e.structuring = STRUCTURING_SIMPLIFY;
            str.erase(0, sp + 1);
        }
        if (factors) e.structuring = STRUCTURING_FACTORIZE;
        transform_expression_for_equals_save(str, e.parse_options);
        // a look at the parse first (parsing evaluates nothing)
        MathStructure pre;
        CALCULATOR->parse(&pre, str, e.parse_options);
        if (pure && side_effects(pre)) {
            CALCULATOR->clearMessages();
            return dup("not previewed: the expression has side effects");
        }
        bool reordered = reorder_integrals(pre);
        if (!reordered) CALCULATOR->clearMessages(); // the string run below parses (and reports) again
        string full = conv.empty() ? str : str + " to " + conv;
        // from the string, so qalc's "where", "to" ... work; from the reordered parse when there is one
        auto run = [&](MathStructure *out, const EvaluationOptions &ev, MathStructure *ps) {
            if (!reordered) return CALCULATOR->calculate(out, full, left(), ev, ps);
            *out = pre;
            return CALCULATOR->calculate(out, left(), ev, conv);
        };
        MathStructure m, parsed;
        if (reordered) {
            ParseOptions pp = e.parse_options;
            pp.preserve_format = true;
            CALCULATOR->parse(&parsed, str, pp);
            reorder_integrals(parsed);
        }
        bool done = run(&m, e, &parsed);
        bool err = false;
        string msgs = messages(&err);
        if (!done || m.isAborted()) return dup("calculation timed out");
        if (err) return dup(msgs);
        // sin(x rad) -> sin(x): radians are the default here
        m.removeDefaultAngleUnit(e);
        parsed.removeDefaultAngleUnit(e);

        // Approximate result of an exact input (sqrt(8) = 2.828...): redo it exactly, like qalc's dual display
        if (m.isApproximate() && full.find('.') == string::npos && !m.containsType(STRUCT_UNIT) && !parsed.containsType(STRUCT_UNIT)) {
            EvaluationOptions ex = e;
            ex.approximation = APPROXIMATION_EXACT;
            ex.expand = -2;
            MathStructure mx;
            CALCULATOR->beginTemporaryStopMessages();
            bool ok = run(&mx, ex, nullptr);
            CALCULATOR->endTemporaryStopMessages();
            mx.removeDefaultAngleUnit(e);
            if (ok && !mx.isAborted() && !mx.isApproximate()) m = mx;
        }

        CALCULATOR->startControl(left());
        if (factors) m.integerFactorize();
        if (pfe) m.expandPartialFractions(e);
        string assigned, input;
        if (parsed.isFunction() && parsed.function()->id() == FUNCTION_ID_SAVE && parsed.size() >= 2 && parsed[1].isSymbolic()) {
            assigned = parsed[1].symbol();
            input = assigned + " ≔ " + show_parsed(parsed[0]);
        } else {
            input = show_parsed(parsed);
        }
        if (!to.empty()) input += " → " + to;
        vector<string> fv;
        unknowns(m, fv);
        std::sort(fv.begin(), fv.end());
        bool units = m.containsType(STRUCT_UNIT);

        string result;
        PrintOptions pd = p;
        pd.number_fraction_format = FRACTION_DECIMAL;
        // qalc-ish: fractions unless the input had decimals or the result has units
        PrintOptions pr = p;
        if (!user_frac) pr.number_fraction_format = units || p.base != BASE_DECIMAL || full.find('.') != string::npos ? FRACTION_DECIMAL : FRACTION_FRACTIONAL;
        if (bases) {
            for (int b : {BASE_BINARY, BASE_OCTAL, BASE_DECIMAL, BASE_HEXADECIMAL}) {
                pd.base = b;
                result += (result.empty() ? "" : " = ") + show(m, pd);
            }
            pd.base = p.base;
        } else {
            bool a = false;
            result = show(m, pr, &a);
            // rounded display of an exact value (2^100, 1/7^20): show it exactly instead
            if (a && !m.isApproximate() && !units && !user_frac) {
                PrintOptions pf = pr;
                pf.number_fraction_format = FRACTION_FRACTIONAL;
                pf.restrict_fraction_length = false;
                pf.min_exp = EXP_NONE;
                string f = show(m, pf, &a);
                // ponytail: 200 chars, past that (1000!) the rounded form is more useful
                if (!a && f.size() <= 200) result = f;
            }
        }

        MathStructure ma(m);
        EvaluationOptions ea = e;
        ea.approximation = APPROXIMATION_APPROXIMATE;
        ma.eval(ea);
        string approx;
        if (fv.empty()) {
            approx = show(ma, pd);
            if (approx == result) approx.clear();
        }

        string alts;
        auto alt = [&](const char *label, const MathStructure &x, PrintOptions q) {
            string s = show(x, q);
            if (s.empty() || s == result || s == approx || alts.find(GS + s + RS) != string::npos) return;
            alts += string(label) + GS + s + RS;
        };
        if (fv.empty() && m.isNumber() && m.number().isReal()) {
            const Number &n = m.number();
            PrintOptions q = pd;
            q.base = BASE_DECIMAL;
            if (n.isRational() && !n.isInteger() && !n.isApproximate()) {
                q.number_fraction_format = FRACTION_FRACTIONAL;
                q.restrict_fraction_length = false;
                alt("fraction", m, q);
                q.number_fraction_format = FRACTION_COMBINED;
                alt("mixed", m, q);
                q.number_fraction_format = FRACTION_DECIMAL;
            }
            alt("decimal", m, q);
            if (n.isInteger() && n.integerLength() <= 64) {
                for (auto b : {std::make_pair("hex", BASE_HEXADECIMAL), {"bin", BASE_BINARY}, {"oct", BASE_OCTAL}}) {
                    q.base = b.second;
                    alt(b.first, m, q);
                }
                q.base = BASE_DECIMAL;
            }
            if (!n.isZero()) {
                q.min_exp = EXP_PURE;
                alt("scientific", ma, q);
            }
        }
        if (units) {
            PrintOptions q = pd;
            q.base = BASE_DECIMAL;
            q.use_unit_prefixes = false;
            alt("base units", CALCULATOR->convertToBaseUnits(m, e), q);
        }

        string oct = units ? "" : octave(m);
        string val = value(m);
        if (val.empty() && fv.empty()) val = value(ma);
        string fvs;
        for (auto &v : fv) fvs += (fvs.empty() ? "" : string(1, RS)) + v;
        string vis = pure ? "" : visual(parsed, m, ma, oct, result, fv);

        if (CALCULATOR->aborted()) return failed("calculation timed out");
        if (set_ans && assigned.empty()) ans_var->set(m);
        CALCULATOR->stopControl();
        string more = messages(nullptr);
        if (!more.empty()) msgs += (msgs.empty() ? "" : string(1, RS)) + more;
        string r;
        for (const string &f : {string(), input, result, approx, alts, val, oct, fvs, msgs, assigned, vis}) r += f + US;
        r.pop_back();
        return dup(r);
    } catch (const std::exception &x) {
        return failed(string("qalculate: ") + x.what());
    } catch (...) {
        return failed("qalculate: internal error");
    }
}

// Math view of an unevaluated expression; "" when it doesn't parse cleanly.
extern "C" char *qs_pretty(const char *expr) {
    try {
        init();
        CALCULATOR->clearMessages();
        MathStructure m;
        ParseOptions pa = eo.parse_options;
        pa.preserve_format = true;
        pa.unknowns_enabled = true; // labels: "2πr" with r undefined is fine
        pa.limit_implicit_multiplication = true; // "Temperature" stays a word, not T·e·m·p...
        CALCULATOR->parse(&m, CALCULATOR->unlocalizeExpression(expr, pa), pa);
        string s = messages(nullptr).empty() ? show_parsed(m) : ""; // any warning: likely not math
        // a quoted unknown word ("Temperature") means prose, not math; 'r' is just a symbol
        s.erase(std::remove(s.begin(), s.end(), '\''), s.end());
        return dup(s.find('"') == string::npos ? s : "");
    } catch (...) {
        return dup("");
    }
}

// Define (or update) a calculator variable from a value expression; NULL on success, else the error.
// Built-in names (units, functions, constants, x/y/z) are refused rather than shadowed.
extern "C" char *qs_set_var(const char *name, const char *val) {
    try {
        init();
        CALCULATOR->clearMessages();
        if (!CALCULATOR->variableNameIsValid(name)) return dup(string("invalid variable name: ") + name);
        ExpressionItem *it = CALCULATOR->getActiveExpressionItem(name);
        KnownVariable *v = nullptr;
        if (it) {
            if (it->type() == TYPE_VARIABLE && it->isLocal() && ((Variable *)it)->isKnown()) v = (KnownVariable *)it;
            else return dup(string("'") + name + "' is a built-in qalculate name");
        }
        MathStructure m;
        CALCULATOR->parse(&m, val, eo.parse_options);
        m.eval(eo);
        bool err = false;
        string msgs = messages(&err);
        if (err) return dup(msgs);
        if (v) v->set(m);
        else CALCULATOR->addVariable(new KnownVariable("Temporary", name, m));
        return nullptr;
    } catch (...) {
        return failed("qalculate: internal error");
    }
}

// out[i] = expr at xvar = xs[i] (and yvar = ys[i] when yvar is non-empty); NaN where not real.
// NULL on success, else the error.
extern "C" char *qs_sample(const char *expr, const char *xvar, const char *yvar, const double *xs, const double *ys, size_t n,
                           double *out, int msecs) {
    try {
        init();
        CALCULATOR->clearMessages();
        MathStructure m;
        bool done = CALCULATOR->calculate(&m, CALCULATOR->unlocalizeExpression(expr, eo.parse_options), msecs, eo);
        bool err = false;
        string msgs = messages(&err);
        if (!done || m.isAborted()) return dup("calculation timed out");
        if (err) return dup(msgs);
        MathStructure xv = CALCULATOR->parse(xvar, eo.parse_options);
        bool two = yvar && *yvar;
        MathStructure yv = two ? CALCULATOR->parse(yvar, eo.parse_options) : MathStructure();
        EvaluationOptions ea = eo;
        ea.approximation = APPROXIMATION_APPROXIMATE;
        CALCULATOR->startControl(msecs);
        size_t i = 0;
        for (; i < n && !CALCULATOR->aborted(); i++) {
            MathStructure t(m);
            Number nx;
            nx.setFloat(xs[i]);
            t.replace(xv, nx);
            if (two) {
                Number ny;
                ny.setFloat(ys[i]);
                t.replace(yv, ny);
            }
            t.eval(ea);
            out[i] = t.isNumber() && !t.number().hasImaginaryPart() ? t.number().floatValue() : NAN;
        }
        bool timeout = CALCULATOR->aborted() || i < n;
        CALCULATOR->stopControl();
        messages(nullptr);
        return timeout ? dup("plot sampling timed out") : nullptr;
    } catch (...) {
        return failed("qalculate: internal error");
    }
}
