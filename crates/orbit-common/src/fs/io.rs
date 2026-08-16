//! Atomic filesystem primitives.
//!
//! Consolidates the variants that historically existed across the workspace:
//! - `orbit-core::fs_utils::atomic_write_text` (volatile)
//! - `orbit-store::file::fs_utils::write_atomic` (volatile, with separate flock helper)
//! - the former `orbit-knowledge` durable write (parent-dir fsync), since removed
//!
//! The durable variant is the canonical one: rename-into-place plus
//! parent-directory fsync so the rename itself is flushed. Volatile is
//! offered for hot paths where the caller accepts post-crash inconsistency.
//!
//! All functions return `io::Result`; callers map to their domain error type
//! (`OrbitError`, `KnowledgeError`, etc.) at the boundary. Keeping this
//! module domain-free preserves the `types::` / `utility::` split inside
//! `orbit-common`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Creates a directory tree for secret-bearing Orbit state.
///
/// On Unix, every directory this call creates is immediately restricted to the
/// current user (`0o700`) instead of relying on the process umask. Existing
/// directories are left unchanged so callers do not unexpectedly chmod a
/// workspace root or home directory.
pub(crate) fn create_private_dir_all(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        create_private_dir_all_unix(path)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
    }
}

/// Create a new secret-bearing file for writing.
///
/// On Unix, the file is opened with and then set to `0o600` so group/other bits
/// cannot leak in through the process umask.
pub(crate) fn create_new_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).truncate(true).write(true);
    open_private_file(path, &mut options)
}

/// Open a secret-bearing append-only file, creating it if needed.
///
/// On Unix, newly created and pre-existing files are set to `0o600`.
pub(crate) fn append_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    open_private_file(path, &mut options)
}

/// Atomically write `content` to `path`, then fsync the parent directory so
/// the rename survives a crash. Creates parent directories as needed.
pub fn atomic_write_text(path: &Path, content: &str) -> io::Result<()> {
    let mut staged = StagedTextFile::new_internal(path, content, true)?;
    staged.commit()
}

/// Atomically write `content` bytes to `path`, then fsync the parent directory
/// so the rename survives a crash. Creates parent directories as needed.
pub fn atomic_write_bytes(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no parent dir for {}", path.display()),
        )
    })?;
    create_private_dir_all(parent)?;

    let temp_path = temp_path_for(path);
    let mut file = create_new_private_file(&temp_path)?;

    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temp_path, metadata.permissions())?;
    }

    file.write_all(content)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp_path, path)?;
    sync_parent_dir(path)
}

/// Atomically write `content` to `path` without fsyncing the parent.
/// Cheaper than [`atomic_write_text`] but post-crash the rename may be lost.
pub fn atomic_write_text_volatile(path: &Path, content: &str) -> io::Result<()> {
    let mut staged = StagedTextFile::new_internal(path, content, false)?;
    staged.commit()
}

/// A staged write that can be committed or dropped. Useful when a caller
/// needs to perform additional validation between staging and commit.
///
/// Drop before `commit()` removes the temp file.
pub struct StagedTextFile {
    target_path: PathBuf,
    temp_path: PathBuf,
    sync_parent: bool,
    committed: bool,
}

impl StagedTextFile {
    /// Stage a durable write. `commit()` renames and fsyncs the parent dir.
    pub fn new(target_path: &Path, content: &str) -> io::Result<Self> {
        Self::new_internal(target_path, content, true)
    }

    /// Stage a volatile write. `commit()` renames without fsyncing.
    pub fn new_volatile(target_path: &Path, content: &str) -> io::Result<Self> {
        Self::new_internal(target_path, content, false)
    }

    fn new_internal(target_path: &Path, content: &str, durable: bool) -> io::Result<Self> {
        let parent = target_path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("no parent dir for {}", target_path.display()),
            )
        })?;
        create_private_dir_all(parent)?;

        let temp_path = temp_path_for(target_path);
        let mut file = create_new_private_file(&temp_path)?;

        if let Ok(metadata) = fs::metadata(target_path) {
            fs::set_permissions(&temp_path, metadata.permissions())?;
        }

        file.write_all(content.as_bytes())?;
        if durable {
            file.sync_all()?;
        }
        drop(file);

        Ok(Self {
            target_path: target_path.to_path_buf(),
            temp_path,
            sync_parent: durable,
            committed: false,
        })
    }

    pub fn commit(&mut self) -> io::Result<()> {
        fs::rename(&self.temp_path, &self.target_path)?;
        self.committed = true;
        if self.sync_parent {
            sync_parent_dir(&self.target_path)?;
        }
        Ok(())
    }
}

impl Drop for StagedTextFile {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let _ = fs::remove_file(&self.temp_path);
    }
}

fn temp_path_for(target_path: &Path) -> PathBuf {
    let file_name = target_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("orbit");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_name = format!(".{file_name}.{nanos}.{counter}.tmp");
    target_path.with_file_name(temp_name)
}

/// fsync the parent directory of `target_path` so the directory entry that
/// names `target_path` is durable.
///
/// fsync on a file or directory persists that object's own data and inode, but
/// not the entry in its *parent* that makes it reachable by path. After freshly
/// creating a file, directory, or rename target, a crash can otherwise leave a
/// fully-fsynced but unreferenced inode that recovery reclaims as an orphan.
/// Call this on the newly created path to close that window.
pub fn sync_parent_dir(target_path: &Path) -> io::Result<()> {
    let parent = target_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no parent dir for {}", target_path.display()),
        )
    })?;
    File::open(parent)?.sync_all()
}

// ---------------------------------------------------------------------------
// Filesystem helpers beyond atomic write
// ---------------------------------------------------------------------------

/// Creates a directory symlink `dst` → `src`. Platform-abstracted over
/// Unix (`symlink`) and Windows (`symlink_dir`).
#[cfg(unix)]
pub fn create_dir_symlink(src: &Path, dst: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
pub fn create_dir_symlink(src: &Path, dst: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(src, dst)
}

/// Removes `path` if it exists, tolerating missing paths. Symlinks are
/// unlinked without following; directories are removed recursively.
pub fn remove_path_if_exists(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Writes `content` to `path`, creating parent directories as needed. Not
/// atomic — for crash-safe writes use [`atomic_write_text`].
pub fn write_text_with_parent(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// Run `op` while holding an exclusive advisory flock on a sibling lock
/// file of `target_path` (`.<filename>.lock`). Creates the parent directory
/// if missing. The lock is released when this function returns.
///
/// The closure returns `Result<T, E>` where any filesystem error hit while
/// acquiring the lock is folded into `E` via `From<std::io::Error>` —
/// callers returning `OrbitError`, `io::Error`, or any error type that
/// implements `From<io::Error>` compose directly.
///
/// `label` prefixes error messages for diagnosability when the lock path
/// alone isn't enough context.
pub fn with_exclusive_file_lock<T, E, F>(target_path: &Path, label: &str, op: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E>,
    E: From<io::Error>,
{
    let parent = target_path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("cannot determine parent for '{}'", target_path.display()),
        )
    })?;
    create_private_dir_all(parent)?;

    let lock_path = lock_path_for(target_path)?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    apply_private_file_mode(&mut options);
    let lock_file = options.open(&lock_path).map_err(|e| {
        io::Error::other(format!("open {label} lock '{}': {e}", lock_path.display()))
    })?;
    set_private_file_permissions(&lock_path).map_err(|e| {
        io::Error::other(format!("chmod {label} lock '{}': {e}", lock_path.display()))
    })?;
    lock_file
        .lock_exclusive()
        .map_err(|e| io::Error::other(format!("lock {label} '{}': {e}", lock_path.display())))?;

    op()
}

fn lock_path_for(path: &Path) -> io::Result<PathBuf> {
    let file_name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path '{}' has no file name", path.display()),
        )
    })?;
    Ok(path.with_file_name(format!(".{file_name}.lock")))
}

fn open_private_file(path: &Path, options: &mut OpenOptions) -> io::Result<File> {
    apply_private_file_mode(options);
    let file = options.open(path)?;
    set_private_file_permissions(path)?;
    Ok(file)
}

#[cfg(unix)]
fn create_private_dir_all_unix(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if current.as_os_str().is_empty() {
            continue;
        }

        match fs::metadata(&current) {
            Ok(metadata) if metadata.is_dir() => continue,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a directory", current.display()),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder.mode(PRIVATE_DIR_MODE);
                match builder.create(&current) {
                    Ok(()) => {
                        fs::set_permissions(
                            &current,
                            fs::Permissions::from_mode(PRIVATE_DIR_MODE),
                        )?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        if !current.is_dir() {
                            return Err(io::Error::new(
                                io::ErrorKind::AlreadyExists,
                                format!("{} exists and is not a directory", current.display()),
                            ));
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

#[cfg(unix)]
fn apply_private_file_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(PRIVATE_FILE_MODE);
}

#[cfg(not(unix))]
fn apply_private_file_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}
