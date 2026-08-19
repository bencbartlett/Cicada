//! Value marshalling across the Python boundary: the tagged
//! `{"k": kind, "v": payload}` wire scheme over `MessagePack`. The
//! marshallable subset is the field-solver tier (docs/12 stage 4):
//! Number, Integer, Boolean, Text, Point, Vector, Domain, and lists
//! thereof (holes = `MessagePack` nil). Everything else refuses loudly —
//! mesh/curve crossings arrive with the exporters (stage 6).

use std::sync::Arc;

use cicada_core::scalar::Domain;
use cicada_core::spatial::{Point, Vector};
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

/// A Cicada value onto the wire.
///
/// # Errors
///
/// [`ScriptError::Marshal`] for kinds outside the stage-4 subset.
pub fn to_wire(value: &HashedValue) -> Result<Wire, ScriptError> {
    Ok(match value.data() {
        ValueData::Number(x) => tagged("Number", Wire::F64(*x)),
        ValueData::Integer(i) => tagged("Integer", Wire::from(*i)),
        ValueData::Boolean(b) => tagged("Boolean", Wire::Boolean(*b)),
        ValueData::Text(s) => tagged("Text", Wire::from(s.as_ref())),
        ValueData::Point(p) => tagged("Point", triple(p.0.x, p.0.y, p.0.z)),
        ValueData::Vector(v) => tagged("Vector", triple(v.0.x, v.0.y, v.0.z)),
        ValueData::Domain(d) => tagged(
            "Domain",
            Wire::Array(vec![Wire::F64(d.start), Wire::F64(d.end)]),
        ),
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
                "kind `{}` does not cross the Python boundary yet (stage-4 subset: \
                 Number, Integer, Boolean, Text, Point, Vector, Domain, List)",
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

/// A wire value back into a sealed Cicada value. NaN from Python refuses
/// at value construction — loud, like every value.
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
    let kind = map_get(map, "k")
        .and_then(wire_str)
        .ok_or_else(|| ScriptError::Marshal("value map has no `k` tag".to_owned()))?;
    let payload = map_get(map, "v")
        .ok_or_else(|| ScriptError::Marshal(format!("`{kind}` value has no `v` payload")))?;
    let data = match kind {
        "Number" => ValueData::Number(wire_f64(payload, "Number")?),
        "Integer" => ValueData::Integer(
            payload
                .as_i64()
                .ok_or_else(|| ScriptError::Marshal("Integer payload is not an int".to_owned()))?,
        ),
        "Boolean" => ValueData::Boolean(
            payload
                .as_bool()
                .ok_or_else(|| ScriptError::Marshal("Boolean payload is not a bool".to_owned()))?,
        ),
        "Text" => {
            ValueData::Text(Arc::from(wire_str(payload).ok_or_else(|| {
                ScriptError::Marshal("Text payload is not a string".to_owned())
            })?))
        }
        "Point" => {
            let [x, y, z] = wire_triple(payload, "Point")?;
            ValueData::Point(Point::new(x, y, z))
        }
        "Vector" => {
            let [x, y, z] = wire_triple(payload, "Vector")?;
            ValueData::Vector(Vector::new(x, y, z))
        }
        "Domain" => {
            let Wire::Array(items) = payload else {
                return Err(ScriptError::Marshal(
                    "Domain payload is not an array".to_owned(),
                ));
            };
            if items.len() != 2 {
                return Err(ScriptError::Marshal(format!(
                    "Domain has {} components (need 2)",
                    items.len()
                )));
            }
            ValueData::Domain(Domain::new(
                wire_f64(&items[0], "Domain")?,
                wire_f64(&items[1], "Domain")?,
            ))
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
    fn out_of_subset_kinds_refuse_loudly() {
        let mesh = seal(ValueData::Mesh(
            cicada_core::geometry::Mesh::new(vec![], vec![]).expect("empty"),
        ));
        assert!(matches!(to_wire(&mesh), Err(ScriptError::Marshal(_))));
    }

    #[test]
    fn nan_from_python_is_refused() {
        let wire = tagged("Number", Wire::F64(f64::NAN));
        assert!(matches!(from_wire(&wire), Err(ScriptError::Marshal(_))));
    }
}
