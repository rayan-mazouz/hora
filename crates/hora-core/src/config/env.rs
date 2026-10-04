//! `${VAR}` expansion of the config's string values.

/// Expand `${VAR}` in every string value of the document, recursively. Keys,
/// comments and non-string values are never touched, so a `${VAR}` in a
/// commented-out example is never looked up.
///
/// # Errors
///
/// Returns an error naming the key and the variable when a `${VAR}` is unset.
pub(super) fn expand_env_table(table: &mut toml::Table) -> anyhow::Result<()> {
    expand_table("", table)
}

fn expand_table(prefix: &str, table: &mut toml::Table) -> anyhow::Result<()> {
    for (key, value) in table.iter_mut() {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        expand_value(&path, value)?;
    }
    Ok(())
}

fn expand_value(path: &str, value: &mut toml::Value) -> anyhow::Result<()> {
    match value {
        toml::Value::String(text) => {
            if text.contains('$') {
                *text = expand_env(text).map_err(|err| anyhow::anyhow!("{path}: {err}"))?;
            }
        }
        toml::Value::Array(items) => {
            for (index, item) in items.iter_mut().enumerate() {
                expand_value(&format!("{path}[{index}]"), item)?;
            }
        }
        toml::Value::Table(table) => expand_table(path, table)?,
        _ => {}
    }
    Ok(())
}

/// Substitute `${VAR}` with the environment value. `${VAR:-fallback}` uses
/// `fallback` when the variable is unset or empty (`${VAR:-}` allows an empty
/// value). `$$` is a literal `$`, so `$${id}` yields a literal `${id}`. Runs on
/// already-parsed string values, so the result needs no TOML escaping - a
/// value with quotes or newlines is stored verbatim and can't break parsing or
/// inject config.
///
/// # Errors
///
/// Returns an error naming the variable when a `${VAR}` without a fallback is
/// unset.
pub(super) fn expand_env(input: &str) -> anyhow::Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(stripped) = after.strip_prefix('$') {
            out.push('$'); // `$$` escape.
            rest = stripped;
        } else if let Some(body) = after.strip_prefix('{') {
            if let Some(end) = body.find('}') {
                let reference = &body[..end];
                let (name, fallback) = match reference.split_once(":-") {
                    Some((name, fallback)) => (name, Some(fallback)),
                    None => (reference, None),
                };
                match (std::env::var(name), fallback) {
                    (Ok(value), Some(fallback)) if value.is_empty() => out.push_str(fallback),
                    (Ok(value), _) => out.push_str(&value),
                    (Err(_), Some(fallback)) => out.push_str(fallback),
                    (Err(_), None) => anyhow::bail!(
                        "environment variable {name:?} is not set \
                         (write ${{{name}:-}} if it may be empty)"
                    ),
                }
                rest = &body[end + 1..];
            } else {
                out.push_str("${"); // No closing brace: emit literally.
                rest = body;
            }
        } else {
            out.push('$'); // A lone `$`.
            rest = after;
        }
    }
    out.push_str(rest);
    Ok(out)
}
