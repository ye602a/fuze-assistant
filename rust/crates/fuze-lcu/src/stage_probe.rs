//! Explicit diagnostic clues only. Never establishes a claim or changes recommendation order.
use serde::Serialize;
use serde_json::Value;
const FIELDS: [&str; 8] = [
    "augments",
    "selectedAugments",
    "augmentSlots",
    "augmentIds",
    "augmentStage",
    "currentAugmentStage",
    "augmentChoiceIndex",
    "augmentCount",
];
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Shape {
    pub field: &'static str,
    pub kind: &'static str,
    pub count: Option<usize>,
    pub small_integer: Option<u64>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub checked_at: u64,
    pub actual_stage: Option<u8>,
    pub endpoints: [&'static str; 3],
    pub schema_fields: Vec<&'static str>,
    pub player_fields: Vec<Shape>,
    pub level: Option<u64>,
    pub augment_event_count: usize,
    pub truncated: bool,
    pub revision: u64,
    pub stable: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Comparison {
    pub status: &'static str,
    pub changed_fields: Vec<&'static str>,
    pub event_delta: Option<i64>,
    pub automatic_eligible: bool,
}
pub fn summarize(responses: [Result<Value, &'static str>; 3], checked_at: u64) -> Report {
    let mut r = Report {
        checked_at,
        ..Default::default()
    };
    for (i, response) in responses.iter().enumerate() {
        r.endpoints[i] = response.as_ref().map(|_| "ok").unwrap_or_else(|e| *e);
    }
    fn walk(v: &Value, depth: u8, budget: &mut usize, r: &mut Report) {
        if *budget == 0 || depth > 24 {
            r.truncated = true;
            return;
        }
        *budget -= 1;
        if let Some(props) = v.get("properties").and_then(Value::as_object) {
            for f in FIELDS {
                if props.contains_key(f) && !r.schema_fields.contains(&f) {
                    r.schema_fields.push(f);
                }
            }
        }
        match v {
            Value::Object(o) => {
                for (k, c) in o {
                    if !matches!(
                        k.as_str(),
                        "example" | "examples" | "description" | "default" | "enum"
                    ) {
                        walk(c, depth + 1, budget, r);
                        if *budget == 0 {
                            r.truncated = true;
                            break;
                        }
                    }
                }
            }
            Value::Array(a) => {
                for c in a {
                    walk(c, depth + 1, budget, r);
                    if *budget == 0 {
                        r.truncated = true;
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    if let Ok(v) = &responses[0] {
        walk(v, 0, &mut 12000, &mut r);
    }
    if let Ok(v) = &responses[1] {
        r.level = v["level"].as_u64().filter(|n| (1..=30).contains(n));
        for f in FIELDS {
            if let Some(v) = v.get(f) {
                let kind = match v {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                };
                let count = v.as_array().map(|a| {
                    r.truncated |= a.len() > 32;
                    a.len().min(32)
                });
                r.player_fields.push(Shape {
                    field: f,
                    kind,
                    count,
                    small_integer: v.as_u64().filter(|n| *n <= 4),
                });
            }
        }
    }
    if let Ok(v) = &responses[2] {
        if let Some(events) = v["Events"].as_array() {
            r.truncated |= events.len() > 2048;
            r.augment_event_count = events
                .iter()
                .take(2048)
                .filter(|e| {
                    matches!(
                        e["EventName"].as_str(),
                        Some("AugmentSelected" | "AugmentChosen" | "AugmentGranted")
                    )
                })
                .count();
        }
    }
    r
}
pub fn compare(a: &Report, b: &Report, now: u64) -> Comparison {
    let mut c = Comparison {
        status: "comparable",
        changed_fields: vec![],
        event_delta: None,
        automatic_eligible: false,
    };
    c.status = if b.checked_at < a.checked_at
        || now < b.checked_at
        || now.saturating_sub(a.checked_at) > 120_000
    {
        "expired"
    } else if !a.stable || !b.stable || a.revision != b.revision {
        "context"
    } else if a.truncated
        || b.truncated
        || a.endpoints.iter().chain(&b.endpoints).any(|s| *s != "ok")
    {
        "incomplete"
    } else {
        "comparable"
    };
    if c.status == "comparable" {
        for f in FIELDS {
            if a.player_fields.iter().find(|s| s.field == f)
                != b.player_fields.iter().find(|s| s.field == f)
            {
                c.changed_fields.push(f);
            }
        }
        c.event_delta = Some(b.augment_event_count as i64 - a.augment_event_count as i64);
    }
    c
}
#[cfg(windows)]
pub fn inspect(context: impl Fn() -> (u64, bool)) -> Report {
    let before = context();
    let responses = std::array::from_fn(|i| match crate::transport::live_probe(i) {
        Ok(mut bytes) => {
            let v = serde_json::from_slice(&bytes).map_err(|_| "invalid-json");
            crate::wipe(&mut bytes);
            v
        }
        Err(crate::Error::ResponseLimit) => Err("too-large"),
        Err(crate::Error::HttpStatus) => Err("http-error"),
        Err(_) => Err("unavailable"),
    });
    let after = context();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut r = summarize(responses, now);
    r.revision = after.0;
    r.stable = before.0 == after.0 && before.1 && after.1;
    if before.0 != after.0 {
        r.player_fields.clear();
        r.schema_fields.clear();
        r.level = None;
        r.augment_event_count = 0;
    }
    r
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn sanitizes_values_and_comparison_never_authorizes_stage() {
        let mut a = summarize(
            [
                Ok(
                    json!({"properties":{"augments":{}},"example":{"properties":{"augmentStage":{}}}}),
                ),
                Ok(
                    json!({"level":6,"augments":["secret-id"],"name":"private-name","augmentStage":1}),
                ),
                Ok(json!({"Events":[{"EventName":"AugmentSelected","SummonerName":"private"}]})),
            ],
            1000,
        );
        a.stable = true;
        a.revision = 5;
        let encoded = serde_json::to_string(&a).unwrap();
        assert!(!encoded.contains("secret-id") && !encoded.contains("private"));
        assert_eq!(a.schema_fields, vec!["augments"]);
        let mut b = a.clone();
        b.checked_at = 31000;
        b.player_fields[0].count = Some(2);
        assert_eq!(compare(&a, &b, 31000).changed_fields, vec!["augments"]);
        assert!(!compare(&a, &b, 31000).automatic_eligible);
        assert!(a.actual_stage.is_none());
        b.revision = 6;
        assert_eq!(compare(&a, &b, 31000).status, "context");
        assert_eq!(compare(&a, &b, 122000).status, "expired");
    }
}
