//! Anzeigelogik der Tray-App `protonsync-tray`, bewusst **ohne Qt** (ADR 0011):
//! Zustand und Beschriftungen aus Status und Ereignissen des Daemons,
//! Benachrichtigungen, Anmeldeschritte, feste Texte in beiden Sprachen sowie
//! die eigenen Einstellungen und der Autostart. Das Crate `gui-qt` zeigt
//! davon nur an, was hier berechnet wird.

pub mod notify;
pub mod session;
pub mod settings;
pub mod texts;
pub mod view;

pub use notify::{Notification, NotificationAction, Severity};
pub use session::{Command, Connector, LoginStage, Session, Update};
pub use settings::{LanguageChoice, TrayPaths, TraySettings};
pub use view::{Snapshot, TrayModel, TrayState, TrayView};
