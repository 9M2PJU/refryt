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
//! on. [`Tray::set_visible`] is the fix: it shows or hides the one icon that was ever built,
//! over a channel into the GTK thread, so the thread and its `TrayIcon` live for the rest of
//! the run regardless of how many times the setting changes.
//!
//! `TrayIconEvent`/`MenuEvent` arrive on whichever thread the crate's backend runs on — the GTK
//! thread on Linux, the icon's own message pump on Windows — never the render thread, and the
//! crate's own channels do not wake winit. So [`spawn`] installs the crate's process-wide event
//! handlers instead of leaving the events to be polled: each handler queues a [`TrayAction`]
//! and calls `egui::Context::request_repaint`. The repaint is the half that matters: an idle
//! window schedules no frames of its own, so without it a tray click sat in the queue until
//! something unrelated happened to paint — which is what made Quit take as long as it took for
//! that something to arrive. [`poll`] drains the queue once a frame, the same way `Backend`'s
//! own event channel already is.

use std::collections::VecDeque;
use std::sync::Mutex;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

const SHOW_HIDE_ID: &str = "ytm-tray-show-hide";
const QUIT_ID: &str = "ytm-tray-quit";

/// What choosing a tray menu item — or clicking the icon itself — means for the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    /// Flip the window between hidden and shown.
    ToggleVisible,
    /// Unhide the window and focus it. A double-click's meaning; it never hides.
    Show,
    Quit,
}

/// Actions queued by the crate's event handlers, drained by [`poll`].
///
/// The handlers run on the tray backend's thread, never the render thread, so the handoff is a
/// plain queue; the `request_repaint` in [`inject`] is what makes the render thread look at it.
static ACTIONS: Mutex<VecDeque<TrayAction>> = Mutex::new(VecDeque::new());

/// Hand `action` to the render thread: queue it, then wake the UI.
///
/// `pub(crate)` for the `--tray-test` diagnostic, which has to reach the same queue a real
/// click does — there is no way to click a tray icon from outside the process.
pub(crate) fn inject(ctx: &egui::Context, action: TrayAction) {
    if let Ok(mut queue) = ACTIONS.lock() {
        queue.push_back(action);
    }
    ctx.request_repaint();
}

/// Install the process-wide handlers that turn the crate's events into [`TrayAction`]s.
///
/// Called from [`spawn`], which runs at most once per process: the crate's handlers are
/// first-set-only, so a second call would silently keep the first closure — and with it the
/// first `egui::Context` — anyway.
fn install_handlers(ctx: &egui::Context) {
    let menu_ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(action) = menu_action(event.id.0.as_str()) {
            inject(&menu_ctx, action);
        }
    }));
    let icon_ctx = ctx.clone();
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let Some(action) = icon_action(&event) {
            inject(&icon_ctx, action);
        }
    }));
}

/// What a menu id means, if it is one of ours.
fn menu_action(id: &str) -> Option<TrayAction> {
    match id {
        SHOW_HIDE_ID => Some(TrayAction::ToggleVisible),
        QUIT_ID => Some(TrayAction::Quit),
        _ => None,
    }
}

/// What a click on the icon itself means, if anything.
///
/// A left release toggles the window on Linux, where a status-notifier host may offer no other
/// interaction at all. On Windows it means nothing: a left click there is the first half of a
/// double-click — the crate emits `Click(Up)`, then `DoubleClick`, then a second `Click(Up)` —
/// so a toggle on the first release would fight the double-click's own show (hide, show, hide).
///
/// `DoubleClick` shows the window on every platform that reports one. `Click` also fires on
/// press and for a right-click; only the release of the left button is considered — a
/// status-notifier host already turns a right-click into the context menu on its own, and
/// reacting to it here as well would toggle the window at the same time.
fn icon_action(event: &TrayIconEvent) -> Option<TrayAction> {
    match event {
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => Some(TrayAction::Show),
        TrayIconEvent::Click {
            button,
            button_state,
            ..
        } => click_action(*button, *button_state),
        _ => None,
    }
}

/// [`icon_action`]'s click rule on plain values, so each platform's answer is a unit test
/// rather than something only checkable by clicking a tray icon.
fn click_action(button: MouseButton, state: MouseButtonState) -> Option<TrayAction> {
    if button != MouseButton::Left || state != MouseButtonState::Up {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        Some(TrayAction::ToggleVisible)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
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
pub fn spawn(ctx: &egui::Context) -> Tray {
    // Installed even if the icon itself never gets built: the handlers are what wake the UI,
    // and they are harmless with no icon to click.
    install_handlers(ctx);
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
            // Windows opens the context menu on left-click by default; left as-is, the menu
            // would open on the first click of a double-click, which now means "show". Right
            // click still opens the menu, the Windows convention, and a single left click does
            // nothing — see `click_action`.
            #[cfg(target_os = "windows")]
            let builder = TrayIconBuilder::new()
                .with_menu(Box::new(menu()))
                .with_icon(icon)
                .with_tooltip("Refryt")
                .with_menu_on_left_click(false);
            #[cfg(not(target_os = "windows"))]
            let builder = TrayIconBuilder::new()
                .with_menu(Box::new(menu()))
                .with_icon(icon)
                .with_tooltip("Refryt");
            builder
                .build()
                .inspect_err(|e| tracing::warn!(%e, "could not create the tray icon"))
                .ok()
        });
        Tray { icon }
    }
}

/// Drain one tray action, if any arrived since the last call.
pub fn poll() -> Option<TrayAction> {
    ACTIONS.lock().ok()?.pop_front()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_map_to_their_actions() {
        assert_eq!(menu_action(SHOW_HIDE_ID), Some(TrayAction::ToggleVisible));
        assert_eq!(menu_action(QUIT_ID), Some(TrayAction::Quit));
        assert_eq!(menu_action("some-other-item"), None);
    }

    #[test]
    fn only_the_left_release_is_a_click() {
        assert_eq!(click_action(MouseButton::Right, MouseButtonState::Up), None);
        assert_eq!(
            click_action(MouseButton::Middle, MouseButtonState::Up),
            None
        );
        assert_eq!(
            click_action(MouseButton::Left, MouseButtonState::Down),
            None
        );
    }

    /// On Linux the left click is the only interaction some status-notifier hosts offer, so it
    /// keeps toggling the window.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_left_release_toggles_on_linux() {
        assert_eq!(
            click_action(MouseButton::Left, MouseButtonState::Up),
            Some(TrayAction::ToggleVisible)
        );
    }

    /// On Windows a left release is the first half of a double-click, whose own action is the
    /// show — see `icon_action`.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn a_left_release_means_nothing_where_double_click_shows() {
        assert_eq!(click_action(MouseButton::Left, MouseButtonState::Up), None);
    }

    fn double_click(button: MouseButton) -> TrayIconEvent {
        TrayIconEvent::DoubleClick {
            id: tray_icon::TrayIconId::new("test"),
            position: tray_icon::Rect::default().position,
            rect: tray_icon::Rect::default(),
            button,
        }
    }

    /// The gesture this module's Windows rule exists for: a left double-click shows the window,
    /// and a right one — the context menu's own double-click, if a host sends one — does not.
    #[test]
    fn a_left_double_click_shows_the_window() {
        assert_eq!(
            icon_action(&double_click(MouseButton::Left)),
            Some(TrayAction::Show)
        );
        assert_eq!(icon_action(&double_click(MouseButton::Right)), None);
        assert_eq!(icon_action(&double_click(MouseButton::Middle)), None);
    }
}
