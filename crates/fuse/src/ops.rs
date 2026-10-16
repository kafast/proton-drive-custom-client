//! Übersetzung zwischen Kernel-Anfragen und dem virtuellen Laufwerk.
//!
//! Alles hier ist ohne eingehängtes Dateisystem testbar: Die Methoden nehmen
//! die Werte, die der Kernel schickt, und liefern Antworten oder einen
//! Fehlercode ([`Errno`]).

use std::ffi::OsStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{Errno, FileAttr, FileType, INodeNo, OpenAccMode, OpenFlags, RenameFlags};
use protonsync_application::drive::{Attributes, DriveError, Ino, VirtualDrive};
use protonsync_application::ports::{Clock, ContentCache, DriveRepo, RemoteStorage, StateRepo};
use protonsync_domain::node::NodeName;
use protonsync_domain::time::Timestamp;

use crate::errno::errno_of;

/// Bevorzugte Blockgröße für `stat()`.
const PREFERRED_IO_BYTES: u32 = 4096;
/// `stat()` zählt belegten Platz in Einheiten von 512 Bytes.
const STAT_BLOCK_BYTES: u64 = 512;
const FOLDER_PERMISSIONS: u16 = 0o755;
const FILE_PERMISSIONS: u16 = 0o644;
/// Ordner haben (wie üblich) zwei Verweise: ihren Namen und `.`.
const FOLDER_LINKS: u32 = 2;
const FILE_LINKS: u32 = 1;

/// Besitzer aller Einträge (der Benutzer, der das Laufwerk einhängt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    /// Benutzer-ID.
    pub uid: u32,
    /// Gruppen-ID.
    pub gid: u32,
}

/// Ein Eintrag beim Auflisten eines Ordners, wie der Kernel ihn braucht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// Inode-Nummer.
    pub ino: INodeNo,
    /// Ordner oder Datei.
    pub kind: FileType,
    /// Name (inklusive `.` und `..`).
    pub name: String,
}

/// Übersetzt Kernel-Anfragen in Aufrufe des Laufwerks.
pub struct DriveOps<R, C, D, S, K> {
    drive: Arc<VirtualDrive<R, C, D, S, K>>,
    owner: Owner,
}

/// Zeitstempel als `SystemTime` (auch vor 1970, falls Proton so etwas liefert).
fn system_time(timestamp: Timestamp) -> SystemTime {
    let seconds = timestamp.unix_seconds();
    let offset = Duration::from_secs(seconds.unsigned_abs());
    if seconds >= 0 {
        UNIX_EPOCH + offset
    } else {
        UNIX_EPOCH - offset
    }
}

/// Wandelt einen Namen vom Kernel in einen gültigen Proton-Namen.
fn node_name(name: &OsStr) -> Result<NodeName, Errno> {
    let text = name.to_str().ok_or(Errno::EINVAL)?;
    NodeName::new(text).map_err(|_| Errno::EINVAL)
}

fn ino(value: INodeNo) -> Ino {
    value.0
}

fn errno(error: &DriveError) -> Errno {
    errno_of(error)
}

fn wants_write(flags: OpenFlags) -> bool {
    !matches!(flags.acc_mode(), OpenAccMode::O_RDONLY)
}

const fn wants_truncate(flags: OpenFlags) -> bool {
    flags.0 & libc::O_TRUNC != 0
}

impl<R, C, D, S, K> DriveOps<R, C, D, S, K>
where
    R: RemoteStorage,
    C: ContentCache,
    D: DriveRepo,
    S: StateRepo,
    K: Clock,
{
    /// Übersetzer für `drive`; alle Einträge gehören `owner`.
    #[must_use]
    pub const fn new(drive: Arc<VirtualDrive<R, C, D, S, K>>, owner: Owner) -> Self {
        Self { drive, owner }
    }

    /// Das Laufwerk.
    #[must_use]
    pub fn drive(&self) -> &Arc<VirtualDrive<R, C, D, S, K>> {
        &self.drive
    }

    /// Attribute im Format des Kernels.
    #[must_use]
    pub fn file_attr(&self, attributes: Attributes) -> FileAttr {
        let modified = system_time(attributes.modified);
        let (kind, perm, nlink) = if attributes.is_folder {
            (FileType::Directory, FOLDER_PERMISSIONS, FOLDER_LINKS)
        } else {
            (FileType::RegularFile, FILE_PERMISSIONS, FILE_LINKS)
        };
        FileAttr {
            ino: INodeNo(attributes.ino),
            size: attributes.size,
            blocks: attributes.size.div_ceil(STAT_BLOCK_BYTES),
            atime: modified,
            mtime: modified,
            ctime: modified,
            crtime: modified,
            kind,
            perm,
            nlink,
            uid: self.owner.uid,
            gid: self.owner.gid,
            rdev: 0,
            blksize: PREFERRED_IO_BYTES,
            flags: 0,
        }
    }

    /// `stat()` auf einen bekannten Eintrag.
    ///
    /// # Errors
    /// `ENOENT`, wenn die Nummer unbekannt ist.
    pub fn getattr(&self, inode: INodeNo) -> Result<FileAttr, Errno> {
        let attributes = self.drive.attributes(ino(inode)).map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Sucht `name` in `parent`.
    ///
    /// # Errors
    /// `ENOENT`, `ENOTDIR`, `EINVAL` (ungültiger Name) oder Fehler bei Proton.
    pub async fn lookup(&self, parent: INodeNo, name: &OsStr) -> Result<FileAttr, Errno> {
        let name = node_name(name)?;
        let attributes = self
            .drive
            .lookup(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Inhalt eines Ordners samt `.` und `..`.
    ///
    /// # Errors
    /// `ENOENT`, `ENOTDIR` oder Fehler bei Proton.
    pub async fn list(&self, folder: INodeNo) -> Result<Vec<Listing>, Errno> {
        let entries = self.drive.list(ino(folder)).await.map_err(|e| errno(&e))?;
        // `..` zeigt hier auf den Ordner selbst; der Kernel ermittelt den
        // echten Elternordner ohnehin selbst.
        let mut listing = vec![
            Listing {
                ino: folder,
                kind: FileType::Directory,
                name: ".".into(),
            },
            Listing {
                ino: folder,
                kind: FileType::Directory,
                name: "..".into(),
            },
        ];
        listing.extend(entries.into_iter().map(|entry| Listing {
            ino: INodeNo(entry.ino),
            kind: if entry.is_folder {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            name: entry.name.to_string(),
        }));
        Ok(listing)
    }

    /// Legt einen Ordner an.
    ///
    /// # Errors
    /// `EEXIST`, `EINVAL` oder Fehler bei Proton.
    pub async fn mkdir(&self, parent: INodeNo, name: &OsStr) -> Result<FileAttr, Errno> {
        let name = node_name(name)?;
        let attributes = self
            .drive
            .make_folder(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Löscht eine Datei (in den Papierkorb).
    ///
    /// # Errors
    /// `ENOENT`, `EISDIR` oder Fehler bei Proton.
    pub async fn unlink(&self, parent: INodeNo, name: &OsStr) -> Result<(), Errno> {
        let name = node_name(name)?;
        self.drive
            .remove_file(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))
    }

    /// Löscht einen leeren Ordner (in den Papierkorb).
    ///
    /// # Errors
    /// `ENOENT`, `ENOTEMPTY`, `ENOTDIR` oder Fehler bei Proton.
    pub async fn rmdir(&self, parent: INodeNo, name: &OsStr) -> Result<(), Errno> {
        let name = node_name(name)?;
        self.drive
            .remove_folder(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))
    }

    /// Benennt um oder verschiebt. `RENAME_NOREPLACE` wird beachtet,
    /// `RENAME_EXCHANGE` (Tauschen) gibt es bei Proton nicht.
    ///
    /// # Errors
    /// `EEXIST` (bei `RENAME_NOREPLACE`), `EINVAL` oder die Fehler von [`VirtualDrive::rename`].
    pub async fn rename(
        &self,
        parent: INodeNo,
        name: &OsStr,
        new_parent: INodeNo,
        new_name: &OsStr,
        flags: RenameFlags,
    ) -> Result<(), Errno> {
        let name = node_name(name)?;
        let new_name = node_name(new_name)?;
        if flags.intersects(RenameFlags::RENAME_EXCHANGE | RenameFlags::RENAME_WHITEOUT) {
            return Err(Errno::EINVAL);
        }
        if flags.contains(RenameFlags::RENAME_NOREPLACE) {
            match self.drive.lookup(ino(new_parent), &new_name).await {
                Ok(_) => return Err(Errno::EEXIST),
                Err(DriveError::NotFound) => {}
                Err(error) => return Err(errno(&error)),
            }
        }
        self.drive
            .rename(ino(parent), &name, ino(new_parent), &new_name)
            .await
            .map_err(|e| errno(&e))
    }

    /// Öffnet eine Datei. Mit `O_TRUNC` wird vorher geleert, damit nicht erst
    /// der alte Inhalt geladen wird.
    ///
    /// # Errors
    /// `ENOENT`, `EISDIR` oder Fehler beim Laden.
    pub async fn open(&self, inode: INodeNo, flags: OpenFlags) -> Result<(), Errno> {
        let write = wants_write(flags);
        if write && wants_truncate(flags) {
            self.drive
                .set_size(ino(inode), 0)
                .await
                .map_err(|e| errno(&e))?;
        }
        self.drive
            .open(ino(inode), write)
            .await
            .map_err(|e| errno(&e))
    }

    /// Liest bis zu `size` Bytes ab `offset`.
    ///
    /// # Errors
    /// `ENOENT`, `EISDIR` oder Fehler beim Laden.
    pub async fn read(&self, inode: INodeNo, offset: u64, size: u32) -> Result<Vec<u8>, Errno> {
        self.drive
            .read(ino(inode), offset, size)
            .await
            .map_err(|e| errno(&e))
    }

    /// Schreibt `data` ab `offset`; liefert die Anzahl geschriebener Bytes.
    ///
    /// # Errors
    /// `ENOENT` oder Fehler im Zwischenspeicher.
    pub fn write(&self, inode: INodeNo, offset: u64, data: &[u8]) -> Result<u32, Errno> {
        self.drive
            .write(ino(inode), offset, data)
            .map_err(|e| errno(&e))
    }

    /// Ändert Attribute. Nur die Größe wird übernommen; Rechte, Besitzer und
    /// Zeiten werden angenommen, aber nicht gespeichert (Proton kennt sie nicht).
    ///
    /// # Errors
    /// `ENOENT`, `EISDIR` oder Fehler beim Laden.
    pub async fn setattr(&self, inode: INodeNo, size: Option<u64>) -> Result<FileAttr, Errno> {
        let attributes = match size {
            Some(size) => self.drive.set_size(ino(inode), size).await,
            None => self.drive.attributes(ino(inode)),
        }
        .map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Legt eine Datei an und öffnet sie.
    ///
    /// # Errors
    /// `EEXIST`, `EINVAL` oder Fehler beim Auflisten.
    pub async fn create(&self, parent: INodeNo, name: &OsStr) -> Result<FileAttr, Errno> {
        let name = node_name(name)?;
        let attributes = self
            .drive
            .create(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Schließt ein Handle; nach dem letzten wird hochgeladen.
    ///
    /// Ein fehlgeschlagener Upload ist für das schließende Programm kein
    /// Fehler mehr (der Kernel ignoriert ihn ohnehin): Die Datei bleibt
    /// vorgemerkt und wird später erneut hochgeladen.
    pub async fn release(&self, inode: INodeNo) {
        if let Err(error) = self.drive.release(ino(inode)).await {
            tracing::warn!(%error, ino = inode.0, "Hochladen nach dem Schließen fehlgeschlagen, wird wiederholt");
        }
    }
}
