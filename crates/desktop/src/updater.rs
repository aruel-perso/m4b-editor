//! Signed native updater. Trust is rooted only in the compiled Ed25519 public key.
//!
//! Rename delivery contract (also consumed by the packaging owner): canonical
//! clients use m4b-editor and update-v2-PLATFORM.json. Version 0.3 cannot follow
//! repository-rename redirects. A reviewer must create a release-only repository
//! at aruel-perso/m4b-craft and publish independently signed update-PLATFORM.json
//! envelopes with exact old asset names/old repository origins. It must retain
//! the same pinned key. Windows keeps launch shims at the old helper paths;
//! the legacy Mac DMG retains m4b-Craft.app/Contents/MacOS/m4b-Craft. Once that
//! client runs 0.4, this updater uses only the canonical origin. Supporting old
//! installed bundle roots below never authorizes old download origins.
use anyhow::{ensure, Context, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use reqwest::{blocking::Client, redirect::Policy, Url};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    time::Duration,
};

const REPO: &str = "https://github.com/aruel-perso/m4b-editor/releases/download/";
const MAX_MSI: u64 = 512 * 1024 * 1024;
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
const PLATFORM: &str = "windows-x86_64";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PLATFORM: &str = "macos-aarch64";
fn extension(platform: &str) -> Result<&'static str> {
    match platform {
        "windows-x86_64" => Ok("msi"),
        "macos-aarch64" => Ok("dmg"),
        _ => anyhow::bail!("Unsupported platform"),
    }
}
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Channel {
    Stable,
    Prerelease,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub platform: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
    pub url: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    payload: String,
    signature: String,
}
#[derive(Clone, Debug)]
pub struct Verified {
    pub manifest: Manifest,
    envelope: Vec<u8>,
    channel: Channel,
}
pub struct Download {
    pub verified: Verified,
    file: tempfile::NamedTempFile,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

pub fn error_log() -> std::path::PathBuf {
    crate::preferences_path().with_file_name("update-install-error.log")
}

pub fn supported() -> bool {
    cfg!(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    ))
}

pub fn guard(busy: bool, dirty: bool, installing: bool) -> Result<()> {
    ensure!(
        !busy,
        "Updates are unavailable during media work or playback"
    );
    ensure!(
        !installing || !dirty,
        "Save the project before installing an update"
    );
    Ok(())
}
fn release_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.as_str().starts_with(REPO)
}
fn allowed_redirect(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
        && (url.host_str() != Some("github.com") || release_url(url))
}
fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent("M4B-Editor/0.4 updater")
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .https_only(true)
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && allowed_redirect(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("Untrusted update redirect")
            }
        }))
        .build()?)
}
fn get(client: &Client, url: &str, max: u64) -> Result<Vec<u8>> {
    let response = client.get(url).send()?.error_for_status()?;
    ensure!(
        response.content_length().is_none_or(|n| n <= max),
        "Oversize response"
    );
    let mut bytes = Vec::new();
    response.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "Oversize response");
    Ok(bytes)
}
fn verify_with(bytes: &[u8], channel: Channel, current: &str, key: &[u8; 32]) -> Result<Verified> {
    verify_platform(bytes, channel, current, key, PLATFORM)
}
fn verify_platform(
    bytes: &[u8],
    channel: Channel,
    current: &str,
    key: &[u8; 32],
    platform: &str,
) -> Result<Verified> {
    ensure!(bytes.len() <= 16384, "Oversize manifest");
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    let signature = Signature::from_slice(&hex::decode(&envelope.signature)?)?;
    VerifyingKey::from_bytes(key)?
        .verify_strict(envelope.payload.as_bytes(), &signature)
        .context("Invalid update signature")?;
    let m: Manifest = serde_json::from_str(&envelope.payload)?;
    let version = Version::parse(&m.version)?;
    ensure!(
        version > Version::parse(current)?,
        "Update must be newer than this application"
    );
    ensure!(version.build.is_empty(), "Build metadata is not supported");
    ensure!(
        channel == Channel::Prerelease || version.pre.is_empty(),
        "Prerelease on stable channel"
    );
    ensure!(m.platform == platform, "Wrong platform");
    ensure!(
        m.filename
            == format!(
                "M4B-Editor-{}-{}.{}",
                m.version,
                platform,
                extension(platform)?
            ),
        "Unexpected filename"
    );
    ensure!(m.size > 0 && m.size <= MAX_MSI, "Invalid installer size");
    ensure!(
        m.sha256.len() == 64 && hex::decode(&m.sha256)?.len() == 32,
        "Invalid hash"
    );
    let expected = format!("{REPO}v{}/{}", m.version, m.filename);
    ensure!(
        m.url == expected && release_url(&Url::parse(&m.url)?),
        "Untrusted installer URL"
    );
    Ok(Verified {
        manifest: m,
        envelope: bytes.to_vec(),
        channel,
    })
}
pub fn verify(bytes: &[u8], channel: Channel) -> Result<Verified> {
    let key: [u8; 32] =
        hex::decode(include_str!("../../../packaging/update-public-key.hex").trim())?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid compiled public key"))?;
    verify_with(bytes, channel, env!("CARGO_PKG_VERSION"), &key)
}
pub fn check(channel: Channel, busy: bool) -> Result<Verified> {
    guard(busy, false, false)?;
    ensure!(supported(), "Updates require Windows x86_64 or macOS ARM64");
    let client = client()?;
    let releases: Vec<Release> = serde_json::from_slice(&get(
        &client,
        "https://api.github.com/repos/aruel-perso/m4b-editor/releases?per_page=100",
        1024 * 1024,
    )?)?;
    let mut versions: Vec<Version> = releases
        .into_iter()
        .filter(|r| !r.draft && (channel == Channel::Prerelease || !r.prerelease))
        .filter_map(|r| Version::parse(r.tag_name.strip_prefix('v')?).ok())
        .filter(|v| {
            (channel == Channel::Prerelease || v.pre.is_empty())
                && *v > Version::parse(env!("CARGO_PKG_VERSION")).expect("package version")
        })
        .collect();
    versions.sort();
    let version = versions.last().context("No newer release found")?;
    let result = verify(
        &get(
            &client,
            &format!("{REPO}v{version}/update-v2-{PLATFORM}.json"),
            16384,
        )?,
        channel,
    )?;
    ensure!(
        result.manifest.version == version.to_string(),
        "Release/manifest version mismatch"
    );
    Ok(result)
}
impl Verified {
    #[cfg(test)]
    pub fn download(self, busy: bool) -> Result<Download> {
        self.download_progress(busy, &std::sync::atomic::AtomicBool::new(false), |_| {})
    }
    pub fn matches_channel(&self, channel: Channel) -> bool {
        self.channel == channel
    }
    pub fn download_progress(
        self,
        busy: bool,
        cancel: &std::sync::atomic::AtomicBool,
        mut progress: impl FnMut(f32),
    ) -> Result<Download> {
        let check_cancel = || {
            ensure!(
                !cancel.load(std::sync::atomic::Ordering::Relaxed),
                "Operation cancelled"
            );
            Ok(())
        };
        check_cancel()?;
        guard(busy, false, false)?;
        ensure!(supported(), "Updates require Windows x86_64 or macOS ARM64");
        let verified = verify(&self.envelope, self.channel)?;
        let client = client()?;
        let response = client
            .get(&verified.manifest.url)
            .send()?
            .error_for_status()?;
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n == verified.manifest.size),
            "Incorrect download size"
        );
        let mut file = tempfile::Builder::new()
            .prefix("m4b-update-")
            .suffix(&format!(".{}", extension(PLATFORM)?))
            .tempfile()?;
        let mut response = response.take(verified.manifest.size + 1);
        let mut count = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            check_cancel()?;
            let n = response.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            ensure!(count <= verified.manifest.size, "Oversize installer");
            file.write_all(&buffer[..n])?;
            progress(count as f32 / verified.manifest.size as f32);
        }
        ensure!(count == verified.manifest.size, "Incorrect download size");
        file.flush()?;
        let download = Download { verified, file };
        download.reverify()?;
        check_cancel()?;
        Ok(download)
    }
}
impl Download {
    fn reverify(&self) -> Result<()> {
        let verified = verify(&self.verified.envelope, self.verified.channel)?;
        verify_file(self.file.reopen()?, &verified.manifest)
    }

    pub fn install(self, busy: bool, dirty: bool) -> Result<()> {
        guard(busy, dirty, true)?;
        self.reverify()?;
        #[cfg(target_os = "windows")]
        {
            let system = std::path::PathBuf::from(
                std::env::var_os("SystemRoot").context("Missing Windows system directory")?,
            )
            .join("System32");
            let executable = std::env::current_exe()?;
            let path = self.file.into_temp_path().keep()?;
            let result =
                std::process::Command::new(system.join("WindowsPowerShell/v1.0/powershell.exe"))
                    .args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_HANDOFF])
                    .env("M4B_UPDATE_PID", std::process::id().to_string())
                    .env("M4B_UPDATE_MSI", &path)
                    .env("M4B_UPDATE_SHA256", &self.verified.manifest.sha256)
                    .env("M4B_UPDATE_APP", &executable)
                    .env("M4B_UPDATE_LOG", error_log())
                    .spawn();
            match result {
                Ok(_) => Ok(()),
                Err(error) => {
                    let _ = std::fs::remove_file(&path);
                    Err(error).context("Launch Windows update helper")
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            self.install_macos()
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        anyhow::bail!("Native installation unsupported on this platform")
    }
}
#[cfg(target_os = "windows")]
const WINDOWS_HANDOFF: &str = r#"$ParentId = [int]$env:M4B_UPDATE_PID
$Installer = $env:M4B_UPDATE_MSI
$Application = $env:M4B_UPDATE_APP
$ErrorActionPreference = 'Stop'
try {
    Wait-Process -Id $ParentId -ErrorAction SilentlyContinue
    $installerHash = (Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash
    if ($installerHash -ne $env:M4B_UPDATE_SHA256) { throw "Verified installer hash changed" }
    $msiexec = Join-Path $env:SystemRoot 'System32\msiexec.exe'
    $child = Start-Process -FilePath $msiexec -ArgumentList @('/i', ('"' + $Installer + '"'), '/passive', '/norestart') -Wait -PassThru
    if ($child.ExitCode -notin @(0, 3010, 1641)) { throw "Windows Installer exited with code $($child.ExitCode)" }
    Remove-Item -LiteralPath $Installer
    Remove-Item -LiteralPath $env:M4B_UPDATE_LOG -ErrorAction SilentlyContinue
    if ($child.ExitCode -ne 1641) { Start-Process -FilePath $Application }
} catch {
    $_ | Out-File -Encoding utf8 -LiteralPath $env:M4B_UPDATE_LOG
    Start-Process -FilePath $Application
}
"#;

#[cfg(any(target_os = "macos", test))]
fn arm64_binary(path: &std::path::Path) -> Result<bool> {
    let mut header = Vec::new();
    std::fs::File::open(path)?
        .take(4096)
        .read_to_end(&mut header)?;
    if header.len() < 8 {
        return Ok(false);
    }
    if header[..4] == [0xcf, 0xfa, 0xed, 0xfe] {
        return Ok(u32::from_le_bytes(header[4..8].try_into()?) == 0x0100000c);
    }
    let stride = match header[..4] {
        [0xca, 0xfe, 0xba, 0xbe] => 20,
        [0xca, 0xfe, 0xba, 0xbf] => 32,
        _ => return Ok(false),
    };
    let count = u32::from_be_bytes(header[4..8].try_into()?) as usize;
    ensure!(
        count <= 64 && header.len() >= 8 + count * stride,
        "Invalid universal binary header"
    );
    Ok((0..count).any(|i| header[8 + i * stride..12 + i * stride] == [1, 0, 0, 12]))
}

#[cfg(any(target_os = "macos", test))]
fn mac_install_location_error(parent: &std::path::Path) -> String {
    format!("Cannot stage update in {}. Run M4B Editor from an installed .app in a user-writable directory such as ~/Applications (create it and copy the app there, eject the DMG, then reopen it). Automatic updates require a writable installation directory; no administrator elevation is requested. / Lancez l’application installée dans un dossier accessible en écriture, par exemple ~/Applications, éjectez le DMG puis relancez-la. Aucune élévation administrateur n’est demandée.", parent.display())
}

#[cfg(any(target_os = "macos", test))]
impl Download {
    fn install_macos(self) -> Result<()> {
        use std::process::Command;
        let exe = std::env::current_exe()?.canonicalize()?;
        let app = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .context("Missing app bundle")?;
        ensure!(
            app.extension().is_some_and(|e| e == "app")
                && (exe.ends_with("Contents/MacOS/m4b-editor") || exe.ends_with("Contents/MacOS/m4b-Craft")),
            "Not running from an installed .app. Copy M4B Editor.app to a user-writable directory such as ~/Applications, eject the DMG, then reopen the installed app."
        );
        let parent = app.parent().context("Missing installation directory")?;
        let stage = tempfile::Builder::new()
            .prefix(".m4b-update-")
            .tempdir_in(parent)
            .with_context(|| mac_install_location_error(parent))?;
        let mount = tempfile::tempdir()?;
        let status = Command::new("/usr/bin/hdiutil")
            .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
            .arg(mount.path())
            .arg(self.file.path())
            .status()?;
        ensure!(status.success(), "Unable to mount verified DMG");
        let staged = stage.path().join("M4B Editor.app");
        let source = mount.path().join("M4B Editor.app");
        let copy = (|| -> Result<()> {
            ensure!(
                source.join("Contents/MacOS/m4b-editor").is_file(),
                "DMG missing application"
            );
            for name in ["m4b-editor", "m4b-editor-cli", "ffmpeg", "ffprobe"] {
                let binary = source.join("Contents/MacOS").join(name);
                ensure!(binary.is_file(), "Bundle missing {name}");
                ensure!(arm64_binary(&binary)?, "Bundle binary is not ARM64: {name}");
            }
            ensure!(
                Command::new("/usr/bin/ditto")
                    .arg(&source)
                    .arg(&staged)
                    .status()?
                    .success(),
                "Unable to stage app next to installation"
            );
            Ok(())
        })();
        let detached = Command::new("/usr/bin/hdiutil")
            .arg("detach")
            .arg(mount.path())
            .status();
        copy?;
        ensure!(detached?.success(), "Unable to detach DMG");
        let script = stage.path().join("handoff.sh");
        std::fs::write(&script, MAC_HANDOFF)?;
        let child = Command::new("/bin/sh")
            .arg(&script)
            .arg(std::process::id().to_string())
            .arg(app)
            .arg(&staged)
            .arg(stage.path())
            .arg(error_log())
            .spawn();
        child.context("Launch update helper")?;
        let _retained = stage.keep();
        Ok(())
    }
}
// Fixed program; every path is a positional argument, never interpolated shell code.
#[cfg(any(target_os = "macos", test))]
const MAC_HANDOFF: &str = r#"#!/bin/sh
pid=$1
app=$2
staged=$3
work=$4
backup="$work/previous.app"
log=$5
exec >>"$log" 2>&1
while kill -0 "$pid" 2>/dev/null; do /bin/sleep 1; done
if ! /bin/mv "$app" "$backup"; then echo "Update failed: cannot move installed app"; exit 1; fi
if ! /bin/mv "$staged" "$app"; then
    /bin/mv "$backup" "$app"
    /usr/bin/open "$app"
    exit 1
fi
if ! /usr/bin/open "$app"; then
    /bin/mv "$app" "$work/failed.app"
    /bin/mv "$backup" "$app"
    /usr/bin/open "$app"
    exit 1
fi
/bin/rm -rf "$backup"
/bin/rm -f "$log"
# Keep diagnostic log and helper; projects are never touched.
"#;

fn verify_file(mut file: std::fs::File, manifest: &Manifest) -> Result<()> {
    ensure!(
        file.metadata()?.len() == manifest.size,
        "Installer size changed"
    );
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut count = 0u64;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        ensure!(count <= manifest.size, "Installer grew during verification");
        digest.update(&buffer[..n]);
    }
    ensure!(
        count == manifest.size,
        "Installer size changed during verification"
    );
    ensure!(
        hex::encode(digest.finalize()) == manifest.sha256,
        "Installer hash mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn signed(version: &str, change: impl FnOnce(&mut serde_json::Value)) -> (Vec<u8>, [u8; 32]) {
        let key = SigningKey::from_bytes(&[42; 32]);
        let mut value = serde_json::json!({"version":version,"platform":PLATFORM,
            "filename":format!("M4B-Editor-{version}-{PLATFORM}.{}", extension(PLATFORM).unwrap()),"sha256":"ab".repeat(32),"size":10,
            "url":format!("{REPO}v{version}/M4B-Editor-{version}-{PLATFORM}.{}", extension(PLATFORM).unwrap())});
        change(&mut value);
        let payload = value.to_string();
        let signature = hex::encode(key.sign(payload.as_bytes()).to_bytes());
        (
            serde_json::to_vec(&serde_json::json!({"payload":payload,"signature":signature}))
                .unwrap(),
            key.verifying_key().to_bytes(),
        )
    }
    #[test]
    fn mac_location_error_explains_recovery() {
        let message = mac_install_location_error(std::path::Path::new("/Applications"));
        assert!(message.contains("/Applications"));
        assert!(message.contains("~/Applications"));
        assert!(message.contains("no administrator elevation"));
    }
    #[test]
    fn candidate_must_match_selected_channel() {
        let (bytes, key) = signed("0.3.0", |_| {});
        let update = verify_with(&bytes, Channel::Stable, "0.2.0", &key).unwrap();
        assert!(update.matches_channel(Channel::Stable));
        assert!(!update.matches_channel(Channel::Prerelease));
    }
    #[cfg(unix)]
    #[test]
    fn mac_helper_waits_for_parent_and_reports_swap_failure() {
        let d = tempfile::tempdir().unwrap();
        let script = d.path().join("helper.sh");
        std::fs::write(&script, MAC_HANDOFF).unwrap();
        let staged = d.path().join("staged.app");
        std::fs::create_dir(&staged).unwrap();
        let log = d.path().join("error.log");
        let mut parent = std::process::Command::new("/bin/sleep")
            .arg("1")
            .spawn()
            .unwrap();
        let mut helper = std::process::Command::new("/bin/sh")
            .arg(&script)
            .arg(parent.id().to_string())
            .arg(d.path().join("missing $(touch injected).app"))
            .arg(&staged)
            .arg(d.path())
            .arg(&log)
            .spawn()
            .unwrap();
        assert!(helper.try_wait().unwrap().is_none());
        parent.wait().unwrap();
        assert!(!helper.wait().unwrap().success());
        assert!(staged.is_dir());
        assert!(!d.path().join("injected").exists());
        assert!(std::fs::read_to_string(log)
            .unwrap()
            .contains("cannot move installed app"));
    }
    #[test]
    fn native_arm64_header_and_fixed_helper_syntax() {
        let _native_handoff: fn(Download) -> Result<()> = Download::install_macos;
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), [0xcf, 0xfa, 0xed, 0xfe, 12, 0, 0, 1]).unwrap();
        assert!(arm64_binary(f.path()).unwrap());
        std::fs::write(f.path(), [0xcf, 0xfa, 0xed, 0xfe, 7, 0, 0, 1]).unwrap();
        assert!(!arm64_binary(f.path()).unwrap());
        #[cfg(unix)]
        {
            std::fs::write(f.path(), MAC_HANDOFF).unwrap();
            assert!(std::process::Command::new("/bin/sh")
                .arg("-n")
                .arg(f.path())
                .status()
                .unwrap()
                .success());
        }
    }
    #[test]
    fn macos_signature_platform_and_suffix() {
        let (bytes, key) = signed("0.4.0-alpha.2", |m| {
            m["platform"] = "macos-aarch64".into();
            m["filename"] = "M4B-Editor-0.4.0-alpha.2-macos-aarch64.dmg".into();
            m["url"] =
                format!("{REPO}v0.4.0-alpha.2/M4B-Editor-0.4.0-alpha.2-macos-aarch64.dmg").into();
        });
        assert!(verify_platform(
            &bytes,
            Channel::Prerelease,
            "0.3.0-alpha.1",
            &key,
            "macos-aarch64"
        )
        .is_ok());
        assert!(verify_platform(
            &bytes,
            Channel::Prerelease,
            "0.3.0-alpha.1",
            &key,
            "windows-x86_64"
        )
        .is_err());
        assert!(MAC_HANDOFF.contains("while kill -0"));
    }
    #[test]
    fn rejects_file_tampering_before_launch() {
        let (bytes, key) = signed("0.3.0", |m| {
            m["sha256"] = hex::encode(Sha256::digest(b"0123456789")).into();
        });
        let update = verify_with(&bytes, Channel::Stable, "0.2.0", &key).unwrap();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"0123456789").unwrap();
        assert!(verify_file(file.reopen().unwrap(), &update.manifest).is_ok());
        // Same length, different bytes: size alone cannot authenticate an MSI.
        std::fs::write(file.path(), b"9876543210").unwrap();
        assert!(verify_file(file.reopen().unwrap(), &update.manifest).is_err());
        std::fs::write(file.path(), b"truncated").unwrap();
        assert!(verify_file(file.reopen().unwrap(), &update.manifest).is_err());
    }
    #[test]
    fn malformed_signature_and_envelope() {
        let (bytes, key) = signed("0.3.0", |_| {});
        let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for signature in ["00", "zz", &"00".repeat(64)] {
            envelope["signature"] = signature.into();
            assert!(verify_with(
                &serde_json::to_vec(&envelope).unwrap(),
                Channel::Stable,
                "0.2.0",
                &key
            )
            .is_err());
        }
        assert!(verify_with(&[b' '; 16385], Channel::Stable, "0.2.0", &key).is_err());
        assert!(verify_with(b"{}", Channel::Stable, "0.2.0", &key).is_err());
    }
    #[test]
    fn valid_and_tampered() {
        let (bytes, key) = signed("0.3.0", |_| {});
        assert!(verify_with(&bytes, Channel::Stable, "0.2.0-alpha.1", &key).is_ok());
        assert!(verify_with(&bytes, Channel::Stable, "0.2.0-alpha.1", &[0; 32]).is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["payload"] = value["payload"]
            .as_str()
            .unwrap()
            .replace("0.3.0", "9.0.0")
            .into();
        assert!(verify_with(
            &serde_json::to_vec(&value).unwrap(),
            Channel::Stable,
            "0.2.0",
            &key
        )
        .is_err());
    }
    #[test]
    fn rejects_signed_bad_fields() {
        for (field, value) in [
            ("platform", serde_json::json!("linux")),
            ("filename", serde_json::json!("../x.msi")),
            ("url", serde_json::json!("https://evil.example/x")),
            ("size", serde_json::json!(MAX_MSI + 1)),
            ("sha256", serde_json::json!("00")),
            ("extra", serde_json::json!(true)),
        ] {
            let (bytes, key) = signed("0.3.0", |m| m[field] = value);
            assert!(
                verify_with(&bytes, Channel::Stable, "0.2.0", &key).is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn versions_channels() {
        for version in ["0.1.9", "0.2.0", "0.3.0-alpha.1"] {
            let (bytes, key) = signed(version, |_| {});
            assert!(verify_with(&bytes, Channel::Stable, "0.2.0", &key).is_err());
        }
        let (bytes, key) = signed("0.3.0-alpha.1", |_| {});
        assert!(verify_with(&bytes, Channel::Prerelease, "0.2.0", &key).is_ok());
        let (bytes, key) = signed("0.10.0", |_| {});
        assert!(verify_with(&bytes, Channel::Stable, "0.9.0", &key).is_ok());
    }
    #[test]
    fn busy_dirty_and_origins() {
        assert!(check(Channel::Stable, true).is_err());
        let (bytes, key) = signed("0.3.0", |_| {});
        let verified = verify_with(&bytes, Channel::Stable, "0.2.0", &key).unwrap();
        assert!(verified.download(true).is_err());
        assert!(guard(true, false, false).is_err());
        assert!(guard(true, false, true).is_err());
        assert!(guard(false, true, true).is_err());
        assert!(guard(false, true, false).is_ok());
        for url in [
            "http://github.com/x",
            "https://github.com.evil.test/x",
            "https://evil.test/x",
            "https://github.com/aruel-perso/other/releases/download/a",
            "https://github.com/aruel-perso/m4b-craft/releases/download/v0.4.0/x",
            "https://api.github.com/repos/aruel-perso/m4b-editor/releases",
        ] {
            assert!(!allowed_redirect(&Url::parse(url).unwrap()));
        }
    }
}
