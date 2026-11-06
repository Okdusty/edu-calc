// Generated from src/gui/simple/catalog.rs and mod.rs: the Functions list and the keypad's keys.
export const GROUPS = [
 [
  "Powers and roots",
  [
   [
    "□^□",
    "Power: a to the b",
    "2^5 = 32"
   ],
   [
    "□²",
    "Square",
    "3² = 9"
   ],
   [
    "√(□)",
    "Square root",
    "√(16) = 4"
   ],
   [
    "∛(□)",
    "Cube root",
    "∛(27) = 3"
   ],
   [
    "root(□, □)",
    "n-th root: root(a, n)",
    "root(16, 4) = 2"
   ],
   [
    "abs(□)",
    "Absolute value (distance from 0)",
    "abs(-7) = 7"
   ],
   [
    "□!",
    "Factorial: n × (n − 1) × … × 1",
    "5! = 120"
   ],
   [
    "%",
    "Percent",
    "20% × 50 = 10"
   ]
  ]
 ],
 [
  "Exponentials and logarithms",
  [
   [
    "e^□",
    "e to a power",
    "e^2 ≈ 7.389056099"
   ],
   [
    "ln(□)",
    "Natural logarithm (base e)",
    "ln(e^3) = 3"
   ],
   [
    "log10(□)",
    "Logarithm base 10",
    "log10(1000) = 3"
   ],
   [
    "log(□, □)",
    "Logarithm with a base: log(a, base)",
    "log(8, 2) = 3"
   ]
  ]
 ],
 [
  "Trigonometry",
  [
   [
    "sin(□)",
    "Sine",
    "sin(30) = 1/2"
   ],
   [
    "cos(□)",
    "Cosine",
    "cos(60) = 1/2"
   ],
   [
    "tan(□)",
    "Tangent",
    "tan(45) = 1"
   ],
   [
    "asin(□)",
    "Inverse sine (sin⁻¹)",
    "asin(1/2) = 30"
   ],
   [
    "acos(□)",
    "Inverse cosine (cos⁻¹)",
    "acos(1/2) = 60"
   ],
   [
    "atan(□)",
    "Inverse tangent (tan⁻¹)",
    "atan(1) = 45"
   ],
   [
    "sec(□)",
    "Secant: 1 / cos",
    "sec(60) = 2"
   ],
   [
    "csc(□)",
    "Cosecant: 1 / sin",
    "csc(30) = 2"
   ],
   [
    "cot(□)",
    "Cotangent: 1 / tan",
    "cot(45) = 1"
   ],
   [
    "sinh(□)",
    "Hyperbolic sine",
    "sinh(0) = 0"
   ],
   [
    "cosh(□)",
    "Hyperbolic cosine",
    "cosh(0) = 1"
   ],
   [
    "tanh(□)",
    "Hyperbolic tangent",
    "tanh(0) = 0"
   ],
   [
    "°",
    "Degrees, whatever DEG / RAD says",
    "sin(90°) = 1"
   ]
  ]
 ],
 [
  "Rounding and whole numbers",
  [
   [
    "round(□)",
    "Round to the nearest whole number",
    "round(2.5) = 3"
   ],
   [
    "floor(□)",
    "Round down",
    "floor(2.7) = 2"
   ],
   [
    "ceil(□)",
    "Round up",
    "ceil(2.1) = 3"
   ],
   [
    "trunc(□)",
    "Drop the decimals",
    "trunc(-2.7) = −2"
   ],
   [
    "sgn(□)",
    "Sign: −1, 0 or 1",
    "sgn(-3) = −1"
   ],
   [
    "mod(□, □)",
    "Remainder of a ÷ b",
    "mod(17, 5) = 2"
   ],
   [
    "gcd(□, □)",
    "Greatest common divisor",
    "gcd(12, 18) = 6"
   ],
   [
    "lcm(□, □)",
    "Least common multiple",
    "lcm(4, 6) = 12"
   ],
   [
    "binomial(□, □)",
    "Combinations nCr: choose r of n",
    "binomial(5, 2) = 10"
   ],
   [
    "perm(□, □)",
    "Permutations nPr: ordered choices",
    "perm(5, 2) = 20"
   ]
  ]
 ],
 [
  "Statistics",
  [
   [
    "mean(□)",
    "Average. □ is a table column, mean(X), or a list",
    "mean([2, 4, 9]) = 5"
   ],
   [
    "median(□)",
    "Middle value",
    "median([3, 1, 7]) = 3"
   ],
   [
    "mode(□)",
    "Most frequent value",
    "mode([1, 2, 2, 3]) = 2"
   ],
   [
    "min(□)",
    "Smallest value",
    "min([3, 8, 5]) = 3"
   ],
   [
    "max(□)",
    "Largest value",
    "max([3, 8, 5]) = 8"
   ],
   [
    "range(□)",
    "Largest minus smallest",
    "range([3, 8, 5]) = 5"
   ],
   [
    "total(□)",
    "Sum of the values",
    "total([1, 2, 3]) = 6"
   ],
   [
    "dimension(□)",
    "How many values (n)",
    "dimension([4, 8, 1]) = 3"
   ],
   [
    "stdev(□)",
    "Standard deviation of a sample",
    "stdev([1, 3, 5]) = 2"
   ],
   [
    "stdevp(□)",
    "Standard deviation of a whole population",
    "stdevp([1, 3]) = 1"
   ],
   [
    "var(□)",
    "Variance of a sample",
    "var([1, 3, 5]) = 4"
   ],
   [
    "varp(□)",
    "Variance of a whole population",
    "varp([1, 3]) = 1"
   ],
   [
    "stderr(□)",
    "Standard error of the mean",
    "stderr([1, 3, 5]) ≈ 1.154700538"
   ],
   [
    "quartile(□, □)",
    "Quartile 1, 2 or 3: quartile(list, 1)",
    "quartile([1, 2, 3, 4, 5], 1) = 2"
   ],
   [
    "percentile(□, □)",
    "Value below a percentage of the list",
    "percentile([1, 2, 3, 4, 5], 50) = 3"
   ],
   [
    "iqr(□)",
    "Interquartile range: Q3 − Q1",
    "iqr([1, 2, 3, 4, 5]) = 2"
   ],
   [
    "geomean(□)",
    "Geometric mean",
    "geomean([1, 4]) = 2"
   ],
   [
    "harmmean(□)",
    "Harmonic mean",
    "harmmean([1, 4]) = 8/5"
   ]
  ]
 ],
 [
  "Correlation and regression",
  [
   [
    "pearson(□, □)",
    "Pearson's r: how straight the trend is, from −1 to 1",
    "pearson([1, 2, 3], [2, 4, 6]) = 1"
   ],
   [
    "pearson(□, □)^2",
    "r²: the share of the change in y the line explains",
    "pearson([1, 2, 3], [2, 4, 6])^2 = 1"
   ],
   [
    "slope(□, □)",
    "Slope m of the best-fit line y = mx + b",
    "slope([1, 2, 3], [2, 4, 6]) = 2"
   ],
   [
    "intercept(□, □)",
    "Intercept b of the best-fit line",
    "intercept([1, 2, 3], [3, 5, 7]) = 1"
   ],
   [
    "spearman(□, □)",
    "Spearman's rank correlation, for curved trends",
    "spearman([1, 2, 3], [1, 8, 27]) = 1"
   ],
   [
    "cov(□, □)",
    "Covariance of a whole population",
    "cov([1, 2, 3], [2, 4, 6]) = 4/3"
   ]
  ]
 ],
 [
  "Constants",
  [
   [
    "π",
    "Pi: a circle's circumference ÷ diameter",
    "π ≈ 3.141592654"
   ],
   [
    "e",
    "Euler's number",
    "e ≈ 2.718281828"
   ],
   [
    "golden",
    "Golden ratio φ",
    "golden ≈ 1.618033989"
   ],
   [
    "x",
    "The variable of a graph",
    "y = 2x + 1"
   ]
  ]
 ]
];
export const PAGES = [["basic", "Basic", [["x", "x"], ["y", "y"], ["x²", "□²"], ["aᵇ", "□^□"], ["√", "√(□)"], ["(", "("], [")", ")"], ["a/b", "□/□"], ["|a|", "abs(□)"], ["π", "π"], ["sin", "sin(□)"], ["cos", "cos(□)"], ["tan", "tan(□)"], ["e", "e"], ["%", "%"], ["ln", "ln(□)"], ["log", "log10(□)"], ["n!", "□!"], ["1/x", "1/□"], [",", ","]]], ["functions", "f(x)", [["∛", "∛(□)"], ["ⁿ√", "root(□, □)"], ["eˣ", "e^□"], ["10ˣ", "10^□"], ["logₐ", "log(□, □)"], ["nCr", "binomial(□, □)"], ["nPr", "perm(□, □)"], ["gcd", "gcd(□, □)"], ["lcm", "lcm(□, □)"], ["mod", "mod(□, □)"], ["round", "round(□)"], ["floor", "floor(□)"], ["ceil", "ceil(□)"], ["|a|", "abs(□)"], ["π", "π"], ["x", "x"], ["(", "("], [")", ")"], [",", ","], ["aᵇ", "□^□"]]], ["trig", "Trig", [["sin", "sin(□)"], ["cos", "cos(□)"], ["tan", "tan(□)"], ["π", "π"], ["°", "°"], ["sin⁻¹", "asin(□)"], ["cos⁻¹", "acos(□)"], ["tan⁻¹", "atan(□)"], ["x", "x"], ["aᵇ", "□^□"], ["sec", "sec(□)"], ["csc", "csc(□)"], ["cot", "cot(□)"], ["(", "("], [")", ")"], ["sinh", "sinh(□)"], ["cosh", "cosh(□)"], ["tanh", "tanh(□)"], ["a/b", "□/□"], ["√", "√(□)"]]], ["stats", "Stats", [["mean", "mean(□)"], ["median", "median(□)"], ["mode", "mode(□)"], ["sd", "stdev(□)"], ["var", "var(□)"], ["min", "min(□)"], ["max", "max(□)"], ["range", "range(□)"], ["Σ", "total(□)"], ["n", "dimension(□)"], ["quart", "quartile(□, □)"], ["%ile", "percentile(□, □)"], ["IQR", "iqr(□)"], ["SE", "stderr(□)"], ["cov", "cov(□, □)"], ["r", "pearson(□, □)"], ["ρ", "spearman(□, □)"], ["slope", "slope(□, □)"], ["y-int", "intercept(□, □)"], [",", ","]]]];
export const NUMBERS = [["7", "7"], ["8", "8"], ["9", "9"], ["÷", "÷"], ["⌫", "@back"], ["Clear", "@clear"], ["4", "4"], ["5", "5"], ["6", "6"], ["×", "×"], ["←", "@left"], ["→", "@right"], ["1", "1"], ["2", "2"], ["3", "3"], ["−", "−"], ["Ans", "@ans"], ["Next □", "@slot"], ["0", "0"], [".", "."], ["=", "="], ["+", "+"], ["Enter", "@enter"]];
