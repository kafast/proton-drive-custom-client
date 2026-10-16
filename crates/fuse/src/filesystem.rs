//! Die Schnittstelle zu `fuser`: nimmt Kernel-Anfragen an und beantwortet sie.
//!
//! Hier steht nur Verdrahtung. Jede Anfrage, die auf Proton warten könnte,
//! läuft als eigene tokio-Aufgabe; die Antwort (`reply`) wird mitgegeben und
//! dort verschickt.

use std::ffi::OsStr;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fuser::{
    BsdFileFlags, Errno, FileHandle, Filesystem, FopenFlags, Generation, INodeNo, LockOwner,
    OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow, WriteFlags,
};
use protonsync_application::ports::{Clock, ContentCache, DriveRepo, RemoteStorage, StateRepo};
use tokio::runtime::Handle;

use crate::ops::DriveOps;

/// So lange darf der Kernel Attribute und Namen zwischenspeichern. Kurz,
/// damit Änderungen aus Proton (über Ereignisse) schnell sichtbar werden.
const ATTRIBUTE_TTL: Duration = Duration::from_secs(1);
/// Inode-Nummern werden pro Sitzung nie wiederverwendet.
const GENERATION: Generation = Generation(0);
/// Das Laufwerk unterscheidet keine Handles; offene Dateien zählt es selbst.
const HANDLE: FileHandle = FileHandle(0);

/// Proton nennt dem Client keinen freien Speicher; gemeldet wird ein fester,
/// großzügiger Wert, damit Programme vor dem Kopieren nicht abbrechen.
const REPORTED_CAPACITY_BYTES: u64 = 1 << 40;
const STATFS_BLOCK_BYTES: u32 = 4096;
const MAX_NAME_BYTES: u32 = 255;
const REPORTED_FREE_INODES: u64 = 1 << 32;

/// Das eingehängte Laufwerk aus Sicht von `fuser`.
pub(crate) struct DriveFilesystem<R, C, D, S, K> {
    ops: Arc<DriveOps<R, C, D, S, K>>,
    runtime: Handle,
}

impl<R, C, D, S, K> DriveFilesystem<R, C, D, S, K>
where
    R: RemoteStorage + 'static,
    C: ContentCache + 'static,
    D: DriveRepo + 'static,
    S: StateRepo + 'static,
    K: Clock + 'static,
{
    pub(crate) const fn new(ops: Arc<DriveOps<R, C, D, S, K>>, runtime: Handle) -> Self {
        Self { ops, runtime }
    }

    /// Startet `work` als tokio-Aufgabe mit Zugriff auf die Übersetzung.
    fn spawn<F, Fut>(&self, work: F)
    where
        F: FnOnce(Arc<DriveOps<R, C, D, S, K>>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.runtime.spawn(work(Arc::clone(&self.ops)));
    }
}

fn reply_entry(reply: ReplyEntry, result: Result<fuser::FileAttr, Errno>) {
    match result {
        Ok(attr) => reply.entry(&ATTRIBUTE_TTL, &attr, GENERATION),
        Err(errno) => reply.error(errno),
    }
}

fn reply_empty(reply: ReplyEmpty, result: Result<(), Errno>) {
    match result {
        Ok(()) => reply.ok(),
        Err(errno) => reply.error(errno),
    }
}

impl<R, C, D, S, K> Filesystem for DriveFilesystem<R, C, D, S, K>
where
    R: RemoteStorage + 'static,
    C: ContentCache + 'static,
    D: DriveRepo + 'static,
    S: StateRepo + 'static,
    K: Clock + 'static,
{
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let name = name.to_owned();
        self.spawn(move |ops| async move {
            reply_entry(reply, ops.lookup(parent, &name).await);
        });
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.ops.getattr(ino) {
            Ok(attr) => reply.attr(&ATTRIBUTE_TTL, &attr),
            Err(errno) => reply.error(errno),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        self.spawn(move |ops| async move {
            match ops.setattr(ino, size).await {
                Ok(attr) => reply.attr(&ATTRIBUTE_TTL, &attr),
                Err(errno) => reply.error(errno),
            }
        });
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name = name.to_owned();
        self.spawn(move |ops| async move {
            reply_entry(reply, ops.mkdir(parent, &name).await);
        });
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = name.to_owned();
        self.spawn(move |ops| async move {
            reply_empty(reply, ops.unlink(parent, &name).await);
        });
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let name = name.to_owned();
        self.spawn(move |ops| async move {
            reply_empty(reply, ops.rmdir(parent, &name).await);
        });
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        new_parent: INodeNo,
        new_name: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let name = name.to_owned();
        let new_name = new_name.to_owned();
        self.spawn(move |ops| async move {
            let result = ops
                .rename(parent, &name, new_parent, &new_name, flags)
                .await;
            reply_empty(reply, result);
        });
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        self.spawn(move |ops| async move {
            match ops.open(ino, flags).await {
                Ok(()) => reply.opened(HANDLE, FopenFlags::empty()),
                Err(errno) => reply.error(errno),
            }
        });
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        self.spawn(move |ops| async move {
            match ops.read(ino, offset, size).await {
                Ok(data) => reply.data(&data),
                Err(errno) => reply.error(errno),
            }
        });
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        // Schreiben geht nur in den lokalen Zwischenspeicher und ist schnell.
        match self.ops.write(ino, offset, data) {
            Ok(written) => reply.written(written),
            Err(errno) => reply.error(errno),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        // Hochgeladen wird erst nach dem letzten Schließen (`release`).
        reply.ok();
    }

    fn release(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.spawn(move |ops| async move {
            ops.release(ino).await;
            reply.ok();
        });
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        // Die Daten liegen im Zwischenspeicher; nach Proton gehen sie beim Schließen.
        reply.ok();
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        self.spawn(move |ops| async move {
            let listing = match ops.list(ino).await {
                Ok(listing) => listing,
                Err(errno) => return reply.error(errno),
            };
            // `offset` ist die Position nach dem zuletzt gelieferten Eintrag.
            let skip = usize::try_from(offset).unwrap_or(usize::MAX);
            for (position, entry) in (1_u64..).zip(&listing).skip(skip) {
                if reply.add(entry.ino, position, entry.kind, &entry.name) {
                    break; // Puffer voll; der Kernel fragt mit neuem `offset` nach.
                }
            }
            reply.ok();
        });
    }

    fn fsyncdir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        let blocks = REPORTED_CAPACITY_BYTES / u64::from(STATFS_BLOCK_BYTES);
        reply.statfs(
            blocks,
            blocks,
            blocks,
            REPORTED_FREE_INODES,
            REPORTED_FREE_INODES,
            STATFS_BLOCK_BYTES,
            MAX_NAME_BYTES,
            STATFS_BLOCK_BYTES,
        );
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let name = name.to_owned();
        self.spawn(move |ops| async move {
            match ops.create(parent, &name).await {
                Ok(attr) => reply.created(
                    &ATTRIBUTE_TTL,
                    &attr,
                    GENERATION,
                    HANDLE,
                    FopenFlags::empty(),
                ),
                Err(errno) => reply.error(errno),
            }
        });
    }
}
