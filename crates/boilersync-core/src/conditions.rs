//! Python-flavoured truthiness and the small condition language used by
//! `template.json` (`children[].condition`, `hooks[].condition`,
//! `github.condition`).

use serde_json::Value;

use crate::error::{Error, Result};
use crate::jinja::{Context, render_string};

/// Python `bool()` of a JSON value.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

fn parse_token(token: &str, context: &Context) -> Value {
    let stripped = token.trim();
    if let Some(value) = context.get(stripped) {
        return value.clone();
    }
    match stripped.to_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => return Value::Bool(true),
        "false" | "no" | "off" | "0" => return Value::Bool(false),
        _ => {}
    }
    if stripped.len() >= 2
        && ((stripped.starts_with('"') && stripped.ends_with('"'))
            || (stripped.starts_with('\'') && stripped.ends_with('\'')))
    {
        return Value::String(stripped[1..stripped.len() - 1].to_string());
    }
    if stripped.contains('.') {
        if let Ok(float) = stripped.parse::<f64>()
            && let Some(number) = serde_json::Number::from_f64(float)
        {
            return Value::Number(number);
        }
    } else if let Ok(int) = stripped.parse::<i64>() {
        return Value::from(int);
    }
    Value::String(stripped.to_string())
}

/// Python `==`: numbers and booleans compare numerically, everything else
/// structurally.
fn py_eq(left: &Value, right: &Value) -> bool {
    fn as_number(value: &Value) -> Option<f64> {
        match value {
            Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
            Value::Number(number) => number.as_f64(),
            _ => None,
        }
    }
    match (as_number(left), as_number(right)) {
        (Some(a), Some(b)) => a == b,
        _ => left == right,
    }
}

/// Evaluate a `condition` value: absent means true; booleans and numbers are
/// their truthiness; strings are rendered then interpreted (`not x`, `a == b`,
/// `a != b`, a context variable, or a literal). Unknown identifiers are false.
pub fn evaluate_condition(condition: Option<&Value>, context: &Context) -> Result<bool> {
    let condition = match condition {
        None | Some(Value::Null) => return Ok(true),
        Some(Value::Bool(flag)) => return Ok(*flag),
        Some(Value::Number(number)) => return Ok(number.as_f64().is_some_and(|n| n != 0.0)),
        Some(Value::String(text)) => text,
        Some(other) => {
            let kind = match other {
                Value::Array(_) => "list",
                _ => "dict",
            };
            return Err(Error::Invalid(format!("Unsupported condition type: {kind}")));
        }
    };

    let rendered = render_string(condition, context)?.trim().to_string();
    if rendered.is_empty() {
        return Ok(false);
    }
    if let Some(token) = rendered.strip_prefix("not ") {
        return Ok(!truthy(Some(&parse_token(token, context))));
    }
    for operator in ["==", "!="] {
        if let Some((left, right)) = rendered.split_once(operator) {
            let equal = py_eq(&parse_token(left, context), &parse_token(right, context));
            return Ok(if operator == "!=" { !equal } else { equal });
        }
    }
    if let Some(value) = context.get(&rendered) {
        return Ok(truthy(Some(value)));
    }
    if is_identifier(&rendered) {
        return Ok(false);
    }
    Ok(truthy(Some(&parse_token(&rendered, context))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(value: Value) -> Context {
        match value {
            Value::Object(map) => map,
            _ => unreachable!(),
        }
    }

    #[test]
    fn evaluates_conditions() {
        let context = ctx(json!({"with_github": true, "tier": "pro", "count": 0}));
        assert!(evaluate_condition(None, &context).unwrap());
        assert!(evaluate_condition(Some(&json!(true)), &context).unwrap());
        assert!(!evaluate_condition(Some(&json!(0)), &context).unwrap());
        assert!(evaluate_condition(Some(&json!("with_github")), &context).unwrap());
        assert!(!evaluate_condition(Some(&json!("missing_flag")), &Context::new()).unwrap());
        assert!(!evaluate_condition(Some(&json!("not with_github")), &context).unwrap());
        assert!(evaluate_condition(Some(&json!("tier == 'pro'")), &context).unwrap());
        assert!(evaluate_condition(Some(&json!("tier != \"free\"")), &context).unwrap());
        assert!(!evaluate_condition(Some(&json!("count")), &context).unwrap());
        // A rendered boolean reads "True", an unknown identifier, so this is false.
        // Matches the original implementation: use the bare variable name instead.
        assert!(!evaluate_condition(Some(&json!("$${with_github}")), &context).unwrap());
        assert!(!evaluate_condition(Some(&json!("")), &context).unwrap());
        assert!(evaluate_condition(Some(&json!("with_github == 1")), &context).unwrap());
        assert!(matches!(evaluate_condition(Some(&json!([1])), &context), Err(Error::Invalid(_))));
    }

    #[test]
    fn truthiness_matches_python() {
        assert!(!truthy(Some(&json!(""))));
        assert!(!truthy(Some(&json!([]))));
        assert!(!truthy(Some(&json!({}))));
        assert!(!truthy(Some(&json!(0.0))));
        assert!(truthy(Some(&json!("x"))));
        assert!(truthy(Some(&json!(2))));
        assert!(!truthy(None));
    }
}
