use crate::config::Profile;
use crate::ops::Reporter;
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use url::Url;

const API_ROOT: &str = "https://api.nexusmods.com/v1";

#[derive(Debug, Clone)]
pub struct NxmLink {
    pub game_domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub key: Option<String>,
    pub expires: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DownloadMirror {
    #[serde(alias = "URI")]
    uri: String,
    #[serde(default)]
    short_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileMetadata {
    file_name: String,
}

impl NxmLink {
    pub fn parse(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)?;
        if url.scheme() != "nxm" {
            bail!("expected nxm:// scheme");
        }

        let game_domain = url
            .host_str()
            .map(str::to_string)
            .filter(|value| !value.is_empty())
            .context("missing Nexus game domain")?;

        let parts: Vec<_> = url
            .path_segments()
            .context("missing nxm path")?
            .filter(|part| !part.is_empty())
            .collect();

        if parts.len() < 4 || parts[0] != "mods" || parts[2] != "files" {
            bail!("expected nxm://<game>/mods/<mod_id>/files/<file_id>");
        }

        let mod_id = parts[1]
            .parse()
            .with_context(|| format!("invalid mod id: {}", parts[1]))?;
        let file_id = parts[3]
            .parse()
            .with_context(|| format!("invalid file id: {}", parts[3]))?;

        let mut key = None;
        let mut expires = None;
        for (name, value) in url.query_pairs() {
            match name.as_ref() {
                "key" => key = Some(value.into_owned()),
                "expires" => expires = Some(value.into_owned()),
                _ => {}
            }
        }

        Ok(Self {
            game_domain,
            mod_id,
            file_id,
            key,
            expires,
        })
    }
}

pub fn download_nxm(
    profile: &Profile,
    api_key: &str,
    link: &NxmLink,
    reporter: &mut impl Reporter,
) -> Result<PathBuf> {
    fs::create_dir_all(&profile.downloads).with_context(|| {
        format!(
            "failed to create downloads: {}",
            profile.downloads.display()
        )
    })?;

    let client = nexus_client(api_key)?;
    reporter.line(format!(
        "Resolving Nexus file {}/mods/{}/files/{}...",
        link.game_domain, link.mod_id, link.file_id
    ));

    let filename = fetch_file_name(&client, link).unwrap_or_else(|error| {
        reporter.warn(format!("could not fetch file metadata: {error}"));
        format!("nexus-{}-{}.archive", link.mod_id, link.file_id)
    });
    let filename = sanitize_filename(&filename);
    let output = unique_path(&profile.downloads.join(filename));

    let mirrors = fetch_download_mirrors(&client, link)?;
    let mirror = choose_mirror(&mirrors).context("Nexus returned no download mirrors")?;
    let label = mirror
        .short_name
        .as_deref()
        .or(mirror.name.as_deref())
        .unwrap_or("download mirror");

    reporter.line(format!("Downloading from {label} to {}", output.display()));
    let mut response = client
        .get(&mirror.uri)
        .send()
        .context("failed to start Nexus download")?
        .error_for_status()
        .context("Nexus download request failed")?;
    let mut file = File::create(&output)
        .with_context(|| format!("failed to create download file: {}", output.display()))?;
    io::copy(&mut response, &mut file).context("failed while writing download")?;
    reporter.line(format!("Downloaded: {}", output.display()));
    Ok(output)
}

fn nexus_client(api_key: &str) -> Result<Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "apikey",
        HeaderValue::from_str(api_key.trim())
            .context("Nexus API key is not a valid header value")?,
    );
    Ok(Client::builder()
        .default_headers(headers)
        .user_agent(format!("kiss-me/{}", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn fetch_file_name(client: &Client, link: &NxmLink) -> Result<String> {
    let url = format!(
        "{API_ROOT}/games/{}/mods/{}/files/{}.json",
        link.game_domain, link.mod_id, link.file_id
    );
    let metadata: FileMetadata = client
        .get(url)
        .send()?
        .error_for_status()?
        .json()
        .context("failed to parse Nexus file metadata")?;
    Ok(metadata.file_name)
}

fn fetch_download_mirrors(client: &Client, link: &NxmLink) -> Result<Vec<DownloadMirror>> {
    let url = format!(
        "{API_ROOT}/games/{}/mods/{}/files/{}/download_link.json",
        link.game_domain, link.mod_id, link.file_id
    );
    let mut request = client.get(url);
    if let Some(key) = &link.key {
        request = request.query(&[("key", key)]);
    }
    if let Some(expires) = &link.expires {
        request = request.query(&[("expires", expires)]);
    }

    request
        .send()
        .context("failed to request Nexus download mirrors")?
        .error_for_status()
        .context("Nexus mirror request failed")?
        .json()
        .context("failed to parse Nexus download mirrors")
}

fn choose_mirror(mirrors: &[DownloadMirror]) -> Option<&DownloadMirror> {
    mirrors
        .iter()
        .find(|mirror| mirror.short_name.as_deref() == Some("Nexus CDN"))
        .or_else(|| mirrors.first())
}

fn sanitize_filename(filename: &str) -> String {
    let name = Path::new(filename)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(filename)
        .trim();
    let sanitized: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | '\0' => '_',
            ch => ch,
        })
        .collect();
    if sanitized.is_empty() {
        "nexus-download.archive".to_string()
    } else {
        sanitized
    }
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("download");
    let extension = path.extension().and_then(OsStr::to_str);

    for index in 1.. {
        let filename = match extension {
            Some(extension) => format!("{stem}-{index}.{extension}"),
            None => format!("{stem}-{index}"),
        };
        let candidate = parent.join(filename);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("infinite range always returns")
}

#[cfg(test)]
mod tests {
    use super::NxmLink;

    #[test]
    fn parses_nxm_links_with_key_and_expiry() {
        let link = NxmLink::parse(
            "nxm://cyberpunk2077/mods/123/files/456?key=abc123&expires=1893456000&user_id=99",
        )
        .unwrap();

        assert_eq!(link.game_domain, "cyberpunk2077");
        assert_eq!(link.mod_id, 123);
        assert_eq!(link.file_id, 456);
        assert_eq!(link.key.as_deref(), Some("abc123"));
        assert_eq!(link.expires.as_deref(), Some("1893456000"));
    }

    #[test]
    fn rejects_non_nxm_links() {
        assert!(NxmLink::parse("https://example.com/mods/123/files/456").is_err());
    }
}
