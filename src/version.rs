//! The version and commit the binary carries (OPS-11).

/// The built version: `MAJOR.MINOR.PATCH+<commit>`.
fn built_version() -> String {
    format!("{}+{}", env!("CARGO_PKG_VERSION"), env!("CIRCITOR_COMMIT"))
}

/// OPS-11: the version this binary reports. A test build lets
/// `CIRCITOR_TEST_VERSION` replace it (TST-1).
pub fn version() -> String {
    #[cfg(feature = "test-endpoint")]
    {
        let test = std::env::var("CIRCITOR_TEST_VERSION").ok();
        version_from(test.as_deref())
    }
    #[cfg(not(feature = "test-endpoint"))]
    {
        version_from(None)
    }
}

/// `Some(v)` gives `v` as it is, even if it doesn't parse (CLI-9); `None`
/// gives the built version.
pub fn version_from(test_version: Option<&str>) -> String {
    match test_version {
        Some(v) => v.to_string(),
        None => built_version(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // OPS-11
    #[test]
    fn ops11_version_is_semver_with_commit() {
        let built = version_from(None);
        let v = semver::Version::parse(&built).expect("semver");
        let pkg = semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("pkg semver");
        assert_eq!(
            (v.major, v.minor, v.patch),
            (pkg.major, pkg.minor, pkg.patch)
        );
        assert!(!v.build.is_empty());
    }

    // TST-1
    #[test]
    fn tst1_test_version_replaces_it() {
        assert_eq!(version_from(Some("9.9.9")), "9.9.9");
        assert_eq!(version_from(Some("not a version")), "not a version");
        assert_eq!(version_from(None), built_version());
    }
}
