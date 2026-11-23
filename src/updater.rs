//! Updates from GitHub releases. The only network access of the whole app, and only when
//! update checks are on (or the user clicks "Check now"): it asks GitHub for the newest release,
//! downloads `DS5Bridge.exe` with the curl.exe that ships with Windows, and refuses to install it
//! unless its SHA-256 matches the checksum GitHub publishes for the file.

use crate::json::{self, Json};
use crate::sys::*;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub const REPO: &str = "aywalh/ds5bridge";
pub const REPO_URL: &str = "https://github.com/aywalh/ds5bridge";
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
    c.args(["-sSfL", "--proto", "=https", "--max-time", "120", "-A", concat!("DS5Bridge/", env!("CARGO_PKG_VERSION"))]);
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

fn get(url: &str) -> Result<String, String> {
    let out = curl().args(["-H", "Accept: application/vnd.github+json", url]).output().map_err(|e| e.to_string())?;
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
    let body = get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))?;
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

/// Download and verify. `progress` receives the bytes written so far.
pub fn download(r: &Release, progress: &AtomicU64) -> Result<PathBuf, UpdateError> {
    let expected = match (&r.sha256, &r.sha256_url) {
        (Some(h), _) => h.clone(),
        (None, Some(u)) => get(u).map_err(UpdateError::Download)?.split_whitespace().next().unwrap_or("").to_lowercase(),
        _ => return Err(UpdateError::NoChecksum),
    };
    if expected.len() != 64 {
        return Err(UpdateError::NoChecksum);
    }
    let path = std::env::temp_dir().join(format!("DS5Bridge-{}.exe", r.version));
    let _ = std::fs::remove_file(&path);
    let mut child = curl().arg("-o").arg(&path).arg(&r.url).spawn().map_err(|e| UpdateError::Download(e.to_string()))?;
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
    if sha256(&data).as_deref() != Some(expected.as_str()) {
        let _ = std::fs::remove_file(&path);
        return Err(UpdateError::BadChecksum);
    }
    Ok(path)
}

/// Swap the verified exe in; the caller starts it. A running exe can be renamed but not overwritten, so:
/// stop the background process, rename the old exe aside, move the new one in, start it.
pub fn install(new_exe: &Path, version: &str) -> Result<PathBuf, UpdateError> {
    let target = if crate::installer::is_installed() { crate::installer::installed_exe() } else { std::env::current_exe().map_err(|e| UpdateError::Install(e.to_string()))? };
    let old = target.with_extension("old.exe");
    crate::installer::stop_background();
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&target, &old).map_err(|e| UpdateError::Install(e.to_string()))?;
    if let Err(e) = std::fs::copy(new_exe, &target) {
        let _ = std::fs::rename(&old, &target); // put the working version back
        return Err(UpdateError::Install(e.to_string()));
    }
    let _ = std::fs::remove_file(new_exe);
    crate::installer::set_registered_version(version);
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
