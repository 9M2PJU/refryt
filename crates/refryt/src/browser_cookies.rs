//! Import YouTube cookies from a browser you are already signed into.
//!
//! # Why this exists
//!
//! Pasting a cookie from the DevTools console cannot work. `document.cookie` is specified not to
//! expose `HttpOnly` cookies, and `HSID`, `SSID`, `LOGIN_INFO`, `__Secure-1PSID` and
//! `__Secure-3PSID` — the ones that actually authenticate a YouTube session — are all HttpOnly.
//! A cookie pasted that way carries `SAPISID`, which is enough to *build* the `SAPISIDHASH`
//! header and therefore enough for `ytmapi-rs` to accept it, but every request made with it comes
//! back anonymous. That is exactly how this app once reported "Signed in" while showing an empty
//! library.
//!
//! The browser has the whole jar. Reading it needs no web engine and no browser process: on
//! Chromium the store is SQLite plus AES-GCM with a key from the OS keyring, and on Firefox it is
//! plain SQLite. `yt-dlp` already implements all of that, and this project already requires
//! `yt-dlp` for stream resolution — so importing costs one subprocess and no new dependency.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

/// A browser `yt-dlp` can read cookies from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Browser {
    /// What `--cookies-from-browser` calls it.
    pub key: &'static str,
    /// What the Settings page calls it.
    pub label: &'static str,
}

/// The browsers we look for, with the profile directory that proves one is installed.
///
/// Ordered by how likely a signed-in music session is to be there rather than alphabetically:
/// a dedicated browser first, then the mainstream ones.
const KNOWN: &[(Browser, &[&str])] = &[
    (
        Browser {
            key: "brave",
            label: "Brave",
        },
        &[
            ".config/BraveSoftware/Brave-Browser",
            "Library/Application Support/BraveSoftware/Brave-Browser",
            "AppData/Local/BraveSoftware/Brave-Browser/User Data",
        ],
    ),
    (
        Browser {
            key: "firefox",
            label: "Firefox",
        },
        &[
            ".mozilla/firefox",
            // Some distros (Arch/CachyOS among them) patch Firefox to follow the XDG base
            // directory spec instead of the historical `~/.mozilla` path.
            ".config/mozilla/firefox",
            "snap/firefox/common/.mozilla/firefox",
            ".var/app/org.mozilla.firefox/.mozilla/firefox",
            "Library/Application Support/Firefox",
            "AppData/Roaming/Mozilla/Firefox",
        ],
    ),
    (
        Browser {
            key: "chrome",
            label: "Chrome",
        },
        &[
            ".config/google-chrome",
            "Library/Application Support/Google/Chrome",
            "AppData/Local/Google/Chrome/User Data",
        ],
    ),
    (
        Browser {
            key: "chromium",
            label: "Chromium",
        },
        &[
            ".config/chromium",
            "Library/Application Support/Chromium",
            "AppData/Local/Chromium/User Data",
        ],
    ),
    (
        Browser {
            key: "vivaldi",
            label: "Vivaldi",
        },
        &[
            ".config/vivaldi",
            "Library/Application Support/Vivaldi",
            "AppData/Local/Vivaldi/User Data",
        ],
    ),
    (
        Browser {
            key: "edge",
            label: "Edge",
        },
        &[
            ".config/microsoft-edge",
            "Library/Application Support/Microsoft Edge",
            "AppData/Local/Microsoft/Edge/User Data",
        ],
    ),
];

fn home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_owned())
}

/// Whether `yt-dlp` can read this browser's cookies on this OS at all.
///
/// On Windows every Chromium browser seals its cookie key to its own process (App-Bound
/// Encryption, Chromium 127+), so the import fails with "Failed to decrypt with DPAPI" no matter
/// what the user does — `yt-dlp` #10927. `webview_login` is the path that works there, and
/// offering these browsers anyway would only produce the failure that window exists to avoid.
/// Firefox has no such scheme, and is offered everywhere.
///
/// On macOS Chromium browsers keep their cookie key in the login Keychain. The first import makes
/// macOS ask for permission to read it, so a password prompt there is expected, not a fault.
fn readable_on_this_os(key: &str) -> bool {
    if cfg!(target_os = "windows") {
        !matches!(key, "brave" | "chrome" | "chromium" | "vivaldi" | "edge")
    } else {
        let _ = key;
        true
    }
}

/// The browsers that have a profile directory on this machine and a store `yt-dlp` can read.
///
/// Presence of a profile is not proof that `yt-dlp` can read it — a locked keyring or a sandboxed
/// profile will still fail — so the import surfaces its error rather than this pretending to know.
pub fn detected() -> Vec<Browser> {
    let Some(home) = home() else {
        return Vec::new();
    };
    KNOWN
        .iter()
        .filter(|(b, _)| readable_on_this_os(b.key))
        .filter(|(_, paths)| paths.iter().any(|p| home.join(p).is_dir()))
        .map(|(b, _)| *b)
        .collect()
}

/// Import the YouTube cookies from `browser` and return them as a `Cookie` header value.
///
/// Runs `yt-dlp --cookies-from-browser <browser> --cookies <file>` with **no URL**: yt-dlp writes
/// the jar and then exits 2 because it has nothing to download, so this makes no network request
/// and opens no browser. The exit status is therefore not a failure signal — the file is.
pub async fn import(browser: &str) -> Result<String> {
    let path = temp_jar()?;
    // Best effort cleanup even on the error paths below.
    let _guard = Cleanup(path.clone());

    let mut cmd = Command::new(crate::backend::yt_dlp_program());
    cmd.arg("--cookies-from-browser")
        .arg(browser)
        .arg("--cookies")
        .arg(&path);
    // See `ytdlp::no_console`'s doc comment: without this, this GUI-subsystem app spawning a
    // console-subsystem binary makes Windows flash a new console window for it.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let output = cmd.output().await.context(
        "could not run yt-dlp; it is required for playback too, so install it and try again",
    )?;

    let jar = match std::fs::read_to_string(&path) {
        Ok(jar) => jar,
        Err(_) => {
            // yt-dlp wrote nothing, so its stderr is the real explanation: a locked keyring, an
            // unsupported browser, a profile it cannot see.
            let why = String::from_utf8_lossy(&output.stderr);
            let why = why
                .lines()
                .find(|l| l.contains("ERROR") || l.contains("could not"))
                .unwrap_or_else(|| why.trim())
                .trim();
            bail!("yt-dlp could not read {browser}'s cookies: {why}");
        }
    };

    let cookie = cookie_header(&jar);
    if cookie.is_empty() {
        bail!("no youtube.com cookies in {browser}; sign in to YouTube Music there first");
    }
    Ok(cookie)
}

/// Delete the exported jar as soon as we are done with it: it is a credential.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn temp_jar() -> Result<PathBuf> {
    let dir = std::env::temp_dir();
    let unique = format!(
        "refryt-cookies-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    Ok(dir.join(unique))
}

/// Turn a Netscape cookie jar into a `Cookie` header, keeping only youtube.com.
///
/// The format is one tab-separated record per line: domain, include-subdomains, path, secure,
/// expiry, name, value. Comment and blank lines are skipped. A name that appears for several
/// domains is kept once — the last wins, which is the more specific host in yt-dlp's output.
pub fn cookie_header(jar: &str) -> String {
    let mut names: Vec<String> = Vec::new();
    let mut values: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for line in jar.lines() {
        let line = line.trim_end();
        // yt-dlp writes `#HttpOnly_` before the domain of an HttpOnly cookie — which is precisely
        // the set a console paste is missing, so it must not be treated as a comment.
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [domain, _, _, _, _, name, value] = fields[..] else {
            continue;
        };
        if !domain.contains("youtube.com") || name.is_empty() {
            continue;
        }
        if values.insert(name.to_owned(), value.to_owned()).is_none() {
            names.push(name.to_owned());
        }
    }

    names
        .iter()
        .map(|n| format!("{n}={}", values[n]))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const JAR: &str = "\
# Netscape HTTP Cookie File
# This file is generated by yt-dlp.

.youtube.com\tTRUE\t/\tTRUE\t1799999999\tSAPISID\tsapisid-value
#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1799999999\t__Secure-3PSID\tsecret-3psid
#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1799999999\tHSID\tsecret-hsid
.google.com\tTRUE\t/\tTRUE\t1799999999\tNID\tnot-ours
music.youtube.com\tFALSE\t/\tTRUE\t1799999999\twide\t1
";

    /// The whole point: yt-dlp marks HttpOnly cookies with a `#HttpOnly_` prefix, and those are
    /// exactly the ones a console paste cannot see. Treating the line as a comment would import a
    /// jar just as useless as the paste it replaces.
    #[test]
    fn httponly_cookies_are_imported_not_skipped() {
        let header = cookie_header(JAR);
        assert!(header.contains("__Secure-3PSID=secret-3psid"), "{header}");
        assert!(header.contains("HSID=secret-hsid"), "{header}");
    }

    #[test]
    fn only_youtube_cookies_are_kept() {
        let header = cookie_header(JAR);
        assert!(header.contains("SAPISID=sapisid-value"));
        assert!(header.contains("wide=1"));
        assert!(
            !header.contains("not-ours"),
            "a google.com cookie leaked in"
        );
    }

    #[test]
    fn the_header_is_well_formed() {
        let header = cookie_header(JAR);
        assert_eq!(header.matches("; ").count(), 3, "{header}");
        assert!(!header.ends_with(';'));
        assert!(header.split("; ").all(|p| p.contains('=')));
    }

    #[test]
    fn junk_does_not_panic() {
        assert_eq!(cookie_header(""), "");
        assert_eq!(cookie_header("# only a comment\n\n"), "");
        assert_eq!(cookie_header("not\ttab\tseparated\n"), "");
    }
}
