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
/// least three numeric components (`1.2.3`, `1.2.3.4`). A `-suffix` (variant
/// like `-alpine` or prerelease like `-rc1`) doesn't affect immutability.
/// Truncated versions (`3`, `3.41`), named tags (`latest`, `alpine`), and
/// everything else float.
/// Tags excluded from tracking entirely: never stored, never fetched.
/// These are per-commit/per-PR CI artifacts, not versions anyone deploys.
pub fn is_excluded(tag: &str) -> bool {
    tag.starts_with("pr-") || tag.starts_with("commit-")
}

pub fn is_immutable(tag: &str) -> bool {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let core = tag.split('-').next().unwrap_or("");

    let components: Vec<&str> = core.split('.').collect();
    components.len() >= 3
        && components
            .iter()
            .all(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{is_excluded, is_immutable};

    #[test]
    fn ci_artifact_tags_are_excluded() {
        assert!(is_excluded("pr-3430"));
        assert!(is_excluded("commit-8f2c1aa"));
        assert!(!is_excluded("latest"));
        assert!(!is_excluded("v3.41.3"));
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
    }
}
