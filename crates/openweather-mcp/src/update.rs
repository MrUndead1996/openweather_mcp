use std::{
    env,
    error::Error,
    fs,
    io::{self, Read, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    time::Duration,
};

use flate2::read::GzDecoder;
use reqwest::{Client, Url, redirect::Policy};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

const REPOSITORY: &str = "MrUndead1996/openweather_mcp";
const BIN_NAME: &str = "openweather-mcp";
const MAX_METADATA_SIZE: usize = 1024 * 1024;
const MAX_ASSET_SIZE: usize = 32 * 1024 * 1024;
const MAX_UNPACKED_SIZE: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    digest: Option<String>,
    size: usize,
}

pub(crate) async fn run() -> Result<(), Box<dyn Error>> {
    let arch = match env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return Err("self-update is supported only on Linux x86_64 and aarch64".into()),
    };
    let asset_name = format!("{BIN_NAME}-linux-{arch}.tar.gz");
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let client = client()?;
    let latest_url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let metadata = download(&client, &latest_url, MAX_METADATA_SIZE).await?;
    let release: Release = serde_json::from_slice(&metadata)?;
    let Some(next) = newer_version(&current, &release.tag_name)? else {
        println!("{BIN_NAME} {current} is already up to date.");
        return Ok(());
    };

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or("the release has no archive for this architecture")?;
    let digest = asset
        .digest
        .as_deref()
        .filter(|digest| {
            digest.strip_prefix("sha256:").is_some_and(|hex| {
                hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        })
        .ok_or("the release archive has no valid SHA-256 digest")?;
    if asset.size == 0 || asset.size > MAX_ASSET_SIZE {
        return Err("the release archive has an unexpected size".into());
    }
    let asset_url =
        format!("https://github.com/{REPOSITORY}/releases/download/v{next}/{asset_name}");
    let archive = download(&client, &asset_url, asset.size).await?;
    if archive.len() != asset.size {
        return Err("the downloaded archive size does not match GitHub release metadata".into());
    }
    install(&env::current_exe()?, &archive, digest)?;
    println!("Updated {BIN_NAME} from {current} to {next}. Restart your MCP client to use it.");
    Ok(())
}

fn newer_version(current: &Version, tag: &str) -> Result<Option<Version>, Box<dyn Error>> {
    let next = Version::parse(tag.strip_prefix('v').ok_or("invalid release tag")?)?;
    if !next.pre.is_empty() || !next.build.is_empty() {
        return Err("expected a stable release tag in the form vMAJOR.MINOR.PATCH".into());
    }
    Ok(next.cmp_precedence(current).is_gt().then_some(next))
}

fn allowed_redirect(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

fn client() -> Result<Client, Box<dyn Error>> {
    let tls = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()?
    .with_root_certificates(rustls::RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    ))
    .with_no_client_auth();
    Ok(Client::builder()
        .tls_backend_preconfigured(tls)
        .user_agent(concat!("openweather-mcp/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && allowed_redirect(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("unexpected GitHub redirect")
            }
        }))
        .build()?)
}

async fn download(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(reqwest::Error::without_url)?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        if chunk.len() > limit - bytes.len() {
            return Err("GitHub response exceeded the expected download size".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn install(executable: &Path, bytes: &[u8], digest: &str) -> Result<(), Box<dyn Error>> {
    if !archive_digest(bytes).eq_ignore_ascii_case(digest) {
        return Err("the downloaded archive failed SHA-256 verification".into());
    }
    let dir = executable
        .parent()
        .ok_or("could not locate executable directory")?;
    let mode = fs::metadata(executable)?.permissions().mode() & 0o777;
    let mut staged = NamedTempFile::new_in(dir)?;
    let mut archive = tar::Archive::new(GzDecoder::new(bytes).take(MAX_UNPACKED_SIZE));
    let mut found = false;
    for entry in archive.entries()?.raw(true) {
        let mut entry = entry?;
        if entry.path()? != Path::new(BIN_NAME) {
            continue;
        }
        if found || !entry.header().entry_type().is_file() || entry.size() == 0 {
            return Err("the release archive has an invalid executable entry".into());
        }
        let size = entry.size();
        if size > MAX_UNPACKED_SIZE || io::copy(&mut entry, &mut staged)? != size {
            return Err("the release archive contains a truncated or oversized executable".into());
        }
        found = true;
    }
    if !found {
        return Err("the release archive does not contain the executable".into());
    }
    staged.flush()?;
    staged
        .as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    staged.as_file().sync_all()?;
    staged.persist(executable).map_err(|error| error.error)?;
    Ok(())
}

fn archive_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut digest = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(digest, "{byte:02x}").expect("writing to a String cannot fail");
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o755);
            header.set_size(data.len() as u64);
            tar.append_data(&mut header, path, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn accepts_only_newer_stable_release_versions() {
        let current = Version::parse("1.0.10").unwrap();
        assert_eq!(newer_version(&current, "v1.0.9").unwrap(), None);
        assert_eq!(newer_version(&current, "v1.0.10").unwrap(), None);
        assert_eq!(
            newer_version(&current, "v1.1.0").unwrap(),
            Some(Version::new(1, 1, 0))
        );
        assert!(newer_version(&current, "v1.1.0-rc.1").is_err());
        assert!(newer_version(&current, "v1.1.0+build").is_err());
    }

    #[test]
    fn rejects_untrusted_redirects() {
        for url in [
            "http://release-assets.githubusercontent.com/file",
            "https://release-assets.githubusercontent.com.example/file",
            "https://release-assets.githubusercontent.com:8443/file",
            "https://user@github.com/file",
        ] {
            assert!(!allowed_redirect(&Url::parse(url).unwrap()));
        }
        assert!(allowed_redirect(
            &Url::parse("https://release-assets.githubusercontent.com/file").unwrap()
        ));
    }

    #[test]
    fn verifies_and_replaces_only_the_executable() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("installed-binary");
        fs::write(&target, b"old binary").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o751)).unwrap();

        let bytes = archive(&[(BIN_NAME, b"new binary"), ("LICENSE", b"license")]);
        let digest = archive_digest(&bytes);
        assert!(install(&target, &bytes, "sha256:bad").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old binary");

        let duplicate = archive(&[(BIN_NAME, b"first"), (BIN_NAME, b"second")]);
        let duplicate_digest = archive_digest(&duplicate);
        assert!(install(&target, &duplicate, &duplicate_digest).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old binary");

        let old_file = fs::File::open(&target).unwrap();
        install(&target, &bytes, &digest).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new binary");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o751
        );
        assert_eq!(dir.path().read_dir().unwrap().count(), 1);
        assert_eq!(io::read_to_string(old_file).unwrap(), "old binary");
    }
}
