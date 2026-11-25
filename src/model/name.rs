//! A PR's name (MOD-1). Built by T02.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `owner/repo#number` (MOD-1). `Eq`, `Hash` and `Ord` ignore case; `Display` keeps GitHub's spelling.
/// `Ord` is the order "of `pr`" (MOD-15, STA-2, API-6): owner, then repo, both lower-cased, then the
/// number as a number (MOD-1). Serialises as the string.
#[derive(Clone, Debug)]
pub struct PrName {
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

/// Not a PR's name in the form asked for.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("not a pull request name")]
pub struct NameError;

/// An owner or a repository as GitHub spells them: letters, digits, `-`, `_` and `.`.
fn part(s: &str) -> Result<String, NameError> {
    let ok = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(s.to_string())
    } else {
        Err(NameError)
    }
}

/// A PR number: digits only, no sign, at least 1, within `u64`.
fn number(s: &str) -> Result<u64, NameError> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(NameError);
    }
    match s.parse::<u64>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(NameError),
    }
}

impl PrName {
    /// "owner/repo#12" only.
    pub fn parse(s: &str) -> Result<PrName, NameError> {
        let (slug, num) = s.split_once('#').ok_or(NameError)?;
        let (owner, repo) = slug.split_once('/').ok_or(NameError)?;
        Ok(PrName {
            owner: part(owner)?,
            repo: part(repo)?,
            number: number(num)?,
        })
    }

    /// `https://github.com/owner/repo/pull/12[/…][?…][#…]`.
    pub fn from_url(s: &str) -> Result<PrName, NameError> {
        const PREFIX: &str = "https://github.com/";
        let head = s.get(..PREFIX.len()).ok_or(NameError)?;
        if !head.eq_ignore_ascii_case(PREFIX) {
            return Err(NameError);
        }
        let rest = &s[PREFIX.len()..];
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        let mut segments = rest.split('/');
        let owner = segments.next().ok_or(NameError)?;
        let repo = segments.next().ok_or(NameError)?;
        if segments.next() != Some("pull") {
            return Err(NameError);
        }
        let num = segments.next().ok_or(NameError)?;
        Ok(PrName {
            owner: part(owner)?,
            repo: part(repo)?,
            number: number(num)?,
        })
    }

    /// "owner/repo".
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

fn cmp_lower(a: &str, b: &str) -> Ordering {
    a.bytes()
        .map(|c| c.to_ascii_lowercase())
        .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
}

impl PartialEq for PrName {
    fn eq(&self, other: &PrName) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for PrName {}

impl Ord for PrName {
    fn cmp(&self, other: &PrName) -> Ordering {
        cmp_lower(&self.owner, &other.owner)
            .then_with(|| cmp_lower(&self.repo, &other.repo))
            .then_with(|| self.number.cmp(&other.number))
    }
}

impl PartialOrd for PrName {
    fn partial_cmp(&self, other: &PrName) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for PrName {
    fn hash<H: Hasher>(&self, h: &mut H) {
        for part in [&self.owner, &self.repo] {
            for b in part.bytes() {
                h.write_u8(b.to_ascii_lowercase());
            }
            h.write_u8(0xff); // never in a name: keeps `ab/c` and `a/bc` apart
        }
        self.number.hash(h);
    }
}

impl std::fmt::Display for PrName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}#{}", self.owner, self.repo, self.number)
    }
}

impl Serialize for PrName {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PrName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<PrName, D::Error> {
        let s = String::deserialize(d)?;
        PrName::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash(n: &PrName) -> u64 {
        let mut h = DefaultHasher::new();
        n.hash(&mut h);
        h.finish()
    }

    // MOD-1: the name, compared and ordered without case.
    #[test]
    fn mod1_pr_name() {
        let n = PrName::parse("owner/repo#12").unwrap();
        assert_eq!(
            (n.owner.as_str(), n.repo.as_str(), n.number),
            ("owner", "repo", 12)
        );
        assert_eq!(n.slug(), "owner/repo");
        for url in [
            "https://github.com/owner/repo/pull/12",
            "https://github.com/owner/repo/pull/12/files",
            "https://github.com/owner/repo/pull/12?tab=checks",
            "https://github.com/owner/repo/pull/12#issuecomment-1",
            "https://github.com/owner/repo/pull/12/files?w=1#diff",
        ] {
            assert_eq!(PrName::from_url(url).unwrap(), n, "{url}");
        }
        for bad in [
            "owner/repo",
            "#12",
            "owner/repo#x",
            "owner/repo#",
            "owner/repo#-1",
            "o/r/x#1",
            "owner/repo#0",
        ] {
            assert!(PrName::parse(bad).is_err(), "{bad}");
        }
        for bad in [
            "https://gitlab.com/owner/repo/pull/12",
            "https://github.com.evil/owner/repo/pull/12",
            "https://github.com/owner/repo/issues/12",
            "https://github.com/owner/repo",
            "owner/repo#12",
        ] {
            assert!(PrName::from_url(bad).is_err(), "{bad}");
        }

        let upper = PrName::parse("Yachiko/Circitor#12").unwrap();
        let lower = PrName::parse("yachiko/circitor#12").unwrap();
        assert_eq!(upper, lower);
        assert_eq!(hash(&upper), hash(&lower));
        assert_eq!(upper.to_string(), "Yachiko/Circitor#12");
        assert_eq!(
            serde_json::to_string(&upper).unwrap(),
            "\"Yachiko/Circitor#12\""
        );
        let back: PrName = serde_json::from_str("\"Yachiko/Circitor#12\"").unwrap();
        assert_eq!(back.to_string(), "Yachiko/Circitor#12");

        let mut names: Vec<PrName> = ["b/a#1", "a/c#1", "a/b#10", "a/b#9"]
            .iter()
            .map(|s| PrName::parse(s).unwrap())
            .collect();
        names.sort();
        let order: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        assert_eq!(order, ["a/b#9", "a/b#10", "a/c#1", "b/a#1"]);
    }
}
