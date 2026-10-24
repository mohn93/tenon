"""Affine transforms in the API's ``{"matrix": [12 numbers]}`` form (column-major 3x4:
x axis, y axis, z axis, translation)."""
import math


def _sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def _dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _scale(a, k):
    return (a[0] * k, a[1] * k, a[2] * k)


def _norm(a):
    n = math.sqrt(_dot(a, a))
    if n < 1e-12:
        raise ValueError("a zero-length vector has no direction")
    return _scale(a, 1.0 / n)


def _vec3(v, what):
    v = tuple(float(x) for x in v)
    if len(v) != 3:
        raise ValueError("%s needs 3 numbers" % what)
    return v


class Transform:
    """A transform from 12 numbers (``m``): the columns x, y, z, then the translation.

    ``a @ b`` applies ``b`` first, then ``a``.
    """

    __slots__ = ("m",)

    def __init__(self, m):
        m = tuple(float(x) for x in m)
        if len(m) != 12:
            raise ValueError("a transform has 12 numbers (column-major 3x4)")
        self.m = m

    @classmethod
    def identity(cls):
        return cls((1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0))

    @classmethod
    def translate(cls, x, y=None, z=None):
        """``translate(x, y, z)`` or ``translate([x, y, z])``."""
        if y is None and z is None:
            x, y, z = _vec3(x, "an offset")
        return cls((1, 0, 0, 0, 1, 0, 0, 0, 1, x, y, z))

    @classmethod
    def scale(cls, sx, sy=None, sz=None):
        sy = sx if sy is None else sy
        sz = sx if sz is None else sz
        return cls((sx, 0, 0, 0, sy, 0, 0, 0, sz, 0, 0, 0))

    @classmethod
    def axis_angle(cls, axis, degrees, origin=(0, 0, 0)):
        """Turn by ``degrees`` counter-clockwise about the line through ``origin`` along ``axis``."""
        x, y, z = _norm(_vec3(axis, "an axis"))
        o = _vec3(origin, "an origin")
        t = math.radians(degrees)
        c, s = math.cos(t), math.sin(t)
        k = 1 - c
        # Rodrigues, columns are the images of the basis vectors
        cols = (
            (c + x * x * k, y * x * k + z * s, z * x * k - y * s),
            (x * y * k - z * s, c + y * y * k, z * y * k + x * s),
            (x * z * k + y * s, y * z * k - x * s, c + z * z * k),
        )
        r = cls(cols[0] + cols[1] + cols[2] + (0, 0, 0))
        # p -> R (p - o) + o
        ro = r.apply(o)
        return cls(r.m[:9] + _sub(o, ro))

    @classmethod
    def look(cls, origin, z_axis, x_hint=(1, 0, 0)):
        """A frame at ``origin`` whose z axis is ``z_axis`` and whose x axis is as near ``x_hint`` as
        the z axis allows (right-handed). Place a part built along +Z so it points along ``z_axis``."""
        z = _norm(_vec3(z_axis, "z_axis"))
        h = _vec3(x_hint, "x_hint")
        x = _sub(h, _scale(z, _dot(h, z)))
        if _dot(x, x) < 1e-12:  # hint parallel to z: pick any perpendicular
            alt = (0.0, 1.0, 0.0) if abs(z[0]) > 0.9 else (1.0, 0.0, 0.0)
            x = _sub(alt, _scale(z, _dot(alt, z)))
        x = _norm(x)
        y = _cross(z, x)
        return cls(x + y + z + _vec3(origin, "origin"))

    def apply(self, p):
        """Transform the point ``p``."""
        m = self.m
        return tuple(m[i] * p[0] + m[3 + i] * p[1] + m[6 + i] * p[2] + m[9 + i] for i in range(3))

    def __matmul__(self, other):
        if not isinstance(other, Transform):
            return NotImplemented
        a, b = self.m, other.m
        out = []
        for col in range(3):  # linear part: A * B columns
            bc = b[3 * col : 3 * col + 3]
            out += [a[i] * bc[0] + a[3 + i] * bc[1] + a[6 + i] * bc[2] for i in range(3)]
        out += list(self.apply(b[9:12]))
        return Transform(out)

    def to_json(self):
        return {"matrix": list(self.m)}

    def __eq__(self, other):
        return isinstance(other, Transform) and all(abs(a - b) < 1e-9 for a, b in zip(self.m, other.m))

    def __hash__(self):
        return hash(tuple(round(x, 9) for x in self.m))

    def __repr__(self):
        return "Transform(%r)" % (self.m,)
