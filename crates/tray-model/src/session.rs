//! Flow between UI and daemon: commands from the UI become requests,
//! responses and events become [`Update`]s for the UI.
//!
//! The Qt side calls [`Session::execute`] in its own thread and
//! [`apply_event`] in the thread of the event subscription; both share the
//! [`TrayModel`] through a `Mutex` that is never held during a request.

use std::sync::{Arc, Mutex, MutexGuard};

use protonsync_api::{ApiError, Event, LoginStep, Password, Request, ResolutionChoice, Response};
use protonsync_client::{ClientError, Lang, Transport};
use serde::Serialize;

use crate::notify::{Notification, notification_for};
use crate::view::{Snapshot, TrayModel, TrayView};

/// A command from the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Query the current state again.
    Refresh,
    /// Sync all pairs; only plan when the daemon is in dry-run mode.
    SyncNow,
    /// Decide a conflict.
    Resolve {
        /// Sync pair.
        pair: String,
        /// ID of the conflict.
        id: i64,
        /// Decision.
        choice: ResolutionChoice,
    },
    /// Start the login.
    LoginStart {
        /// Username or email.
        username: String,
        /// Password.
        password: Password,
    },
    /// Submit the 2FA code.
    LoginCode(String),
    /// Submit the mailbox password.
    LoginDataPassword(Password),
    /// Log out.
    Logout,
}

/// Where the login currently stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginStage {
    /// Enter username and password.
    Credentials,
    /// Enter the 2FA code.
    Code,
    /// Enter the mailbox password.
    DataPassword,
    /// Logged in; close the dialog.
    Done,
}

/// What the UI should show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// New view.
    View(TrayView),
    /// New login state, possibly with an error message.
    Login {
        /// Next step.
        stage: LoginStage,
        /// Why the last step failed.
        error: Option<String>,
    },
    /// A message for the status line (e.g. an error response).
    Message(String),
    /// A desktop notification.
    Notify(Notification),
}

/// Opens connections to the daemon (a scripted daemon in tests).
pub trait Connector: Send {
    /// New connection.
    ///
    /// # Errors
    /// [`ClientError::Unreachable`] if no daemon is running.
    fn connect(&self) -> Result<Box<dyn Transport + Send>, ClientError>;
}

/// Locks the model. A thread that crashed while holding the lock still
/// leaves it in a usable state (display data only).
fn lock(model: &Mutex<TrayModel>) -> MutexGuard<'_, TrayModel> {
    model
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Applies a daemon event and returns what changes.
/// With `notify = false` only the view is updated; topics switched off in the
/// settings never notify.
pub fn apply_event(model: &Mutex<TrayModel>, event: &Event, notify: bool) -> Vec<Update> {
    let mut model = lock(model);
    model.apply_event(event);
    let mut updates = vec![Update::View(model.view())];
    if notify
        && let Some(notification) = notification_for(event, model.lang())
        && model.notification_topics().shows(notification.topic)
    {
        updates.push(Update::Notify(notification));
    }
    updates
}

/// The daemon is not (or no longer) reachable.
pub fn mark_offline(model: &Mutex<TrayModel>) -> Update {
    let mut model = lock(model);
    model.mark_offline();
    Update::View(model.view())
}

/// Command execution with a reused connection.
pub struct Session<C: Connector> {
    connector: C,
    transport: Option<Box<dyn Transport + Send>>,
    model: Arc<Mutex<TrayModel>>,
}

impl<C: Connector> Session<C> {
    /// New session; it connects only on the first command.
    pub fn new(connector: C, model: Arc<Mutex<TrayModel>>) -> Self {
        Self {
            connector,
            transport: None,
            model,
        }
    }

    /// Executes a command. If the connection fails, the daemon counts as
    /// unreachable; the next command reconnects.
    pub fn execute(&mut self, command: Command) -> Vec<Update> {
        match self.try_execute(command) {
            Ok(updates) => updates,
            Err(error) => {
                self.transport = None;
                vec![
                    mark_offline(&self.model),
                    Update::Message(error.to_string()),
                ]
            }
        }
    }

    fn try_execute(&mut self, command: Command) -> Result<Vec<Update>, ClientError> {
        let lang = lock(&self.model).lang();
        let mut updates = Vec::new();
        match command {
            Command::Refresh => {}
            Command::SyncNow => {
                let apply = !lock(&self.model).view().dry_run;
                let response = self.request(Request::Sync { pair: None, apply })?;
                updates.extend(error_message(&response, lang));
            }
            Command::Resolve { pair, id, choice } => {
                let response = self.request(Request::ResolveConflict {
                    pair: pair.clone(),
                    id,
                    resolution: choice,
                })?;
                if response == Response::Done {
                    lock(&self.model).forget_conflict(&pair, id);
                }
                updates.extend(error_message(&response, lang));
            }
            Command::LoginStart { username, password } => {
                let response = self.request(Request::LoginStart { username, password })?;
                updates.push(self.next_login_step(response, lang)?);
            }
            Command::LoginCode(code) => {
                let response = self.request(Request::LoginSecondFactor { code })?;
                updates.push(self.next_login_step(response, lang)?);
            }
            Command::LoginDataPassword(password) => {
                let response = self.request(Request::LoginFinish {
                    data_password: Some(password),
                })?;
                updates.push(login_result(response, lang));
            }
            Command::Logout => {
                let response = self.request(Request::Logout)?;
                updates.extend(error_message(&response, lang));
            }
        }
        // Fetch the state again after every command so the view is correct.
        let snapshot = Snapshot::fetch(self.transport()?)?;
        let mut model = lock(&self.model);
        model.set_snapshot(snapshot);
        updates.insert(0, Update::View(model.view()));
        Ok(updates)
    }

    /// After `LoginStart` or the 2FA code: either the next step or, if
    /// nothing is missing any more, the finish right away.
    fn next_login_step(&mut self, response: Response, lang: Lang) -> Result<Update, ClientError> {
        match response {
            Response::LoginStep(LoginStep {
                needs_second_factor: true,
                ..
            }) => Ok(login_stage(LoginStage::Code)),
            Response::LoginStep(LoginStep {
                needs_data_password: true,
                ..
            }) => Ok(login_stage(LoginStage::DataPassword)),
            Response::LoginStep(_) => {
                let finished = self.request(Request::LoginFinish {
                    data_password: None,
                })?;
                Ok(login_result(finished, lang))
            }
            other => Ok(login_failed(&other, lang)),
        }
    }

    fn transport(&mut self) -> Result<&mut (dyn Transport + Send + 'static), ClientError> {
        if self.transport.is_none() {
            self.transport = Some(self.connector.connect()?);
        }
        self.transport
            .as_deref_mut()
            .ok_or_else(|| ClientError::Io("no connection".into()))
    }

    fn request(&mut self, request: Request) -> Result<Response, ClientError> {
        self.transport()?.request(request)
    }
}

fn login_stage(stage: LoginStage) -> Update {
    Update::Login { stage, error: None }
}

fn login_result(response: Response, lang: Lang) -> Update {
    match response {
        Response::LoggedIn { .. } => login_stage(LoginStage::Done),
        other => login_failed(&other, lang),
    }
}

/// Every error starts the login over; the daemon then also discards the
/// half-finished attempt.
fn login_failed(response: &Response, lang: Lang) -> Update {
    Update::Login {
        stage: LoginStage::Credentials,
        error: Some(match response {
            Response::Error(error) => describe(error, lang),
            other => format!("{other:?}"),
        }),
    }
}

fn error_message(response: &Response, lang: Lang) -> Option<Update> {
    match response {
        Response::Error(error) => Some(Update::Message(describe(error, lang))),
        _ => None,
    }
}

fn describe(error: &ApiError, lang: Lang) -> String {
    format!("{}: {}", lang.pick("Fehler", "Error"), error.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    use jiff::tz::TimeZone;
    use protonsync_api::{ConflictInfo, ConflictKindInfo, ErrorCode, StatusInfo};

    /// A daemon that returns preset responses in order and records all
    /// requests.
    #[derive(Clone, Default)]
    struct Script {
        answers: Arc<Mutex<VecDeque<Result<Response, ClientError>>>>,
        requests: Arc<Mutex<Vec<Request>>>,
    }

    impl Script {
        fn answer(&self, response: Response) -> &Self {
            self.answers.lock().unwrap().push_back(Ok(response));
            self
        }

        fn fail(&self) -> &Self {
            self.answers
                .lock()
                .unwrap()
                .push_back(Err(ClientError::Io("gone".into())));
            self
        }

        /// The three responses of a query (status, conflicts, mounts).
        fn snapshot(&self, account: Option<&str>, conflicts: Vec<ConflictInfo>) -> &Self {
            self.answer(Response::Status(status(account)))
                .answer(Response::Conflicts { conflicts })
                .answer(Response::Mounts { mounts: vec![] })
        }

        fn requests(&self) -> Vec<Request> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Transport for Script {
        fn request(&mut self, request: Request) -> Result<Response, ClientError> {
            self.requests.lock().unwrap().push(request);
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(Response::Done))
        }
    }

    impl Connector for Script {
        fn connect(&self) -> Result<Box<dyn Transport + Send>, ClientError> {
            Ok(Box::new(self.clone()))
        }
    }

    fn status(account: Option<&str>) -> StatusInfo {
        StatusInfo {
            daemon_version: "0.1.0".into(),
            account: account.map(Into::into),
            login_pending: false,
            dry_run: false,
            pairs: vec![],
        }
    }

    fn conflict(id: i64) -> ConflictInfo {
        ConflictInfo {
            pair: "docs".into(),
            id,
            path: "a.txt".into(),
            kind: ConflictKindInfo::BothModified,
            conflict_copy: None,
            detected_at: 0,
        }
    }

    fn session(script: &Script) -> Session<Script> {
        let model = Arc::new(Mutex::new(TrayModel::new(Lang::En, TimeZone::UTC)));
        Session::new(script.clone(), model)
    }

    fn view(updates: &[Update]) -> &TrayView {
        match &updates[0] {
            Update::View(view) => view,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn refresh_fetches_status_conflicts_and_mounts() {
        let script = Script::default();
        script.snapshot(Some("k@pm.me"), vec![conflict(1)]);
        let updates = session(&script).execute(Command::Refresh);
        assert_eq!(view(&updates).account.as_deref(), Some("k@pm.me"));
        assert_eq!(view(&updates).conflicts.len(), 1);
        assert_eq!(
            script.requests(),
            [Request::Status, Request::Conflicts, Request::Mounts]
        );
    }

    #[test]
    fn sync_now_respects_dry_run() {
        let script = Script::default();
        let mut session = session(&script);
        script
            .answer(Response::Status(StatusInfo {
                dry_run: true,
                ..status(Some("k"))
            }))
            .answer(Response::Conflicts { conflicts: vec![] })
            .answer(Response::Mounts { mounts: vec![] });
        session.execute(Command::Refresh);
        session.execute(Command::SyncNow);
        assert_eq!(
            script.requests()[3],
            Request::Sync {
                pair: None,
                apply: false
            }
        );
    }

    #[test]
    fn resolving_removes_the_conflict_and_reports_errors() {
        let script = Script::default();
        let mut session = session(&script);
        script.snapshot(Some("k"), vec![conflict(1), conflict(2)]);
        session.execute(Command::Refresh);

        script
            .answer(Response::Done)
            .snapshot(Some("k"), vec![conflict(2)]);
        let updates = session.execute(Command::Resolve {
            pair: "docs".into(),
            id: 1,
            choice: ResolutionChoice::KeepBoth,
        });
        assert_eq!(view(&updates).conflicts.len(), 1);

        script
            .answer(Response::Error(ApiError::new(ErrorCode::NotFound, "gone")))
            .snapshot(Some("k"), vec![conflict(2)]);
        let updates = session.execute(Command::Resolve {
            pair: "docs".into(),
            id: 9,
            choice: ResolutionChoice::KeepLocal,
        });
        assert_eq!(updates[1], Update::Message("Error: gone".into()));
    }

    #[test]
    fn login_walks_through_code_and_data_password() {
        let script = Script::default();
        let mut session = session(&script);
        script
            .answer(Response::LoginStep(LoginStep {
                needs_second_factor: true,
                needs_data_password: true,
            }))
            .snapshot(None, vec![]);
        let updates = session.execute(Command::LoginStart {
            username: "k".into(),
            password: Password::new("secret"),
        });
        assert_eq!(updates[1], login_stage(LoginStage::Code));

        script
            .answer(Response::LoginStep(LoginStep {
                needs_second_factor: false,
                needs_data_password: true,
            }))
            .snapshot(None, vec![]);
        let updates = session.execute(Command::LoginCode("123456".into()));
        assert_eq!(updates[1], login_stage(LoginStage::DataPassword));

        script
            .answer(Response::LoggedIn {
                username: "k".into(),
            })
            .snapshot(Some("k"), vec![]);
        let updates = session.execute(Command::LoginDataPassword(Password::new("mailbox")));
        assert_eq!(updates[1], login_stage(LoginStage::Done));
        assert_eq!(view(&updates).account.as_deref(), Some("k"));
    }

    #[test]
    fn login_without_further_steps_finishes_at_once() {
        let script = Script::default();
        script
            .answer(Response::LoginStep(LoginStep {
                needs_second_factor: false,
                needs_data_password: false,
            }))
            .answer(Response::LoggedIn {
                username: "k".into(),
            })
            .snapshot(Some("k"), vec![]);
        let updates = session(&script).execute(Command::LoginStart {
            username: "k".into(),
            password: Password::new("secret"),
        });
        assert_eq!(updates[1], login_stage(LoginStage::Done));
        assert_eq!(
            script.requests()[1],
            Request::LoginFinish {
                data_password: None
            }
        );
    }

    #[test]
    fn a_rejected_login_starts_over() {
        let script = Script::default();
        script
            .answer(Response::Error(ApiError::new(
                ErrorCode::LoginRejected,
                "wrong code",
            )))
            .snapshot(None, vec![]);
        let updates = session(&script).execute(Command::LoginCode("000000".into()));
        assert_eq!(
            updates[1],
            Update::Login {
                stage: LoginStage::Credentials,
                error: Some("Error: wrong code".into())
            }
        );
    }

    #[test]
    fn a_lost_connection_marks_the_daemon_offline_and_reconnects() {
        let script = Script::default();
        let mut session = session(&script);
        script.snapshot(Some("k"), vec![]);
        session.execute(Command::Refresh);

        script.fail();
        let updates = session.execute(Command::Logout);
        assert!(!view(&updates).daemon_online);
        assert!(matches!(updates[1], Update::Message(_)));

        script.snapshot(Some("k"), vec![]);
        let updates = session.execute(Command::Refresh);
        assert!(view(&updates).daemon_online);
    }

    #[test]
    fn events_update_the_view_and_notify_when_wanted() {
        let model = Mutex::new(TrayModel::new(Lang::De, TimeZone::UTC));
        lock(&model).set_snapshot(Snapshot {
            status: status(Some("k")),
            conflicts: vec![],
            mounts: vec![],
        });
        let event = Event::ConflictDetected(conflict(5));
        let updates = apply_event(&model, &event, true);
        assert_eq!(view(&updates).conflicts.len(), 1);
        assert!(matches!(&updates[1], Update::Notify(n) if n.title == "Konflikt"));
        assert_eq!(apply_event(&model, &event, false).len(), 1);
        // Conflicts reported twice appear only once.
        assert_eq!(view(&apply_event(&model, &event, false)).conflicts.len(), 1);
    }

    #[test]
    fn switched_off_topics_do_not_notify() {
        let topics = crate::notify::NotificationTopics {
            conflicts: false,
            ..crate::notify::NotificationTopics::default()
        };
        let model =
            Mutex::new(TrayModel::new(Lang::De, TimeZone::UTC).with_notification_topics(topics));
        let updates = apply_event(&model, &Event::ConflictDetected(conflict(5)), true);
        assert_eq!(updates.len(), 1, "only the view");
    }
}
