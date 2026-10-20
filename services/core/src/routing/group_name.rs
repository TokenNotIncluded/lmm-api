//! Routing groups are exact Unicode labels, not service identifiers or paths.
//! No trimming, case folding, NFC/NFKC normalization, decoding, or alias lookup.
//! This matches PostgreSQL's 64-character group columns while bounding UTF-8.
use regex::Regex;
use std::sync::OnceLock;

/// Keep ASCII identifiers strict while accepting deployed Unicode group labels.
/// Slash/brackets/parentheses are literal label bytes, never path separators.
/// Whitespace, control/format characters and CSV/scope/URL delimiters are denied.
pub fn valid_group_name(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 256
        || value.chars().count() > 64
        || !value
            .bytes()
            .all(|b| !b.is_ascii() || b.is_ascii_alphanumeric() || b"._-/()[]".contains(&b))
    {
        return false;
    }
    static LABEL: OnceLock<Regex> = OnceLock::new();
    LABEL
        .get_or_init(|| {
            Regex::new(r"\A[\p{L}\p{M}\p{N}\p{S}._/()\[\]-]+\z").expect("fixed group label class")
        })
        .is_match(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_group_labels_preserve_deployed_forms_and_scalar_bounds() {
        for name in [
            "default",
            "国产",
            "官key",
            "国产[Kimi/Deepseek/GLM]",
            "国产-便宜但是速度慢",
            "group(test)",
            "🦀",
        ] {
            assert!(valid_group_name(name), "rejected exact group {name:?}");
        }
        for name in ["界".repeat(64), "🦀".repeat(64)] {
            assert!(valid_group_name(&name));
        }
        for name in ["a".repeat(65), "界".repeat(65), "🦀".repeat(65)] {
            assert!(!valid_group_name(&name));
        }
        let nfc = "é";
        let nfd = "e\u{301}";
        assert!(valid_group_name(nfc));
        assert!(valid_group_name(nfd));
        assert_ne!(nfc.as_bytes(), nfd.as_bytes());
    }
    #[test]
    fn unicode_group_labels_reject_controls_whitespace_and_structural_delimiters() {
        for name in [
            "",
            " group",
            "group ",
            "a b",
            "a\tb",
            "a\nb",
            "a,b",
            "a;b",
            "a:b",
            "a\\b",
            "a%b",
            "a?b",
            "a#b",
            "a@b",
            "a=b",
            "a|b",
            "a&b",
            "a\"b",
            "a'b",
            "<group>",
            "a\u{85}b",
            "a\u{200b}b",
            "a\u{202e}b",
            "a\u{2028}b",
            "a\u{3000}b",
            "a\u{e000}b",
        ] {
            assert!(
                !valid_group_name(name),
                "accepted ambiguous/structural group {name:?}"
            );
        }
    }
}
