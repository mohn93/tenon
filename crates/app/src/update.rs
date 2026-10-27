//! Help > Check for Updates: asks GitHub for the repository's latest release and compares its
//! version with the running one. It only reports and links to the release page; nothing is
//! downloaded or installed, and nothing is asked until the person chooses the item.

#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

/// The version this program was built as.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The short git hash of the commit this program was built from, when the build knew it.
pub const GIT_HASH: Option<&str> = option_env!("TENON_GIT_HASH");

/// A release's page: this and its tag. A link of any other shape is not trusted.
const RELEASE_TAG_PREFIX: &str = "https://github.com/mohn93/sketchup/releases/tag/";

#[cfg(not(target_arch = "wasm32"))]
const LATEST_URL: &str = "https://api.github.com/repos/mohn93/sketchup/releases/latest";

/// Where the releases are listed, for the person to open.
pub const RELEASES_PAGE: &str = "https://github.com/mohn93/sketchup/releases";

/// The version with the build's commit, as About shows it: `0.1.0 (3fbb7ed)`.
pub fn version_label() -> String {
    match GIT_HASH.filter(|h| !h.is_empty()) {
        Some(hash) => format!("{VERSION} ({hash})"),
        None => VERSION.to_string(),
    }
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A newer release exists: its version and its page.
    Available {
        version: String,
        url: String,
    },
    UpToDate,
    /// The repository has no releases yet.
    NoReleases,
    Failed(String),
}

/// A version as `major.minor.patch` and its pre-release identifiers (`1.2.0-beta.1`: `beta`,
/// `1`; none for a release).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Version<'a> {
    core: (u64, u64, u64),
    pre: Vec<&'a str>,
}

impl<'a> Version<'a> {
    /// Accepts a leading `v`, one to three numeric parts, a `-pre-release` and `+build` suffix.
    fn parse(text: &'a str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let text = text.split('+').next()?;
        let (numbers, pre) = match text.split_once('-') {
            Some((numbers, pre)) => (numbers, pre.split('.').collect()),
            None => (text, Vec::new()),
        };
        let mut parts = numbers.split('.');
        let mut next = |required: bool| match parts.next() {
            Some(p) => p.parse::<u64>().ok(),
            None => (!required).then_some(0),
        };
        let core = (next(true)?, next(false)?, next(false)?);
        parts.next().is_none().then_some(Self { core, pre })
    }
}

impl Ord for Version<'_> {
    /// Semantic versioning's precedence: by number, a pre-release before its release, and
    /// pre-releases by identifier (numbers by value and before words, a longer list after its
    /// own beginning). Build metadata is ignored.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        let identifier = |a: &str, b: &str| match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => a.cmp(b),
        };
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self
                    .pre
                    .iter()
                    .zip(&other.pre)
                    .map(|(a, b)| identifier(a, b))
                    .find(|o| o.is_ne())
                    .unwrap_or_else(|| self.pre.len().cmp(&other.pre.len())),
            })
    }
}

impl PartialOrd for Version<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Whether `latest` names a newer version than `current`. A version that cannot be read is
/// never newer. A pre-release is older than its release.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(latest), Some(current)) = (Version::parse(latest), Version::parse(current)) else {
        return false;
    };
    latest > current
}

/// Whether `url` is a release page of the repository: the tag page, with a tag of letters,
/// digits and `.`, `-`, `_`, `+` only, so the browser cannot be led elsewhere by `..`, an
/// escape, a backslash, a query or a host trick.
fn is_release_page(url: &str) -> bool {
    url.strip_prefix(RELEASE_TAG_PREFIX).is_some_and(|tag| {
        !tag.is_empty()
            && tag.chars().any(|c| c != '.')
            && tag
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
    })
}

/// The outcome of the GitHub response `status`/`body` for a program at `current`.
pub fn interpret(status: u16, body: &str, current: &str) -> Outcome {
    match status {
        200 => {}
        404 => return Outcome::NoReleases,
        403 | 429 => {
            return Outcome::Failed("GitHub is limiting requests; try again later".into());
        }
        other => return Outcome::Failed(format!("GitHub answered with status {other}")),
    }
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return Outcome::Failed("GitHub's answer was not understood".into());
    };
    let Some(tag) = json.get("tag_name").and_then(|t| t.as_str()) else {
        return Outcome::Failed("GitHub's answer named no release".into());
    };
    if !is_newer(tag, current) {
        return Outcome::UpToDate;
    }
    let url = json
        .get("html_url")
        .and_then(|u| u.as_str())
        .filter(|u| is_release_page(u))
        .unwrap_or(RELEASES_PAGE);
    Outcome::Available {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: url.to_string(),
    }
}

/// Asks GitHub for the latest release (blocking; run it off the window's thread).
#[cfg(not(target_arch = "wasm32"))]
pub fn check() -> Outcome {
    use std::time::Duration;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into();
    let response = agent
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            concat!("tenon/", env!("CARGO_PKG_VERSION")),
        )
        .call();
    let mut response = match response {
        Ok(r) => r,
        Err(e) => return Outcome::Failed(format!("Could not reach GitHub: {e}")),
    };
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(1 << 20)
        .read_to_string()
        .unwrap_or_default();
    interpret(status, &body, VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_text() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("v0.1.0+build5", "0.1.0"));
    }

    #[test]
    fn a_pre_release_is_older_than_its_release() {
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        assert!(is_newer("0.2.0-beta.1", "0.1.0"));
    }

    #[test]
    fn unreadable_versions_are_never_newer() {
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("1.2.3.4", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer("2.0.0", "unknown"));
    }

    fn release(tag: &str, url: &str) -> String {
        serde_json::json!({"tag_name": tag, "html_url": url, "draft": false}).to_string()
    }

    #[test]
    fn a_newer_release_is_offered_with_its_page() {
        let url = "https://github.com/mohn93/sketchup/releases/tag/v0.3.0";
        assert_eq!(
            interpret(200, &release("v0.3.0", url), "0.2.0"),
            Outcome::Available {
                version: "0.3.0".into(),
                url: url.into()
            }
        );
    }

    #[test]
    fn the_same_or_an_older_release_is_up_to_date() {
        let url = "https://github.com/mohn93/sketchup/releases/tag/v0.2.0";
        assert_eq!(
            interpret(200, &release("v0.2.0", url), "0.2.0"),
            Outcome::UpToDate
        );
        assert_eq!(
            interpret(200, &release("v0.1.0", url), "0.2.0"),
            Outcome::UpToDate
        );
    }

    #[test]
    fn a_link_outside_the_repository_falls_back_to_the_releases_page() {
        assert_eq!(
            interpret(200, &release("v9.0.0", "https://evil.example/x"), "0.1.0"),
            Outcome::Available {
                version: "9.0.0".into(),
                url: RELEASES_PAGE.into()
            }
        );
    }

    #[test]
    fn only_a_release_page_of_the_repository_is_trusted() {
        let offered = |url: &str| match interpret(200, &release("v9.0.0", url), "0.1.0") {
            Outcome::Available { url, .. } => url,
            other => panic!("{other:?}"),
        };
        let good = "https://github.com/mohn93/sketchup/releases/tag/v9.0.0-rc.1+b2";
        assert_eq!(offered(good), good);
        for bad in [
            "https://github.com/mohn93/sketchup-evil/releases/tag/v9.0.0",
            "https://github.com/mohn93/sketchup@evil.example/releases/tag/v9.0.0",
            "https://github.com/mohn93/sketchup/releases/tag/../../../evil/repo",
            "https://github.com/mohn93/sketchup/releases/tag/%2e%2e",
            "https://github.com/mohn93/sketchup/releases/tag/..",
            "https://github.com/mohn93/sketchup/releases/tag/v9\\@evil.example",
            "https://github.com/mohn93/sketchup/releases/tag/v9 x",
            "https://github.com/mohn93/sketchup/releases/tag/v9?x=1",
            "https://github.com/mohn93/sketchup/releases/tag/",
            "https://github.com/mohn93/sketchup/pulls",
            "http://github.com/mohn93/sketchup/releases/tag/v9.0.0",
        ] {
            assert_eq!(offered(bad), RELEASES_PAGE, "{bad}");
        }
    }

    #[test]
    fn pre_releases_compare_by_their_identifiers() {
        assert!(is_newer("1.0.0-beta.2", "1.0.0-beta.1"));
        assert!(is_newer("1.0.0-beta.11", "1.0.0-beta.2"));
        assert!(is_newer("1.0.0-beta", "1.0.0-alpha.9"));
        assert!(is_newer("1.0.0-alpha.1", "1.0.0-alpha"));
        assert!(is_newer("1.0.0-alpha.beta", "1.0.0-alpha.1"));
        assert!(!is_newer("1.0.0-beta.1", "1.0.0-beta.1"));
        assert!(!is_newer("1.0.0-beta.1+b7", "1.0.0-beta.1"));
    }

    #[test]
    fn no_releases_and_failures_are_told_apart() {
        assert_eq!(interpret(404, "{}", "0.1.0"), Outcome::NoReleases);
        assert!(matches!(interpret(500, "", "0.1.0"), Outcome::Failed(m) if m.contains("500")));
        assert!(
            matches!(interpret(403, "", "0.1.0"), Outcome::Failed(m) if m.contains("limiting"))
        );
        assert!(matches!(
            interpret(200, "not json", "0.1.0"),
            Outcome::Failed(_)
        ));
        assert!(matches!(interpret(200, "{}", "0.1.0"), Outcome::Failed(_)));
    }

    #[test]
    fn the_label_has_the_version() {
        assert!(version_label().starts_with(VERSION));
    }
}
