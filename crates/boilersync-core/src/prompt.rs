//! Minimal interactive prompts on stdin/stdout.

use std::io::{BufRead, Write};

use crate::error::{Error, Result};

/// Ask a free-text question; returns the trimmed answer (possibly empty).
pub fn prompt(question: &str) -> Result<String> {
    let mut stdout = std::io::stdout();
    write!(stdout, "{question}: ")?;
    stdout.flush()?;
    let mut line = String::new();
    let read = std::io::stdin().lock().read_line(&mut line)?;
    if read == 0 {
        return Err(Error::Usage("Aborted: no input available.".to_string()));
    }
    Ok(line.trim().to_string())
}

/// Ask a question with a default used for an empty answer.
pub fn prompt_with_default(question: &str, default: &str) -> Result<String> {
    let answer = prompt(&format!("{question} [{default}]"))?;
    Ok(if answer.is_empty() { default.to_string() } else { answer })
}

/// Return `default` without prompting when `no_input` is set.
pub fn prompt_or_default(question: &str, default: &str, no_input: bool) -> Result<String> {
    if no_input {
        return Ok(default.to_string());
    }
    prompt_with_default(question, default)
}
