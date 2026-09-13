//! The system tray icon: Show/Hide and Quit, shown only while "Hide tray on close" is on.
//!
//! Winit runs its own event loop and never touches GTK, but `tray-icon`'s Linux backend
//! (libappindicator) is built on GTK and needs GTK's own loop pumping to show anything or
//! deliver a click — so on Linux the tray lives on a dedicated thread that does nothing but
//! `gtk::init()` then block in `gtk::main()`, fully decoupled from egui's render thread. On
//! Windows and macOS the tray icon can be built directly on the calling thread — see the
//! crate's own `examples/egui.rs`, which this follows.
//!
//! [`spawn`] is called at most once per process, the first time "Hide tray on close" is turned
//! on — not once per toggle. `gtk::main()` never returns (nothing here ever calls
//! `gtk_main_quit`), so an earlier version that called `spawn` again on every toggle leaked a
//! thread, a GTK loop and a duplicate tray icon each time the setting was flipped off and back
//! on. [`Tray::set_visible`] is the fix: it show or hides the one icon that was ever built,
//! over a channel into the GTK thread, so the thread and its `TrayIcon` live for the rest of
//! the run regardless of how many times the setting changes.
//!
//! However it was built, `TrayIconEvent`/`MenuEvent` arrive through the crate's own global
//! channels rather than anything this module hands back, so reading them from the main thread
//! via [`poll`] costs nothing and needs no cross-thread plumbing: it is drained once a frame
//! from [`crate::app::App::ui`], the same way `Backend`'s own event channel already is.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

const SHOW_HIDE_ID: &str = "ytm-tray-show-hide";
const QUIT_ID: &str = "ytm-tray-quit";

/// What choosing a tray menu item means for the app.
pub enum TrayAction {
    ToggleVisible,
    Quit,
}

/// The icon, decoded once per call from the same PNG [`crate::window_icon`] uses for the
/// titlebar/taskbar — one source of truth for what Refryt's icon looks like.
fn icon() -> Option<tray_icon::Icon> {
    let bytes = include_bytes!("../../../assets/icons/app/refryt-256.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), width, height).ok()
}

fn menu() -> Menu {
    let show_hide = MenuItem::with_id(SHOW_HIDE_ID, "Show/Hide", true, None);
    let quit = MenuItem::with_id(QUIT_ID, "Quit", true, None);
    Menu::with_items(&[&show_hide, &quit]).unwrap_or_else(|_| Menu::new())
}

/// A live tray icon's handle, kept for the rest of the run once built. On Linux this is a
/// channel into the thread that actually owns the `TrayIcon` (GTK objects can't cross
/// threads); on Windows/macOS the `TrayIcon` can just live here directly. Either way,
/// `set_visible` is the only thing this is for after construction — see the module doc comment
/// for why toggling visibility, not rebuilding the icon, is what a setting toggle should do.
pub struct Tray {
    #[cfg(target_os = "linux")]
    visibility: Option<glib::Sender<bool>>,
    #[cfg(not(target_os = "linux"))]
    icon: Option<tray_icon::TrayIcon>,
}

impl Tray {
    /// Show or hide the icon. A no-op if [`spawn`] never managed to build one (GTK init or the
    /// icon build failed) — reported already, at construction time, so this stays quiet.
    pub fn set_visible(&self, visible: bool) {
        #[cfg(target_os = "linux")]
        if let Some(tx) = &self.visibility {
            let _ = tx.send(visible);
        }
        #[cfg(not(target_os = "linux"))]
        if let Some(icon) = &self.icon {
            let _ = icon.set_visible(visible);
        }
    }
}

/// Build the tray icon. Call once, the first time "Hide tray on close" is turned on — not on
/// every toggle after that, which is exactly the mistake this module's doc comment describes;
/// use [`Tray::set_visible`] for every toggle from then on.
pub fn spawn() -> Tray {
    #[cfg(target_os = "linux")]
    {
        // Rendezvous with the GTK thread once, to hand back the channel that will control it
        // for the rest of the run — `recv()` blocks only until that thread finishes its
        // one-time setup (or gives up), not for the thread's whole lifetime.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Err(e) = gtk::init() {
                tracing::warn!(%e, "could not init gtk; no tray icon this run");
                let _ = ready_tx.send(None);
                return;
            }
            let Some(icon) = icon() else {
                tracing::warn!("could not decode the tray icon");
                let _ = ready_tx.send(None);
                return;
            };
            let tray_icon = match TrayIconBuilder::new()
                .with_menu(Box::new(menu()))
                .with_icon(icon)
                .with_tooltip("Refryt")
                .build()
            {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(%e, "could not create the tray icon");
                    let _ = ready_tx.send(None);
                    return;
                }
            };
            // A GLib channel, not `std::sync::mpsc`: its receiver runs its callback on
            // whichever main context it's attached to (here) rather than needing this thread
            // to poll it, which is what lets a plain `gtk::main()` below pick up sends from
            // any other thread safely despite GTK objects themselves not being thread-safe.
            #[allow(deprecated)] // superseded by `spawn_future_local`, but this stays a plain
            // synchronous GTK3 loop rather than pulling in an async runtime just for one channel.
            let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);
            rx.attach(None, move |visible| {
                let _ = tray_icon.set_visible(visible);
                glib::ControlFlow::Continue
            });
            let _ = ready_tx.send(Some(tx));
            // Kept alive by staying in scope for `gtk::main()`'s whole run — which is the
            // process's whole remaining lifetime, since nothing here ever calls
            // `gtk_main_quit`.
            gtk::main();
        });
        Tray {
            visibility: ready_rx.recv().ok().flatten(),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let icon = icon().and_then(|icon| {
            TrayIconBuilder::new()
                .with_menu(Box::new(menu()))
                .with_icon(icon)
                .with_tooltip("Refryt")
                .build()
                .inspect_err(|e| tracing::warn!(%e, "could not create the tray icon"))
                .ok()
        });
        Tray { icon }
    }
}

/// Drain one tray/menu event, if any arrived since the last call.
pub fn poll() -> Option<TrayAction> {
    if let Ok(event) = MenuEvent::receiver().try_recv() {
        return match event.id.0.as_str() {
            SHOW_HIDE_ID => Some(TrayAction::ToggleVisible),
            QUIT_ID => Some(TrayAction::Quit),
            _ => None,
        };
    }
    // A left-click on the icon itself (no menu involved) toggles the window too — the ordinary
    // convention, and the only interaction some Linux status-notifier hosts offer at all. Only
    // the release, and only the left button: `Click` also fires on press, and fires for a
    // right-click too — which a status-notifier host already turns into opening the context
    // menu on its own, so reacting to it here as well would toggle the window at the same time.
    if let Ok(TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    }) = TrayIconEvent::receiver().try_recv()
    {
        return Some(TrayAction::ToggleVisible);
    }
    None
}
