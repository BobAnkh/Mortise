//! Implement pidfd manually mimicking PR #1868 since it is not yet merged into nix.
//! Link: https://github.com/nix-rust/nix/pull/1868
//!
//! In our implementation, since we are using and passing i32, we do not need to wrap the
//! i32 result into Fd/RawFd. We can just return the i32 result directly.
//!
//! After the PR is merged, this mod might be removed for maintenance
//! or reserved for simple direct interface.

use crate::Result;
use nix::errno::Errno;

/// Allocates a new file descriptor in the calling process. This new file descriptor is a duplicate
/// of an existing file descriptor, `target`, in the process referred to by the PID file descriptor
/// `pid`.
///
/// The duplicate file descriptor refers to the same open file description (see
/// [open(2)](https://man7.org/linux/man-pages/man2/open.2.html)) as the original file descriptor in
/// the process referred to by `pid`.  The two file descriptors thus share file status flags and
/// file offset.  Furthermore, operations on the underlying file object (for example, assigning an
/// address to a socket object using [bind(2)](https://man7.org/linux/man-pages/man2/bind.2.html))
/// can equally be performed via the duplicate file descriptor.
///
/// The close-on-exec flag ([`libc::FD_CLOEXEC`]; see
/// [fcntl(2)](https://man7.org/linux/man-pages/man2/fcntl.2.html)) is set on the returned file
/// descriptor.
///
/// Permission to duplicate another process's file descriptor is governed by a ptrace access mode
/// PTRACE_MODE_ATTACH_REALCREDS check (see
/// [ptrace(2)](https://man7.org/linux/man-pages/man2/ptrace.2.html)).
pub fn pidfd_getfd(pid_fd: i32, target_fd: i32) -> Result<i32> {
    #[allow(clippy::useless_conversion)] // Not useless on all OSes
    match unsafe { libc::syscall(libc::SYS_pidfd_getfd, pid_fd, target_fd, 0) } {
        -1 => Err(Errno::last().into()),
        fd @ 0.. => Ok(i32::try_from(fd).unwrap()),
        _ => unreachable!(),
    }
}

/// Creates a file descriptor that refers to the process whose PID is specified in `pid`.  The file
/// descriptor is returned as the function result; the close-on-exec flag is set on the file
/// descriptor.
///
/// If `nonblock == true` returns a nonblocking file descriptor.  If the process
/// referred to by the file descriptor has not yet terminated,
/// then an attempt to wait on the file descriptor using
/// waitid(2) will immediately return the error EAGAIN rather
/// than blocking.
pub fn pid_open(pid: i32, nonblock: bool) -> Result<i32> {
    #[allow(clippy::useless_conversion)] // Not useless on all OSes
    match unsafe {
        libc::syscall(
            libc::SYS_pidfd_open,
            pid,
            if nonblock { libc::PIDFD_NONBLOCK } else { 0 },
        )
    } {
        -1 => Err(Errno::last().into()),
        fd @ 0.. => Ok(i32::try_from(fd).unwrap()),
        _ => unreachable!(),
    }
}
