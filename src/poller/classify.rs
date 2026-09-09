/// Tags excluded from tracking entirely: never stored, never fetched.
///
/// - `pr-*` / `commit-*` / `sha-*`: per-PR and per-commit CI artifacts
/// - `sha256-*`: cosign signature/attestation/SBOM artifacts
///   (`sha256-<digest>.sig` and friends)
/// - `nightly-*` and anything containing `unstable`: dated nightly/unstable
///   CI builds (linuxserver.io publishes thousands); the bare `nightly`
///   channel pointer is kept
/// - `bionic-*`: distro-prefixed duplicates of the main tags
/// - arch-prefixed or -suffixed tags (`amd64-*`, `*-arm64`, ...): single-arch
///   duplicates of the multi-arch tags
/// - `version-*` / `*-version-*`: linuxserver.io's mutable alias of the
///   newest `-lsN` build for an upstream version (`develop-version-4.0.9.2513`
///   duplicates `develop-4.0.9.2513-ls100`)
/// - tags carrying a commit hash or CI run id segment (`main-cb3cbef`,
///   `heads-branch-0-g44452566d`, `13.1.0-25893932881-ubuntu`): per-commit
///   branch builds; date-shaped digit runs (`20200413`) are not hashes
pub fn is_excluded(tag: &str) -> bool {
    const EXCLUDED_PREFIXES: &[&str] = &[
        "pr-", "commit-", "sha-", "sha256-", "nightly-", "bionic-", "version-",
    ];
    const ARCH: &[&str] = &[
        "amd64", "arm64", "arm", "armv6", "armv7", "arm64v8", "arm32v6", "arm32v7", "armhf",
        "i386", "386", "ppc64le", "s390x", "riscv64",
    ];
    if EXCLUDED_PREFIXES.iter().any(|p| tag.starts_with(p)) || tag.contains("unstable") {
        return true;
    }
    if ARCH
        .iter()
        .any(|a| tag.starts_with(a) && tag[a.len()..].starts_with('-'))
    {
        return true;
    }
    tag.split('-')
        .any(|seg| ARCH.contains(&seg) || seg == "version" || is_commit_ish(seg))
}

/// A dash-delimited segment that looks like a commit hash or CI run id: 7 to
/// 40 hex characters, optionally `git describe`-style `g`-prefixed. Pure
/// digit runs that form a valid date (`20200413`, `202004131200`) are
/// build dates, not hashes.
fn is_commit_ish(seg: &str) -> bool {
    let hex = seg.strip_prefix('g').unwrap_or(seg);
    (7..=40).contains(&hex.len())
        && hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        && hex.bytes().any(|b| b.is_ascii_digit())
        && !is_date_like(hex)
}

/// `YYYYMMDD` optionally followed by `HH`, `HHMM` or `HHMMSS`, with every
/// component in range.
fn is_date_like(s: &str) -> bool {
    if !matches!(s.len(), 8 | 10 | 12 | 14) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let num = |range: std::ops::Range<usize>| s[range].parse::<u32>().unwrap_or(u32::MAX);
    let (year, month, day) = (num(0..4), num(4..6), num(6..8));
    if !(1990..=2099).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    let in_range =
        |range: std::ops::Range<usize>, max: u32| range.end > s.len() || num(range) <= max;
    in_range(8..10, 23) && in_range(10..12, 59) && in_range(12..14, 59)
}

/// The version a tag names and the variant it names it for, when the tag
/// is shaped like one. Consumers (cicd) resolve ranges over these instead
/// of parsing raw tags, so the rules live here, next to the immutability
/// ones.
///
/// After an optional leading `v`, the core (everything before the first
/// `-`) must be two to four numeric components: `15.11`, `1.27.3`,
/// `4.0.19.2979`. Whatever follows the first `-` is the variant
/// (`alpine`, `java25`, `ls323`, `rc1`): the same version built another
/// way, or a prerelease of it; telling those apart is the consumer's job.
/// One-part tags (`18`), named tags (`latest`), glued cores
/// (`10.11.8ubu2404`) and underscored ones (`5.2.3_v2.0.14`) name no
/// version. `version` is the core as written, `v` removed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TagVersion {
    pub version: String,
    pub variant: Option<String>,
}

pub fn parse_version(tag: &str) -> Option<TagVersion> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    let (core, variant) = match stripped.split_once('-') {
        Some((_, "")) => return None,
        Some((core, variant)) => (core, Some(variant)),
        None => (stripped, None),
    };
    let components: Vec<&str> = core.split('.').collect();
    let numeric = |c: &&str| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit());
    if !(2..=4).contains(&components.len()) || !components.iter().all(numeric) {
        return None;
    }
    Some(TagVersion {
        version: core.to_string(),
        variant: variant.map(String::from),
    })
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
    use super::{TagVersion, is_excluded, is_immutable, parse_version};

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
        assert!(is_excluded("sha-f2c9d2f"));
        assert!(is_excluded("main-cb3cbef"));
        assert!(is_excluded("main-cb3cbef-arm64"));
        assert!(is_excluded("k99-5206e3a"));
        assert!(is_excluded("groupcache-6f1c2ab-WIP-2"));
        assert!(is_excluded(
            "heads-storage-downsampling-per-tenant-0-gd596e1475"
        ));
        assert!(is_excluded("13.1.0-25893932881-ubuntu"));
        assert!(is_excluded("dependabot-uv-dev-utilities-minor-da626189b9"));
        assert!(is_excluded("latest-arm64"));
        assert!(is_excluded("helm-loki-5.44.1-arm"));
        assert!(is_excluded("v1.53.1-enterprise-ppc64le"));
        assert!(is_excluded("v1.53.1-386"));
        assert!(is_excluded("version-12.0ubu2604"));
        assert!(is_excluded("develop-version-4.0.9.2513"));
        assert!(is_excluded("v4-version-4.0.0.247"));
        assert!(is_excluded("libtorrentv1-version-release-5.2.0_v1.2.20"));
        assert!(!is_excluded("latest"));
        assert!(!is_excluded("nightly"));
        assert!(!is_excluded("v3.41.3"));
        assert!(!is_excluded("10.8.13-ls249"));
        assert!(!is_excluded("prod"));
        assert!(!is_excluded("commitment"));
        assert!(!is_excluded("20200413"));
        assert!(!is_excluded("2021.9.0-openj9-11"));
        assert!(!is_excluded("develop-4.0.9.2513-ls100"));
        assert!(!is_excluded("6.4.4-nightly"));
        assert!(!is_excluded("13.1.0-boringcrypto"));
        assert!(!is_excluded("v1.53.1-enterprise-scratch"));
        assert!(!is_excluded(
            "4.4.0202012141920-7145-c01d28a47ubuntu18.04.1"
        ));
        assert!(!is_excluded("2.0.0.5344-ls9"));
        assert!(!is_excluded("armada"));
        assert!(!is_excluded("shadow"));
        assert!(!is_excluded("openj9-nightly"));
    }

    #[test]
    fn date_shaped_digit_runs_are_not_hashes() {
        use super::{is_commit_ish, is_date_like};
        assert!(is_date_like("20200413"));
        assert!(is_date_like("2024030717"));
        assert!(is_date_like("202403071730"));
        assert!(is_date_like("20240307173059"));
        assert!(!is_date_like("20201345"));
        assert!(!is_date_like("2024030725"));
        assert!(!is_date_like("25893932881"));
        assert!(!is_date_like("1234567"));
        assert!(!is_commit_ish("20200413"));
        assert!(is_commit_ish("25893932881"));
        assert!(is_commit_ish("cb3cbef"));
        assert!(is_commit_ish("g44452566d"));
        assert!(!is_commit_ish("g"));
        assert!(!is_commit_ish("abcdef"));
        assert!(!is_commit_ish("698b54o"));
        assert!(!is_commit_ish("CB3CBEF1"));
        assert!(!is_commit_ish("defaced"));
        assert!(!is_commit_ish("gdefaced"));
    }

    fn version(version: &str, variant: Option<&str>) -> Option<TagVersion> {
        Some(TagVersion {
            version: version.to_string(),
            variant: variant.map(String::from),
        })
    }

    #[test]
    fn versions_have_two_to_four_numeric_parts() {
        assert_eq!(parse_version("1.27.3"), version("1.27.3", None));
        assert_eq!(parse_version("v1.12.1"), version("1.12.1", None));
        assert_eq!(parse_version("15.11"), version("15.11", None));
        assert_eq!(parse_version("4.0.19.2979"), version("4.0.19.2979", None));
        assert_eq!(parse_version("2021.12.07"), version("2021.12.07", None));
        assert_eq!(parse_version("18"), None, "one part is a channel");
        assert_eq!(parse_version("latest"), None);
        assert_eq!(parse_version("java25"), None);
        assert_eq!(parse_version("1.2.3.4.5"), None);
        assert_eq!(parse_version("10.11.8ubu2404"), None, "glued distro");
        assert_eq!(parse_version("5.2.3_v2.0.14"), None);
        assert_eq!(parse_version("1..3"), None);
        assert_eq!(parse_version("v"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn the_first_dash_starts_the_variant() {
        assert_eq!(
            parse_version("2.1.2-alpine"),
            version("2.1.2", Some("alpine"))
        );
        assert_eq!(
            parse_version("2026.9.0-java25"),
            version("2026.9.0", Some("java25"))
        );
        assert_eq!(
            parse_version("4.0.19.2979-ls323"),
            version("4.0.19.2979", Some("ls323"))
        );
        assert_eq!(parse_version("1.2.3-rc1"), version("1.2.3", Some("rc1")));
        assert_eq!(
            parse_version("15.11-bookworm"),
            version("15.11", Some("bookworm"))
        );
        assert_eq!(
            parse_version("2026.9.0-java25-alpine"),
            version("2026.9.0", Some("java25-alpine"))
        );
        assert_eq!(
            parse_version("v1.53.1-enterprise-scratch"),
            version("1.53.1", Some("enterprise-scratch"))
        );
        assert_eq!(parse_version("1.2.3-"), None);
        assert_eq!(parse_version("-alpine"), None);
        assert_eq!(parse_version("8-alpine"), None);
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
