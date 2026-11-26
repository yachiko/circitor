//! Strict query parameters (API-1; T43).

use std::str::FromStr;

use crate::model::PrName;
use crate::state::Cursor;

use super::types::{ApiError, ErrorCode};

/// Strict query parameters (API-1).
pub struct Params {
    pairs: Vec<(String, String)>,
}

/// `%XX` escapes decoded; `+` stays a plus. `None` for a bad escape or invalid UTF-8.
fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl Params {
    /// A name given twice is `bad_request`.
    pub fn parse(query: Option<&str>) -> Result<Params, ApiError> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        // Split on `&` and the first `=` before decoding, so an encoded `&` or `=` stays in its value.
        for part in query.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
            let (raw_name, raw_value) = part.split_once('=').unwrap_or((part, ""));
            let name = decode(raw_name)
                .ok_or_else(|| bad_request(format!("parameter `{raw_name}` is not valid")))?;
            let value = decode(raw_value).ok_or_else(|| {
                bad_request(format!("parameter `{name}` has a bad percent-escape"))
            })?;
            if pairs.iter().any(|(n, _)| *n == name) {
                return Err(bad_request(format!("parameter `{name}` is given twice")));
            }
            pairs.push((name, value));
        }
        Ok(Params { pairs })
    }

    pub fn take(&mut self, name: &str) -> Option<String> {
        let at = self.pairs.iter().position(|(n, _)| n == name)?;
        Some(self.pairs.remove(at).1)
    }

    /// `1` or `0`; absent is `false`.
    pub fn flag(&mut self, name: &str) -> Result<bool, ApiError> {
        match self.take(name).as_deref() {
            None | Some("0") => Ok(false),
            Some("1") => Ok(true),
            Some(_) => Err(bad_request(format!("parameter `{name}` must be 1 or 0"))),
        }
    }

    /// Never clamped.
    pub fn number(&mut self, name: &str, min: u32, max: u32) -> Result<Option<u32>, ApiError> {
        let Some(value) = self.take(name) else {
            return Ok(None);
        };
        // Digits only: `u32::from_str` would take a leading `+`.
        let n = if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
            u32::from_str(&value).ok()
        } else {
            None
        };
        match n {
            Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(bad_request(format!(
                "parameter `{name}` must be a whole number from {min} to {max}"
            ))),
        }
    }

    /// STA-9's form.
    pub fn cursor(&mut self, name: &str) -> Result<Option<Cursor>, ApiError> {
        match self.take(name) {
            None => Ok(None),
            Some(v) => Cursor::from_str(&v)
                .map(Some)
                .map_err(|_| bad_request(format!("parameter `{name}` is not a cursor"))),
        }
    }

    /// Anything left: `bad_request` naming it.
    pub fn finish(self) -> Result<(), ApiError> {
        if self.pairs.is_empty() {
            return Ok(());
        }
        let names: Vec<String> = self.pairs.iter().map(|(n, _)| format!("`{n}`")).collect();
        let word = if names.len() == 1 {
            "parameter"
        } else {
            "parameters"
        };
        Err(bad_request(format!("unknown {word} {}", names.join(", "))))
    }
}

pub fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError {
        code: ErrorCode::BadRequest,
        message: message.into(),
        until: None,
    }
}

/// `{owner}/{repo}/{number}` from a path; a number that isn't one is `bad_request`.
pub fn pr_from_path(owner: &str, repo: &str, number: &str) -> Result<PrName, ApiError> {
    // MOD-1: digits only, at least 1.
    let valid =
        number.bytes().all(|b| b.is_ascii_digit()) && number.parse::<u64>().is_ok_and(|n| n > 0);
    if !valid {
        return Err(bad_request(format!(
            "`{number}` is not a pull request number"
        )));
    }
    PrName::parse(&format!("{owner}/{repo}#{number}"))
        .map_err(|_| bad_request(format!("`{owner}/{repo}` is not a repository name")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(q: &str) -> Result<Params, ApiError> {
        Params::parse(Some(q))
    }

    // API-1: strict parameters.
    #[test]
    fn api1_params() {
        // Unknown, and named.
        let e = parse("bogus=1").unwrap().finish().unwrap_err();
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert!(e.message.contains("bogus"), "{}", e.message);
        // Twice.
        let e = parse("a=1&a=2").err().unwrap();
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert!(e.message.contains('a'));
        // Flags.
        assert!(parse("checks=1").unwrap().flag("checks").unwrap());
        assert!(!parse("checks=0").unwrap().flag("checks").unwrap());
        assert!(!Params::parse(None).unwrap().flag("checks").unwrap());
        for bad in ["checks=true", "checks=", "checks"] {
            let e = parse(bad).unwrap().flag("checks").unwrap_err();
            assert_eq!(e.code, ErrorCode::BadRequest, "{bad}");
            assert!(e.message.contains("checks"));
        }
        // Numbers: never clamped.
        let e = parse("timeout=3601")
            .unwrap()
            .number("timeout", 0, 3600)
            .unwrap_err();
        assert!(e.message.contains("timeout"));
        assert_eq!(
            parse("timeout=3600")
                .unwrap()
                .number("timeout", 0, 3600)
                .unwrap(),
            Some(3600)
        );
        assert_eq!(
            Params::parse(None)
                .unwrap()
                .number("timeout", 0, 3600)
                .unwrap(),
            None
        );
        for bad in [
            "timeout=abc",
            "timeout=-1",
            "timeout=%2B5",
            "timeout=",
            "timeout=99999999999",
        ] {
            let e = parse(bad).unwrap().number("timeout", 0, 3600).unwrap_err();
            assert!(e.message.contains("timeout"), "{bad}");
        }
        // Cursors.
        assert_eq!(
            parse("since=k3x9a2bq-5").unwrap().cursor("since").unwrap(),
            Some(Cursor {
                epoch: "k3x9a2bq".into(),
                rev: 5
            })
        );
        let e = parse("since=k3x9a2bq")
            .unwrap()
            .cursor("since")
            .unwrap_err();
        assert!(e.message.contains("since"));
        let e = parse("since=%zz").err().unwrap();
        assert_eq!(e.code, ErrorCode::BadRequest);
        assert!(e.message.contains("since"));
        let e = parse("since=%ff").err().unwrap();
        assert!(e.message.contains("since"));
        // A plus is a plus.
        assert_eq!(parse("x=a+b").unwrap().take("x").as_deref(), Some("a+b"));
        // An encoded `&` and `=` stay in the value.
        assert_eq!(
            parse("x=a%26b%3Dc").unwrap().take("x").as_deref(),
            Some("a&b=c")
        );
        // `finish` names the one left.
        let mut p = parse("a=1&b=2").unwrap();
        p.take("a");
        let e = p.finish().unwrap_err();
        assert!(e.message.contains("`b`") && !e.message.contains("`a`"));
    }

    // API-1: a path's number.
    #[test]
    fn api1_pr_from_path() {
        assert_eq!(pr_from_path("o", "r", "12").unwrap().to_string(), "o/r#12");
        for bad in ["x", "-1", "0", "", "+3", "1.5"] {
            let e = pr_from_path("o", "r", bad).unwrap_err();
            assert_eq!(e.code, ErrorCode::BadRequest);
            assert!(
                e.message.contains(&format!("`{bad}`")),
                "{bad}: {}",
                e.message
            );
        }
    }
}
