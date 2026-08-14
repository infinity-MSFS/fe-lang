//! `file:` URIs and paths.
//!
//! The protocol speaks URIs and the filesystem speaks paths, so every request
//! crosses this boundary twice. The conversion is small but it is not obvious —
//! `C:\dev\a.fe` is `file:///C:/dev/a.fe`, a UNC share is `file://server/share`,
//! and a space is `%20` — and it is exactly the sort of thing that works on the
//! machine it was written on and fails on everybody else's.
//!
//! So the rules are written as pure functions taking `windows: bool` rather than
//! behind `cfg`, and the tests exercise both conventions on whatever platform
//! they run on. Windows behaviour that only Windows can test is Windows
//! behaviour nobody tests.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

/// Unreserved characters, plus the sub-delimiters and `:`/`@` that RFC 3986
/// permits inside a path segment. Everything else is percent-encoded.
fn is_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~:@!$&'()*+,;=".contains(&b)
}

pub fn from_path(path: &Path) -> Option<Uri> {
    let text = path.to_str()?;
    Uri::from_str(&encode(text, cfg!(windows))).ok()
}

pub fn to_path(uri: &Uri) -> Option<PathBuf> {
    decode(uri.as_str(), cfg!(windows)).map(PathBuf::from)
}

/// Path text to `file:` URI text.
fn encode(path: &str, windows: bool) -> String {
    let mut out = String::from("file://");

    let rest = if windows {
        let path = path.replace('\\', "/");
        match path.strip_prefix("//") {
            // A UNC path: `//server/share/x` puts the host in the authority.
            Some(after) => {
                let (host, tail) = after.split_once('/').unwrap_or((after, ""));
                push_encoded(&mut out, host);
                tail.to_string()
            }
            // A drive path: `C:/x` becomes `/C:/x`, against the empty authority
            // already written above.
            None => path.strip_prefix('/').unwrap_or(&path).to_string(),
        }
    } else {
        path.strip_prefix('/').unwrap_or(path).to_string()
    };

    // The path always begins with `/`, so the root is `file:///` and not an
    // empty path with a stray separator.
    for segment in rest.split('/') {
        out.push('/');
        push_encoded(&mut out, segment);
    }
    out
}

fn push_encoded(out: &mut String, text: &str) {
    for &b in text.as_bytes() {
        if is_safe(b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
}

/// `file:` URI text back to path text. `None` for any other scheme — a client
/// may legitimately open an `untitled:` buffer, and that has no path.
fn decode(uri: &str, windows: bool) -> Option<String> {
    let rest = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("FILE://"))?;
    // Everything from `#` or `?` on is fragment or query, not path.
    let rest = rest.split(['#', '?']).next().unwrap_or(rest);
    let (authority, path) = match rest.find('/') {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };

    let authority = percent_decode(authority)?;
    let path = percent_decode(path)?;

    if !windows {
        // A host on a POSIX path is not something we can open. `localhost` is
        // the one spelling of "this machine" that appears in the wild.
        if !authority.is_empty() && authority != "localhost" {
            return None;
        }
        return Some(path);
    }

    let path = if !authority.is_empty() && authority != "localhost" {
        format!("\\\\{authority}{}", path.replace('/', "\\"))
    } else {
        // `/C:/x` is a drive path; anything else is rooted and stays that way.
        let trimmed = path.strip_prefix('/').unwrap_or(&path);
        if is_drive_prefixed(trimmed) {
            trimmed.replace('/', "\\")
        } else {
            path.replace('/', "\\")
        }
    };
    Some(path)
}

fn is_drive_prefixed(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn percent_decode(text: &str) -> Option<String> {
    if !text.contains('%') {
        return Some(text.to_string());
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn round_trip(path: &str, uri: &str, windows: bool) {
        assert_eq!(encode(path, windows), uri, "encoding {path}");
        assert_eq!(
            decode(uri, windows).as_deref(),
            Some(path),
            "decoding {uri}"
        );
    }

    #[test]
    fn posix_paths() {
        round_trip(
            "/home/user/dc10/hydraulic.fe",
            "file:///home/user/dc10/hydraulic.fe",
            false,
        );
        round_trip("/tmp/a b.fe", "file:///tmp/a%20b.fe", false);
        round_trip("/", "file:///", false);
    }

    #[test]
    fn windows_drive_paths() {
        round_trip(
            "C:\\dev\\dc10\\fuel.fe",
            "file:///C:/dev/dc10/fuel.fe",
            true,
        );
        round_trip(
            "C:\\Program Files\\a.fe",
            "file:///C:/Program%20Files/a.fe",
            true,
        );
    }

    #[test]
    fn windows_unc_paths() {
        round_trip(
            "\\\\build\\share\\dc10\\fuel.fe",
            "file://build/share/dc10/fuel.fe",
            true,
        );
    }

    /// MSFS addon trees live under `...\Packages\`, and people do put accented
    /// and CJK characters in project folder names.
    #[test]
    fn non_ascii_is_utf8_percent_encoded() {
        round_trip(
            "/home/josé/procédures.fe",
            "file:///home/jos%C3%A9/proc%C3%A9dures.fe",
            false,
        );
        round_trip(
            "/home/航空/a.fe",
            "file:///home/%E8%88%AA%E7%A9%BA/a.fe",
            false,
        );
    }

    /// `#` would otherwise start a fragment and silently truncate the path.
    #[test]
    fn characters_that_would_change_the_uri_are_encoded() {
        round_trip("/tmp/a#b.fe", "file:///tmp/a%23b.fe", false);
        round_trip("/tmp/a?b.fe", "file:///tmp/a%3Fb.fe", false);
    }

    #[test]
    fn a_query_or_fragment_is_not_part_of_the_path() {
        assert_eq!(
            decode("file:///tmp/a.fe#L3", false).as_deref(),
            Some("/tmp/a.fe")
        );
        assert_eq!(
            decode("file:///tmp/a.fe?v=1", false).as_deref(),
            Some("/tmp/a.fe")
        );
    }

    #[test]
    fn localhost_is_this_machine() {
        assert_eq!(
            decode("file://localhost/tmp/a.fe", false).as_deref(),
            Some("/tmp/a.fe")
        );
    }

    #[test]
    fn other_schemes_have_no_path() {
        assert_eq!(decode("untitled:Untitled-1", false), None);
        assert_eq!(decode("https://example.com/a.fe", false), None);
        // A remote host is not something this process can open.
        assert_eq!(decode("file://elsewhere/tmp/a.fe", false), None);
    }

    #[test]
    fn malformed_escapes_do_not_panic() {
        assert_eq!(decode("file:///tmp/a%", false), None);
        assert_eq!(decode("file:///tmp/a%zz", false), None);
        assert_eq!(decode("file:///tmp/a%FF", false), None); // not valid UTF-8
    }

    /// The encoded form has to be something the protocol's own URI parser will
    /// accept, or the client never sees it.
    #[test]
    fn encoded_uris_parse() {
        for path in ["/home/user/a b.fe", "/home/josé/a.fe", "/tmp/a#b.fe"] {
            let text = encode(path, false);
            assert!(Uri::from_str(&text).is_ok(), "{text} should parse");
        }
    }
}
