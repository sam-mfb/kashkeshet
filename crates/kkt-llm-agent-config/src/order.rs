//! Writing a client's JSON file back with its keys in the order the file
//! had them. kkt's JSON objects keep their keys sorted, so without this a
//! rewrite would sort a person's file, and in some clients, such as
//! OpenCode's `permission` rules, the order of keys decides which rule
//! wins. Keys the file had come first, in its order; keys added after it
//! was read follow, sorted.

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::Value;
use std::fmt;

/// The order of keys in a JSON text: for each object, its keys in the order
/// they appear, each with the order inside its value.
enum Shape {
    Object(Vec<(String, Shape)>),
    Array(Vec<Shape>),
    Other,
}

impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Shape, D::Error> {
        deserializer.deserialize_any(ShapeVisitor)
    }
}

struct ShapeVisitor;

impl<'de> Visitor<'de> for ShapeVisitor {
    type Value = Shape;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Shape, A::Error> {
        let mut keys = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            keys.push((key, map.next_value::<Shape>()?));
        }
        Ok(Shape::Object(keys))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Shape, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Shape>()? {
            items.push(item);
        }
        Ok(Shape::Array(items))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Shape, E> {
        Ok(Shape::Other)
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Shape, E> {
        Ok(Shape::Other)
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Shape, E> {
        Ok(Shape::Other)
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Shape, E> {
        Ok(Shape::Other)
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<Shape, E> {
        Ok(Shape::Other)
    }
    fn visit_unit<E: de::Error>(self) -> Result<Shape, E> {
        Ok(Shape::Other)
    }
}

/// `value` written with the key order of `shape`, where it has one.
struct Ordered<'a> {
    value: &'a Value,
    shape: Option<&'a Shape>,
}

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.value {
            Value::Object(map) => {
                let known: &[(String, Shape)] = match self.shape {
                    Some(Shape::Object(keys)) => keys,
                    _ => &[],
                };
                let mut out = serializer.serialize_map(Some(map.len()))?;
                let mut written = Vec::new();
                for (key, shape) in known {
                    // A key repeated in the file is written once, as its
                    // value is kept once.
                    if written.contains(&key) {
                        continue;
                    }
                    if let Some(value) = map.get(key) {
                        let value = Ordered {
                            value,
                            shape: Some(shape),
                        };
                        out.serialize_entry(key, &value)?;
                        written.push(key);
                    }
                }
                for (key, value) in map {
                    if !written.contains(&key) {
                        out.serialize_entry(key, &Ordered { value, shape: None })?;
                    }
                }
                out.end()
            }
            Value::Array(items) => {
                let known: &[Shape] = match self.shape {
                    Some(Shape::Array(items)) => items,
                    _ => &[],
                };
                let mut out = serializer.serialize_seq(Some(items.len()))?;
                for (i, value) in items.iter().enumerate() {
                    out.serialize_element(&Ordered {
                        value,
                        shape: known.get(i),
                    })?;
                }
                out.end()
            }
            other => other.serialize(serializer),
        }
    }
}

/// `document` as pretty JSON ending in a newline, with the key order of
/// `original`, the text it was read from (`None` for a new file).
pub(super) fn pretty(original: Option<&str>, document: &Value) -> Result<String, String> {
    let shape = match original {
        Some(text) => Some(
            serde_json::from_str::<Shape>(text)
                .map_err(|e| format!("could not read the order of the file's keys: {e}"))?,
        ),
        None => None,
    };
    let ordered = Ordered {
        value: document,
        shape: shape.as_ref(),
    };
    serde_json::to_string_pretty(&ordered)
        .map(|text| format!("{text}\n"))
        .map_err(|_| "could not encode the config".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_keep_the_files_order_and_new_ones_follow() {
        let original = r#"{"zeta": 1, "alpha": {"rm -rf *": "deny", "rm *": "ask"},
                           "list": [{"b": 1, "a": 2}], "gone": true, "mid": null}"#;
        let mut document: Value = serde_json::from_str(original).expect("JSON");
        let root = document.as_object_mut().expect("object");
        root.remove("gone");
        root.insert("new_b".into(), json!(1));
        root.insert("new_a".into(), json!(2));
        root["alpha"]
            .as_object_mut()
            .expect("object")
            .insert("kkt watch *".into(), json!("allow"));
        root["list"]
            .as_array_mut()
            .expect("list")
            .push(json!({"y": 1, "x": 2}));
        let text = pretty(Some(original), &document).expect("written");
        let keys: Vec<_> = text
            .lines()
            .filter_map(|line| line.trim().split('"').nth(1))
            .collect();
        assert_eq!(
            keys,
            [
                "zeta",
                "alpha",
                "rm -rf *",
                "rm *",
                "kkt watch *",
                "list",
                "b",
                "a",
                "x",
                "y",
                "mid",
                "new_a",
                "new_b"
            ]
        );
        assert!(text.ends_with("}\n"));
        let back: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(back, document);
    }

    #[test]
    fn a_repeated_key_is_written_once() {
        let original = r#"{"a": 1, "b": 2, "a": 3}"#;
        let document: Value = serde_json::from_str(original).expect("JSON");
        let text = pretty(Some(original), &document).expect("written");
        assert_eq!(text, "{\n  \"a\": 3,\n  \"b\": 2\n}\n");
    }

    #[test]
    fn a_new_file_is_written_sorted() {
        let text = pretty(None, &json!({"b": 1, "a": 2})).expect("written");
        assert_eq!(text, "{\n  \"a\": 2,\n  \"b\": 1\n}\n");
    }
}
