//! What the tray shows, as a pure function of the linked folders, the session, the
//! global pause and the language (tested without a GUI). `tray/mod.rs` turns a
//! [`TrayModel`] into the real menu and rebuilds it only when the model changes.

use cloudit_client::{LinkedFolder, SyncState};

use crate::i18n::{Lang, Msg, text};

/// Folders listed by name at most; the rest are summed up in one line.
pub const MAX_FOLDERS: usize = 8;

/// What a folder's menu item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderAction {
    /// This device's conflict: the decision page.
    Resolve,
    /// A conflict left by another device: the folder's detail (history on the web).
    Review,
    /// A manual sync (local changes to upload).
    Upload,
    /// A manual sync (server changes to download).
    Download,
    /// Something to fix (missing folder, error...): the folder's detail.
    Show,
}

/// A menu item's action, encoded in its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Folder {
        folder: i64,
        action: FolderAction,
    },
    /// Install the available app update (or show where to get it).
    Update,
    SyncAll,
    Pause,
    Resume,
    Open,
    Web,
    Quit,
}

impl MenuAction {
    /// The menu item id.
    #[must_use]
    pub fn id(self) -> String {
        match self {
            Self::Folder { folder, action } => {
                let action = match action {
                    FolderAction::Resolve => "resolve",
                    FolderAction::Review => "review",
                    FolderAction::Upload => "upload",
                    FolderAction::Download => "download",
                    FolderAction::Show => "show",
                };
                format!("folder:{folder}:{action}")
            }
            Self::Update => "update".into(),
            Self::SyncAll => "sync_all".into(),
            Self::Pause => "pause".into(),
            Self::Resume => "resume".into(),
            Self::Open => "open".into(),
            Self::Web => "web".into(),
            Self::Quit => "quit".into(),
        }
    }

    /// Reads a menu item id back (`None` for the disabled informational items).
    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "update" => return Some(Self::Update),
            "sync_all" => return Some(Self::SyncAll),
            "pause" => return Some(Self::Pause),
            "resume" => return Some(Self::Resume),
            "open" => return Some(Self::Open),
            "web" => return Some(Self::Web),
            "quit" => return Some(Self::Quit),
            _ => {}
        }
        let rest = id.strip_prefix("folder:")?;
        let (folder, action) = rest.split_once(':')?;
        let action = match action {
            "resolve" => FolderAction::Resolve,
            "review" => FolderAction::Review,
            "upload" => FolderAction::Upload,
            "download" => FolderAction::Download,
            "show" => FolderAction::Show,
            _ => return None,
        };
        Some(Self::Folder {
            folder: folder.parse().ok()?,
            action,
        })
    }
}

/// The inputs of the tray.
#[derive(Debug, Clone, Copy)]
pub struct TrayInput<'a> {
    pub signed_in: bool,
    /// An account is known (its server can be opened on the web).
    pub has_account: bool,
    pub folders: &'a [LinkedFolder],
    /// The end of the global pause as a local `HH:MM` (`None`: not paused).
    pub paused_until: Option<&'a str>,
    /// A newer app version the user can act on.
    pub update: Option<TrayUpdate<'a>>,
    pub lang: Lang,
}

/// An available app update.
#[derive(Debug, Clone, Copy)]
pub struct TrayUpdate<'a> {
    pub version: &'a str,
    /// It installs from the app (else the item only shows where to get it).
    pub in_app: bool,
}

/// One item of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// `None`: a disabled line of information.
    pub action: Option<MenuAction>,
    pub label: String,
    pub enabled: bool,
}

/// The whole tray: icon variant, tooltip and the menu's sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayModel {
    /// Some folder needs the user (the icon gets its dot).
    pub attention: bool,
    pub tooltip: String,
    /// The header line(s) and the folder items.
    pub status: Vec<Item>,
    /// Sync all and pause / resume.
    pub sync: Vec<Item>,
    /// The app update (if any), open the app, open on the web.
    pub open: Vec<Item>,
    pub quit: Item,
}

/// The action a folder's state calls for (`None`: nothing to do, or not known yet).
/// Mirrors the UI's `displayState` (`src/lib/status.ts`): a paused folder only shows
/// the states that need the user anyway.
#[must_use]
pub fn folder_action(link: &LinkedFolder) -> Option<FolderAction> {
    let state = link.last_state?;
    let needs_you = matches!(
        state,
        SyncState::Conflict
            | SyncState::MissingLocalPath
            | SyncState::RemoteMissing
            | SyncState::Error
    );
    if link.paused && !needs_you {
        return None;
    }
    match state {
        SyncState::Conflict => Some(FolderAction::Resolve),
        SyncState::ServerConflict => Some(FolderAction::Review),
        SyncState::LocalChanges => Some(FolderAction::Upload),
        SyncState::RemoteChanges => Some(FolderAction::Download),
        SyncState::MissingLocalPath | SyncState::RemoteMissing | SyncState::Error => {
            Some(FolderAction::Show)
        }
        SyncState::InSync => None,
    }
}

fn info(label: String) -> Item {
    Item {
        action: None,
        label,
        enabled: false,
    }
}

fn item(action: MenuAction, label: String, enabled: bool) -> Item {
    Item {
        action: Some(action),
        label,
        enabled,
    }
}

/// The tray for these inputs.
#[must_use]
pub fn build(input: TrayInput<'_>) -> TrayModel {
    let t = |msg| text(input.lang, msg);
    let pending: Vec<(&LinkedFolder, FolderAction)> = if input.signed_in {
        input
            .folders
            .iter()
            .filter_map(|f| folder_action(f).map(|a| (f, a)))
            .collect()
    } else {
        Vec::new()
    };
    let headline = if !input.signed_in {
        t(Msg::TraySignedOut)
    } else if input.folders.is_empty() {
        t(Msg::TrayNoFolders)
    } else if pending.is_empty() {
        t(Msg::TrayInSync)
    } else {
        t(Msg::TrayAttention(pending.len()))
    };
    let mut status = vec![info(headline.clone())];
    if let Some(time) = input.paused_until {
        status.push(info(t(Msg::TrayPausedUntil(time))));
    }
    for (link, action) in pending.iter().take(MAX_FOLDERS) {
        let verb = t(match action {
            FolderAction::Resolve | FolderAction::Review => Msg::ActionResolve,
            FolderAction::Upload => Msg::ActionUpload,
            FolderAction::Download => Msg::ActionDownload,
            FolderAction::Show => Msg::ActionShow,
        });
        let name = match link.emoji.as_deref() {
            Some(emoji) if !emoji.is_empty() => format!("{emoji} {}", link.folder_name),
            _ => link.folder_name.clone(),
        };
        status.push(item(
            MenuAction::Folder {
                folder: link.id,
                action: *action,
            },
            format!("{name} — {verb}"),
            true,
        ));
    }
    if pending.len() > MAX_FOLDERS {
        status.push(info(t(Msg::TrayMore(pending.len() - MAX_FOLDERS))));
    }
    let can_sync = input.signed_in && input.folders.iter().any(|f| !f.paused);
    let pause = match input.paused_until {
        Some(_) => item(MenuAction::Resume, t(Msg::Resume), true),
        None => item(MenuAction::Pause, t(Msg::PauseHour), true),
    };
    let tooltip = match input.paused_until {
        Some(time) => format!("cloudit — {}", t(Msg::TrayPausedUntil(time))),
        None => format!("cloudit — {headline}"),
    };
    TrayModel {
        attention: !pending.is_empty(),
        tooltip,
        status,
        sync: vec![item(MenuAction::SyncAll, t(Msg::SyncAll), can_sync), pause],
        open: input
            .update
            .map(|u| {
                let label = if u.in_app {
                    t(Msg::UpdateTo(u.version))
                } else {
                    t(Msg::UpdateAvailable(u.version))
                };
                item(MenuAction::Update, label, true)
            })
            .into_iter()
            .chain([
                item(MenuAction::Open, t(Msg::OpenApp), true),
                item(MenuAction::Web, t(Msg::OpenWeb), input.has_account),
            ])
            .collect(),
        quit: item(MenuAction::Quit, t(Msg::Quit), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: i64, name: &str, state: Option<SyncState>, paused: bool) -> LinkedFolder {
        LinkedFolder {
            id,
            local_path: std::path::PathBuf::from(format!("/saves/{id}")),
            scope_id: "s".into(),
            folder_id: format!("f{id}"),
            scope_name: "Games".into(),
            folder_name: name.into(),
            emoji: (id == 1).then(|| "🎮".to_owned()),
            ignore: Vec::new(),
            paused,
            base_snapshot_id: None,
            pending_conflict_id: None,
            last_state: state,
            last_error: None,
            last_sync_at: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            auto_sync: false,
            quiet_secs: 30,
            process_name: None,
        }
    }

    fn input<'a>(folders: &'a [LinkedFolder], paused: Option<&'a str>) -> TrayInput<'a> {
        TrayInput {
            signed_in: true,
            has_account: true,
            folders,
            paused_until: paused,
            update: None,
            lang: Lang::En,
        }
    }

    fn labels(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn folders_needing_action_are_listed_with_their_action() {
        let folders = [
            folder(1, "Elden Ring", Some(SyncState::Conflict), false),
            folder(2, "Hades", Some(SyncState::InSync), false),
            folder(3, "Celeste", Some(SyncState::LocalChanges), false),
            folder(4, "Doom", Some(SyncState::RemoteChanges), false),
            folder(5, "Gone", Some(SyncState::MissingLocalPath), false),
            folder(6, "Shared", Some(SyncState::ServerConflict), false),
            // Paused: only states that need the user anyway.
            folder(7, "Later", Some(SyncState::LocalChanges), true),
            folder(8, "Broken", Some(SyncState::Error), true),
            // Not checked yet.
            folder(9, "New", None, false),
        ];
        let model = build(input(&folders, None));
        assert!(model.attention);
        assert_eq!(
            labels(&model.status),
            vec![
                "6 folders need attention",
                "🎮 Elden Ring — Resolve",
                "Celeste — Upload",
                "Doom — Download",
                "Gone — Show",
                "Shared — Resolve",
                "Broken — Show",
            ]
        );
        assert_eq!(
            model.status[1].action,
            Some(MenuAction::Folder {
                folder: 1,
                action: FolderAction::Resolve
            })
        );
        assert_eq!(
            model.status[5].action,
            Some(MenuAction::Folder {
                folder: 6,
                action: FolderAction::Review
            })
        );
        assert!(!model.status[0].enabled);
        assert_eq!(model.tooltip, "cloudit — 6 folders need attention");
        assert_eq!(labels(&model.sync), vec!["Sync all", "Pause for 1 hour"]);
        assert_eq!(model.quit.action, Some(MenuAction::Quit));
    }

    #[test]
    fn in_sync_paused_signed_out_and_long_lists() {
        let calm = [folder(2, "Hades", Some(SyncState::InSync), false)];
        let model = build(input(&calm, None));
        assert!(!model.attention);
        assert_eq!(labels(&model.status), vec!["Everything is in sync"]);

        let model = build(input(&calm, Some("14:30")));
        assert_eq!(
            labels(&model.status),
            vec!["Everything is in sync", "Sync paused until 14:30"]
        );
        assert_eq!(model.sync[1].action, Some(MenuAction::Resume));
        assert_eq!(model.tooltip, "cloudit — Sync paused until 14:30");

        let model = build(input(&[], None));
        assert_eq!(labels(&model.status), vec!["No folders linked yet"]);
        assert!(!model.sync[0].enabled);

        let busy = [folder(3, "Celeste", Some(SyncState::LocalChanges), false)];
        let model = build(TrayInput {
            signed_in: false,
            has_account: false,
            lang: Lang::It,
            ..input(&busy, None)
        });
        assert_eq!(labels(&model.status), vec!["Accesso non effettuato"]);
        assert!(!model.attention);
        assert!(!model.sync[0].enabled);
        assert!(!model.open[1].enabled);

        let many: Vec<LinkedFolder> = (10..21)
            .map(|id| folder(id, "Game", Some(SyncState::RemoteChanges), false))
            .collect();
        let model = build(input(&many, None));
        assert_eq!(model.status.len(), 1 + MAX_FOLDERS + 1);
        assert_eq!(
            model.status.last().map(|i| i.label.as_str()),
            Some("3 more…")
        );
    }

    #[test]
    fn an_available_update_comes_first_in_the_open_section() {
        let model = build(input(&[], None));
        assert_eq!(labels(&model.open), vec!["Open cloudit", "Open on the web"]);
        let model = build(TrayInput {
            update: Some(TrayUpdate {
                version: "0.3.0",
                in_app: true,
            }),
            ..input(&[], None)
        });
        assert_eq!(model.open[0].label, "Update to 0.3.0");
        assert_eq!(model.open[0].action, Some(MenuAction::Update));
        let model = build(TrayInput {
            update: Some(TrayUpdate {
                version: "0.3.0",
                in_app: false,
            }),
            lang: Lang::It,
            ..input(&[], None)
        });
        assert_eq!(model.open[0].label, "È disponibile la versione 0.3.0");
    }

    #[test]
    fn menu_ids_round_trip() {
        let actions = [
            MenuAction::Folder {
                folder: 42,
                action: FolderAction::Download,
            },
            MenuAction::Folder {
                folder: -1,
                action: FolderAction::Review,
            },
            MenuAction::Update,
            MenuAction::SyncAll,
            MenuAction::Pause,
            MenuAction::Resume,
            MenuAction::Open,
            MenuAction::Web,
            MenuAction::Quit,
        ];
        for action in actions {
            assert_eq!(MenuAction::parse(&action.id()), Some(action));
        }
        assert_eq!(MenuAction::parse("folder:x:show"), None);
        assert_eq!(MenuAction::parse("folder:1:delete"), None);
        assert_eq!(MenuAction::parse("info"), None);
    }
}
