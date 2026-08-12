use anyhow::{Result, bail};

pub(crate) fn validate_atcoder_identifier(value: &str, kind: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        bail!("Invalid {kind}: '{value}'.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_atcoder_ids_and_rejects_path_components() {
        assert!(validate_atcoder_identifier("abc999", "contest ID").is_ok());
        assert!(validate_atcoder_identifier("math-and-algorithm", "contest ID").is_ok());
        assert!(validate_atcoder_identifier("abc999_a", "problem ID").is_ok());
        assert!(validate_atcoder_identifier("../abc999", "contest ID").is_err());
        assert!(validate_atcoder_identifier("abc/999", "contest ID").is_err());
        assert!(validate_atcoder_identifier("コンテスト", "contest ID").is_err());
    }
}
