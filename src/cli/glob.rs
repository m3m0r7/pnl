//! A tiny glob matcher for the `find`/`list` filter patterns. Supports the two
//! wildcards users reach for on the command line — `*` (any run of characters,
//! including none) and `?` (exactly one character) — and nothing else, so a
//! a bare name stays an exact match.

/// Whether `value` matches the glob `pattern`. Matching is case-sensitive;
/// package names are already normalised to lowercase before they reach here.
pub fn glob_match(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();

    // Two-pointer scan with backtracking to the last `*`: `star`/`mark` remember
    // where to resume the pattern and input when a later literal fails to match.
    let (mut p, mut v) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;

    while v < value.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = v;
            p += 1;
        } else if let Some(star_pos) = star {
            // Backtrack: let the last `*` swallow one more input character.
            p = star_pos + 1;
            mark += 1;
            v = mark;
        } else {
            return false;
        }
    }

    // Trailing `*`s in the pattern still match the empty remainder.
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Whether a glob `pattern` matches a `vendor/extension` package name — either
/// the full name or its leaf segment, so `gfx*` finds `acme/gfx` and `gfx`.
pub fn package_name_matches(pattern: &str, name: &str) -> bool {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    glob_match(pattern, name) || glob_match(pattern, leaf)
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn matches_prefix_wildcard() {
        assert!(glob_match("lib*", "libusb"));
        assert!(glob_match("lib*", "lib"));
        assert!(!glob_match("lib*", "zlib"));
    }

    #[test]
    fn matches_exact_without_wildcards() {
        assert!(glob_match("libusb", "libusb"));
        assert!(!glob_match("libusb", "libusb-1.0"));
    }

    #[test]
    fn matches_inner_and_single_char_wildcards() {
        assert!(glob_match("lib*usb", "libfoousb"));
        assert!(glob_match("lib*usb", "libusb"));
        assert!(glob_match("lib?", "libx"));
        assert!(!glob_match("lib?", "lib"));
        assert!(!glob_match("lib?", "libxx"));
    }

    #[test]
    fn matches_bare_star() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
    }
}
