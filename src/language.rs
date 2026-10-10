use anyhow::{Result, bail};

/// Converts common language names to LeetCode's `langSlug` values. Unknown
/// values remain supported as long as they look like a safe language slug, so
/// newly added LeetCode languages do not require a leetui release.
pub fn normalize(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    let slug = match value.as_str() {
        "c++" | "cplusplus" => "cpp",
        "c#" | "c-sharp" => "csharp",
        "go" => "golang",
        "js" | "node" | "nodejs" => "javascript",
        "ts" => "typescript",
        "py" | "py3" | "python 3" | "python-3" => "python3",
        "rb" => "ruby",
        "rs" => "rust",
        "kt" => "kotlin",
        "oracle-sql" => "oraclesql",
        "ms-sql" | "sqlserver" => "mssql",
        _ => value.as_str(),
    };

    let valid = !slug.is_empty()
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-');
    if !valid {
        bail!(
            "invalid language `{value}`; use a LeetCode language slug such as `rust`, `cpp`, or `python3`"
        );
    }
    Ok(slug.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_common_language_aliases() {
        assert_eq!(normalize("C++").unwrap(), "cpp");
        assert_eq!(normalize("C#").unwrap(), "csharp");
        assert_eq!(normalize("Go").unwrap(), "golang");
        assert_eq!(normalize("py").unwrap(), "python3");
        assert_eq!(normalize("Rust").unwrap(), "rust");
    }

    #[test]
    fn accepts_future_safe_slugs_but_rejects_paths() {
        assert_eq!(normalize("new-language2").unwrap(), "new-language2");
        assert!(normalize("../../escape").is_err());
        assert!(normalize("").is_err());
    }
}
