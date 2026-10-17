//! Drive errors as error codes for the kernel (`errno`).

use fuser::Errno;
use protonsync_application::drive::DriveError;
use protonsync_application::ports::{LocalError, RemoteError};

/// The matching error code, as shown by e.g. `ls` or a file manager.
#[must_use]
pub fn errno_of(error: &DriveError) -> Errno {
    match error {
        DriveError::NotFound => Errno::ENOENT,
        DriveError::AlreadyExists => Errno::EEXIST,
        DriveError::NotAFolder => Errno::ENOTDIR,
        DriveError::IsAFolder => Errno::EISDIR,
        DriveError::NotEmpty => Errno::ENOTEMPTY,
        DriveError::InvalidName(_) => Errno::EINVAL,
        DriveError::Remote(remote) => remote_errno(remote),
        DriveError::Local(local) => local_errno(local),
        DriveError::State(_) | DriveError::Internal(_) => Errno::EIO,
    }
}

fn remote_errno(error: &RemoteError) -> Errno {
    match error {
        RemoteError::NotFound(_) => Errno::ENOENT,
        RemoteError::AlreadyExists(_) => Errno::EEXIST,
        RemoteError::NotAFolder(_) => Errno::ENOTDIR,
        RemoteError::NotAFile(_) => Errno::EISDIR,
        RemoteError::InvalidRange { .. } | RemoteError::InvalidMove(_) => Errno::EINVAL,
        RemoteError::Unauthorized | RemoteError::Rejected(_) => Errno::EACCES,
        // Network gone: programs should be able to retry later.
        RemoteError::Unavailable(_) => Errno::EAGAIN,
        RemoteError::Transfer(_) => Errno::EIO,
    }
}

fn local_errno(error: &LocalError) -> Errno {
    match error {
        LocalError::NotFound(_) => Errno::ENOENT,
        LocalError::AlreadyExists(_) => Errno::EEXIST,
        LocalError::NotAFolder(_) => Errno::ENOTDIR,
        LocalError::Io(_) => Errno::EIO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protonsync_application::ports::StateError;
    use protonsync_domain::node::NodeId;

    #[test]
    fn drive_errors_map_to_familiar_codes() {
        let cases = [
            (DriveError::NotFound, Errno::ENOENT),
            (DriveError::AlreadyExists, Errno::EEXIST),
            (DriveError::NotAFolder, Errno::ENOTDIR),
            (DriveError::IsAFolder, Errno::EISDIR),
            (DriveError::NotEmpty, Errno::ENOTEMPTY),
            (DriveError::InvalidName("a/b".into()), Errno::EINVAL),
            (DriveError::Internal("x"), Errno::EIO),
            (
                DriveError::State(StateError::Unavailable("x".into())),
                Errno::EIO,
            ),
            (
                DriveError::Remote(RemoteError::NotFound(NodeId::new("n"))),
                Errno::ENOENT,
            ),
            (DriveError::Remote(RemoteError::Unauthorized), Errno::EACCES),
            (
                DriveError::Remote(RemoteError::Unavailable("network".into())),
                Errno::EAGAIN,
            ),
            (
                DriveError::Remote(RemoteError::Transfer("broken".into())),
                Errno::EIO,
            ),
            (DriveError::Local(LocalError::Io("full".into())), Errno::EIO),
        ];
        for (error, expected) in cases {
            assert_eq!(errno_of(&error), expected, "{error:?}");
        }
    }
}
