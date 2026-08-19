# Cicada Python worker (stage 4, extended in stage 6): one process of the
# script-node pool.
#
# Protocol (docs/12, DECISIONS.md row 41 - MessagePack at the Python
# boundary): length-framed MessagePack messages over stdio. Every frame is
# a u32 little-endian byte length followed by one MessagePack value. The
# msgpack subset below is implemented inline so worker Pythons need NO
# packages installed (numpy etc. are the SCRIPT's business, not the
# protocol's). Supported: nil, bool, the full int family, float32/64,
# str, bin (8/16/32 - flat f64/u32 buffers ride as bin, never as arrays
# of floats), array, map.
#
# Requests (maps):
#   {"op": "describe", "path": <file>, "source": <text>}
#                                               -> {"ok": True, "python": sys.version, "nodes": [...]}
#   {"op": "invoke", "path": <file>, "source": <text>, "fn": <name>,
#    "inputs": {name: value}}                   -> {"ok": True, "outputs": [value, ...]}
#                                                  (one per declared output, declaration order;
#                                                   empty for `-> None`)
#   {"op": "ping"}                              -> {"ok": True, "python": sys.version}
# Any failure -> {"ok": False, "error": "<message with traceback tail>"}
#
# A node description: {"name", "title", "description", "effectful": bool,
# "inputs": [{"name", "type", "default": value-or-None}],
# "outputs": [{"name", "type"}]}.
#
# Values on the wire. Scalars and lists are tagged maps {"k": kind, "v": payload}:
#   Number (float), Integer (int), Boolean (bool), Text (str),
#   Point/Vector ([x, y, z]), Domain ([start, end]),
#   List ([value-or-None, ...])  -- None is an absent Optional slot.
# Geometry kinds (stage 6) are self-describing maps keyed by "kind":
#   {"kind": "Mesh", "positions": <bin f64 LE, 3n>, "indices": <bin u32 LE, 3m>}
#   {"kind": "Plane", "origin": [x,y,z], "x": [x,y,z], "y": [x,y,z]}
#   {"kind": "Curve", "curve": "polyline", "points": <bin f64 LE, 3n>, "closed": bool}
#   {"kind": "Curve", "curve": "line", "a": [x,y,z], "b": [x,y,z]}
#   {"kind": "Curve", "curve": "circle", "plane": <Plane map>, "radius": r}
#   {"kind": "Curve", "curve": "rectangle", "plane": <Plane map>, "x": [a,b], "y": [a,b]}
# Refinements (Closed<Curve>, Watertight<Mesh>) never appear on the wire:
# the value is the plain curve/mesh; the HOST re-checks the predicate on
# values coming back from a script whose output declares the refinement.
#
# Script files self-register with the decorator (doc 10 SS5):
#
#   import cicada
#   @cicada.node(title="Field Solve", description="inverse-square field.")
#   def solve_field(points: "[Point]", amps: "Number" = 12.0) -> "[Number]":
#       ...
#
# Port types are cicada catalog notation in STRING annotations - explicit
# and honest; inference from Python types would guess. The return
# annotation is exactly one of:
#   -> "<notation>"                  one output port named `out`
#   -> {"name": "<notation>", ...}   multi-output (dict literal, insertion
#                                    order = port order); the function MUST
#                                    return a dict with exactly these keys
#   -> None                          no outputs (effectful exporters); the
#                                    function returns None
# `@cicada.node(..., effectful=True)` marks a node the engine never
# memoizes or auto-runs (exporters).
#
# Python-side value types (the `cicada` module): a plain 3-tuple is a
# Point; cicada.Vector / cicada.Domain are namedtuples; cicada.Plane,
# cicada.Mesh (array('d') positions + array('I') indices; Mesh.from_triangles,
# .vertices, .triangles), cicada.Polyline, cicada.Line, cicada.Circle,
# cicada.Rectangle are the geometry carriers. A script may return any
# object with .positions/.indices arrays where a Mesh is declared; numpy
# arrays are accepted for positions/indices when numpy exists, never
# required.

import collections
import hashlib
import inspect
import itertools
import struct
import sys
import traceback
import types
from array import array

# ----------------------------------------------------------- msgpack ----

# The 4-byte unsigned typecode on this platform ('I' everywhere that
# matters; 'L' is the fallback where 'I' is not 4 bytes).
_U32 = next(code for code in ("I", "L") if array(code).itemsize == 4)
_BIG_ENDIAN = sys.byteorder == "big"


def _pack(value, out):
    # bool BEFORE int: bool is an int subclass in Python.
    if value is None:
        out.append(b"\xc0")
    elif value is True:
        out.append(b"\xc3")
    elif value is False:
        out.append(b"\xc2")
    elif isinstance(value, int):
        if not (-(2**63) <= value < 2**64):
            raise TypeError("integer %d does not fit msgpack int64/uint64" % value)
        if 0 <= value < 128:
            out.append(struct.pack("B", value))
        elif -32 <= value < 0:
            out.append(struct.pack("b", value))
        elif value >= 0:
            out.append(b"\xcf" + struct.pack(">Q", value))
        else:
            out.append(b"\xd3" + struct.pack(">q", value))
    elif isinstance(value, float):
        out.append(b"\xcb" + struct.pack(">d", value))
    elif isinstance(value, str):
        raw = value.encode("utf-8")
        n = len(raw)
        if n < 32:
            out.append(struct.pack("B", 0xA0 | n))
        elif n < 256:
            out.append(b"\xd9" + struct.pack("B", n))
        elif n < 65536:
            out.append(b"\xda" + struct.pack(">H", n))
        else:
            out.append(b"\xdb" + struct.pack(">I", n))
        out.append(raw)
    elif isinstance(value, (bytes, bytearray, memoryview, array)):
        # bin 8/16/32. Arrays and bytearrays go through a byte-view so
        # the join below copies them exactly once (no tobytes() detour).
        raw = value if isinstance(value, bytes) else memoryview(value).cast("B")
        n = len(raw)
        if n < 256:
            out.append(b"\xc4" + struct.pack("B", n))
        elif n < 65536:
            out.append(b"\xc5" + struct.pack(">H", n))
        elif n < 2**32:
            out.append(b"\xc6" + struct.pack(">I", n))
        else:
            raise TypeError("binary payload of %d bytes exceeds msgpack bin32" % n)
        out.append(raw)
    elif isinstance(value, (list, tuple)):
        n = len(value)
        if n < 16:
            out.append(struct.pack("B", 0x90 | n))
        elif n < 65536:
            out.append(b"\xdc" + struct.pack(">H", n))
        else:
            out.append(b"\xdd" + struct.pack(">I", n))
        for item in value:
            _pack(item, out)
    elif isinstance(value, dict):
        n = len(value)
        if n < 16:
            out.append(struct.pack("B", 0x80 | n))
        elif n < 65536:
            out.append(b"\xde" + struct.pack(">H", n))
        else:
            out.append(b"\xdf" + struct.pack(">I", n))
        for key, item in value.items():
            _pack(key, out)
            _pack(item, out)
    else:
        raise TypeError("unpackable value: %r" % (type(value),))


def pack(value):
    out = []
    _pack(value, out)
    return b"".join(out)


def _unpack(buf, at):
    tag = buf[at]
    at += 1
    if tag == 0xC0:
        return None, at
    if tag == 0xC2:
        return False, at
    if tag == 0xC3:
        return True, at
    if tag < 0x80:  # positive fixint
        return tag, at
    if tag >= 0xE0:  # negative fixint
        return tag - 256, at
    if 0xA0 <= tag < 0xC0:  # fixstr
        n = tag & 0x1F
        return buf[at : at + n].decode("utf-8"), at + n
    if 0x90 <= tag < 0xA0:  # fixarray
        return _unpack_seq(buf, at, tag & 0x0F)
    if 0x80 <= tag < 0x90:  # fixmap
        return _unpack_map(buf, at, tag & 0x0F)
    if tag == 0xCB:  # float64
        return struct.unpack_from(">d", buf, at)[0], at + 8
    if tag == 0xCA:  # float32 (rmpv never sends one today; complete anyway)
        return struct.unpack_from(">f", buf, at)[0], at + 4
    # The FULL integer family: rmpv always emits the most compact marker,
    # so uint8/16/32 and int8/16/32 arrive for everyday values.
    if tag == 0xCC:
        return buf[at], at + 1
    if tag == 0xCD:
        return struct.unpack_from(">H", buf, at)[0], at + 2
    if tag == 0xCE:
        return struct.unpack_from(">I", buf, at)[0], at + 4
    if tag == 0xCF:
        return struct.unpack_from(">Q", buf, at)[0], at + 8
    if tag == 0xD0:
        return struct.unpack_from(">b", buf, at)[0], at + 1
    if tag == 0xD1:
        return struct.unpack_from(">h", buf, at)[0], at + 2
    if tag == 0xD2:
        return struct.unpack_from(">i", buf, at)[0], at + 4
    if tag == 0xD3:
        return struct.unpack_from(">q", buf, at)[0], at + 8
    if tag == 0xD9:
        n = buf[at]
        return buf[at + 1 : at + 1 + n].decode("utf-8"), at + 1 + n
    if tag == 0xDA:
        n = struct.unpack_from(">H", buf, at)[0]
        return buf[at + 2 : at + 2 + n].decode("utf-8"), at + 2 + n
    if tag == 0xDB:
        n = struct.unpack_from(">I", buf, at)[0]
        return buf[at + 4 : at + 4 + n].decode("utf-8"), at + 4 + n
    # bin 8/16/32 -> bytes (a flat f64/u32 buffer; the geometry layer
    # reinterprets it with array.frombytes - no per-element Python work).
    if tag == 0xC4:
        n = buf[at]
        return bytes(buf[at + 1 : at + 1 + n]), at + 1 + n
    if tag == 0xC5:
        n = struct.unpack_from(">H", buf, at)[0]
        return bytes(buf[at + 2 : at + 2 + n]), at + 2 + n
    if tag == 0xC6:
        n = struct.unpack_from(">I", buf, at)[0]
        return bytes(buf[at + 4 : at + 4 + n]), at + 4 + n
    if tag == 0xDC:
        n = struct.unpack_from(">H", buf, at)[0]
        return _unpack_seq(buf, at + 2, n)
    if tag == 0xDD:
        n = struct.unpack_from(">I", buf, at)[0]
        return _unpack_seq(buf, at + 4, n)
    if tag == 0xDE:
        n = struct.unpack_from(">H", buf, at)[0]
        return _unpack_map(buf, at + 2, n)
    if tag == 0xDF:
        n = struct.unpack_from(">I", buf, at)[0]
        return _unpack_map(buf, at + 4, n)
    raise ValueError("unsupported msgpack tag 0x%02x" % tag)


def _unpack_seq(buf, at, n):
    items = []
    for _ in range(n):
        item, at = _unpack(buf, at)
        items.append(item)
    return items, at


def _unpack_map(buf, at, n):
    items = {}
    for _ in range(n):
        key, at = _unpack(buf, at)
        value, at = _unpack(buf, at)
        items[key] = value
    return items, at


def unpack(buf):
    value, at = _unpack(buf, 0)
    if at != len(buf):
        raise ValueError("trailing bytes in frame")
    return value


# ------------------------------------------------- the cicada module ----

# Distinct tuple types so Vector and Domain survive the boundary - a bare
# 3-tuple is a Point; these are not (Point/Vector conflation killed real
# wall time, docs/08).
Vector = collections.namedtuple("Vector", ["x", "y", "z"])
Domain = collections.namedtuple("Domain", ["start", "end"])


def _triple(value, what):
    """Any 3-sequence of numbers -> a tuple of 3 floats."""
    try:
        x, y, z = value
    except (TypeError, ValueError):
        raise TypeError("%s must be an (x, y, z) triple, got %r" % (what, value))
    return (float(x), float(y), float(z))


def _pair(value, what):
    if isinstance(value, Domain):
        return value
    try:
        a, b = value
    except (TypeError, ValueError):
        raise TypeError("%s must be an (a, b) pair or cicada.Domain, got %r" % (what, value))
    return Domain(float(a), float(b))


def _is_numpy(value):
    module = type(value).__module__
    return module == "numpy" or module.startswith("numpy.")


def _f64_array(value, what):
    """positions/points as array('d'): arrays pass through (no copy),
    numpy arrays and plain sequences convert."""
    if isinstance(value, array) and value.typecode == "d":
        return value
    if _is_numpy(value):
        out = array("d")
        out.frombytes(value.astype("<f8", copy=False).ravel().tobytes())
        if _BIG_ENDIAN:
            out.byteswap()
        return out
    try:
        return array("d", value)
    except TypeError:
        raise TypeError(
            "%s must be an array('d') or a flat sequence of numbers, got %r" % (what, type(value))
        )


def _u32_array(value, what):
    if isinstance(value, array) and value.itemsize == 4 and value.typecode in ("I", "L"):
        return value
    if _is_numpy(value):
        out = array(_U32)
        out.frombytes(value.astype("<u4", copy=False).ravel().tobytes())
        if _BIG_ENDIAN:
            out.byteswap()
        return out
    try:
        return array(_U32, value)  # OverflowError on negatives - loud
    except (TypeError, OverflowError) as error:
        raise TypeError(
            "%s must be an array('I') or a flat sequence of non-negative ints: %s" % (what, error)
        )


def _le_bytes(arr):
    """An array as little-endian bytes for the wire (a byte view on LE
    hosts - the packer copies it once into the frame)."""
    if _BIG_ENDIAN:
        swapped = array(arr.typecode, arr)
        swapped.byteswap()
        return swapped.tobytes()
    return arr


def _f64_from_bin(raw, what):
    if not isinstance(raw, (bytes, bytearray)):
        raise TypeError("%s must arrive as msgpack bin, got %r" % (what, type(raw)))
    if len(raw) % 8:
        raise ValueError("%s: %d bytes is not a whole number of f64" % (what, len(raw)))
    out = array("d")
    out.frombytes(raw)
    if _BIG_ENDIAN:
        out.byteswap()
    return out


def _u32_from_bin(raw, what):
    if not isinstance(raw, (bytes, bytearray)):
        raise TypeError("%s must arrive as msgpack bin, got %r" % (what, type(raw)))
    if len(raw) % 4:
        raise ValueError("%s: %d bytes is not a whole number of u32" % (what, len(raw)))
    out = array(_U32)
    out.frombytes(raw)
    if _BIG_ENDIAN:
        out.byteswap()
    return out


def _triples_of(flat):
    """A flat xyz array -> list of 3-tuples (C-speed slicing, no per-float
    Python loop)."""
    return list(zip(flat[0::3], flat[1::3], flat[2::3]))


def _flatten_triples(items, what):
    flat = array("d")
    try:
        flat.extend(itertools.chain.from_iterable(items))
    except TypeError:
        raise TypeError("%s must be a sequence of (x, y, z) triples" % what)
    if len(flat) % 3:
        raise TypeError(
            "%s must be a sequence of (x, y, z) triples (got %d coordinates)" % (what, len(flat))
        )
    return flat


class Mesh(object):
    """A triangle mesh: `positions` is an array('d') of [x0, y0, z0, x1, ...],
    `indices` an array('I') of [a0, b0, c0, a1, ...] (three per triangle,
    counter-clockwise = outward). Construct from flat arrays (kept by
    reference - no copy) or via Mesh.from_triangles(vertices, triangles)."""

    __slots__ = ("positions", "indices")

    def __init__(self, positions, indices):
        self.positions = _f64_array(positions, "Mesh positions")
        self.indices = _u32_array(indices, "Mesh indices")
        if len(self.positions) % 3:
            raise ValueError(
                "Mesh positions length %d is not a multiple of 3" % len(self.positions)
            )
        if len(self.indices) % 3:
            raise ValueError("Mesh indices length %d is not a multiple of 3" % len(self.indices))

    @classmethod
    def from_triangles(cls, vertices, triangles):
        """From a list of (x, y, z) vertices and a list of (i, j, k) triangles."""
        positions = _flatten_triples(vertices, "Mesh vertices")
        indices = array(_U32)
        try:
            indices.extend(itertools.chain.from_iterable(triangles))
        except (TypeError, OverflowError) as error:
            raise TypeError("Mesh triangles must be (i, j, k) index triples: %s" % error)
        return cls(positions, indices)

    @property
    def vertices(self):
        """The vertices as a list of (x, y, z) tuples (a copy)."""
        return _triples_of(self.positions)

    @property
    def triangles(self):
        """The triangles as a list of (i, j, k) tuples (a copy)."""
        return _triples_of(self.indices)

    @property
    def vertex_count(self):
        return len(self.positions) // 3

    @property
    def triangle_count(self):
        return len(self.indices) // 3

    def __repr__(self):
        return "cicada.Mesh(vertices=%d, triangles=%d)" % (self.vertex_count, self.triangle_count)


class Plane(object):
    """An oriented frame: origin (Point 3-tuple) plus x/y axes (cicada.Vector);
    z = x cross y is derived. Stored as given - normalization is the
    engine's business (construct_plane)."""

    __slots__ = ("origin", "x", "y")

    def __init__(self, origin, x, y):
        self.origin = _triple(origin, "Plane origin")
        self.x = Vector(*_triple(x, "Plane x axis"))
        self.y = Vector(*_triple(y, "Plane y axis"))

    def __eq__(self, other):
        return isinstance(other, Plane) and (self.origin, self.x, self.y) == (
            other.origin,
            other.x,
            other.y,
        )

    def __hash__(self):
        return hash((self.origin, self.x, self.y))

    def __repr__(self):
        return "cicada.Plane(origin=%r, x=%r, y=%r)" % (self.origin, self.x, self.y)


class Polyline(object):
    """A vertex chain: `points` is a list of (x, y, z) tuples; `closed`
    marks the implicit final edge back to points[0] (the closing vertex
    is NOT repeated)."""

    __slots__ = ("points", "closed")

    def __init__(self, points, closed=False):
        if isinstance(points, array) and points.typecode == "d":
            if len(points) % 3:
                raise ValueError(
                    "Polyline points array length %d is not a multiple of 3" % len(points)
                )
            self.points = _triples_of(points)
        else:
            self.points = [_triple(p, "Polyline point") for p in points]
        self.closed = bool(closed)

    def __eq__(self, other):
        return isinstance(other, Polyline) and (self.points, self.closed) == (
            other.points,
            other.closed,
        )

    def __repr__(self):
        return "cicada.Polyline(%d points, closed=%r)" % (len(self.points), self.closed)


class Line(object):
    """A straight segment from a to b (Point 3-tuples)."""

    __slots__ = ("a", "b")

    def __init__(self, a, b):
        self.a = _triple(a, "Line a")
        self.b = _triple(b, "Line b")

    def __eq__(self, other):
        return isinstance(other, Line) and (self.a, self.b) == (other.a, other.b)

    def __repr__(self):
        return "cicada.Line(%r, %r)" % (self.a, self.b)


class Circle(object):
    """An analytic circle: plane.origin is the center, the radius sweeps
    the plane's x/y axes."""

    __slots__ = ("plane", "radius")

    def __init__(self, plane, radius):
        if not isinstance(plane, Plane):
            raise TypeError("Circle plane must be a cicada.Plane, got %r" % type(plane))
        self.plane = plane
        self.radius = float(radius)

    def __eq__(self, other):
        return isinstance(other, Circle) and (self.plane, self.radius) == (
            other.plane,
            other.radius,
        )

    def __repr__(self):
        return "cicada.Circle(%r, radius=%r)" % (self.plane, self.radius)


class Rectangle(object):
    """An analytic rectangle in a plane: corners span x by y in plane
    coordinates (cicada.Domain each; (a, b) pairs accepted). Always closed."""

    __slots__ = ("plane", "x", "y")

    def __init__(self, plane, x, y):
        if not isinstance(plane, Plane):
            raise TypeError("Rectangle plane must be a cicada.Plane, got %r" % type(plane))
        self.plane = plane
        self.x = _pair(x, "Rectangle x")
        self.y = _pair(y, "Rectangle y")

    def __eq__(self, other):
        return isinstance(other, Rectangle) and (self.plane, self.x, self.y) == (
            other.plane,
            other.x,
            other.y,
        )

    def __repr__(self):
        return "cicada.Rectangle(%r, x=%r, y=%r)" % (self.plane, self.x, self.y)


class _CicadaModule(object):
    """The `cicada` module scripts import: the @node decorator, plus the
    boundary types (cicada.Vector, cicada.Domain, cicada.Plane,
    cicada.Mesh, cicada.Polyline, cicada.Line, cicada.Circle,
    cicada.Rectangle; a plain 3-tuple is a Point)."""

    Vector = Vector
    Domain = Domain
    Plane = Plane
    Mesh = Mesh
    Polyline = Polyline
    Line = Line
    Circle = Circle
    Rectangle = Rectangle

    def node(self, title, description, effectful=False):
        # Script nodes are PURE by contract (docs/08 rule 1) unless they
        # say otherwise: identical inputs give identical outputs and the
        # engine memoizes results. `effectful=True` (exporters) opts out:
        # the engine never memoizes or auto-runs the node - running it is
        # an explicit action (doc 10 SS7), so its effect never silently
        # skips on a warm run.
        def wrap(fn):
            fn.__cicada_node__ = {
                "title": title,
                "description": description,
                "effectful": bool(effectful),
            }
            return fn

        return wrap


_CICADA = _CicadaModule()
_MODULES = {}


def _load(path, source):
    # Compile the SOURCE TEXT the HOST sent - never read the file, never
    # the import machinery. The engine owns caching (source-hash NodeKeys):
    # the executed bytes are the hashed bytes BY CONSTRUCTION (no
    # time-of-check/time-of-use gap against concurrent edits), and
    # Python's .pyc cache (which validates by whole-second mtime + size
    # and can serve stale bytecode after a same-size edit) never enters
    # the picture. The module cache is keyed by content, so a long-lived
    # worker serves edited scripts correctly too. `path` is diagnostics
    # only (compile filename for tracebacks).
    key = (path, hashlib.sha256(source.encode("utf-8")).hexdigest())
    if key in _MODULES:
        return _MODULES[key]
    sys.modules["cicada"] = _CICADA
    module = types.ModuleType("cicada_script_%d" % len(_MODULES))
    module.__file__ = path
    exec(compile(source, path, "exec"), module.__dict__)
    _MODULES[key] = module
    return module


def _outputs_of(fn, name):
    """The declared outputs of a node function, from its return annotation:
    (form, [(port name, type notation), ...]) with form one of "none"
    (`-> None`), "single" (`-> "notation"`, port `out`) or "multi" (a dict
    literal). One source of truth for describe AND invoke (a worker that
    never described a script still validates its return shape)."""
    returns = inspect.signature(fn).return_annotation
    if returns is None:
        return ("none", [])
    if isinstance(returns, str):
        return ("single", [("out", returns)])
    if isinstance(returns, dict):
        if not returns:
            raise TypeError(
                "`%s` declares an empty output dict; declare `-> None` for a node "
                "without outputs" % name
            )
        outputs = []
        for port, notation in returns.items():
            if not isinstance(port, str) or not port.isidentifier():
                raise TypeError("`%s`: output port name %r is not an identifier" % (name, port))
            if not isinstance(notation, str):
                raise TypeError(
                    '`%s`: output `%s` needs a cicada type notation string (like "[Point]"), '
                    "got %r" % (name, port, notation)
                )
            outputs.append((port, notation))
        return ("multi", outputs)
    raise TypeError(
        '`%s` needs a cicada return annotation: a notation string like "[Number]" '
        '(one output `out`), a dict literal {"name": "notation", ...} (multi-output), '
        "or None (no outputs)" % name
    )


def _describe(path, source):
    module = _load(path, source)
    nodes = []
    for name, fn in sorted(vars(module).items()):
        meta = getattr(fn, "__cicada_node__", None)
        if meta is None:
            continue
        signature = inspect.signature(fn)
        inputs = []
        for parameter in signature.parameters.values():
            annotation = parameter.annotation
            if not isinstance(annotation, str):
                raise TypeError(
                    "port `%s` of `%s` needs a cicada type annotation "
                    '(a string like "[Point]" or "Number")' % (parameter.name, name)
                )
            default = None
            if parameter.default is not inspect.Parameter.empty:
                default = _to_wire(parameter.default)
            inputs.append({"name": parameter.name, "type": annotation, "default": default})
        _, outputs = _outputs_of(fn, name)
        nodes.append(
            {
                "name": name,
                "title": meta["title"],
                "description": meta["description"],
                "effectful": meta["effectful"],
                "inputs": inputs,
                "outputs": [{"name": port, "type": notation} for port, notation in outputs],
            }
        )
    if not nodes:
        raise ValueError("no @cicada.node functions in %s" % path)
    return nodes


# ------------------------------------------------- value marshalling ----


def _from_wire(value):
    if value is None:
        return None
    kind = value.get("k")
    if kind is None:
        return _geometry_from_wire(value)
    payload = value["v"]
    if kind in ("Number", "Integer", "Boolean", "Text"):
        return payload
    if kind == "Point":
        return tuple(payload)
    if kind == "Vector":
        return Vector(*payload)
    if kind == "Domain":
        return Domain(*payload)
    if kind == "List":
        return [_from_wire(item) for item in payload]
    raise TypeError("kind `%s` has no Python mapping" % kind)


def _geometry_from_wire(value):
    kind = value.get("kind")
    if kind == "Mesh":
        mesh = Mesh.__new__(Mesh)
        mesh.positions = _f64_from_bin(value["positions"], "Mesh positions")
        mesh.indices = _u32_from_bin(value["indices"], "Mesh indices")
        return mesh
    if kind == "Plane":
        return Plane(value["origin"], value["x"], value["y"])
    if kind == "Curve":
        variant = value.get("curve")
        if variant == "polyline":
            return Polyline(_f64_from_bin(value["points"], "Polyline points"), value["closed"])
        if variant == "line":
            return Line(value["a"], value["b"])
        if variant == "circle":
            return Circle(_geometry_from_wire(value["plane"]), value["radius"])
        if variant == "rectangle":
            return Rectangle(_geometry_from_wire(value["plane"]), value["x"], value["y"])
        raise TypeError("curve variant %r has no Python mapping (tessellate first)" % (variant,))
    raise TypeError("wire value %r has no Python mapping" % (kind,))


def _plane_to_wire(plane):
    return {
        "kind": "Plane",
        "origin": list(plane.origin),
        "x": list(plane.x),
        "y": list(plane.y),
    }


def _mesh_to_wire(positions, indices):
    mesh = Mesh(positions, indices)  # normalizes arrays; no copy for array inputs
    return {
        "kind": "Mesh",
        "positions": _le_bytes(mesh.positions),
        "indices": _le_bytes(mesh.indices),
    }


def _to_wire(value):
    # Type-check order matters: bool < int (subclass), and the cicada
    # Vector/Domain namedtuples < tuple (subclass).
    if isinstance(value, bool):
        return {"k": "Boolean", "v": value}
    if isinstance(value, int):
        return {"k": "Integer", "v": value}
    if isinstance(value, float):
        return {"k": "Number", "v": value}
    if isinstance(value, str):
        return {"k": "Text", "v": value}
    if isinstance(value, Vector):
        return {"k": "Vector", "v": [float(c) for c in value]}
    if isinstance(value, Domain):
        return {"k": "Domain", "v": [float(c) for c in value]}
    if isinstance(value, Plane):
        return _plane_to_wire(value)
    if isinstance(value, Mesh):
        return _mesh_to_wire(value.positions, value.indices)
    if isinstance(value, Polyline):
        return {
            "kind": "Curve",
            "curve": "polyline",
            "points": _le_bytes(_flatten_triples(value.points, "Polyline points")),
            "closed": value.closed,
        }
    if isinstance(value, Line):
        return {"kind": "Curve", "curve": "line", "a": list(value.a), "b": list(value.b)}
    if isinstance(value, Circle):
        return {
            "kind": "Curve",
            "curve": "circle",
            "plane": _plane_to_wire(value.plane),
            "radius": value.radius,
        }
    if isinstance(value, Rectangle):
        return {
            "kind": "Curve",
            "curve": "rectangle",
            "plane": _plane_to_wire(value.plane),
            "x": [value.x.start, value.x.end],
            "y": [value.y.start, value.y.end],
        }
    if isinstance(value, tuple) and len(value) == 3:
        return {"k": "Point", "v": [float(c) for c in value]}
    if isinstance(value, list):
        return {"k": "List", "v": [None if v is None else _to_wire(v) for v in value]}
    if isinstance(value, array):
        # A flat numeric array where a list is declared: numbers or
        # integers by typecode.
        if value.typecode in ("d", "f"):
            return {"k": "List", "v": [{"k": "Number", "v": float(v)} for v in value]}
        return {"k": "List", "v": [{"k": "Integer", "v": int(v)} for v in value]}
    # Duck-typed meshes: any object with .positions/.indices arrays.
    if hasattr(value, "positions") and hasattr(value, "indices"):
        return _mesh_to_wire(value.positions, value.indices)
    # Numpy arrays and scalars quack like sequences/floats; coerce
    # explicitly so scripts can return them directly (numpy is present by
    # construction when such a value exists - it made the value).
    if _is_numpy(value):
        import numpy

        if isinstance(value, numpy.ndarray):
            return {"k": "List", "v": [_to_wire(v) for v in value.tolist()]}
        if isinstance(value, numpy.floating):
            return {"k": "Number", "v": float(value)}
        if isinstance(value, numpy.integer):
            return {"k": "Integer", "v": int(value)}
        if isinstance(value, numpy.bool_):
            return {"k": "Boolean", "v": bool(value)}
    raise TypeError("script returned unmarshallable %r" % (type(value),))


def _output_to_wire(value, fn_name, port):
    try:
        return _to_wire(value)
    except (TypeError, ValueError, OverflowError) as error:
        raise TypeError("`%s` output `%s`: %s" % (fn_name, port, error))


def _invoke(path, source, fn_name, inputs):
    module = _load(path, source)
    fn = getattr(module, fn_name, None)
    if fn is None or not hasattr(fn, "__cicada_node__"):
        raise NameError("no @cicada.node function `%s` in %s" % (fn_name, path))
    kwargs = {name: _from_wire(value) for name, value in inputs.items()}
    result = fn(**kwargs)
    form, outputs = _outputs_of(fn, fn_name)
    if form == "none":
        # `-> None`: the function's work IS its effect; a value here is a
        # declaration bug, refused rather than dropped on the floor.
        if result is not None:
            raise TypeError(
                "`%s` is declared `-> None` (no outputs) but returned %r"
                % (fn_name, type(result))
            )
        return []
    if form == "single":
        return [_output_to_wire(result, fn_name, "out")]
    declared = [port for port, _ in outputs]
    if not isinstance(result, dict):
        raise TypeError(
            "`%s` declares outputs {%s} and must return a dict with exactly those keys, not %r"
            % (fn_name, ", ".join(declared), type(result))
        )
    missing = [port for port in declared if port not in result]
    extra = [key for key in result if key not in declared]
    if missing or extra:
        raise TypeError(
            "`%s` returned a dict whose keys do not match its declared outputs {%s}: "
            "%d missing [%s], %d extra [%s]"
            % (
                fn_name,
                ", ".join(declared),
                len(missing),
                ", ".join(missing),
                len(extra),
                ", ".join(str(key) for key in extra),
            )
        )
    return [_output_to_wire(result[port], fn_name, port) for port in declared]


# ----------------------------------------------------------- the loop ----


def _read_frame_bytes(stream):
    # Length framing is the OUTER protocol: read the whole body before any
    # decode, so a malformed body never desynchronizes the stream - the
    # loop can answer with an error frame and keep serving.
    header = stream.read(4)
    if len(header) < 4:
        return None
    (length,) = struct.unpack("<I", header)
    body = stream.read(length)
    if len(body) < length:
        return None
    return body


def _write_frame(stream, value):
    body = pack(value)
    stream.write(struct.pack("<I", len(body)))
    stream.write(body)
    stream.flush()


def _handle(body):
    """Decode + dispatch one request body. EVERY failure - decode,
    dispatch, or marshalling - becomes an error RESPONSE; the worker dies
    only when the host closes the pipe."""
    try:
        request = unpack(body)
        op = request.get("op")
        if op == "ping":
            return {"ok": True, "python": sys.version}
        if op == "describe":
            return {
                "ok": True,
                "python": sys.version,
                "nodes": _describe(request["path"], request["source"]),
            }
        if op == "invoke":
            return {
                "ok": True,
                "outputs": _invoke(
                    request["path"],
                    request["source"],
                    request["fn"],
                    request.get("inputs", {}),
                ),
            }
        return {"ok": False, "error": "unknown op %r" % (op,)}
    except BaseException:
        return {"ok": False, "error": traceback.format_exc(limit=8)}


def main():
    stdin = sys.stdin.buffer
    stdout = sys.stdout.buffer
    while True:
        body = _read_frame_bytes(stdin)
        if body is None:
            return  # host closed the pipe; exit quietly
        response = _handle(body)
        try:
            _write_frame(stdout, response)
        except BaseException:
            # The RESPONSE failed to pack (marshalling gap): degrade to an
            # error frame carrying the reason instead of dying silently.
            _write_frame(
                stdout,
                {"ok": False, "error": traceback.format_exc(limit=8)},
            )


if __name__ == "__main__":
    main()
