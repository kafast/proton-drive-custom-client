//! Contract-Tests von `RemoteStorage` gegen ein **echtes** Proton-Konto.
//!
//! Standardmäßig ignoriert. Jeder Test arbeitet in einem frischen Unterordner
//! eines eigens dafür angelegten Testordners in „Meine Dateien“; andere Daten
//! werden nicht angefasst. Die Unterordner bleiben liegen und können im
//! Proton-Webclient gelöscht werden.
//!
//! Aufruf (Proton-Events gelten für das ganze Volume, deshalb nacheinander):
//!
//! ```text
//! export PROTONSYNC_LIVE_USERNAME=…        # Konto
//! export PROTONSYNC_LIVE_PASSWORD=…        # Login-Passwort
//! export PROTONSYNC_LIVE_DATA_PASSWORD=…   # nur im Zwei-Passwort-Modus
//! export PROTONSYNC_LIVE_TOTP=123456       # nur mit 2FA, aktueller Code
//! export PROTONSYNC_LIVE_FOLDER=protonsync-live-tests   # optional
//! cargo test -p protonsync-adapter-proton --test live_contract -- --ignored --test-threads=1
//! ```
//!
//! Danach die Variablen wieder löschen (`unset …`); sie gehören nie in Dateien im Repository.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use protonsync_adapter_proton::ProtonRemote;
use protonsync_adapter_proton::auth::{self, PendingLogin, StoredSession};
use protonsync_application::ports::{RemoteError, RemoteStorage};
use protonsync_domain::node::NodeName;

const DEFAULT_TEST_FOLDER: &str = "protonsync-live-tests";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// Meldet sich einmal pro Testlauf an; jeder Test setzt die Sitzung danach fort.
fn session() -> &'static StoredSession {
    static SESSION: OnceLock<StoredSession> = OnceLock::new();
    SESSION.get_or_init(|| {
        // Die Tests laufen schon in einer Tokio-Laufzeit; die Anmeldung braucht
        // eine eigene, deshalb in einem eigenen Thread.
        std::thread::spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(login())
        })
        .join()
        .unwrap()
    })
}

async fn login() -> StoredSession {
    let username = env("PROTONSYNC_LIVE_USERNAME").expect("PROTONSYNC_LIVE_USERNAME fehlt");
    let password = env("PROTONSYNC_LIVE_PASSWORD").expect("PROTONSYNC_LIVE_PASSWORD fehlt");
    let mut login = PendingLogin::start(&username, password.as_bytes())
        .await
        .unwrap();
    if login.needs_second_factor() {
        let code = env("PROTONSYNC_LIVE_TOTP").expect("Konto verlangt PROTONSYNC_LIVE_TOTP");
        login.submit_totp(&code).await.unwrap();
    }
    let data_password = if login.needs_data_password() {
        env("PROTONSYNC_LIVE_DATA_PASSWORD").expect("PROTONSYNC_LIVE_DATA_PASSWORD fehlt")
    } else {
        password
    };
    let (_client, stored) = login.finish(data_password.as_bytes()).await.unwrap();
    stored
}

/// Liefert einen leeren, nur für diesen Test angelegten Ordner als Wurzel.
async fn fresh_remote(test_name: &str) -> ProtonRemote {
    let client = auth::resume_without_store(session()).unwrap();
    let my_files = ProtonRemote::my_files(client.clone()).await.unwrap();
    let my_files_root = my_files.root_folder().await.unwrap();

    let folder_name = env("PROTONSYNC_LIVE_FOLDER").unwrap_or_else(|| DEFAULT_TEST_FOLDER.into());
    let folder_name = NodeName::new(folder_name).unwrap();
    let test_folder = match my_files
        .create_folder(&my_files_root.id, &folder_name)
        .await
    {
        Ok(folder) => folder,
        Err(RemoteError::AlreadyExists(_)) => my_files
            .list_children(&my_files_root.id)
            .await
            .unwrap()
            .into_iter()
            .find(|node| node.name == folder_name)
            .unwrap(),
        Err(error) => panic!("Testordner nicht anlegbar: {error}"),
    };

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let run_name = NodeName::new(format!("{test_name}-{nanos}")).unwrap();
    let run_folder = my_files
        .create_folder(&test_folder.id, &run_name)
        .await
        .unwrap();
    ProtonRemote::with_root(client, &run_folder.id).unwrap()
}

protonsync_testkit::remote_storage_contract_tests!(live: fresh_remote);

/// Nur lesende Prüfungen der Fotobibliothek: Fotos haben keine Ordner, ein
/// schreibender Test würde in der echten Zeitleiste landen (ADR 0012).
mod photos_read_only {
    use protonsync_adapter_proton::ProtonPhotos;
    use protonsync_application::ports::PhotoLibrary;

    use super::{auth, session};

    #[tokio::test]
    #[ignore = "Live-Test: braucht ein echtes Proton-Konto (siehe Testdatei)"]
    async fn library_can_be_listed_and_read() {
        let library = ProtonPhotos::new(auth::resume_without_store(session()).unwrap());
        let (events, _cursor) = library.events_since(None).await.unwrap();
        assert_eq!(events, Vec::new());
        let photos = library.photos().await.unwrap();
        // Ein Konto ohne Fotos ist erlaubt; dann gibt es nichts weiter zu prüfen.
        let Some(smallest) = photos.iter().min_by_key(|photo| photo.size) else {
            return;
        };
        assert_eq!(&library.photo(&smallest.id).await.unwrap(), smallest);
        let mut sink = std::io::sink();
        let written = library.download_to(&smallest.id, &mut sink).await.unwrap();
        assert_eq!(written, smallest.size);
    }
}
