//! The tray icon and its menu (Tauri's built-in tray).
//!
//! - The menu (`model.rs`, pure) lists the folders that need the user with one action
//!   each (resolve a conflict, upload, download, show what is wrong), then "Sync all",
//!   "Pause for 1 hour" / "Resume", "Update to X.Y.Z" when an app update is
//!   available (`updater`), "Open cloudit", "Open on the web" and "Quit".
//! - The icon is the monochrome cloud, white or black to suit the taskbar
//!   (`theme.rs`, read again every [`theme::RECHECK`]); it gets a dot when a folder
//!   needs attention (`icon.rs`); the tooltip says the same in short.
//! - It is rebuilt when something it shows may have changed: the end of every
//!   operation (statuses), the folder list, the session, the pause and the UI language
//!   ([`Handle::refresh`]). Requests are coalesced ([`COALESCE`]) and the menu is
//!   replaced only when the model differs, so progress never rebuilds it.
//! - Every action goes through the same paths as the UI: syncs are manual runs through
//!   `ops` (the engine's safety rule holds), "Resolve" and "Show" open the window and
//!   send the UI a `navigate` event.
//! - Left click shows the window on Windows; on Linux (AppIndicator) icons have no
//!   click events: the menu's "Open cloudit" does it.

mod icon;
pub mod model;
mod theme;

use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::commands::{self, auto as settings};
use crate::events::{NAVIGATE, NavigateEvent, NavigatePage, OpAction, emit};
use crate::state::AppState;
use crate::updater::{self, Install, UpdatePhase};
use crate::window;
use model::{FolderAction, Item, MenuAction, TrayInput, TrayModel, TrayUpdate};
use theme::Taskbar;

/// The tray icon's id.
const TRAY_ID: &str = "main";
/// Refresh requests closer than this become one rebuild.
const COALESCE: Duration = Duration::from_millis(250);

/// Asks for a rebuild (cheap to clone; does nothing once the app exits).
#[derive(Debug, Clone)]
pub struct Handle {
    tx: UnboundedSender<()>,
}

impl Handle {
    pub fn refresh(&self) {
        let _closed = self.tx.send(());
    }
}

/// The tray's channel: the handle for `AppState`, the receiver for [`start`].
pub fn channel() -> (Handle, UnboundedReceiver<()>) {
    let (tx, rx) = unbounded_channel();
    (Handle { tx }, rx)
}

/// The tray, managed by Tauri once it exists.
pub struct Tray {
    icon: TrayIcon,
    /// The icons for the current taskbar.
    icons: Mutex<Icons>,
    /// What the menu shows now.
    shown: Mutex<Option<TrayModel>>,
}

/// The plain and the attention icon for one taskbar brightness.
struct Icons {
    taskbar: Taskbar,
    normal: Image<'static>,
    attention: Image<'static>,
}

impl Icons {
    fn new(taskbar: Taskbar) -> tauri::Result<Self> {
        let (normal, attention) = icon::icons(taskbar)?;
        Ok(Self {
            taskbar,
            normal,
            attention,
        })
    }

    fn pick(&self, attention: bool) -> Image<'static> {
        if attention {
            self.attention.clone()
        } else {
            self.normal.clone()
        }
    }
}

/// Whether the tray exists (closing the window then only hides it).
pub fn exists(app: &AppHandle) -> bool {
    app.try_state::<Tray>().is_some()
}

/// Creates the tray and starts its rebuild task (in `setup`, after `AppState`).
pub fn start(app: &AppHandle, mut rx: UnboundedReceiver<()>) -> tauri::Result<()> {
    let icons = Icons::new(theme::taskbar())?;
    let icon = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icons.pick(false))
        .tooltip("cloudit")
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(Tray {
        icon,
        icons: Mutex::new(icons),
        shown: Mutex::new(None),
    });
    render(app);
    follow_taskbar(app.clone());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while rx.recv().await.is_some() {
            tokio::time::sleep(COALESCE).await;
            while rx.try_recv().is_ok() {}
            render(&app);
        }
    });
    Ok(())
}

/// The end of the pause as the local `HH:MM`.
pub fn local_time(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).format("%H:%M").to_string()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Reads the taskbar brightness every [`theme::RECHECK`] and swaps the icons when it
/// changed.
fn follow_taskbar(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(theme::RECHECK).await;
            let Ok(taskbar) = tauri::async_runtime::spawn_blocking(theme::taskbar).await else {
                continue;
            };
            let Some(tray) = app.try_state::<Tray>() else {
                return;
            };
            if lock(&tray.icons).taskbar == taskbar {
                continue;
            }
            let icons = match Icons::new(taskbar) {
                Ok(icons) => icons,
                Err(error) => {
                    tracing::warn!(%error, "could not load the tray icons");
                    continue;
                }
            };
            let attention = lock(&tray.shown).as_ref().is_some_and(|m| m.attention);
            if let Err(error) = tray.icon.set_icon(Some(icons.pick(attention))) {
                tracing::warn!(%error, "could not change the tray icon");
                continue;
            }
            tracing::debug!(?taskbar, "the taskbar theme changed");
            *lock(&tray.icons) = icons;
        }
    });
}

/// Rebuilds the tray if what it shows changed (failures are logged).
fn render(app: &AppHandle) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    let state = app.state::<AppState>();
    let folders = state.engine.folders().unwrap_or_else(|error| {
        tracing::warn!(%error, "tray: could not read the folders");
        Vec::new()
    });
    let has_account = matches!(state.engine.account(), Ok(Some(_)));
    let paused = state.engine.paused_until().ok().flatten().map(local_time);
    let update = updater::current(app);
    let shown_update = matches!(update.phase, UpdatePhase::Available | UpdatePhase::Failed)
        .then_some(update.latest_version.as_deref())
        .flatten()
        .map(|version| TrayUpdate {
            version,
            in_app: update.install == Install::InApp,
        });
    let model = model::build(TrayInput {
        signed_in: state.engine.signed_in(),
        has_account,
        folders: &folders,
        paused_until: paused.as_deref(),
        update: shown_update,
        lang: state.lang(),
    });
    let mut shown = tray
        .shown
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if shown.as_ref() == Some(&model) {
        return;
    }
    if let Err(error) = apply(app, &tray, shown.as_ref(), &model) {
        tracing::warn!(%error, "could not update the tray");
        return;
    }
    *shown = Some(model);
}

fn apply(
    app: &AppHandle,
    tray: &Tray,
    shown: Option<&TrayModel>,
    model: &TrayModel,
) -> tauri::Result<()> {
    let menu = Menu::new(app)?;
    for (i, section) in [&model.status, &model.sync, &model.open].iter().enumerate() {
        if i > 0 {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        for item in section.iter() {
            menu.append(&menu_item(app, item)?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&menu_item(app, &model.quit)?)?;
    tray.icon.set_menu(Some(menu))?;
    if shown.is_none_or(|s| s.attention != model.attention) {
        let icon = lock(&tray.icons).pick(model.attention);
        tray.icon.set_icon(Some(icon))?;
    }
    if shown.is_none_or(|s| s.tooltip != model.tooltip) {
        tray.icon.set_tooltip(Some(&model.tooltip))?;
    }
    Ok(())
}

fn menu_item(app: &AppHandle, item: &Item) -> tauri::Result<MenuItem<Wry>> {
    // `&` marks a mnemonic; folder names may contain one.
    let label = item.label.replace('&', "&&");
    match item.action {
        Some(action) => MenuItem::with_id(app, action.id(), label, item.enabled, None::<&str>),
        None => MenuItem::new(app, label, item.enabled, None::<&str>),
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let Some(action) = MenuAction::parse(event.id().as_ref()) else {
        return;
    };
    tracing::debug!(?action, "tray action");
    match action {
        MenuAction::Folder { folder, action } => match action {
            FolderAction::Resolve => navigate(app, folder, NavigatePage::Conflict),
            FolderAction::Review | FolderAction::Show => {
                navigate(app, folder, NavigatePage::Detail);
            }
            FolderAction::Upload | FolderAction::Download => {
                commands::folders::spawn_all(app.clone(), vec![folder], OpAction::Sync);
            }
        },
        MenuAction::Update => match updater::current(app).install {
            Install::InApp => {
                window::show_main(app);
                if let Err(error) = updater::install(app) {
                    tracing::warn!(kind = ?error.kind, message = %error.message, "tray: update failed");
                }
            }
            Install::ReleasePage => commands::updates::open_release_page(app.clone()),
            Install::Never => window::show_main(app),
        },
        MenuAction::SyncAll => {
            if let Err(error) = commands::folders::sync_all_folders(app) {
                tracing::warn!(kind = ?error.kind, message = %error.message, "tray: sync all failed");
            }
        }
        MenuAction::Pause => {
            if let Err(error) = settings::start_pause(app, settings::TRAY_PAUSE_MINUTES) {
                tracing::warn!(kind = ?error.kind, message = %error.message, "tray: pause failed");
            }
        }
        MenuAction::Resume => {
            if let Err(error) = settings::end_pause(app) {
                tracing::warn!(kind = ?error.kind, message = %error.message, "tray: resume failed");
            }
        }
        MenuAction::Open => window::show_main(app),
        MenuAction::Web => {
            let account = app.state::<AppState>().engine.account();
            match account {
                Ok(Some(account)) => commands::open_url(app, &account.server_url),
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "tray: could not read the account"),
            }
        }
        MenuAction::Quit => {
            tracing::info!("quitting from the tray");
            app.exit(0);
        }
    }
}

/// Shows the window on a folder's page.
fn navigate(app: &AppHandle, folder: i64, page: NavigatePage) {
    window::show_main(app);
    emit(app, NAVIGATE, &NavigateEvent { folder, page });
}
