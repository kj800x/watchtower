/// Tags excluded from tracking entirely: never stored, never fetched.
///
/// - `pr-*` / `commit-*`: per-PR and per-commit CI artifacts
/// - `sha256-*`: cosign signature/attestation/SBOM artifacts
///   (`sha256-<digest>.sig` and friends)
/// - `nightly-*` and anything containing `unstable`: dated nightly/unstable
///   CI builds (linuxserver.io publishes thousands); the bare `nightly`
///   channel pointer is kept
/// - `bionic-*`: distro-prefixed duplicates of the main tags
/// - arch-prefixed tags (`amd64-*`, `arm64v8-*`, ...): single-arch
///   duplicates of the multi-arch tags
pub fn is_excluded(tag: &str) -> bool {
    const EXCLUDED_PREFIXES: &[&str] = &[
        "pr-", "commit-", "sha256-", "nightly-", "bionic-", "amd64-", "arm64v8-", "arm64v6-",
        "arm32v7-", "arm32v6-", "armhf-", "i386-", "ppc64le-", "s390x-", "riscv64-",
    ];
    EXCLUDED_PREFIXES.iter().any(|p| tag.starts_with(p)) || tag.contains("unstable")
}

/// Whether a tag names an exact release and can be assumed immutable.
///
/// Immutable tags get their digest fetched once and cached forever; everything
/// else is treated as a floating pointer and re-fetched on every repo refresh.
/// Misclassifying floating-as-immutable means a permanently stale digest, while
/// immutable-as-floating just costs a redundant HEAD per refresh — so when in
/// doubt, classify as floating.
///
/// A tag is immutable iff, after stripping an optional leading `v`, its core
/// (everything before the first `-`) is a complete dotted version with at
/// least three numeric components (`1.2.3`, `1.2.3.4`). The final component
/// may carry a trailing alphanumeric run (`10.10.0ubu2404`, as linuxserver.io
/// glues the distro onto the patch version). A `-suffix` (variant like
/// `-alpine`, prerelease like `-rc1`, or build like `-ls38`) doesn't affect
/// immutability. Truncated versions (`3`, `3.41`), named tags (`latest`,
/// `alpine`), and everything else float.
pub fn is_immutable(tag: &str) -> bool {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let core = tag.split('-').next().unwrap_or("");

    let components: Vec<&str> = core.split('.').collect();
    let Some((last, init)) = components.split_last() else {
        return false;
    };
    components.len() >= 3
        && init
            .iter()
            .all(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
        && is_digits_then_alnum(last)
}

/// One or more digits, optionally followed by alphanumerics ("0", "0ubu2404").
fn is_digits_then_alnum(s: &str) -> bool {
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    digits > 0 && s.bytes().skip(digits).all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::{is_excluded, is_immutable};

    #[test]
    fn ci_artifact_tags_are_excluded() {
        assert!(is_excluded("pr-3430"));
        assert!(is_excluded("commit-8f2c1aa"));
        assert!(is_excluded(
            "sha256-12284c68c09a6a50b4bbb7195e3d9cdb6ff50c102ce543f901ad7ed9d7d52844.sig"
        ));
        assert!(is_excluded("amd64-10.8.13-ls249"));
        assert!(is_excluded("arm64v8-latest"));
        assert!(is_excluded("arm32v7-2021.10.06"));
        assert!(is_excluded("arm32v6-2.0.12"));
        assert!(is_excluded("nightly-20201215.24-unstable-ls174"));
        assert!(is_excluded("nightly-version-2024030717ubu2204"));
        assert!(is_excluded("20201102.25-unstable-ls107"));
        assert!(is_excluded("version-20201102.25-unstable"));
        assert!(is_excluded("bionic-10.6.4-1-ls10"));
        assert!(!is_excluded("latest"));
        assert!(!is_excluded("nightly"));
        assert!(!is_excluded("v3.41.3"));
        assert!(!is_excluded("10.8.13-ls249"));
        assert!(!is_excluded("prod"));
        assert!(!is_excluded("commitment"));
    }

    #[test]
    fn full_versions_are_immutable() {
        assert!(is_immutable("v3.41.3"));
        assert!(is_immutable("3.5.7"));
        assert!(is_immutable("1.2.3.4"));
        assert!(is_immutable("7.0.15-alpine"));
        assert!(is_immutable("1.2.3-rc1"));
        assert!(is_immutable("v0.14.9"));
        assert!(is_immutable("10.10.0ubu2404-ls38"));
        assert!(is_immutable("10.6.4-1-ls10"));
    }

    #[test]
    fn truncated_versions_float() {
        assert!(!is_immutable("v3"));
        assert!(!is_immutable("3.41"));
        assert!(!is_immutable("12.4"));
        assert!(!is_immutable("18"));
        assert!(!is_immutable("8-alpine"));
    }

    #[test]
    fn named_tags_float() {
        assert!(!is_immutable("latest"));
        assert!(!is_immutable("alpine"));
        assert!(!is_immutable("edge"));
        assert!(!is_immutable("noble"));
        assert!(!is_immutable("pr-3430"));
        assert!(!is_immutable("java21"));
        assert!(!is_immutable("18beta1"));
    }

    #[test]
    fn degenerate_tags_float() {
        assert!(!is_immutable(""));
        assert!(!is_immutable("v"));
        assert!(!is_immutable("1..3"));
        assert!(!is_immutable("-alpine"));
        assert!(!is_immutable("a.b.c"));
        assert!(!is_immutable("1.2.ubu2404"));
        assert!(!is_immutable("nightly-2024030717ubu2204-ls3"));
    }
}
