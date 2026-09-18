//! "Start on boot".
//!
//! Two very different mechanisms for one setting: an XDG autostart entry on Linux (matching the
//! Linux-first install path in `scripts/appimage-install.sh`), and the per-user `Run` key on
//! Windows — the registry location Task Manager's own Startup tab reads, so the switch can be
//! audited and turned off from outside the app the same way every other Windows app's is.

#[cfg(target_os = "linux")]
mod linux {
    use std::path::PathBuf;

    use anyhow::{Context, Result};

    fn path() -> Result<PathBuf> {
        let dirs = directories::BaseDirs::new().context("could not find the config directory")?;
        Ok(dirs.config_dir().join("autostart").join("refryt.desktop"))
    }

    /// The binary to launch on login.
    ///
    /// Not simply `Exec=refryt`, the way the application-launcher entry gets away with it: that
    /// one assumes whatever installed it also put `refryt` on `PATH`, which is true for the
    /// AppImage installer's own symlink-free copy into `~/.local/bin` today but is not a safe
    /// assumption in general — and `std::env::current_exe()` alone is worse for the AppImage case
    /// specifically, since an AppImage runs from a temporary squashfs mount that stops existing
    /// the moment this process exits, so writing *that* path would autostart something that is
    /// gone by next login. AppImage's own runtime sets `$APPIMAGE` to the real, persistent
    /// `.AppImage` file it was launched from, so that takes priority; `current_exe()` is the
    /// fallback for every other way this binary might be installed.
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

    /// Write the autostart entry. Called only when "Start on boot" is turned on.
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
}

#[cfg(target_os = "windows")]
mod windows {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;
    use std::ptr;

    use anyhow::{Context, Result, bail};
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegCreateKeyExW,
        RegDeleteValueW, RegSetValueExW,
    };

    /// The per-user Run key: this user only, which is the scope the setting is about, and no
    /// elevation needed to write it.
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    /// One fixed value name, so enabling twice overwrites rather than accumulates entries.
    const VALUE_NAME: &str = "refryt";

    /// What the Run value should say for a binary at `exe`.
    ///
    /// Quoted: a Run value is a command line, and an unquoted `C:\Program Files\...` path would
    /// be split at the first space. Pure, so the quoting is testable without a registry.
    fn run_value(exe: &Path) -> String {
        format!("\"{}\"", exe.display())
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    /// Open (creating if needed) the Run key for writing.
    fn open_key() -> Result<HKEY> {
        let subkey = wide(RUN_KEY);
        let mut key: HKEY = 0;
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                ptr::null(),
                0,
                KEY_SET_VALUE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            bail!("could not open HKCU\\{RUN_KEY} (error {status})");
        }
        Ok(key)
    }

    /// Point the Run key at this binary. Called only when "Start on boot" is turned on.
    pub fn enable() -> Result<()> {
        let exe = std::env::current_exe().context("could not determine this binary's own path")?;
        let name = wide(VALUE_NAME);
        let data = wide(&run_value(&exe));
        let key = open_key()?;
        let status = unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * size_of::<u16>()) as u32,
            )
        };
        unsafe { RegCloseKey(key) };
        if status != ERROR_SUCCESS {
            bail!("could not write the Run value (error {status})");
        }
        Ok(())
    }

    /// Remove the Run value. Quiet if it was never there, like the Linux backend.
    pub fn disable() -> Result<()> {
        let name = wide(VALUE_NAME);
        let key = open_key()?;
        let status = unsafe { RegDeleteValueW(key, name.as_ptr()) };
        unsafe { RegCloseKey(key) };
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            bail!("could not remove the Run value (error {status})");
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_run_value_quotes_a_path_with_spaces() {
            let value = run_value(Path::new(r"C:\Program Files\refryt\refryt.exe"));
            assert_eq!(value, "\"C:\\Program Files\\refryt\\refryt.exe\"");
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{disable, enable};

#[cfg(target_os = "windows")]
pub use windows::{disable, enable};
