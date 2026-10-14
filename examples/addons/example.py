"""Example qcalcp addon. Copy to ~/.config/qcalcp/addons/ and these become callable from
Octave code: sinc(x), [lo, hi] = minmax(v), movavg(v, n), [r, h] = projectile(v0, deg),
hypotn(a, b, ...).

Arguments arrive as plain Python: scalars as float, vectors as lists, matrices as lists of
row lists, strings as str, logicals as bool, cells as lists, structs as dicts.
Return numbers, bools, strings, (nested) lists, dicts, numpy arrays or None; return a tuple
for multiple outputs. Exceptions show up in qcalcp as "python: ExcType: message".
From the prompt: pyexec('import math'), pyeval('math.tau'), pyrun('file.py'), pyaddons().
"""
import math

import qcalcp

try:
    import numpy as np
except ImportError:
    np = None


def _each(x, f):
    """Apply f to a scalar, or element-wise through (nested) lists."""
    return [_each(v, f) for v in x] if isinstance(x, list) else f(x)


@qcalcp.function
def sinc(x):
    """Normalized sinc, sin(pi x) / (pi x), element-wise."""
    return _each(x, lambda v: 1.0 if v == 0 else math.sin(math.pi * v) / (math.pi * v))


@qcalcp.function
def minmax(v):
    """[lo, hi] = minmax(v): a tuple spreads into multiple outputs."""
    v = v if isinstance(v, list) else [v]
    return min(v), max(v)


@qcalcp.function("movavg")  # explicit Octave name
def moving_average(v, n):
    """Moving average over windows of n samples ("valid" part only). Uses numpy if present."""
    n = int(n)
    if np is not None:
        return np.convolve(v, np.ones(n) / n, "valid")  # numpy arrays convert via .tolist()
    return [sum(v[i:i + n]) / n for i in range(len(v) - n + 1)]


@qcalcp.function
def projectile(v0, angle_deg, g=9.81):
    """[range, height] = projectile(v0, angle_deg[, g]): flat ground, no drag, SI units."""
    a = math.radians(angle_deg)
    return v0**2 * math.sin(2 * a) / g, (v0 * math.sin(a)) ** 2 / (2 * g)


qcalcp.register("hypotn", math.hypot)  # any callable works: hypotn(3, 4, 12) == 13
