"""Offline stand-in for the `cicada` module the Python worker injects.

The corpus scripts (corpus/scripts/*.py) start with `import cicada` and
are normally executed by the engine's worker (crates/cicada-script/src/
worker.py), which provides the @cicada.node decorator and the boundary
types of the stage-6 script ABI (frozen contract section 1):

    cicada.node(title, description, effectful=False)
    cicada.Mesh(positions: array('d'), indices: array('I'))
        .from_triangles(vertices, triangles) / .vertices / .triangles
    cicada.Plane(origin, x, y)      cicada.Polyline(points, closed)
    cicada.Line(a, b)               cicada.Circle(plane, radius)
    cicada.Rectangle(plane, x, y)   cicada.Vector(x, y, z)
    cicada.Domain(start, end)       (a plain 3-tuple is a Point)

The unit tests run WITHOUT the engine, so they install this module as
`sys.modules["cicada"]` before importing a script (see test_support.py).
It mirrors the contract's surface only — no marshalling, no protocol.
"""

import array
import collections

Vector = collections.namedtuple("Vector", ["x", "y", "z"])
Domain = collections.namedtuple("Domain", ["start", "end"])
Plane = collections.namedtuple("Plane", ["origin", "x", "y"])
Polyline = collections.namedtuple("Polyline", ["points", "closed"])
Line = collections.namedtuple("Line", ["a", "b"])
Circle = collections.namedtuple("Circle", ["plane", "radius"])
Rectangle = collections.namedtuple("Rectangle", ["plane", "x", "y"])


class Mesh(object):
    """Flat SoA mesh: positions = array('d') of 3n floats, indices =
    array('I') of 3m vertex indices (three per triangle)."""

    def __init__(self, positions, indices):
        if not isinstance(positions, array.array) or positions.typecode != "d":
            raise TypeError("Mesh.positions must be array('d')")
        if not isinstance(indices, array.array) or indices.typecode != "I":
            raise TypeError("Mesh.indices must be array('I')")
        if len(positions) % 3 != 0:
            raise ValueError("Mesh.positions length %d is not a multiple of 3" % len(positions))
        if len(indices) % 3 != 0:
            raise ValueError("Mesh.indices length %d is not a multiple of 3" % len(indices))
        self.positions = positions
        self.indices = indices

    @classmethod
    def from_triangles(cls, vertices, triangles):
        pos = array.array("d")
        for (x, y, z) in vertices:
            pos.append(float(x))
            pos.append(float(y))
            pos.append(float(z))
        idx = array.array("I")
        for (a, b, c) in triangles:
            idx.append(int(a))
            idx.append(int(b))
            idx.append(int(c))
        return cls(pos, idx)

    @property
    def vertices(self):
        p = self.positions
        return [(p[3 * i], p[3 * i + 1], p[3 * i + 2]) for i in range(len(p) // 3)]

    @property
    def triangles(self):
        t = self.indices
        return [(t[3 * i], t[3 * i + 1], t[3 * i + 2]) for i in range(len(t) // 3)]


def node(title, description, effectful=False):
    """The decorator: records the node metadata on the function (the real
    worker does the same, then reads the signature for the ports)."""

    def wrap(fn):
        fn.__cicada_node__ = {
            "title": title,
            "description": description,
            "effectful": bool(effectful),
        }
        return fn

    return wrap
