//! "Start on boot", via an XDG autostart entry — Linux only, matching this project's other
//! Linux-first install path (`scripts/appimage-install.sh` writes the same shape of `.desktop`
//! file for the application-launcher entry).

use std::path::PathBuf;

use anyhow::{Context, Result};

fn path() -> Result<PathBuf> {
    let dirs = directories::BaseDirs::new().context("could not find the config directory")?;
    Ok(dirs.config_dir().join("autostart").join("refryt.desktop"))
}

/// The binary to launch on login.
///
/// Not simply `Exec=refryt`, the way the application-launcher entry gets away with it: that one
/// assumes whatever installed it also put `refryt` on `PATH`, which is true for the AppImage
/// installer's own symlink-free copy into `~/.local/bin` today but is not a safe assumption in
/// general — and `std::env::current_exe()` alone is worse for the AppImage case specifically,
/// since an AppImage runs from a temporary squashfs mount that stops existing the moment this
/// process exits, so writing *that* path would autostart something that is gone by next login.
/// AppImage's own runtime sets `$APPIMAGE` to the real, persistent `.AppImage` file it was
/// launched from, so that takes priority; `current_exe()` is the fallback for every other way
/// this binary might be installed.
fn exec_path() -> Option<String> {
    std::env::var("APPIMAGE").ok().or_else(|| {
        std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
    })
}

fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Refryt\n\
         Comment=A fast, native YouTube Music client\n\
         Exec={exec}\n\
         Icon=refryt\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// Write the autostart entry. Called only on Linux, and only when "Start on boot" is turned on.
pub fn enable() -> Result<()> {
    let Some(exec) = exec_path() else {
        anyhow::bail!("could not determine this binary's own path");
    };
    let path = path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    std::fs::write(&path, desktop_entry(&exec))
        .with_context(|| format!("could not write {}", path.display()))
}

/// Remove the autostart entry. Quiet if it was never there — turning the setting on and off
/// without ever having restarted in between should not surface an error either way.
pub fn disable() -> Result<()> {
    let path = path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("could not remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_names_the_given_binary() {
        let entry = desktop_entry("/home/user/.local/bin/refryt.AppImage");
        assert!(entry.contains("Exec=/home/user/.local/bin/refryt.AppImage"));
        assert!(entry.contains("Type=Application"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
    }
}
