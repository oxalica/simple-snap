use std::ffi::OsStr;
use std::mem;
use std::os::fd::AsFd;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;

use linux_raw_sys::{btrfs, ioctl};
use rustix::io::Errno;
use rustix::io::Result;
use rustix::ioctl::{Getter, Setter, ioctl};

fn copy_os_str<const N: usize>(src: &OsStr, dst: &mut [std::ffi::c_char; N]) -> Result<()> {
    // NB. Reject equal size because of the mandatory NUL byte.
    if src.as_bytes().contains(&0) || src.len() >= dst.len() {
        return Err(Errno::INVAL);
    }
    for (&b, out) in src.as_bytes().iter().zip(dst) {
        *out = b as _;
    }
    Ok(())
}

/// BTRFS_IOC_SNAP_CREATE_V2
pub fn snap_create_v2<F: AsFd, G: AsFd, S: AsRef<OsStr>>(
    parent_dir_fd: F,
    name: S,
    src_subvol_fd: G,
    readonly: bool,
) -> Result<()> {
    // SAFETY: Zero is a valid value for `btrfs_ioctl_vol_args_v2`.
    let mut args = unsafe { mem::zeroed::<btrfs::btrfs_ioctl_vol_args_v2>() };
    args.fd = src_subvol_fd.as_fd().as_raw_fd().into();
    if readonly {
        args.flags = btrfs::BTRFS_SUBVOL_RDONLY.into();
    }
    // SAFETY: Zero is an initialized value for the union.
    copy_os_str(name.as_ref(), unsafe { &mut args.__bindgen_anon_2.name })?;
    // SAFETY: Arguments are valid according to the doc:
    // <https://btrfs.readthedocs.io/en/latest/btrfs-ioctl.html#btrfs-ioc-snap-create-v2>
    unsafe {
        ioctl(
            parent_dir_fd,
            <Setter<{ ioctl::BTRFS_IOC_SNAP_CREATE_V2 }, _>>::new(args),
        )?;
    }
    Ok(())
}

/// BTRFS_IOC_SUBVOL_GETFLAGS
pub fn subvol_getflags<F: AsFd>(fd: F) -> Result<u64> {
    // SAFETY: Arguments are valid according to the doc:
    // <https://btrfs.readthedocs.io/en/latest/btrfs-ioctl.html#btrfs-ioc-subvol-getflags>
    unsafe {
        ioctl(
            fd,
            <Getter<{ ioctl::BTRFS_IOC_SUBVOL_GETFLAGS }, u64>>::new(),
        )
    }
}

/// BTRFS_IOC_SNAP_DESTROY_V2
pub fn snap_destroy_v2<F: AsFd, S: AsRef<OsStr>>(parent_dir_fd: F, name: S) -> Result<()> {
    // SAFETY: Zero is a valid value for `btrfs_ioctl_vol_args_v2`.
    let mut args = unsafe { mem::zeroed::<btrfs::btrfs_ioctl_vol_args_v2>() };
    // Use parent-name pair to locate subvolume.
    args.flags = 0;
    // SAFETY: Zero is an initialized value for the union.
    copy_os_str(name.as_ref(), unsafe { &mut args.__bindgen_anon_2.name })?;
    // SAFETY: Arguments are valid according to the doc:
    // <https://btrfs.readthedocs.io/en/latest/btrfs-ioctl.html#btrfs-ioc-snap-destroy-v2>
    unsafe {
        ioctl(
            parent_dir_fd,
            <Setter<{ ioctl::BTRFS_IOC_SNAP_DESTROY_V2 }, _>>::new(args),
        )?;
    }
    Ok(())
}
