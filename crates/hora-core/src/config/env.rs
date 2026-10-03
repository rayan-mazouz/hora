//! `${VAR}` expansion of the config's string values.

/// Expand `${VAR}` in every string value of the document, recursively. Keys,
/// comments and non-string values are never touched, so a `${VAR}` in a
/// commented-out example neither warns nor pulls a secret into the file's
/// parse context.
pub(super) fn expand_env_table(table: &mut toml::Table) {
    for (_key, value) in table.iter_mut() {
        expand_env_value(value);
    }
}

fn expand_env_value(value: &mut toml::Value) {
    match value {
        toml::Value::String(text) => {
            if text.contains('$') {
                *text = expand_env(text);
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                expand_env_value(item);
            }
        }
        toml::Value::Table(table) => expand_env_table(table),
        _ => {}
    }
}

/// Substitute `${VAR}` with the environment value (empty if unset). `$$` is a
/// literal `$`, so `$${id}` yields a literal `${id}`. Runs on already-parsed
/// string values, so the result needs no TOML escaping - a value with quotes
/// or newlines is stored verbatim and can't break parsing or inject config.
pub(super) fn expand_env(input: &str) -> String {
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
                let name = &body[..end];
                if let Ok(value) = std::env::var(name) {
                    out.push_str(&value);
                } else {
                    tracing::warn!("config references unset environment variable {name:?}");
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
    out
}
