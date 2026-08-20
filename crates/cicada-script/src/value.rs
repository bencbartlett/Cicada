//! Value marshalling across the Python boundary over `MessagePack`.
//!
//! Two map shapes travel the wire (the worker's `_to_wire`/`_from_wire`
//! are the mirror — `worker.py` header):
//!
//! - **Scalars and lists** (stage 4) are tagged `{"k": kind, "v": payload}`
//!   maps: Number, Integer, Boolean, Text, Point, Vector, Domain, List
//!   (holes = `MessagePack` nil).
//! - **Geometry** (stage 6) is a self-describing map keyed by `"kind"`,
//!   with the flat `f64`/`u32` buffers as `MessagePack` **bin** (little-
//!   endian, never arrays of floats — a 7,200-mesh list must cross in
//!   well under a second, so no per-float work on either side):
//!   - `{"kind":"Mesh", "positions": bin f64 LE (3n), "indices": bin u32 LE (3m)}`
//!   - `{"kind":"Plane", "origin":[x,y,z], "x":[x,y,z], "y":[x,y,z]}`
//!   - `{"kind":"Curve", "curve":"polyline", "points": bin f64 LE (3n), "closed": bool}`
//!   - `{"kind":"Curve", "curve":"line", "a":[x,y,z], "b":[x,y,z]}`
//!   - `{"kind":"Curve", "curve":"circle", "plane": <Plane map>, "radius": r}`
//!   - `{"kind":"Curve", "curve":"rectangle", "plane": <Plane map>, "x":[a,b], "y":[a,b]}`
//!
//! Refinements (`Closed<Curve>`, `Watertight<Mesh>`) are port-type-level:
//! the wire carries the plain curve/mesh (same as every other boundary,
//! core `marshal`); the script host re-checks the predicate on values
//! coming back from Python where the output declares the refinement.
//! Every other kind (Color, `IndexMap`, Xform, Nothing) refuses loudly.

use std::sync::Arc;

use cicada_core::geometry::{Circle, Curve, Line, Mesh, Polyline, Rectangle};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Point, Vector};
use cicada_core::value::{HashedValue, List, ValueData};
use rmpv::Value as Wire;

use crate::ScriptError;

fn tagged(kind: &str, payload: Wire) -> Wire {
    Wire::Map(vec![
        (Wire::from("k"), Wire::from(kind)),
        (Wire::from("v"), payload),
    ])
}

fn triple(x: f64, y: f64, z: f64) -> Wire {
    Wire::Array(vec![Wire::F64(x), Wire::F64(y), Wire::F64(z)])
}

fn pair(a: f64, b: f64) -> Wire {
    Wire::Array(vec![Wire::F64(a), Wire::F64(b)])
}

fn entry(key: &str, value: Wire) -> (Wire, Wire) {
    (Wire::from(key), value)
}

/// A flat `f64` buffer as little-endian bin.
fn f64_bin(values: &[f64]) -> Wire {
    let mut bytes = Vec::with_capacity(values.len() * 8);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Wire::Binary(bytes)
}

/// A flat `u32` buffer as little-endian bin.
fn u32_bin(values: &[u32]) -> Wire {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Wire::Binary(bytes)
}

fn plane_wire(plane: &Plane) -> Wire {
    Wire::Map(vec![
        entry("kind", Wire::from("Plane")),
        entry(
            "origin",
            triple(plane.origin.0.x, plane.origin.0.y, plane.origin.0.z),
        ),
        entry("x", triple(plane.x.0.x, plane.x.0.y, plane.x.0.z)),
        entry("y", triple(plane.y.0.x, plane.y.0.y, plane.y.0.z)),
    ])
}

fn curve_wire(curve: &Curve) -> Wire {
    // Exhaustive on purpose: a future analytic variant (Arc, Ellipse,
    // Nurbs — v0.1) makes this a compile error, forcing the explicit
    // decision the contract asks for (marshal it, or refuse loudly with
    // "tessellate first") instead of a silent fall-through.
    let mut map = vec![entry("kind", Wire::from("Curve"))];
    match curve {
        Curve::Polyline(polyline) => {
            let mut flat = Vec::with_capacity(polyline.vertices.len() * 3);
            for vertex in &polyline.vertices {
                flat.extend_from_slice(&[vertex.0.x, vertex.0.y, vertex.0.z]);
            }
            map.push(entry("curve", Wire::from("polyline")));
            map.push(entry("points", f64_bin(&flat)));
            map.push(entry("closed", Wire::Boolean(polyline.closed)));
        }
        Curve::Line(line) => {
            map.push(entry("curve", Wire::from("line")));
            map.push(entry("a", triple(line.a.0.x, line.a.0.y, line.a.0.z)));
            map.push(entry("b", triple(line.b.0.x, line.b.0.y, line.b.0.z)));
        }
        Curve::Circle(circle) => {
            map.push(entry("curve", Wire::from("circle")));
            map.push(entry("plane", plane_wire(&circle.plane)));
            map.push(entry("radius", Wire::F64(circle.radius)));
        }
        Curve::Rectangle(rectangle) => {
            map.push(entry("curve", Wire::from("rectangle")));
            map.push(entry("plane", plane_wire(&rectangle.plane)));
            map.push(entry("x", pair(rectangle.x.start, rectangle.x.end)));
            map.push(entry("y", pair(rectangle.y.start, rectangle.y.end)));
        }
    }
    Wire::Map(map)
}

fn mesh_wire(mesh: &Mesh) -> Wire {
    Wire::Map(vec![
        entry("kind", Wire::from("Mesh")),
        entry("positions", f64_bin(mesh.positions())),
        entry("indices", u32_bin(mesh.indices())),
    ])
}

/// A Cicada value onto the wire.
///
/// # Errors
///
/// [`ScriptError::Marshal`] for kinds outside the marshallable set
/// (Color, `IndexMap`, Xform, Nothing).
pub fn to_wire(value: &HashedValue) -> Result<Wire, ScriptError> {
    Ok(match value.data() {
        ValueData::Number(x) => tagged("Number", Wire::F64(*x)),
        ValueData::Integer(i) => tagged("Integer", Wire::from(*i)),
        ValueData::Boolean(b) => tagged("Boolean", Wire::Boolean(*b)),
        ValueData::Text(s) => tagged("Text", Wire::from(s.as_ref())),
        ValueData::Point(p) => tagged("Point", triple(p.0.x, p.0.y, p.0.z)),
        ValueData::Vector(v) => tagged("Vector", triple(v.0.x, v.0.y, v.0.z)),
        ValueData::Domain(d) => tagged("Domain", pair(d.start, d.end)),
        ValueData::Plane(plane) => plane_wire(plane),
        ValueData::Curve(curve) => curve_wire(curve),
        ValueData::Mesh(mesh) => mesh_wire(mesh),
        ValueData::List(list) => {
            let mut items = Vec::with_capacity(list.slots.len());
            for slot in &list.slots {
                items.push(match slot {
                    None => Wire::Nil,
                    Some(element) => to_wire(element)?,
                });
            }
            tagged("List", Wire::Array(items))
        }
        other => {
            return Err(ScriptError::Marshal(format!(
                "kind `{}` does not cross the Python boundary (marshallable: Number, \
                 Integer, Boolean, Text, Point, Vector, Domain, Plane, Curve, Mesh, List)",
                other.kind_name()
            )));
        }
    })
}

fn wire_str(value: &Wire) -> Option<&str> {
    value.as_str()
}

fn map_get<'w>(map: &'w [(Wire, Wire)], key: &str) -> Option<&'w Wire> {
    map.iter()
        .find(|(k, _)| wire_str(k) == Some(key))
        .map(|(_, v)| v)
}

fn map_field<'w>(map: &'w [(Wire, Wire)], key: &str, what: &str) -> Result<&'w Wire, ScriptError> {
    map_get(map, key).ok_or_else(|| ScriptError::Marshal(format!("{what} map has no `{key}`")))
}

fn wire_f64(value: &Wire, what: &str) -> Result<f64, ScriptError> {
    // Python ints appearing where floats are expected widen EXACTLY (the
    // marshal layer's one sanctioned widening, mirroring core). Never
    // rmpv's as_f64 for integers — it casts unconditionally and would
    // silently shift values beyond 2^53.
    match value {
        Wire::F64(x) => Ok(*x),
        Wire::F32(x) => Ok(f64::from(*x)),
        Wire::Integer(_) => {
            let i = value.as_i64().ok_or_else(|| {
                ScriptError::Marshal(format!("{what}: integer exceeds i64 range"))
            })?;
            cicada_core::marshal::integer_to_number_exact(i).ok_or_else(|| {
                ScriptError::Marshal(format!(
                    "{what}: integer {i} does not convert exactly to Number"
                ))
            })
        }
        other => Err(ScriptError::Marshal(format!(
            "{what} is not a number (got {other})"
        ))),
    }
}

fn wire_bool(value: &Wire, what: &str) -> Result<bool, ScriptError> {
    value
        .as_bool()
        .ok_or_else(|| ScriptError::Marshal(format!("{what} is not a bool (got {value})")))
}

fn wire_triple(value: &Wire, what: &str) -> Result<[f64; 3], ScriptError> {
    let Wire::Array(items) = value else {
        return Err(ScriptError::Marshal(format!("{what} is not an array")));
    };
    if items.len() != 3 {
        return Err(ScriptError::Marshal(format!(
            "{what} has {} components (need 3)",
            items.len()
        )));
    }
    Ok([
        wire_f64(&items[0], what)?,
        wire_f64(&items[1], what)?,
        wire_f64(&items[2], what)?,
    ])
}

fn wire_pair(value: &Wire, what: &str) -> Result<[f64; 2], ScriptError> {
    let Wire::Array(items) = value else {
        return Err(ScriptError::Marshal(format!("{what} is not an array")));
    };
    if items.len() != 2 {
        return Err(ScriptError::Marshal(format!(
            "{what} has {} components (need 2)",
            items.len()
        )));
    }
    Ok([wire_f64(&items[0], what)?, wire_f64(&items[1], what)?])
}

fn wire_point(value: &Wire, what: &str) -> Result<Point, ScriptError> {
    let [x, y, z] = wire_triple(value, what)?;
    Ok(Point::new(x, y, z))
}

fn wire_vector(value: &Wire, what: &str) -> Result<Vector, ScriptError> {
    let [x, y, z] = wire_triple(value, what)?;
    Ok(Vector::new(x, y, z))
}

fn wire_kind_name(value: &Wire) -> &'static str {
    match value {
        Wire::Nil => "nil",
        Wire::Boolean(_) => "a bool",
        Wire::Integer(_) => "an integer",
        Wire::F32(_) | Wire::F64(_) => "a float",
        Wire::String(_) => "a string",
        Wire::Binary(_) => "bin",
        Wire::Array(_) => "an array",
        Wire::Map(_) => "a map",
        Wire::Ext(..) => "an ext",
    }
}

fn wire_bytes<'w>(value: &'w Wire, what: &str) -> Result<&'w [u8], ScriptError> {
    match value {
        Wire::Binary(bytes) => Ok(bytes),
        other => Err(ScriptError::Marshal(format!(
            "{what} must arrive as msgpack bin (a flat little-endian buffer), got {}",
            wire_kind_name(other)
        ))),
    }
}

/// bin f64 LE → `Vec<f64>`; refuses ragged byte counts.
fn bin_f64(value: &Wire, what: &str) -> Result<Vec<f64>, ScriptError> {
    let bytes = wire_bytes(value, what)?;
    if !bytes.len().is_multiple_of(8) {
        return Err(ScriptError::Marshal(format!(
            "{what}: {} bytes is not a whole number of f64",
            bytes.len()
        )));
    }
    Ok(bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| {
            let mut raw = [0_u8; 8];
            raw.copy_from_slice(chunk);
            f64::from_le_bytes(raw)
        })
        .collect())
}

/// bin u32 LE → `Vec<u32>`; refuses ragged byte counts.
fn bin_u32(value: &Wire, what: &str) -> Result<Vec<u32>, ScriptError> {
    let bytes = wire_bytes(value, what)?;
    if !bytes.len().is_multiple_of(4) {
        return Err(ScriptError::Marshal(format!(
            "{what}: {} bytes is not a whole number of u32",
            bytes.len()
        )));
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| {
            let mut raw = [0_u8; 4];
            raw.copy_from_slice(chunk);
            u32::from_le_bytes(raw)
        })
        .collect())
}

fn wire_plane_map(map: &[(Wire, Wire)]) -> Result<Plane, ScriptError> {
    Ok(Plane {
        origin: wire_point(map_field(map, "origin", "Plane")?, "Plane origin")?,
        x: wire_vector(map_field(map, "x", "Plane")?, "Plane x")?,
        y: wire_vector(map_field(map, "y", "Plane")?, "Plane y")?,
    })
}

fn wire_plane(value: &Wire, what: &str) -> Result<Plane, ScriptError> {
    let Wire::Map(map) = value else {
        return Err(ScriptError::Marshal(format!("{what} is not a Plane map")));
    };
    match map_get(map, "kind").and_then(wire_str) {
        Some("Plane") => wire_plane_map(map),
        other => Err(ScriptError::Marshal(format!(
            "{what} is not a Plane map (kind {other:?})"
        ))),
    }
}

fn wire_curve(map: &[(Wire, Wire)]) -> Result<Curve, ScriptError> {
    let variant = map_field(map, "curve", "Curve")?;
    let Some(variant) = wire_str(variant) else {
        return Err(ScriptError::Marshal(format!(
            "Curve `curve` variant is not a string (got {variant})"
        )));
    };
    Ok(match variant {
        "polyline" => {
            let flat = bin_f64(
                map_field(map, "points", "Curve/polyline")?,
                "Polyline points",
            )?;
            if !flat.len().is_multiple_of(3) {
                return Err(ScriptError::Marshal(format!(
                    "Polyline points: {} coordinates is not a whole number of xyz triples",
                    flat.len()
                )));
            }
            let vertices = flat
                .as_chunks::<3>()
                .0
                .iter()
                .map(|&[x, y, z]| Point::new(x, y, z))
                .collect();
            Curve::Polyline(Polyline {
                vertices,
                closed: wire_bool(
                    map_field(map, "closed", "Curve/polyline")?,
                    "Polyline closed",
                )?,
            })
        }
        "line" => Curve::Line(Line {
            a: wire_point(map_field(map, "a", "Curve/line")?, "Line a")?,
            b: wire_point(map_field(map, "b", "Curve/line")?, "Line b")?,
        }),
        "circle" => Curve::Circle(Circle {
            plane: wire_plane(map_field(map, "plane", "Curve/circle")?, "Circle plane")?,
            radius: wire_f64(map_field(map, "radius", "Curve/circle")?, "Circle radius")?,
        }),
        "rectangle" => {
            let [x0, x1] = wire_pair(map_field(map, "x", "Curve/rectangle")?, "Rectangle x")?;
            let [y0, y1] = wire_pair(map_field(map, "y", "Curve/rectangle")?, "Rectangle y")?;
            Curve::Rectangle(Rectangle {
                plane: wire_plane(
                    map_field(map, "plane", "Curve/rectangle")?,
                    "Rectangle plane",
                )?,
                x: Domain::new(x0, x1),
                y: Domain::new(y0, y1),
            })
        }
        other => {
            return Err(ScriptError::Marshal(format!(
                "curve variant `{other}` does not cross the Python boundary (polyline, line, \
                 circle, rectangle do — tessellate first)"
            )));
        }
    })
}

fn wire_mesh(map: &[(Wire, Wire)]) -> Result<Mesh, ScriptError> {
    let positions = bin_f64(map_field(map, "positions", "Mesh")?, "Mesh positions")?;
    let indices = bin_u32(map_field(map, "indices", "Mesh")?, "Mesh indices")?;
    Mesh::new(positions, indices).map_err(|error| ScriptError::Marshal(format!("Mesh: {error}")))
}

/// A `"kind"`-keyed geometry map → value data.
fn geometry_from_wire(map: &[(Wire, Wire)], kind: &str) -> Result<ValueData, ScriptError> {
    Ok(match kind {
        "Mesh" => ValueData::Mesh(wire_mesh(map)?),
        "Plane" => ValueData::Plane(wire_plane_map(map)?),
        "Curve" => ValueData::Curve(wire_curve(map)?),
        other => {
            return Err(ScriptError::Marshal(format!(
                "unknown wire geometry kind `{other}`"
            )));
        }
    })
}

/// A wire value back into a sealed Cicada value. NaN from Python refuses
/// at value construction — loud, like every value; a structurally invalid
/// mesh (ragged buffers, out-of-range or degenerate triangles) refuses
/// at `Mesh::new`.
///
/// # Errors
///
/// [`ScriptError::Marshal`] on malformed wire data or refused values.
pub fn from_wire(wire: &Wire) -> Result<Arc<HashedValue>, ScriptError> {
    let Wire::Map(map) = wire else {
        return Err(ScriptError::Marshal(format!(
            "expected a tagged value map, got {wire}"
        )));
    };
    let Some(kind) = map_get(map, "k").and_then(wire_str) else {
        // No `k` tag: a self-describing geometry map (or garbage).
        let kind = map_get(map, "kind").and_then(wire_str).ok_or_else(|| {
            ScriptError::Marshal("value map has neither a `k` tag nor a `kind`".to_owned())
        })?;
        let data = geometry_from_wire(map, kind)?;
        return HashedValue::new(data).map_err(|error| ScriptError::Marshal(error.to_string()));
    };
    let payload = map_get(map, "v")
        .ok_or_else(|| ScriptError::Marshal(format!("`{kind}` value has no `v` payload")))?;
    let data = match kind {
        "Number" => ValueData::Number(wire_f64(payload, "Number")?),
        "Integer" => ValueData::Integer(
            payload
                .as_i64()
                .ok_or_else(|| ScriptError::Marshal("Integer payload is not an int".to_owned()))?,
        ),
        "Boolean" => ValueData::Boolean(wire_bool(payload, "Boolean payload")?),
        "Text" => {
            ValueData::Text(Arc::from(wire_str(payload).ok_or_else(|| {
                ScriptError::Marshal("Text payload is not a string".to_owned())
            })?))
        }
        "Point" => ValueData::Point(wire_point(payload, "Point")?),
        "Vector" => ValueData::Vector(wire_vector(payload, "Vector")?),
        "Domain" => {
            let [start, end] = wire_pair(payload, "Domain")?;
            ValueData::Domain(Domain::new(start, end))
        }
        "List" => {
            let Wire::Array(items) = payload else {
                return Err(ScriptError::Marshal(
                    "List payload is not an array".to_owned(),
                ));
            };
            let mut slots = Vec::with_capacity(items.len());
            for item in items {
                slots.push(match item {
                    Wire::Nil => None,
                    present => Some(from_wire(present)?),
                });
            }
            ValueData::List(List { axis: None, slots })
        }
        other => {
            return Err(ScriptError::Marshal(format!("unknown wire kind `{other}`")));
        }
    };
    HashedValue::new(data).map_err(|error| ScriptError::Marshal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seal(data: ValueData) -> Arc<HashedValue> {
        HashedValue::new(data).expect("valid")
    }

    fn tetrahedron() -> Mesh {
        Mesh::new(
            vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                0.0, 0.0, 1.0,
            ],
            vec![0, 2, 1, 0, 1, 3, 1, 2, 3, 0, 3, 2],
        )
        .expect("tetrahedron is valid")
    }

    fn tilted_plane() -> Plane {
        Plane {
            origin: Point::new(1.0, 2.0, 3.0),
            x: Vector::new(0.0, 1.0, 0.0),
            y: Vector::new(-1.0, 0.0, 0.0),
        }
    }

    /// Encode then decode through rmpv — the bytes that actually travel.
    fn through_msgpack(wire: &Wire) -> Wire {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, wire).expect("encodes");
        rmpv::decode::read_value(&mut bytes.as_slice()).expect("decodes")
    }

    #[test]
    fn subset_roundtrips_hash_identical() {
        let points = seal(ValueData::List(List {
            axis: None,
            slots: vec![
                Some(seal(ValueData::Point(Point::new(1.0, 2.0, 3.0)))),
                None,
                Some(seal(ValueData::Number(4.25))),
            ],
        }));
        for value in [
            seal(ValueData::Number(1.5)),
            seal(ValueData::Integer(-7)),
            seal(ValueData::Boolean(true)),
            seal(ValueData::Text(Arc::from("cicada"))),
            seal(ValueData::Vector(Vector::new(0.0, -1.0, 2.0))),
            seal(ValueData::Domain(Domain::new(0.0, 12.0))),
            points,
        ] {
            let wire = to_wire(&value).expect("marshals");
            let back = from_wire(&wire).expect("unmarshals");
            assert_eq!(back.hash(), value.hash(), "roundtrip is hash-identical");
        }
    }

    #[test]
    fn geometry_roundtrips_hash_identical_through_msgpack_bytes() {
        let plane = tilted_plane();
        let meshes = seal(ValueData::List(List {
            axis: None,
            slots: vec![
                Some(seal(ValueData::Mesh(tetrahedron()))),
                None,
                Some(seal(ValueData::Mesh(
                    Mesh::new(vec![], vec![]).expect("empty"),
                ))),
            ],
        }));
        for value in [
            seal(ValueData::Mesh(tetrahedron())),
            seal(ValueData::Plane(plane)),
            seal(ValueData::Curve(Curve::Polyline(Polyline {
                vertices: vec![
                    Point::new(0.0, 0.0, 0.0),
                    Point::new(1.0, 0.0, 0.0),
                    Point::new(0.5, 2.0, -1.0),
                ],
                closed: true,
            }))),
            seal(ValueData::Curve(Curve::Polyline(Polyline {
                vertices: vec![Point::new(0.0, 0.0, 0.0), Point::new(1.0, 0.0, 0.0)],
                closed: false,
            }))),
            seal(ValueData::Curve(Curve::Line(Line {
                a: Point::new(0.0, 0.0, 0.0),
                b: Point::new(1.0, 1.0, 1.0),
            }))),
            seal(ValueData::Curve(Curve::Circle(Circle {
                plane,
                radius: 2.5,
            }))),
            seal(ValueData::Curve(Curve::Rectangle(Rectangle {
                plane,
                x: Domain::new(-1.0, 2.0),
                y: Domain::new(0.0, 0.5),
            }))),
            meshes,
        ] {
            let wire = through_msgpack(&to_wire(&value).expect("marshals"));
            let back = from_wire(&wire).expect("unmarshals");
            assert_eq!(
                back.hash(),
                value.hash(),
                "roundtrip is hash-identical for {}",
                value.data().kind_name()
            );
        }
    }

    #[test]
    fn mesh_buffers_travel_as_little_endian_bin() {
        let wire = to_wire(&seal(ValueData::Mesh(tetrahedron()))).expect("marshals");
        let Wire::Map(map) = &wire else { panic!("map") };
        assert_eq!(map_get(map, "kind").and_then(Wire::as_str), Some("Mesh"));
        let Some(Wire::Binary(positions)) = map_get(map, "positions") else {
            panic!("positions must be bin, got {:?}", map_get(map, "positions"))
        };
        assert_eq!(positions.len(), 4 * 3 * 8);
        assert_eq!(&positions[24..32], &1.0_f64.to_le_bytes());
        let Some(Wire::Binary(indices)) = map_get(map, "indices") else {
            panic!("indices must be bin")
        };
        assert_eq!(indices.len(), 12 * 4);
        assert_eq!(&indices[4..8], &2_u32.to_le_bytes());
    }

    #[test]
    fn out_of_subset_kinds_refuse_loudly() {
        let color = seal(ValueData::Color(cicada_core::scalar::Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }));
        let Err(ScriptError::Marshal(message)) = to_wire(&color) else {
            panic!("Color must refuse")
        };
        assert!(message.contains("`Color`"), "{message}");
        let xform = seal(ValueData::Xform(cicada_core::spatial::Xform::identity()));
        assert!(matches!(to_wire(&xform), Err(ScriptError::Marshal(_))));
    }

    #[test]
    fn nan_from_python_is_refused() {
        let wire = tagged("Number", Wire::F64(f64::NAN));
        assert!(matches!(from_wire(&wire), Err(ScriptError::Marshal(_))));
        let wire = Wire::Map(vec![
            entry("kind", Wire::from("Mesh")),
            entry("positions", f64_bin(&[0.0, f64::NAN, 0.0])),
            entry("indices", u32_bin(&[])),
        ]);
        let Err(ScriptError::Marshal(message)) = from_wire(&wire) else {
            panic!("NaN mesh must refuse")
        };
        assert!(message.contains("NaN"), "{message}");
    }

    #[test]
    fn malformed_geometry_maps_refuse_with_the_reason() {
        // Ragged position bytes.
        let wire = Wire::Map(vec![
            entry("kind", Wire::from("Mesh")),
            entry("positions", Wire::Binary(vec![0_u8; 13])),
            entry("indices", u32_bin(&[])),
        ]);
        let Err(ScriptError::Marshal(message)) = from_wire(&wire) else {
            panic!("ragged bin must refuse")
        };
        assert!(message.contains("13 bytes"), "{message}");

        // Positions sent as an array of floats instead of bin.
        let wire = Wire::Map(vec![
            entry("kind", Wire::from("Mesh")),
            entry("positions", Wire::Array(vec![Wire::F64(0.0)])),
            entry("indices", u32_bin(&[])),
        ]);
        let Err(ScriptError::Marshal(message)) = from_wire(&wire) else {
            panic!("non-bin must refuse")
        };
        assert!(message.contains("msgpack bin"), "{message}");

        // An index past the last vertex — Mesh::new's structural check.
        let wire = Wire::Map(vec![
            entry("kind", Wire::from("Mesh")),
            entry("positions", f64_bin(&[0.0; 9])),
            entry("indices", u32_bin(&[0, 1, 7])),
        ]);
        let Err(ScriptError::Marshal(message)) = from_wire(&wire) else {
            panic!("bad index must refuse")
        };
        assert!(message.contains("out of range"), "{message}");

        // An unknown curve variant names itself.
        let wire = Wire::Map(vec![
            entry("kind", Wire::from("Curve")),
            entry("curve", Wire::from("nurbs")),
        ]);
        let Err(ScriptError::Marshal(message)) = from_wire(&wire) else {
            panic!("unknown variant must refuse")
        };
        assert!(
            message.contains("`nurbs`") && message.contains("tessellate"),
            "{message}"
        );

        // A map with neither tag.
        let wire = Wire::Map(vec![entry("foo", Wire::from("bar"))]);
        assert!(matches!(from_wire(&wire), Err(ScriptError::Marshal(_))));
    }

    #[test]
    fn integers_widen_exactly_inside_geometry() {
        // Python scripts hand back ints where floats are meant all the
        // time ((0, 0, 1) as a vector): exact widening, like everywhere.
        let wire = Wire::Map(vec![
            entry("kind", Wire::from("Plane")),
            entry(
                "origin",
                Wire::Array(vec![Wire::from(1), Wire::from(2), Wire::from(3)]),
            ),
            entry(
                "x",
                Wire::Array(vec![Wire::from(1), Wire::from(0), Wire::from(0)]),
            ),
            entry(
                "y",
                Wire::Array(vec![Wire::from(0), Wire::from(1), Wire::from(0)]),
            ),
        ]);
        let back = from_wire(&wire).expect("unmarshals");
        assert_eq!(
            *back.data(),
            ValueData::Plane(Plane {
                origin: Point::new(1.0, 2.0, 3.0),
                x: Vector::new(1.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            })
        );
    }
}
