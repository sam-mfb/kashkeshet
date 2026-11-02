use super::*;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::fmt;

pub(super) const MARKER: &str = "kkt-workboard/1\n";
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Operation {
    Create {
        kind: Kind,
        payload: Value,
        supersedes: Option<String>,
    },
    Update {
        base_message_id: i64,
        payload: Value,
        reason: Option<String>,
    },
    Withdraw {
        base_message_id: i64,
        reason: String,
    },
    Verify {
        item_revision: i64,
        #[serde(default)]
        evidence_message_ids: Vec<i64>,
        observed_at: Option<String>,
        success: bool,
    },
    Resolve {
        base_message_id: i64,
        resolution_note: String,
        evidence_description: String,
        #[serde(default)]
        evidence_message_ids: Vec<i64>,
        evidence_url: Option<String>,
    },
}
pub(super) struct Report {
    pub request_id: String,
    pub item_id: String,
    pub operation: Operation,
}
impl Report {
    pub fn canonical(&self, existing_kind: Option<Kind>) -> Result<String, String> {
        let mut operation = self.operation.clone();
        let (kind, raw) = match &mut operation {
            Operation::Create { kind, payload, .. } => (Some(*kind), Some(payload)),
            Operation::Update { payload, .. } => (existing_kind, Some(payload)),
            _ => (None, None),
        };
        if let Some(raw) = raw {
            let kind = kind.ok_or("Item not found")?;
            *raw = serde_json::to_value(payload(kind, raw.clone())?).map_err(|e| e.to_string())?;
        }
        serde_json::to_string(&(self.item_id.as_str(), operation)).map_err(|e| e.to_string())
    }
}
pub(super) fn timestamp(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp())
}
pub(super) fn safe_url(s: &str) -> bool {
    if s.len() > WORKBOARD_MAX_URL_BYTES
        || s.chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return false;
    }
    let Ok(parsed) = url::Url::parse(s) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.has_host()
        && parsed.username().is_empty()
        && parsed.password().is_none()
}
pub(super) fn text(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= WORKBOARD_MAX_TEXT_CHARS
}
pub(super) fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= WORKBOARD_MAX_ID_BYTES
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_/.".contains(c))
}
pub(super) fn parse(body: &str) -> Result<Option<Report>, String> {
    if !body.starts_with("kkt-workboard/") {
        return Ok(None);
    }
    let Some(json) = body.strip_prefix(MARKER) else {
        return Err("Unsupported workboard convention version or marker".into());
    };
    if body.len() > WORKBOARD_MAX_ENVELOPE_BYTES {
        return Err("Envelope byte limit exceeded".into());
    }
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let mut value = Strict(0)
        .deserialize(&mut deserializer)
        .map_err(|e| format!("Invalid envelope: {e}"))?;
    deserializer.end().map_err(|e| e.to_string())?;
    let obj = value.as_object_mut().ok_or("Envelope must be an object")?;
    let request_id = obj
        .remove("request_id")
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or("Missing request_id")?;
    let item_id = obj
        .remove("item_id")
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or("Missing item_id")?;
    if !id(&request_id) || !id(&item_id) {
        return Err("Invalid request or item ID".into());
    }
    let operation = serde_json::from_value(value).map_err(|e| format!("Invalid operation: {e}"))?;
    Ok(Some(Report {
        request_id,
        item_id,
        operation,
    }))
}
pub(super) fn payload(kind: Kind, v: Value) -> Result<Payload, String> {
    fn read<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
        serde_json::from_value(v).map_err(|e| format!("Invalid payload: {e}"))
    }
    Ok(match kind {
        Kind::Task => Payload::Task(read(v)?),
        Kind::Decision => Payload::Decision(read(v)?),
        Kind::Approval => Payload::Approval(read(v)?),
        Kind::PullRequest | Kind::Issue => Payload::Reference(read(v)?),
    })
}
struct Strict(usize);
impl<'de> DeserializeSeed<'de> for Strict {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        if self.0 > WORKBOARD_MAX_DEPTH {
            return Err(de::Error::custom("Nesting limit exceeded"));
        }
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Strict {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded JSON without duplicate keys")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        serde_json::Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("Invalid number"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        if v.chars().count() > WORKBOARD_MAX_TEXT_CHARS {
            return Err(E::custom("Text limit exceeded"));
        }
        Ok(v.into())
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(v) = a.next_element_seed(Strict(self.0 + 1))? {
            if values.len() >= WORKBOARD_MAX_LIST {
                return Err(de::Error::custom("List limit exceeded"));
            }
            values.push(v);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut obj = serde_json::Map::new();
        while let Some(k) = a.next_key::<String>()? {
            if obj.contains_key(&k) {
                return Err(de::Error::custom("Duplicate JSON key"));
            }
            let v = a.next_value_seed(Strict(self.0 + 1))?;
            obj.insert(k, v);
        }
        Ok(Value::Object(obj))
    }
}
