// Optional ownership control for outputs.
//
// Backs the global `--chown` flag. When set, every file or directory the tool
// creates or atomically replaces gets chowned to the requested uid/gid.
// Unix-only — `apply` is a compile-time no-op on other platforms, and the
// flag is rejected at startup before reaching any code path that would call
// it.

use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChownSpec {
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

static CHOWN_SPEC: OnceLock<ChownSpec> = OnceLock::new();

pub fn set(spec: ChownSpec) {
    let _ = CHOWN_SPEC.set(spec);
}

/// Parse the `--chown` argument value. Accepts `OWNER`, `OWNER:GROUP`, or
/// `:GROUP`, where each component is either a numeric id or a name resolved
/// via NSS (`getpwnam_r` / `getgrnam_r`). Rejects empty input, unknown
/// names, and the degenerate `:` (which would specify neither).
pub fn parse_spec(s: &str) -> Result<ChownSpec, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("--chown value is empty".to_string());
    }

    let (uid_part, gid_part) = match trimmed.split_once(':') {
        Some((u, g)) => (u, Some(g)),
        None => (trimmed, None),
    };

    let uid = if uid_part.is_empty() {
        None
    } else {
        Some(resolve_user(uid_part)?)
    };
    let gid = match gid_part {
        None => None,
        Some(g) if g.is_empty() => {
            return Err(format!(
                "--chown {:?}: group is empty after ':'",
                trimmed
            ));
        }
        Some(g) => Some(resolve_group(g)?),
    };

    if uid.is_none() && gid.is_none() {
        return Err(format!(
            "--chown {:?}: must specify at least a user or :group",
            trimmed
        ));
    }
    Ok(ChownSpec { uid, gid })
}

fn resolve_user(part: &str) -> Result<u32, String> {
    if let Ok(n) = part.parse::<u32>() {
        return Ok(n);
    }
    #[cfg(unix)]
    {
        match uzers::get_user_by_name(part) {
            Some(user) => Ok(user.uid()),
            None => Err(format!("unknown user {:?} in --chown", part)),
        }
    }
    #[cfg(not(unix))]
    Err(format!(
        "invalid user {:?} in --chown (name lookup is Unix-only)",
        part
    ))
}

fn resolve_group(part: &str) -> Result<u32, String> {
    if let Ok(n) = part.parse::<u32>() {
        return Ok(n);
    }
    #[cfg(unix)]
    {
        match uzers::get_group_by_name(part) {
            Some(group) => Ok(group.gid()),
            None => Err(format!("unknown group {:?} in --chown", part)),
        }
    }
    #[cfg(not(unix))]
    Err(format!(
        "invalid group {:?} in --chown (name lookup is Unix-only)",
        part
    ))
}

/// Apply the configured chown to `path`. No-op when no spec is set, or on
/// non-Unix platforms (the flag is refused at startup there).
#[cfg(unix)]
pub fn apply(path: &Path) -> std::io::Result<()> {
    if let Some(spec) = CHOWN_SPEC.get().copied() {
        std::os::unix::fs::chown(path, spec.uid, spec.gid)?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn apply(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// True iff the current process has effective uid 0.
#[cfg(unix)]
pub fn is_root() -> bool {
    // SAFETY: geteuid is always safe; it has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
pub fn is_root() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uid_only() {
        assert_eq!(
            parse_spec("1000").unwrap(),
            ChownSpec { uid: Some(1000), gid: None }
        );
    }

    #[test]
    fn parses_uid_gid() {
        assert_eq!(
            parse_spec("1000:2000").unwrap(),
            ChownSpec { uid: Some(1000), gid: Some(2000) }
        );
    }

    #[test]
    fn parses_gid_only() {
        assert_eq!(
            parse_spec(":2000").unwrap(),
            ChownSpec { uid: None, gid: Some(2000) }
        );
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_spec("").is_err());
        assert!(parse_spec("   ").is_err());
    }

    #[test]
    fn rejects_bare_colon() {
        assert!(parse_spec(":").is_err());
    }

    #[test]
    fn rejects_unknown_names() {
        // Names that are practically guaranteed not to resolve via NSS on any
        // host. Numeric forms still work; this asserts non-numeric input that
        // matches no user/group is reported, not silently dropped.
        let bogus = "mcmap_test_definitely_no_such_principal_xyz123";
        assert!(parse_spec(bogus).is_err());
        assert!(parse_spec(&format!("{}:1000", bogus)).is_err());
        assert!(parse_spec(&format!("1000:{}", bogus)).is_err());
    }

    #[test]
    fn rejects_negative() {
        assert!(parse_spec("-1").is_err());
        assert!(parse_spec("1000:-1").is_err());
    }

    #[test]
    fn rejects_trailing_colon() {
        assert!(parse_spec("1000:").is_err());
    }
}
