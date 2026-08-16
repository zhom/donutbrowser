//! Best-effort secure deletion.
//!
//! "Best-effort" is load-bearing and is not a hedge. On copy-on-write
//! filesystems (APFS, Btrfs, ZFS, ReFS) and on any SSD with wear levelling, the
//! blocks holding the old contents may survive an overwrite entirely, because
//! the write lands somewhere else. RAM-backed storage can also be paged out,
//! and zeroing a file cannot reach the swap slot that held it. Treat these
//! helpers as raising the cost of recovery, never as a guarantee of erasure.
//!
//! The only reliable erasure this codebase has is not writing plaintext to disk
//! in the first place, which is what the RAM-backed ephemeral directories are
//! for. These helpers exist for the paths where that failed.

use std::fs;
use std::io::Write;
use std::path::Path;

/// Zero a file's bytes and flush before unlinking, so the contents are not
/// trivially recoverable from the freed blocks.
///
/// The overwrite must never gate the unlink. A write that fails part-way
/// (ENOSPC on a copy-on-write volume, EIO) would otherwise leave the file both
/// un-wiped and un-deleted, which is strictly worse than the plain remove this
/// replaces, because callers report success either way and the data would
/// silently survive.
pub fn secure_remove_file(path: &Path) -> std::io::Result<()> {
  if let Ok(meta) = fs::metadata(path) {
    let len = meta.len();
    if len > 0 {
      if let Ok(mut f) = fs::OpenOptions::new().write(true).open(path) {
        let zeros = vec![0u8; 64 * 1024];
        let mut remaining = len;
        while remaining > 0 {
          let chunk = remaining.min(zeros.len() as u64) as usize;
          if f.write_all(&zeros[..chunk]).is_err() {
            break;
          }
          remaining -= chunk as u64;
        }
        // One flush per file, not per chunk: syncing every 64 KiB turns a
        // profile teardown into thousands of barriers for no extra safety.
        let _ = f.flush();
        let _ = f.sync_all();
      }
    }
  }
  fs::remove_file(path)
}

/// Whether zeroing this file would even mean anything.
///
/// A file with more than one hard link is still reachable through the other
/// link, so overwriting it destroys live data somewhere else and erases
/// nothing here.
#[cfg(unix)]
fn is_last_link(meta: &fs::Metadata) -> bool {
  use std::os::unix::fs::MetadataExt;
  meta.nlink() <= 1
}

#[cfg(not(unix))]
fn is_last_link(_meta: &fs::Metadata) -> bool {
  true
}

/// Recursively delete a directory, optionally zeroing regular files first.
///
/// `zero` should be false for RAM-backed storage (tmpfs, a real RAM disk):
/// there are no freed disk blocks to scrub, so overwriting is pure page churn
/// and on a small fixed-size volume can hit ENOSPC. Pass true only when the
/// tree is genuinely on disk.
///
/// Symlinks are unlinked, never followed and never zeroed: following one would
/// destroy a target outside the tree.
///
/// Returns the number of files removed. The tree is removed even when
/// individual steps fail, because leaving a half-wiped directory in place is
/// the worst outcome available.
pub fn secure_remove_dir_all(root: &Path, zero: bool) -> std::io::Result<u64> {
  let mut removed = 0u64;
  if !root.exists() {
    return Ok(0);
  }

  remove_tree(root, zero, &mut removed);

  // Unconditional backstop: a walk that failed part-way must still not leave
  // the directory behind.
  match fs::remove_dir_all(root) {
    Ok(()) => Ok(removed),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(removed),
    Err(e) => Err(e),
  }
}

fn remove_tree(dir: &Path, zero: bool, removed: &mut u64) {
  let entries = match fs::read_dir(dir) {
    Ok(entries) => entries,
    Err(e) => {
      log::warn!("Secure erase could not read {}: {e}", dir.display());
      return;
    }
  };

  for entry in entries.flatten() {
    let path = entry.path();
    // file_type() on the DirEntry is lstat-based, so a symlink reports as a
    // symlink rather than as whatever it points at.
    let file_type = match entry.file_type() {
      Ok(ft) => ft,
      Err(_) => continue,
    };

    if file_type.is_symlink() {
      let _ = fs::remove_file(&path);
      *removed += 1;
    } else if file_type.is_dir() {
      remove_tree(&path, zero, removed);
      let _ = fs::remove_dir(&path);
    } else {
      let should_zero = zero
        && fs::symlink_metadata(&path)
          .map(|m| is_last_link(&m))
          .unwrap_or(false);
      let outcome = if should_zero {
        secure_remove_file(&path)
      } else {
        fs::remove_file(&path)
      };
      if outcome.is_ok() {
        *removed += 1;
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn zeroing_erase_removes_a_nested_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let nested = tmp.path().join("Default/Network");
    fs::create_dir_all(&nested).unwrap();
    let cookies = nested.join("Cookies");
    fs::write(&cookies, b"session=supersecretvalue").unwrap();

    let removed = secure_remove_dir_all(tmp.path(), true).unwrap();
    assert!(
      removed >= 1,
      "expected at least the cookie file to be counted"
    );
    assert!(!tmp.path().exists());
  }

  #[test]
  fn erase_without_zeroing_still_removes_everything() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("a"), b"x").unwrap();
    fs::create_dir_all(tmp.path().join("d")).unwrap();
    fs::write(tmp.path().join("d/b"), b"y").unwrap();

    secure_remove_dir_all(tmp.path(), false).unwrap();
    assert!(!tmp.path().exists());
  }

  #[test]
  fn missing_root_is_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let absent = tmp.path().join("never-existed");
    assert_eq!(secure_remove_dir_all(&absent, true).unwrap(), 0);
  }

  #[cfg(unix)]
  #[test]
  fn a_symlink_is_unlinked_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    // The target lives OUTSIDE the tree being erased. Following the link would
    // destroy a user's real file, which is the failure this guards against.
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("precious");
    fs::write(&target, b"must survive intact").unwrap();

    let tmp = tempfile::tempdir().unwrap();
    symlink(&target, tmp.path().join("link")).unwrap();

    secure_remove_dir_all(tmp.path(), true).unwrap();

    assert!(!tmp.path().exists());
    assert_eq!(fs::read(&target).unwrap(), b"must survive intact");
  }

  #[cfg(unix)]
  #[test]
  fn a_second_hard_link_is_not_zeroed_through() {
    let tmp = tempfile::tempdir().unwrap();
    let inside = tmp.path().join("shared");
    fs::write(&inside, b"still referenced elsewhere").unwrap();

    let outside = tempfile::tempdir().unwrap();
    let other = outside.path().join("other-name");
    fs::hard_link(&inside, &other).unwrap();

    secure_remove_dir_all(tmp.path(), true).unwrap();

    // The link inside the tree is gone, and the surviving link still holds the
    // original bytes rather than a run of zeros.
    assert!(!tmp.path().exists());
    assert_eq!(fs::read(&other).unwrap(), b"still referenced elsewhere");
  }
}
