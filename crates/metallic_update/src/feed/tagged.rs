//! Velopack feed and packages pinned to the release selected by the update check.

use std::path::Path;
use std::sync::{Arc, Mutex, mpsc::Sender};

use semver::Version;
use serde::Deserialize;
use velopack::sources::UpdateSource;
use velopack::{Error, HttpHeader, HttpOptions, VelopackAsset, VelopackAssetFeed};

use super::{Channel, ReleaseFeed, USER_AGENT};
use crate::http::{HttpClient, Token, form_encode};

// Package transfers need more time than metadata, but must not wait forever.
const PACKAGE_TIMEOUT_MS: u64 = 10 * 60 * 1_000;

#[derive(Clone, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}

#[derive(Clone, Deserialize)]
struct Asset {
    id: u64,
    name: String,
}

pub(super) struct TaggedGithubSource {
    feed: ReleaseFeed,
    channel: Channel,
    version: Version,
    token: Token,
    api_base: String,
    http: Arc<dyn HttpClient>,
    release: Mutex<Option<Release>>,
}

fn failure(message: &str) -> Error {
    Error::Other(message.to_string())
}

impl TaggedGithubSource {
    pub(super) fn new(
        feed: ReleaseFeed,
        channel: Channel,
        version: Version,
        token: Token,
        api_base: String,
        http: Arc<dyn HttpClient>,
    ) -> Self {
        Self {
            feed,
            channel,
            version,
            token,
            api_base,
            http,
            release: Mutex::new(None),
        }
    }

    fn headers(&self, accept: &str) -> Vec<(String, String)> {
        vec![
            ("Accept".into(), accept.into()),
            ("X-GitHub-Api-Version".into(), "2022-11-28".into()),
            ("User-Agent".into(), USER_AGENT.into()),
            (
                "Authorization".into(),
                format!("Bearer {}", self.token.expose()),
            ),
        ]
    }

    fn get_text(&self, url: &str, accept: &str) -> Result<String, Error> {
        let headers = self.headers(accept);
        let borrowed: Vec<_> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        // Do not surface response bodies or arbitrary transport text in installer logs.
        let response = self
            .http
            .get(url, &borrowed)
            .map_err(|_| failure("could not retrieve the selected update release"))?;
        match response.status {
            200..=299 => Ok(response.body),
            401 | 403 | 404 => Err(failure(
                "the selected update release is unavailable; check access and try again",
            )),
            _ => Err(failure(
                "GitHub could not serve the selected update release",
            )),
        }
    }

    fn selected_release(&self) -> Result<Release, Error> {
        let mut cached = self
            .release
            .lock()
            .map_err(|_| failure("update source lock was poisoned"))?;
        if let Some(release) = cached.as_ref() {
            return Ok(release.clone());
        }
        let tag = format!(
            "{}{}-{}",
            self.feed.tag_prefix,
            self.version,
            self.channel.as_str()
        );
        let url = format!(
            "{}/repos/{}/releases/tags/{}",
            self.api_base,
            self.feed.repo,
            form_encode(&tag)
        );
        let text = self.get_text(&url, "application/vnd.github+json")?;
        let release: Release = serde_json::from_str(&text)
            .map_err(|_| failure("the selected update release has invalid metadata"))?;
        if release.draft || release.tag_name != tag {
            return Err(failure(
                "GitHub did not return the selected published update tag",
            ));
        }
        *cached = Some(release.clone());
        Ok(release)
    }

    fn asset_url(&self, name: &str) -> Result<String, Error> {
        let release = self.selected_release()?;
        let mut matching = release.assets.iter().filter(|asset| asset.name == name);
        let asset = matching
            .next()
            .ok_or_else(|| failure("a package is missing from the selected update release"))?;
        if asset.id == 0 || matching.next().is_some() {
            return Err(failure(
                "the selected update release has ambiguous asset metadata",
            ));
        }
        // Construct the authenticated API URL from the numeric ID. Never send the token
        // to an arbitrary URL embedded in release metadata or feed filenames.
        Ok(format!(
            "{}/repos/{}/releases/assets/{}",
            self.api_base, self.feed.repo, asset.id
        ))
    }
}

impl UpdateSource for TaggedGithubSource {
    fn get_release_feed(
        &self,
        channel: &str,
        _app: &velopack::bundle::Manifest,
        _staged_user_id: &str,
    ) -> Result<VelopackAssetFeed, Error> {
        if channel != self.feed.velopack_channel(self.channel) {
            return Err(failure(
                "the update feed channel differs from the selected release",
            ));
        }
        let url = self.asset_url(&format!("releases.{channel}.json"))?;
        let text = self.get_text(&url, "application/octet-stream")?;
        let mut feed: VelopackAssetFeed = serde_json::from_str(&text)
            .map_err(|_| failure("the selected release has an invalid package feed"))?;
        // This exact-tag source cannot prove an intermediate delta chain for a
        // skipped version. Offer only full packages, whose checksum Velopack verifies.
        let version = self.version.to_string();
        feed.Assets
            .retain(|asset| asset.Version == version && asset.Type.eq_ignore_ascii_case("full"));
        if feed.Assets.is_empty() {
            return Err(failure("the selected release has no full update package"));
        }
        for asset in &feed.Assets {
            self.asset_url(&asset.FileName)?;
        }
        Ok(feed)
    }

    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        local_file: &Path,
        progress_sender: Option<Sender<i16>>,
    ) -> Result<(), Error> {
        if asset.Version != self.version.to_string() {
            return Err(failure(
                "a package does not belong to the selected update version",
            ));
        }
        let url = self.asset_url(&asset.FileName)?;
        let options = HttpOptions {
            Headers: self
                .headers("application/octet-stream")
                .into_iter()
                .map(|(name, value)| HttpHeader {
                    Name: name,
                    Value: value,
                })
                .collect(),
            TimeoutMilliseconds: PACKAGE_TIMEOUT_MS,
        };
        velopack::download::download_url_to_file(&url, local_file, Some(&options), |percent| {
            if let Some(sender) = &progress_sender {
                let _ = sender.send(percent);
            }
        })
        .map_err(|_| failure("could not download a package from the selected update release"))
    }
}

#[cfg(test)]
mod tests;
