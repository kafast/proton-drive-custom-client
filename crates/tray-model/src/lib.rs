//! Display logic of the tray app `protonsync-tray`, deliberately **without Qt** (ADR 0011):
//! state and labels from the daemon's status and events, notifications,
//! login steps, fixed texts in both languages, plus the app's own settings
//! and autostart. The `gui-qt` crate only shows what is computed here.

pub mod notify;
pub mod session;
pub mod settings;
pub mod texts;
pub mod view;

pub use notify::{
    Notification, NotificationAction, NotificationTopic, NotificationTopics, Severity,
};
pub use session::{Command, Connector, LoginStage, Session, Update};
pub use settings::{LanguageChoice, TrayPaths, TraySettings};
pub use view::{Snapshot, TrayModel, TrayState, TrayView};
