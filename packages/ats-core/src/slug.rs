//! Run-folder slug sanitizer (AC-6.4).
//!
//! Turns a raw posting title into a filesystem-safe slug. Rules are locked:
//!
//! 1. Lowercase (Unicode-aware `char::to_lowercase`).
//! 2. Replace runs of non-alphanumeric characters with a single `-`
//!    (alphanumeric tested via [`char::is_alphanumeric`], which accepts any
//!    Unicode letter/digit — including diacritics such as `é`).
//! 3. Strip leading/trailing `-`.
//! 4. Truncate to 60 chars at a char boundary (never mid-code-point).
//! 5. Fallback to `"untitled"` when the result is empty.
//!
//! Diacritics are **preserved** — `is_alphanumeric` accepts them and the
//! policy here is "lowercase only, no transliteration". A caller that wants
//! pure ASCII should normalise upstream.

/// Maximum number of characters in the sanitized slug. Measured in chars, not
/// bytes, so non-ASCII stays safe.
pub const MAX_SLUG_CHARS: usize = 60;

/// Sanitize a raw title into a filesystem-friendly slug.
///
/// See module docs for the exact rules.
pub fn sanitize_title(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut in_sep = true; // true at start so leading separators collapse to nothing.

    for ch in input.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            slug.push(ch);
            in_sep = false;
            continue;
        }
        if !in_sep {
            slug.push('-');
            in_sep = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    let truncated = truncate_chars(&slug, MAX_SLUG_CHARS);
    let truncated_str = truncated.trim_end_matches('-');
    if truncated_str.is_empty() {
        return "untitled".into();
    }
    truncated_str.to_string()
}

fn truncate_chars(s: &str, max_chars: usize) -> &str {
    let mut boundary = s.len();
    for (idx, (byte_idx, _)) in s.char_indices().enumerate() {
        if idx == max_chars {
            boundary = byte_idx;
            break;
        }
    }
    &s[..boundary]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typical_title() {
        assert_eq!(
            sanitize_title("Senior Rust Engineer (Remote) — Acme Inc."),
            "senior-rust-engineer-remote-acme-inc"
        );
    }

    #[test]
    fn collapses_whitespace_and_punctuation_runs() {
        assert_eq!(sanitize_title("  !!  Hello // World  !!  "), "hello-world");
    }

    #[test]
    fn preserves_unicode_letters_lowercased() {
        // `is_alphanumeric` accepts `é`; policy keeps diacritics.
        let out = sanitize_title("Ingeniero de Software Sénior");
        assert_eq!(out, "ingeniero-de-software-sénior");
    }

    #[test]
    fn long_title_truncated_at_char_boundary() {
        let raw = "a".repeat(120);
        let out = sanitize_title(&raw);
        assert!(out.chars().count() <= MAX_SLUG_CHARS);
        assert!(out.chars().all(|c| c == 'a'));
    }

    #[test]
    fn multibyte_truncation_keeps_valid_utf8() {
        // 58 'a's + 'é' (2 bytes) + padding — slice at char boundary, not byte.
        let raw = format!("{}{}{}", "a".repeat(58), 'é', "bbbb");
        let out = sanitize_title(&raw);
        assert!(out.chars().count() <= MAX_SLUG_CHARS);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn truncation_drops_trailing_hyphen_from_cut() {
        let raw = format!("{}-extra", "a".repeat(60));
        let out = sanitize_title(&raw);
        assert!(!out.ends_with('-'), "got: {out}");
        assert!(out.chars().count() <= MAX_SLUG_CHARS);
    }

    #[test]
    fn empty_input_falls_back_to_untitled() {
        assert_eq!(sanitize_title(""), "untitled");
    }

    #[test]
    fn all_punctuation_falls_back_to_untitled() {
        assert_eq!(sanitize_title("  !!!  ---  @@@  "), "untitled");
    }

    #[test]
    fn hyphen_runs_collapse_with_surrounding_whitespace() {
        assert_eq!(sanitize_title("foo -- bar"), "foo-bar");
    }

    #[test]
    fn digits_are_preserved() {
        assert_eq!(sanitize_title("Rust 2024 Engineer"), "rust-2024-engineer");
    }
}
