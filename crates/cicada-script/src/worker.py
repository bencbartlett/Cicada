# Cicada Python worker (stage 4): one process of the script-node pool.
#
# Protocol (docs/12, DECISIONS.md row 41 - MessagePack at the Python
# boundary): length-framed MessagePack messages over stdio. Every frame is
# a u32 little-endian byte length followed by one MessagePack value. The
# msgpack subset below is implemented inline so worker Pythons need NO
# packages installed (numpy etc. are the SCRIPT's business, not the
# protocol's).
#
# Requests (maps):
#   {"op": "describe", "path": <file>}          -> {"ok": True, "nodes": [...]}
#   {"op": "invoke", "path": <file>, "fn": <name>, "inputs": {name: value}}
#                                               -> {"ok": True, "output": value}
#   {"op": "ping"}                              -> {"ok": True}
# Any failure -> {"ok": False, "error": "<message with traceback tail>"}
#
# Values are tagged maps {"k": kind, "v": payload}:
#   Number (float), Integer (int), Boolean (bool), Text (str),
#   Point/Vector ([x, y, z]), Domain ([start, end]),
#   List ([value-or-None, ...])  -- None is an absent Optional slot.
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
# annotation types the single output port `out`.

import hashlib
import inspect
import struct
import sys
import traceback
import types

# ----------------------------------------------------------- msgpack ----


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


import collections

# Distinct tuple types so Vector and Domain survive the boundary — a bare
# 3-tuple is a Point; these are not (Point/Vector conflation killed real
# wall time, docs/08).
Vector = collections.namedtuple("Vector", ["x", "y", "z"])
Domain = collections.namedtuple("Domain", ["start", "end"])


class _CicadaModule:
    """The `cicada` module scripts import: the @node decorator registry,
    plus the boundary types (cicada.Vector, cicada.Domain; a plain 3-tuple
    is a Point)."""

    Vector = Vector
    Domain = Domain

    def __init__(self):
        self.registered = []

    def node(self, title, description):
        # Script nodes are PURE by contract (docs/08 rule 1): identical
        # inputs must give identical outputs, and results are memoized by
        # the engine — a side-effectful script would have its effect
        # silently skipped on warm runs. Effectful script nodes (the
        # ported exporters) arrive with stage 6 and will declare it.
        def wrap(fn):
            fn.__cicada_node__ = {"title": title, "description": description}
            return fn

        return wrap


_CICADA = _CicadaModule()
_MODULES = {}


def _load(path, source):
    # Compile the SOURCE TEXT the HOST sent — never read the file, never
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
            inputs.append(
                {"name": parameter.name, "type": annotation, "default": default}
            )
        returns = signature.return_annotation
        if not isinstance(returns, str):
            raise TypeError(
                "`%s` needs a cicada return annotation "
                '(a string like "[Number]")' % name
            )
        nodes.append(
            {
                "name": name,
                "title": meta["title"],
                "description": meta["description"],
                "inputs": inputs,
                "output": returns,
            }
        )
    if not nodes:
        raise ValueError("no @cicada.node functions in %s" % path)
    return nodes


# ------------------------------------------------- value marshalling ----


def _from_wire(value):
    if value is None:
        return None
    kind = value["k"]
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
    raise TypeError("kind `%s` has no Python mapping (stage-4 subset)" % kind)


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
    if isinstance(value, tuple) and len(value) == 3:
        return {"k": "Point", "v": [float(c) for c in value]}
    if isinstance(value, (list,)):
        return {"k": "List", "v": [None if v is None else _to_wire(v) for v in value]}
    # Numpy arrays and scalars quack like sequences/floats; coerce
    # explicitly so scripts can return them directly.
    try:
        import numpy

        if isinstance(value, numpy.ndarray):
            return {"k": "List", "v": [_to_wire(float(v)) for v in value.tolist()]}
        if isinstance(value, numpy.floating):
            return {"k": "Number", "v": float(value)}
        if isinstance(value, numpy.integer):
            return {"k": "Integer", "v": int(value)}
    except ImportError:
        pass
    raise TypeError("script returned unmarshallable %r" % (type(value),))


def _invoke(path, source, fn_name, inputs):
    module = _load(path, source)
    fn = getattr(module, fn_name, None)
    if fn is None or not hasattr(fn, "__cicada_node__"):
        raise NameError("no @cicada.node function `%s` in %s" % (fn_name, path))
    kwargs = {name: _from_wire(value) for name, value in inputs.items()}
    return _to_wire(fn(**kwargs))


# ----------------------------------------------------------- the loop ----


def _read_frame_bytes(stream):
    # Length framing is the OUTER protocol: read the whole body before any
    # decode, so a malformed body never desynchronizes the stream — the
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
    """Decode + dispatch one request body. EVERY failure — decode,
    dispatch, or marshalling — becomes an error RESPONSE; the worker dies
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
                "output": _invoke(
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
