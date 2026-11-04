//! Collecting template variables: `--var` parsing, type conversion, automatic
//! environment defaults and interactive prompting for whatever is missing.

use std::collections::HashSet;
use std::process::Command;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::interpolation::InterpolationContext;
use crate::prompt;

/// Convert a string from the user to the most appropriate JSON value: a
/// boolean for yes/no words, a number when it parses, otherwise the string.
pub fn convert_string_to_appropriate_type(value: &str) -> Value {
    let stripped = value.trim();
    let lower = stripped.to_lowercase();
    if matches!(lower.as_str(), "true" | "yes" | "y" | "1" | "on" | "enable" | "enabled") {
        return Value::Bool(true);
    }
    if matches!(lower.as_str(), "false" | "no" | "n" | "0" | "off" | "disable" | "disabled" | "") {
        return Value::Bool(false);
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
    Value::String(value.to_string())
}

/// Parse `KEY=VALUE` items (from `--var` / `--option`) into a context map.
pub fn parse_key_value_options(items: &[String]) -> Result<crate::jinja::Context> {
    let mut result = crate::jinja::Context::new();
    for item in items {
        let Some((key, value)) = item.split_once('=') else {
            return Err(Error::Usage(format!("Value must be in KEY=VALUE format, got: {item}")));
        };
        result.insert(key.trim().to_string(), convert_string_to_appropriate_type(value));
    }
    Ok(result)
}

/// Populate defaults that can be discovered from the local environment:
/// `github_user` from `gh api user` when a template references it.
pub fn apply_automatic_variable_defaults(ctx: &mut InterpolationContext, template_variables: &HashSet<String>) {
    if !template_variables.contains("github_user") || ctx.has_variable("github_user") {
        return;
    }
    let Ok(output) = Command::new("gh").args(["api", "user", "--jq", ".login"]).output() else { return };
    let login = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && !login.is_empty() {
        ctx.set_collected_variable("github_user", Value::String(login));
    }
}

fn prompt_hint(variable: &str) -> &'static str {
    let lower = variable.to_lowercase();
    if lower.ends_with("_name") {
        " (name)"
    } else if lower.ends_with("_url") {
        " (URL)"
    } else if lower.ends_with("_email") {
        " (email address)"
    } else if lower.ends_with("_version") {
        " (version number)"
    } else if lower.ends_with("_description") {
        " (description)"
    } else {
        ""
    }
}

/// Collect any variables the template needs that the context lacks, prompting
/// the user or failing when `no_input` is set.
pub fn collect_missing_variables(
    ctx: &mut InterpolationContext,
    template_variables: &HashSet<String>,
    no_input: bool,
) -> Result<()> {
    apply_automatic_variable_defaults(ctx, template_variables);

    let mut missing: Vec<&String> = template_variables.iter().filter(|var| !ctx.has_variable(var)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    missing.sort();

    if no_input {
        let names: Vec<&str> = missing.iter().map(|s| s.as_str()).collect();
        return Err(Error::Invalid(format!(
            "Missing required template variables: {}. Use --var to provide them.",
            names.join(", ")
        )));
    }

    println!("\n🔧 Additional variables needed for this template:");
    println!("{}", "=".repeat(50));
    for var in missing {
        let value = prompt::prompt(&format!("Enter value for '{var}'{}", prompt_hint(var)))?;
        ctx.set_collected_variable(var, convert_string_to_appropriate_type(&value));
    }
    println!("{}", "=".repeat(50));
    println!("✅ All variables collected!\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_booleans() {
        for text in ["true", "True", "TRUE", "yes", "y", "1", "on", "enable", "enabled", " true ", "  1  "] {
            assert_eq!(convert_string_to_appropriate_type(text), json!(true), "{text}");
        }
        for text in ["false", "False", "no", "n", "0", "off", "disable", "disabled", "", " false "] {
            assert_eq!(convert_string_to_appropriate_type(text), json!(false), "{text}");
        }
    }

    #[test]
    fn converts_numbers_and_keeps_strings() {
        assert_eq!(convert_string_to_appropriate_type("42"), json!(42));
        assert_eq!(convert_string_to_appropriate_type("-5"), json!(-5));
        assert_eq!(convert_string_to_appropriate_type(" 42 "), json!(42));
        assert_eq!(convert_string_to_appropriate_type("2.75"), json!(2.75));
        assert_eq!(convert_string_to_appropriate_type("-2.5"), json!(-2.5));
        assert_eq!(convert_string_to_appropriate_type("1.0"), json!(1.0));
        assert_eq!(convert_string_to_appropriate_type("hello"), json!("hello"));
        assert_eq!(convert_string_to_appropriate_type("v1.2.3"), json!("v1.2.3"));
        assert_eq!(convert_string_to_appropriate_type("user@example.com"), json!("user@example.com"));
    }

    #[test]
    fn parses_key_value_options() {
        let parsed = parse_key_value_options(&["with_ci=true".to_string(), "retries=3".to_string()]).unwrap();
        assert_eq!(parsed["with_ci"], json!(true));
        assert_eq!(parsed["retries"], json!(3));
        assert!(matches!(parse_key_value_options(&["novalue".to_string()]), Err(Error::Usage(_))));
    }

    #[test]
    fn missing_variables_fail_without_input() {
        let mut ctx = InterpolationContext::new();
        ctx.set_collected_variable("present", json!(1));
        let needed: HashSet<String> = ["present", "absent_b", "absent_a"].iter().map(|s| s.to_string()).collect();
        match collect_missing_variables(&mut ctx, &needed, true) {
            Err(Error::Invalid(message)) => assert!(message.contains("absent_a, absent_b"), "{message}"),
            other => panic!("expected invalid error, got {other:?}"),
        }
    }
}
