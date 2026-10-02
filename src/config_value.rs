use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

/// Runtime-typed value exchanged by clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ConfigValue {
    String(String),
    Integer(Number),
    Float(f64),
    Boolean(bool),
    Object(BTreeMap<String, ConfigValue>),
    Array(Vec<ConfigValue>),
    #[serde(rename = "str_list")]
    StringList(Vec<String>),
    #[serde(rename = "numeric_list")]
    NumericList(Vec<Number>),
    Null,
}

impl ConfigValue {
    pub fn from_json(value: Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Boolean(value),
            Value::Number(value) if value.is_i64() || value.is_u64() => Self::Integer(value),
            Value::Number(value) => Self::Float(value.as_f64().unwrap_or_default()),
            Value::String(value) => Self::String(value),
            Value::Array(values) => Self::Array(values.into_iter().map(Self::from_json).collect()),
            Value::Object(values) => Self::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, Self::from_json(value)))
                    .collect(),
            ),
        }
    }

    pub fn into_json(self) -> Result<Value, String> {
        self.into_json_with_types("/").map(|(value, _)| value)
    }

    pub fn into_json_with_types(
        self,
        path: &str,
    ) -> Result<(Value, BTreeMap<String, String>), String> {
        match self {
            Self::Null => Ok((Value::Null, BTreeMap::new())),
            Self::Boolean(value) => Ok((Value::Bool(value), BTreeMap::new())),
            Self::Integer(value) if value.is_i64() || value.is_u64() => {
                Ok((Value::Number(value), BTreeMap::new()))
            }
            Self::Integer(_) => Err("integer value must be a whole JSON number".to_string()),
            Self::Float(value) => Number::from_f64(value)
                .map(Value::Number)
                .map(|value| (value, BTreeMap::new()))
                .ok_or_else(|| "float value must be finite".to_string()),
            Self::String(value) => Ok((Value::String(value), BTreeMap::new())),
            Self::Object(values) => {
                let mut output = serde_json::Map::new();
                let mut types = BTreeMap::new();
                for (key, value) in values {
                    let child_path = append_path(path, &key);
                    let (value, child_types) = value.into_json_with_types(&child_path)?;
                    output.insert(key, value);
                    types.extend(child_types);
                }
                Ok((Value::Object(output), types))
            }
            Self::Array(values) => {
                let mut output = Vec::with_capacity(values.len());
                let mut types = BTreeMap::new();
                for (index, value) in values.into_iter().enumerate() {
                    let child_path = append_path(path, &index.to_string());
                    let (value, child_types) = value.into_json_with_types(&child_path)?;
                    output.push(value);
                    types.extend(child_types);
                }
                Ok((Value::Array(output), types))
            }
            Self::StringList(values) => {
                let mut types = BTreeMap::new();
                types.insert(normalize_path(path), "str_list".to_string());
                Ok((
                    Value::Array(values.into_iter().map(Value::String).collect()),
                    types,
                ))
            }
            Self::NumericList(values) => {
                if values
                    .iter()
                    .any(|value| !value.is_i64() && !value.is_u64() && value.as_f64().is_none())
                {
                    return Err("numeric_list values must be numbers".to_string());
                }
                let mut types = BTreeMap::new();
                types.insert(normalize_path(path), "numeric_list".to_string());
                Ok((
                    Value::Array(values.into_iter().map(Value::Number).collect()),
                    types,
                ))
            }
        }
    }

    pub fn from_json_with_types(
        value: Value,
        path: &str,
        types: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let path = normalize_path(path);
        if let Some(kind) = types.get(&path) {
            let values = value
                .as_array()
                .ok_or_else(|| format!("typed list at {} must be a JSON array", path))?;
            return match kind.as_str() {
                "str_list" => values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| format!("str_list at {} contains a non-string", path))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Self::StringList),
                "numeric_list" => values
                    .iter()
                    .map(|value| {
                        value.as_number().cloned().ok_or_else(|| {
                            format!("numeric_list at {} contains a non-number", path)
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Self::NumericList),
                _ => Err(format!("unknown typed list '{}' at {}", kind, path)),
            };
        }

        match value {
            Value::Object(values) => values
                .into_iter()
                .map(|(key, value)| {
                    let child_path = append_path(&path, &key);
                    Ok((key, Self::from_json_with_types(value, &child_path, types)?))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()
                .map(Self::Object),
            Value::Array(values) => values
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    let child_path = append_path(&path, &index.to_string());
                    Self::from_json_with_types(value, &child_path, types)
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Self::Array),
            value => Ok(Self::from_json(value)),
        }
    }
}

fn append_path(parent: &str, child: &str) -> String {
    let parent = normalize_path(parent);
    if parent == "/" {
        format!("/{}", child)
    } else {
        format!("{}/{}", parent, child)
    }
}

fn normalize_path(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", trimmed)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::ConfigValue;
    use serde_json::json;

    #[test]
    fn runtime_values_round_trip_with_explicit_types() {
        let value: ConfigValue = serde_json::from_value(json!({
            "type": "object",
            "value": {
                "count": { "type": "integer", "value": 5 },
                "labels": { "type": "str_list", "value": ["temperature"] },
                "thresholds": { "type": "numeric_list", "value": [50.5, 80] }
            }
        }))
        .unwrap();

        assert_eq!(value.clone().into_json().unwrap()["count"], json!(5));
        assert_eq!(
            value.clone().into_json().unwrap()["labels"],
            json!(["temperature"])
        );
        assert_eq!(value.into_json().unwrap()["thresholds"], json!([50.5, 80]));
    }

    #[test]
    fn rejects_non_finite_float_values() {
        assert!(ConfigValue::Float(f64::NAN).into_json().is_err());
    }

    #[test]
    fn typed_empty_lists_keep_their_type_in_metadata() {
        let value = ConfigValue::Object(BTreeMap::from([
            ("labels".to_string(), ConfigValue::StringList(Vec::new())),
            ("levels".to_string(), ConfigValue::NumericList(Vec::new())),
        ]));
        let (json, types) = value.into_json_with_types("/").unwrap();
        assert_eq!(json, json!({"labels":[],"levels":[]}));
        assert_eq!(types["/labels"], "str_list");
        assert_eq!(types["/levels"], "numeric_list");
        assert_eq!(
            ConfigValue::from_json_with_types(json["labels"].clone(), "/labels", &types).unwrap(),
            ConfigValue::StringList(Vec::new())
        );
    }
}
