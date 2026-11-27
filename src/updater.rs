//! Updates from GitHub releases. The only network access of the whole app, and only when
//! update checks are on (or the user clicks "Check now"): it asks GitHub for the newest release,
//! downloads `DS5Bridge.exe` with the curl.exe that ships with Windows, and refuses to install it
//! unless its SHA-256 matches the checksum GitHub publishes for the file.

use crate::json::{self, Json};
use crate::sys::*;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub fn repository() -> Option<&'static str> {
    let repo = option_env!("DS5BRIDGE_UPDATE_REPOSITORY")?;
    let (owner, name) = repo.split_once('/')?;
    let valid = |part: &str| !part.is_empty() && part.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    (valid(owner) && valid(name)).then_some(repo)
}

pub fn repository_url() -> Option<String> {
    repository().map(|repo| format!("https://github.com/{repo}"))
}
const DEFAULT_REPOSITORY_ID: u64 = 1471190113;
const ASSET: &str = "DS5Bridge.exe";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String, // without the leading "v"
    pub notes: String,
    pub url: String,
    pub size: u64,
    pub sha256: Option<String>, // lowercase hex
    pub sha256_url: Option<String>, // fallback: a DS5Bridge.exe.sha256 asset
}

fn curl() -> std::process::Command {
    let sys = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let mut c = std::process::Command::new(Path::new(&sys).join("System32").join("curl.exe"));
    c.args(["--disable", "-sSfL", "--proto", "=https", "--max-time", "120", "-A", concat!("DS5Bridge/", env!("CARGO_PKG_VERSION"))]);
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

fn get(url: &str) -> Result<String, String> {
    let out = curl().args(["--max-filesize", "2097152", "-H", "Accept: application/vnd.github+json", url]).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

/// Parse GitHub's "latest release" answer.
pub fn parse_release(body: &str) -> Option<Release> {
    let j = json::parse(body)?;
    let version = j.get("tag_name")?.str()?.trim_start_matches('v').to_string();
    let assets = j.get("assets")?.arr();
    let named = |n: &str| assets.iter().find(|a| a.get("name").and_then(Json::str) == Some(n));
    let exe = named(ASSET)?;
    Some(Release {
        version,
        notes: j.get("body").and_then(Json::str).unwrap_or("").trim().to_string(),
        url: exe.get("browser_download_url")?.str()?.to_string(),
        size: exe.get("size").and_then(Json::num).unwrap_or(0.0) as u64,
        sha256: exe.get("digest").and_then(Json::str).and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase),
        sha256_url: named(&format!("{ASSET}.sha256")).and_then(|a| a.get("browser_download_url")?.str()).map(String::from),
    })
}

/// "1.10.0" > "1.9.3", ignoring a pre-release suffix.
pub fn newer(remote: &str, local: &str) -> bool {
    let nums = |v: &str| -> Vec<u64> { v.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    nums(remote) > nums(local)
}

/// Newest release if it is newer than this build.
pub fn check() -> Result<Option<Release>, String> {
    let endpoint = match repository() {
        Some(repo) => format!("repos/{repo}"),
        None => format!("repositories/{DEFAULT_REPOSITORY_ID}"),
    };
    let body = get(&format!("https://api.github.com/{endpoint}/releases/latest"))?;
    let r = parse_release(&body).ok_or("unexpected answer from GitHub")?;
    Ok(newer(&r.version, env!("CARGO_PKG_VERSION")).then_some(r))
}

pub fn sha256(data: &[u8]) -> Option<String> {
    let mut h = 0isize;
    let mut out = [0u8; 32];
    unsafe {
        if BCryptOpenAlgorithmProvider(&mut h, wide("SHA256").as_ptr(), std::ptr::null(), 0) != 0 {
            return None;
        }
        let ok = BCryptHash(h, std::ptr::null(), 0, data.as_ptr(), data.len() as u32, out.as_mut_ptr(), 32) == 0;
        BCryptCloseAlgorithmProvider(h, 0);
        ok.then(|| out.iter().map(|b| format!("{b:02x}")).collect())
    }
}

#[derive(Debug, PartialEq)]
pub enum UpdateError {
    Download(String),
    NoChecksum,
    BadChecksum,
    Install(String),
}

pub struct VerifiedDownload {
    path: PathBuf,
    checksum: String,
}

impl Drop for VerifiedDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() { let _ = std::fs::remove_dir(parent); }
    }
}

fn is_release_asset(repo_url: &str, url: &str, name: &str) -> bool {
    let prefix = format!("{repo_url}/releases/download/");
    let Some((tag, asset)) = url.strip_prefix(&prefix).and_then(|tail| tail.split_once('/')) else { return false };
    !tag.is_empty() && tag.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) && asset == name
}

/// Download and verify. `progress` receives the bytes written so far.
pub fn download(r: &Release, progress: &AtomicU64) -> Result<VerifiedDownload, UpdateError> {
    let repo = repository_url().ok_or_else(|| UpdateError::Download("Update repository is not configured".into()))?;
    if !is_release_asset(&repo, &r.url, ASSET) {
        return Err(UpdateError::Download("Unexpected GitHub asset URL".into()));
    }
    let expected = match (&r.sha256, &r.sha256_url) {
        (Some(h), _) => h.clone(),
        (None, Some(u)) if is_release_asset(&repo, u, &format!("{ASSET}.sha256")) => {
            get(u).map_err(UpdateError::Download)?.split_whitespace().next().unwrap_or("").to_lowercase()
        }
        _ => return Err(UpdateError::NoChecksum),
    };
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UpdateError::NoChecksum);
    }
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Relaxed);
    let dir = std::env::temp_dir().join(format!("DS5Bridge-update-{}-{sequence}", std::process::id()));
    std::fs::create_dir(&dir).map_err(|e| UpdateError::Download(e.to_string()))?;
    let verified = VerifiedDownload { path: dir.join(ASSET), checksum: expected };
    let path = &verified.path;
    let mut child = curl().args(["--max-filesize", "67108864"]).arg("-o").arg(path).arg(&r.url)
        .spawn().map_err(|e| UpdateError::Download(e.to_string()))?;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| UpdateError::Download(e.to_string()))? {
            if !status.success() {
                return Err(UpdateError::Download(format!("curl exit {status}")));
            }
            break;
        }
        progress.store(std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0), Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let data = std::fs::read(&path).map_err(|e| UpdateError::Download(e.to_string()))?;
    progress.store(data.len() as u64, Relaxed);
    if (r.size != 0 && r.size != data.len() as u64) || sha256(&data).as_deref() != Some(verified.checksum.as_str()) {
        return Err(UpdateError::BadChecksum);
    }
    Ok(verified)
}

/// Swap the verified exe in; the caller starts it. A running exe can be renamed but not overwritten, so:
/// stop the background process, rename the old exe aside, move the new one in, start it.
pub fn install(new_exe: &VerifiedDownload, version: &str) -> Result<PathBuf, UpdateError> {
    let installed = crate::installer::running_installed();
    let target = std::env::current_exe().map_err(|e| UpdateError::Install(e.to_string()))?;
    crate::installer::stop_background();
    crate::installer::replace_executable(&new_exe.path, &target, Some(&new_exe.checksum)).map_err(UpdateError::Install)?;
    if installed {
        crate::installer::set_registered_version(version);
    }
    Ok(target)
}

/// Leftover from the last update (the renamed old exe): delete it on start.
pub fn cleanup() {
    if let Ok(me) = std::env::current_exe() {
        let _ = std::fs::remove_file(me.with_extension("old.exe"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_release_assets_from_this_repository_are_accepted() {
        let repo = "https://github.com/example/bridge";
        assert!(is_release_asset(repo, &format!("{repo}/releases/download/v1.0.1/{ASSET}"), ASSET));
        assert!(!is_release_asset(repo, &format!("{repo}/releases/download/v1.0.1/{ASSET}?copy=1"), ASSET));
        assert!(!is_release_asset(repo, "https://github.com/example/other/releases/download/v1.0.1/DS5Bridge.exe", ASSET));
    }

    #[test]
    fn versions_and_release_parsing() {
        assert!(newer("1.10.0", "1.9.3"));
        assert!(newer("2.0.0", "1.99.99"));
        assert!(!newer("1.0.0", "1.0.0"));
        assert!(!newer("1.0.0-beta", "1.0.0"));
        let body = r#"{"tag_name":"v1.1.0","body":"- faster","assets":[
            {"name":"DS5Bridge.exe","size":10,"digest":"sha256:ABCDEF","browser_download_url":"https://x/DS5Bridge.exe"},
            {"name":"DS5Bridge.exe.sha256","browser_download_url":"https://x/DS5Bridge.exe.sha256"}]}"#;
        let r = parse_release(body).unwrap();
        assert_eq!(r.version, "1.1.0");
        assert_eq!(r.sha256.as_deref(), Some("abcdef"));
        assert_eq!(r.sha256_url.as_deref(), Some("https://x/DS5Bridge.exe.sha256"));
        assert!(parse_release(r#"{"tag_name":"v1","assets":[]}"#).is_none());
        assert_eq!(sha256(b"abc").unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
