//! Partial clip changes share the stored clip shape. The generated schema is
//! used only to reject misspelled fields; deserialization and Project::validate
//! remain responsible for types and domain invariants.
use crate::{Clip, EditError};
use serde_json::{Map, Value};
use std::sync::OnceLock;

fn merge(target: &mut Value, patch: Value, depth: usize) -> Result<(), String> {
    if depth > 32 {
        return Err("patch nesting exceeds 32".into());
    }
    match patch {
        Value::Object(fields) => {
            if !target.is_object() {
                *target = Value::Object(Map::new());
            }
            let target = target.as_object_mut().ok_or("patch must be an object")?;
            for (key, value) in fields {
                if value.is_null() {
                    target.remove(&key);
                } else {
                    merge(target.entry(key).or_insert(Value::Null), value, depth + 1)?;
                }
            }
        }
        value => *target = value,
    }
    Ok(())
}

fn resolved<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    schema["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix('#'))
        .and_then(|pointer| root.pointer(pointer))
        .unwrap_or(schema)
}

// Schemars expresses nullable fields as anyOf and tagged enums as oneOf. Pick
// their object/array branch before walking properties. Scalar constraints are
// checked by serde and the domain rather than a second schema validator.
fn shape<'a>(schema: &'a Value, root: &'a Value, value: &Value) -> &'a Value {
    let schema = resolved(schema, root);
    for union in ["oneOf", "anyOf"] {
        if let Some(variants) = schema[union].as_array() {
            for variant in variants {
                let candidate = resolved(variant, root);
                let tag = &candidate["properties"]["type"]["const"];
                if (!tag.is_null() && value.get("type") == Some(tag))
                    || (tag.is_null()
                        && ((value.is_object() && candidate.get("properties").is_some())
                            || (value.is_array() && candidate.get("items").is_some())))
                {
                    return shape(candidate, root, value);
                }
            }
        }
    }
    schema
}

fn check_fields(
    value: &Value,
    schema: &Value,
    root: &Value,
    path: &str,
    merged: &Value,
) -> Result<(), String> {
    let schema = shape(schema, root, merged);
    if let (Some(object), Some(properties)) = (value.as_object(), schema["properties"].as_object())
    {
        for (key, value) in object {
            let path = format!("{path}.{key}");
            let property = properties
                .get(key)
                .ok_or_else(|| format!("unknown field {path}"))?;
            check_fields(
                value,
                property,
                root,
                &path,
                merged.get(key).unwrap_or(value),
            )?;
        }
    } else if let Some(items) = value.as_array() {
        for (index, value) in items.iter().enumerate() {
            let item = schema
                .get("items")
                .or_else(|| schema["prefixItems"].get(index));
            if let Some(item) = item {
                check_fields(
                    value,
                    item,
                    root,
                    &format!("{path}[{index}]"),
                    merged.get(index).unwrap_or(value),
                )?;
            }
        }
    }
    Ok(())
}

pub(crate) fn merge_clip(clip: &Clip, patch: Map<String, Value>) -> Result<Clip, EditError> {
    let invalid = |detail: String| EditError::Invalid(format!("clip {} patch: {detail}", clip.id));
    for field in ["id", "linked_id", "group_id"] {
        if patch.contains_key(field) {
            return Err(invalid(format!(
                "{field} is stable; use the relationship commands"
            )));
        }
    }
    let mut value = serde_json::to_value(clip).map_err(|error| invalid(error.to_string()))?;
    let patch = Value::Object(patch);
    merge(&mut value, patch.clone(), 0).map_err(&invalid)?;
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    let schema = SCHEMA.get_or_init(|| schemars::schema_for!(Clip).to_value());
    check_fields(&patch, schema, schema, "clip", &value).map_err(&invalid)?;
    check_fields(&value, schema, schema, "clip", &value).map_err(&invalid)?;
    serde_json::from_value(value).map_err(|error| invalid(error.to_string()))
}
