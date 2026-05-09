// Normalize UUID strings to canonical dashed form.
//
// Different FTB releases store UUIDs in different shapes inside their NBT
// player files: GTNH ServerUtilities and 1.12.2 FTBU 5.x both omit the
// dashes (32-char hex), while modern FTB Chunks/Teams writes them dashed.
// We normalize at parse time so callers always see the dashed form
// regardless of source format — matching the documented schema.

/// Insert dashes into a 32-char hex string (`069a79f444e94726a5befca90e38aaf5`
/// → `069a79f4-44e9-4726-a5be-fca90e38aaf5`). Pass-through for already-dashed
/// or unrecognized strings.
pub fn normalize(s: &str) -> String {
    if s.len() == 36
        && s.as_bytes()[8] == b'-'
        && s.as_bytes()[13] == b'-'
        && s.as_bytes()[18] == b'-'
        && s.as_bytes()[23] == b'-'
    {
        return s.to_string();
    }
    if s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return format!(
            "{}-{}-{}-{}-{}",
            &s[0..8],
            &s[8..12],
            &s[12..16],
            &s[16..20],
            &s[20..32],
        );
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_dashed() {
        let s = "069a79f4-44e9-4726-a5be-fca90e38aaf5";
        assert_eq!(normalize(s), s);
    }

    #[test]
    fn dashes_undashed_hex() {
        assert_eq!(
            normalize("069a79f444e94726a5befca90e38aaf5"),
            "069a79f4-44e9-4726-a5be-fca90e38aaf5"
        );
    }

    #[test]
    fn passes_through_unknown_shape() {
        assert_eq!(normalize("8559re"), "8559re");
        assert_eq!(normalize(""), "");
    }
}
