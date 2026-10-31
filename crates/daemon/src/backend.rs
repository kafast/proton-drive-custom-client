//! The chosen Proton connection (ADR 0025): one account type for the daemon
//! that forwards to either the official SDK helper or the community crates.
//!
//! The ports use generic methods (`impl Future`), so they cannot be used as
//! `dyn` trait objects. Instead, small enums forward every call to the
//! variant that was chosen at startup.

use std::io::{Read, Write};

use protonsync_adapter_proton::{ProtonAccount, ProtonPhotos, ProtonRemote};
use protonsync_adapter_sdk_helper::{HelperAccount, HelperRemote, NoPhotos};
use protonsync_application::ports::{
    Account, AccountError, EventCursor, LoginProgress, NewPhoto, PhotoLibrary, RemoteError,
    RemoteEvent, RemoteStorage, SecretStore,
};
use protonsync_domain::node::{Node, NodeId, NodeName};
use protonsync_domain::path::RelativePath;
use protonsync_domain::photo::Photo;
use protonsync_domain::time::Timestamp;

/// Calls the same method on whichever variant `$value` holds.
macro_rules! forward {
    ($value:expr, $kind:ident, $inner:ident => $call:expr) => {
        match $value {
            $kind::Official($inner) => $call,
            $kind::Community($inner) => $call,
        }
    };
}

/// The account of the chosen connection.
// The daemon holds exactly one account, so the size difference of the
// variants does not matter.
#[allow(clippy::large_enum_variant)]
pub enum AnyAccount {
    /// Official SDK through the helper process.
    Official(HelperAccount),
    /// Community crates.
    Community(ProtonAccount<dyn SecretStore>),
}

/// Remote storage of the chosen connection.
pub enum AnyRemote {
    /// Official SDK through the helper process.
    Official(HelperRemote),
    /// Community crates.
    Community(ProtonRemote),
}

/// Photo library of the chosen connection.
pub enum AnyPhotos {
    /// The helper has no photos yet; never constructed in practice.
    Official(NoPhotos),
    /// Community crates.
    Community(ProtonPhotos),
}

impl Account for AnyAccount {
    type Remote = AnyRemote;
    type Photos = AnyPhotos;

    async fn resume(&self) -> Result<Option<String>, AccountError> {
        forward!(self, AnyAccount, a => a.resume().await)
    }

    async fn login_start(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LoginProgress, AccountError> {
        forward!(self, AnyAccount, a => a.login_start(username, password).await)
    }

    async fn login_second_factor(&self, code: &str) -> Result<LoginProgress, AccountError> {
        forward!(self, AnyAccount, a => a.login_second_factor(code).await)
    }

    async fn login_finish(&self, data_password: Option<&str>) -> Result<String, AccountError> {
        forward!(self, AnyAccount, a => a.login_finish(data_password).await)
    }

    async fn browser_login_start(&self) -> Result<String, AccountError> {
        forward!(self, AnyAccount, a => a.browser_login_start().await)
    }

    async fn browser_login_wait(&self) -> Result<Option<String>, AccountError> {
        forward!(self, AnyAccount, a => a.browser_login_wait().await)
    }

    fn uses_browser_login(&self) -> bool {
        forward!(self, AnyAccount, a => a.uses_browser_login())
    }

    async fn logout(&self) -> Result<(), AccountError> {
        forward!(self, AnyAccount, a => a.logout().await)
    }

    async fn username(&self) -> Option<String> {
        forward!(self, AnyAccount, a => a.username().await)
    }

    async fn login_pending(&self) -> bool {
        forward!(self, AnyAccount, a => a.login_pending().await)
    }

    async fn open_remote(&self, remote_path: &RelativePath) -> Result<AnyRemote, AccountError> {
        match self {
            Self::Official(a) => a.open_remote(remote_path).await.map(AnyRemote::Official),
            Self::Community(a) => a.open_remote(remote_path).await.map(AnyRemote::Community),
        }
    }

    async fn open_shared_remote(
        &self,
        remote_path: &RelativePath,
    ) -> Result<AnyRemote, AccountError> {
        match self {
            Self::Official(a) => a
                .open_shared_remote(remote_path)
                .await
                .map(AnyRemote::Official),
            Self::Community(a) => a
                .open_shared_remote(remote_path)
                .await
                .map(AnyRemote::Community),
        }
    }

    async fn open_photos(&self) -> Result<AnyPhotos, AccountError> {
        match self {
            Self::Official(a) => a.open_photos().await.map(AnyPhotos::Official),
            Self::Community(a) => a.open_photos().await.map(AnyPhotos::Community),
        }
    }
}

impl RemoteStorage for AnyRemote {
    async fn root_folder(&self) -> Result<Node, RemoteError> {
        forward!(self, AnyRemote, r => r.root_folder().await)
    }

    async fn node(&self, id: &NodeId) -> Result<Node, RemoteError> {
        forward!(self, AnyRemote, r => r.node(id).await)
    }

    async fn list_children(&self, folder: &NodeId) -> Result<Vec<Node>, RemoteError> {
        forward!(self, AnyRemote, r => r.list_children(folder).await)
    }

    async fn create_folder(&self, parent: &NodeId, name: &NodeName) -> Result<Node, RemoteError> {
        forward!(self, AnyRemote, r => r.create_folder(parent, name).await)
    }

    async fn upload_file<R: Read + Send>(
        &self,
        parent: &NodeId,
        name: &NodeName,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        forward!(self, AnyRemote, r => r.upload_file(parent, name, content, size, modified).await)
    }

    async fn upload_revision<R: Read + Send>(
        &self,
        file: &NodeId,
        content: R,
        size: u64,
        modified: Timestamp,
    ) -> Result<Node, RemoteError> {
        forward!(self, AnyRemote, r => r.upload_revision(file, content, size, modified).await)
    }

    async fn download_to<W: Write + Send>(
        &self,
        file: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        forward!(self, AnyRemote, r => r.download_to(file, target).await)
    }

    async fn download_range(
        &self,
        file: &NodeId,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, RemoteError> {
        forward!(self, AnyRemote, r => r.download_range(file, offset, length).await)
    }

    async fn rename(&self, id: &NodeId, new_name: &NodeName) -> Result<Node, RemoteError> {
        forward!(self, AnyRemote, r => r.rename(id, new_name).await)
    }

    async fn move_node(&self, id: &NodeId, new_parent: &NodeId) -> Result<Node, RemoteError> {
        forward!(self, AnyRemote, r => r.move_node(id, new_parent).await)
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        forward!(self, AnyRemote, r => r.trash(id).await)
    }

    async fn restore(&self, id: &NodeId) -> Result<Node, RemoteError> {
        forward!(self, AnyRemote, r => r.restore(id).await)
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        forward!(self, AnyRemote, r => r.delete_permanently(id).await)
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        forward!(self, AnyRemote, r => r.events_since(cursor).await)
    }
}

impl PhotoLibrary for AnyPhotos {
    async fn photos(&self) -> Result<Vec<Photo>, RemoteError> {
        forward!(self, AnyPhotos, p => p.photos().await)
    }

    async fn photo(&self, id: &NodeId) -> Result<Photo, RemoteError> {
        forward!(self, AnyPhotos, p => p.photo(id).await)
    }

    async fn events_since(
        &self,
        cursor: Option<&EventCursor>,
    ) -> Result<(Vec<RemoteEvent>, EventCursor), RemoteError> {
        forward!(self, AnyPhotos, p => p.events_since(cursor).await)
    }

    async fn download_to<W: Write + Send>(
        &self,
        id: &NodeId,
        target: &mut W,
    ) -> Result<u64, RemoteError> {
        forward!(self, AnyPhotos, p => p.download_to(id, target).await)
    }

    async fn upload<R: Read + Send>(
        &self,
        photo: &NewPhoto,
        content: R,
    ) -> Result<Photo, RemoteError> {
        forward!(self, AnyPhotos, p => p.upload(photo, content).await)
    }

    async fn trash(&self, id: &NodeId) -> Result<(), RemoteError> {
        forward!(self, AnyPhotos, p => p.trash(id).await)
    }

    async fn restore(&self, id: &NodeId) -> Result<(), RemoteError> {
        forward!(self, AnyPhotos, p => p.restore(id).await)
    }

    async fn delete_permanently(&self, id: &NodeId) -> Result<(), RemoteError> {
        forward!(self, AnyPhotos, p => p.delete_permanently(id).await)
    }
}
