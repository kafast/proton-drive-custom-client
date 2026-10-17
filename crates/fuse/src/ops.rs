//! Translation between kernel requests and the virtual drive.
//!
//! Everything here is testable without a mounted file system: the methods take
//! the values the kernel sends and return answers or an error code
//! ([`Errno`]).

use std::ffi::OsStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{Errno, FileAttr, FileType, INodeNo, OpenAccMode, OpenFlags, RenameFlags};
use protonsync_application::drive::{Attributes, DriveError, Ino, VirtualDrive};
use protonsync_application::ports::{Clock, ContentCache, DriveRepo, RemoteStorage, StateRepo};
use protonsync_domain::node::NodeName;
use protonsync_domain::time::Timestamp;

use crate::errno::errno_of;

/// Preferred block size for `stat()`.
const PREFERRED_IO_BYTES: u32 = 4096;
/// `stat()` counts used space in units of 512 bytes.
const STAT_BLOCK_BYTES: u64 = 512;
const FOLDER_PERMISSIONS: u16 = 0o755;
const FILE_PERMISSIONS: u16 = 0o644;
/// Folders have (as usual) two links: their name and `.`.
const FOLDER_LINKS: u32 = 2;
const FILE_LINKS: u32 = 1;

/// Owner of all entries (the user who mounts the drive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owner {
    /// User ID.
    pub uid: u32,
    /// Group ID.
    pub gid: u32,
}

/// An entry when listing a folder, as the kernel needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// Inode number.
    pub ino: INodeNo,
    /// Folder or file.
    pub kind: FileType,
    /// Name (including `.` and `..`).
    pub name: String,
}

/// Translates kernel requests into calls to the drive.
pub struct DriveOps<R, C, D, S, K> {
    drive: Arc<VirtualDrive<R, C, D, S, K>>,
    owner: Owner,
}

/// Timestamp as `SystemTime` (also before 1970, in case Proton returns such a thing).
fn system_time(timestamp: Timestamp) -> SystemTime {
    let seconds = timestamp.unix_seconds();
    let offset = Duration::from_secs(seconds.unsigned_abs());
    if seconds >= 0 {
        UNIX_EPOCH + offset
    } else {
        UNIX_EPOCH - offset
    }
}

/// Converts a name from the kernel into a valid Proton name.
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
    /// Translator for `drive`; all entries belong to `owner`.
    #[must_use]
    pub const fn new(drive: Arc<VirtualDrive<R, C, D, S, K>>, owner: Owner) -> Self {
        Self { drive, owner }
    }

    /// The drive.
    #[must_use]
    pub fn drive(&self) -> &Arc<VirtualDrive<R, C, D, S, K>> {
        &self.drive
    }

    /// Attributes in the kernel's format.
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

    /// `stat()` on a known entry.
    ///
    /// # Errors
    /// `ENOENT` if the number is unknown.
    pub fn getattr(&self, inode: INodeNo) -> Result<FileAttr, Errno> {
        let attributes = self.drive.attributes(ino(inode)).map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Looks up `name` in `parent`.
    ///
    /// # Errors
    /// `ENOENT`, `ENOTDIR`, `EINVAL` (invalid name) or errors from Proton.
    pub async fn lookup(&self, parent: INodeNo, name: &OsStr) -> Result<FileAttr, Errno> {
        let name = node_name(name)?;
        let attributes = self
            .drive
            .lookup(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Contents of a folder including `.` and `..`.
    ///
    /// # Errors
    /// `ENOENT`, `ENOTDIR` or errors from Proton.
    pub async fn list(&self, folder: INodeNo) -> Result<Vec<Listing>, Errno> {
        let entries = self.drive.list(ino(folder)).await.map_err(|e| errno(&e))?;
        // `..` points to the folder itself here; the kernel determines the
        // real parent folder on its own anyway.
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

    /// Creates a folder.
    ///
    /// # Errors
    /// `EEXIST`, `EINVAL` or errors from Proton.
    pub async fn mkdir(&self, parent: INodeNo, name: &OsStr) -> Result<FileAttr, Errno> {
        let name = node_name(name)?;
        let attributes = self
            .drive
            .make_folder(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Deletes a file (into the trash).
    ///
    /// # Errors
    /// `ENOENT`, `EISDIR` or errors from Proton.
    pub async fn unlink(&self, parent: INodeNo, name: &OsStr) -> Result<(), Errno> {
        let name = node_name(name)?;
        self.drive
            .remove_file(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))
    }

    /// Deletes an empty folder (into the trash).
    ///
    /// # Errors
    /// `ENOENT`, `ENOTEMPTY`, `ENOTDIR` or errors from Proton.
    pub async fn rmdir(&self, parent: INodeNo, name: &OsStr) -> Result<(), Errno> {
        let name = node_name(name)?;
        self.drive
            .remove_folder(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))
    }

    /// Renames or moves. `RENAME_NOREPLACE` is respected, `RENAME_EXCHANGE`
    /// (swapping) does not exist in Proton.
    ///
    /// # Errors
    /// `EEXIST` (with `RENAME_NOREPLACE`), `EINVAL` or the errors of [`VirtualDrive::rename`].
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

    /// Opens a file. With `O_TRUNC` it is emptied first, so the old content is
    /// not downloaded first.
    ///
    /// # Errors
    /// `ENOENT`, `EISDIR` or download errors.
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

    /// Reads up to `size` bytes from `offset`.
    ///
    /// # Errors
    /// `ENOENT`, `EISDIR` or download errors.
    pub async fn read(&self, inode: INodeNo, offset: u64, size: u32) -> Result<Vec<u8>, Errno> {
        self.drive
            .read(ino(inode), offset, size)
            .await
            .map_err(|e| errno(&e))
    }

    /// Writes `data` at `offset`; returns the number of bytes written.
    ///
    /// # Errors
    /// `ENOENT` or cache errors.
    pub fn write(&self, inode: INodeNo, offset: u64, data: &[u8]) -> Result<u32, Errno> {
        self.drive
            .write(ino(inode), offset, data)
            .map_err(|e| errno(&e))
    }

    /// Changes attributes. Only the size is applied; permissions, owner and
    /// times are accepted but not stored (Proton does not know them).
    ///
    /// # Errors
    /// `ENOENT`, `EISDIR` or download errors.
    pub async fn setattr(&self, inode: INodeNo, size: Option<u64>) -> Result<FileAttr, Errno> {
        let attributes = match size {
            Some(size) => self.drive.set_size(ino(inode), size).await,
            None => self.drive.attributes(ino(inode)),
        }
        .map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Creates a file and opens it.
    ///
    /// # Errors
    /// `EEXIST`, `EINVAL` or listing errors.
    pub async fn create(&self, parent: INodeNo, name: &OsStr) -> Result<FileAttr, Errno> {
        let name = node_name(name)?;
        let attributes = self
            .drive
            .create(ino(parent), &name)
            .await
            .map_err(|e| errno(&e))?;
        Ok(self.file_attr(attributes))
    }

    /// Closes a handle; after the last one the file is uploaded.
    ///
    /// A failed upload is no longer an error for the closing program (the
    /// kernel ignores it anyway): the file stays marked and is uploaded again
    /// later.
    pub async fn release(&self, inode: INodeNo) {
        if let Err(error) = self.drive.release(ino(inode)).await {
            tracing::warn!(%error, ino = inode.0, "upload after close failed, will retry");
        }
    }
}
