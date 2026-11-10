//! The few texts the backend shows itself (tray menu, tooltip, notifications), in the
//! UI's languages. The web view has its own dictionaries (`packages/ui`); it tells the
//! backend its language with the `set_ui_language` command, at startup and on every
//! switch. Until then (and for unknown codes) the language is English.

/// A language of the backend texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    It,
}

impl Lang {
    /// The UI's language code (`en`, `it`); `None` for any other.
    #[must_use]
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Self::En),
            "it" => Some(Self::It),
            _ => None,
        }
    }
}

/// A text, with its parameters.
#[derive(Debug, Clone, Copy)]
pub enum Msg<'a> {
    TraySignedOut,
    TrayNoFolders,
    TrayInSync,
    /// `n` folders need attention.
    TrayAttention(usize),
    /// Paused until a local `HH:MM`.
    TrayPausedUntil(&'a str),
    /// `n` more folders not listed.
    TrayMore(usize),
    ActionResolve,
    ActionUpload,
    ActionDownload,
    ActionShow,
    SyncAll,
    PauseHour,
    Resume,
    OpenApp,
    /// Install the named app version.
    UpdateTo(&'a str),
    /// The named app version is out (installed elsewhere: the release page).
    UpdateAvailable(&'a str),
    OpenWeb,
    Quit,
    /// A conflict this device must decide, in the named folder.
    ConflictTitle(&'a str),
    ConflictBody,
    ServerConflictBody,
    /// Automatic sync of the named folder failed (first of a streak).
    SyncFailedTitle(&'a str),
    SyncFailedBody,
    SignInTitle,
    SignInBody,
    TrayNoticeTitle,
    TrayNoticeBody,
}

/// The text of `msg` in `lang`.
#[must_use]
pub fn text(lang: Lang, msg: Msg<'_>) -> String {
    match lang {
        Lang::En => en(msg),
        Lang::It => it(msg),
    }
}

fn en(msg: Msg<'_>) -> String {
    match msg {
        Msg::TraySignedOut => "Not signed in".into(),
        Msg::TrayNoFolders => "No folders linked yet".into(),
        Msg::TrayInSync => "Everything is in sync".into(),
        Msg::TrayAttention(1) => "1 folder needs attention".into(),
        Msg::TrayAttention(n) => format!("{n} folders need attention"),
        Msg::TrayPausedUntil(time) => format!("Sync paused until {time}"),
        Msg::TrayMore(n) => format!("{n} more…"),
        Msg::ActionResolve => "Resolve".into(),
        Msg::ActionUpload => "Upload".into(),
        Msg::ActionDownload => "Download".into(),
        Msg::ActionShow => "Show".into(),
        Msg::SyncAll => "Sync all".into(),
        Msg::PauseHour => "Pause for 1 hour".into(),
        Msg::Resume => "Resume".into(),
        Msg::OpenApp => "Open cloudit".into(),
        Msg::UpdateTo(version) => format!("Update to {version}"),
        Msg::UpdateAvailable(version) => format!("Version {version} is available"),
        Msg::OpenWeb => "Open on the web".into(),
        Msg::Quit => "Quit".into(),
        Msg::ConflictTitle(folder) => format!("Conflict in {folder}"),
        Msg::ConflictBody => {
            "This computer and the server both changed it: open cloudit to choose the version to keep.".into()
        }
        Msg::ServerConflictBody => {
            "Another device left a conflict on the server: check the history on the web.".into()
        }
        Msg::SyncFailedTitle(folder) => format!("Couldn't sync {folder}"),
        Msg::SyncFailedBody => "cloudit will try again automatically.".into(),
        Msg::SignInTitle => "Sign in again".into(),
        Msg::SignInBody => {
            "The server no longer accepts this computer: open cloudit to sign in.".into()
        }
        Msg::TrayNoticeTitle => "cloudit keeps running".into(),
        Msg::TrayNoticeBody => {
            "cloudit keeps running in the tray. To close it, choose Quit from its menu.".into()
        }
    }
}

fn it(msg: Msg<'_>) -> String {
    match msg {
        Msg::TraySignedOut => "Accesso non effettuato".into(),
        Msg::TrayNoFolders => "Nessuna cartella collegata".into(),
        Msg::TrayInSync => "Tutto sincronizzato".into(),
        Msg::TrayAttention(1) => "1 cartella richiede attenzione".into(),
        Msg::TrayAttention(n) => format!("{n} cartelle richiedono attenzione"),
        Msg::TrayPausedUntil(time) => format!("Sincronizzazione in pausa fino alle {time}"),
        Msg::TrayMore(n) => format!("altre {n}…"),
        Msg::ActionResolve => "Risolvi".into(),
        Msg::ActionUpload => "Carica".into(),
        Msg::ActionDownload => "Scarica".into(),
        Msg::ActionShow => "Mostra".into(),
        Msg::SyncAll => "Sincronizza tutto".into(),
        Msg::PauseHour => "Metti in pausa per 1 ora".into(),
        Msg::Resume => "Riprendi".into(),
        Msg::OpenApp => "Apri cloudit".into(),
        Msg::UpdateTo(version) => format!("Aggiorna alla {version}"),
        Msg::UpdateAvailable(version) => format!("È disponibile la versione {version}"),
        Msg::OpenWeb => "Apri sul web".into(),
        Msg::Quit => "Esci".into(),
        Msg::ConflictTitle(folder) => format!("Conflitto in {folder}"),
        Msg::ConflictBody => {
            "Questo PC e il server l'hanno modificata entrambi: apri cloudit per scegliere la versione da tenere.".into()
        }
        Msg::ServerConflictBody => {
            "Un altro dispositivo ha lasciato un conflitto sul server: controlla la cronologia sul web.".into()
        }
        Msg::SyncFailedTitle(folder) => format!("Impossibile sincronizzare {folder}"),
        Msg::SyncFailedBody => "cloudit riproverà automaticamente.".into(),
        Msg::SignInTitle => "Accedi di nuovo".into(),
        Msg::SignInBody => {
            "Il server non accetta più questo PC: apri cloudit per accedere.".into()
        }
        Msg::TrayNoticeTitle => "cloudit resta attivo".into(),
        Msg::TrayNoticeBody => {
            "cloudit resta attivo nell'area di notifica. Per chiuderlo scegli Esci dal suo menu.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_plurals() {
        assert_eq!(Lang::parse("it"), Some(Lang::It));
        assert_eq!(Lang::parse("fr"), None);
        assert_eq!(
            text(Lang::En, Msg::TrayAttention(1)),
            "1 folder needs attention"
        );
        assert_eq!(
            text(Lang::It, Msg::TrayAttention(3)),
            "3 cartelle richiedono attenzione"
        );
        assert_eq!(
            text(Lang::It, Msg::ConflictTitle("Saves")),
            "Conflitto in Saves"
        );
    }
}
