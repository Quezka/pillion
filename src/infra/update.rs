//! Finding out whether GitHub has a newer Pillion. Installing it is Android's job, and Android only accepts an update signed
//! with the same key as the installed app.

use serde_json::Value;

pub use crate::application::ports::Release;
use crate::application::ports::ReleaseFeed;

/// The ReleaseFeed the app uses: GitHub's releases of the public repository.
pub struct GitHubReleases;

impl ReleaseFeed for GitHubReleases {
    fn latest_url(&self) -> String {
        latest_release_url()
    }

    fn parse_latest(&self, json: &str, current: &str) -> Result<Option<Release>, String> {
        parse_latest(json, current)
    }
}

/// Where releases are published. It has to be a public repository, as the app asks without logging in.
const REPOSITORY: &str = match option_env!("PILLION_UPDATE_REPO") {
    Some(repository) => repository,
    None => "Quezka/pillion",
};
const NOTES_LIMIT: usize = 600;

pub fn latest_release_url() -> String {
    format!("https://api.github.com/repos/{REPOSITORY}/releases/latest")
}

/// The parts of a version such as `v1.2.3`, or `None` if it isn't one.
fn version_parts(version: &str) -> Option<[u32; 3]> {
    let mut parts = version.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    let found = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(found)
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((version_parts(candidate), version_parts(current)), (Some(a), Some(b)) if a > b)
}

/// The release, if GitHub's latest is newer than `current` and has an app to install.
pub fn parse_latest(json: &str, current: &str) -> Result<Option<Release>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("the release has no version")?;
    if !is_newer(tag, current) {
        return Ok(None);
    }
    let assets = v["assets"].as_array().ok_or("the release has no files")?;
    let apk = assets
        .iter()
        .find(|a| a["name"].as_str().is_some_and(|n| n.ends_with(".apk")))
        .ok_or("the release has no app to install")?;
    let url = apk["browser_download_url"].as_str().ok_or("the release's app has no address")?;
    if !url.starts_with("https://") {
        return Err("the release's address isn't secure".to_string());
    }
    let sha256 = apk["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).unwrap_or_default();
    let notes: String = v["body"].as_str().unwrap_or_default().trim().chars().take(NOTES_LIMIT).collect();
    Ok(Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: url.to_string(),
        sha256: sha256.to_string(),
        notes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = r#"{"tag_name":"v0.3.0","body":" Better things. ","assets":[
        {"name":"notes.txt","browser_download_url":"https://example.org/notes.txt"},
        {"name":"Pillion-0.3.0.apk","browser_download_url":"https://example.org/Pillion-0.3.0.apk","digest":"sha256:abc123"}]}"#;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("v0.1.9", "0.2.0"));
        assert!(!is_newer("nightly", "0.2.0"));
        assert!(!is_newer("v1.2", "0.2.0"));
        assert!(!is_newer("v1.2.3.4", "0.2.0"));
    }

    #[test]
    fn finds_the_app_in_a_newer_release() {
        let release = parse_latest(REPLY, "0.2.0").unwrap().unwrap();
        assert_eq!(release.version, "0.3.0");
        assert_eq!(release.url, "https://example.org/Pillion-0.3.0.apk");
        assert_eq!(release.sha256, "abc123");
        assert_eq!(release.notes, "Better things.");
    }

    #[test]
    fn stays_quiet_when_up_to_date() {
        assert_eq!(parse_latest(REPLY, "0.3.0").unwrap(), None);
        assert_eq!(parse_latest(REPLY, "0.4.0").unwrap(), None);
    }

    #[test]
    fn a_broken_or_unsafe_release_is_an_error() {
        assert!(parse_latest("not json", "0.2.0").is_err());
        assert!(parse_latest(r#"{"message":"Not Found"}"#, "0.2.0").is_err());
        assert!(parse_latest(r#"{"tag_name":"v9.0.0","assets":[]}"#, "0.2.0").is_err());
        let plain = r#"{"tag_name":"v9.0.0","assets":[{"name":"a.apk","browser_download_url":"http://example.org/a.apk"}]}"#;
        assert!(parse_latest(plain, "0.2.0").is_err());
    }
}
