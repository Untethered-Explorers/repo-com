use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// The application directory beneath the OS user-data root.
pub const APPLICATION_DIRECTORY: &str = "repo-com";
/// The single version-1 operational database file name.
pub const DATABASE_FILE_NAME: &str = "state.sqlite3";

/// A typed failure while resolving or preparing the user-data path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathError {
    /// The platform did not provide a user-data directory.
    UserDataUnavailable,
    /// The path could not be created, inspected, or secured.
    Io {
        /// The attempted filesystem operation.
        operation: &'static str,
        /// The path involved in the operation.
        path: PathBuf,
        /// A platform error message.
        message: String,
    },
    /// An existing path is not a regular database file.
    NotAFile(PathBuf),
    /// Refusing to follow a symbolic link for the database path.
    SymbolicLink(PathBuf),
}

impl std::fmt::Display for PathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserDataUnavailable => {
                formatter.write_str("OS user-data directory is unavailable")
            }
            Self::Io {
                operation,
                path,
                message,
            } => write!(
                formatter,
                "state path {operation} failed for {}: {message}",
                path.display()
            ),
            Self::NotAFile(path) => write!(
                formatter,
                "state database path is not a file: {}",
                path.display()
            ),
            Self::SymbolicLink(path) => write!(
                formatter,
                "state database path must not be a symbolic link: {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PathError {}

/// Returns the OS-native user-local application-data root.
///
/// `dirs::data_local_dir` maps to the XDG data directory on Linux, the user
/// Application Support directory on macOS, and the Known Folder local data
/// directory on Windows.
pub fn user_data_root() -> Result<PathBuf, PathError> {
    dirs::data_local_dir().ok_or(PathError::UserDataUnavailable)
}

/// Returns the OS-native user-data directory used by repo-com.
pub fn application_data_dir() -> Result<PathBuf, PathError> {
    Ok(user_data_root()?.join(APPLICATION_DIRECTORY))
}

/// Compatibility alias for [`user_data_root`].
pub fn resolve_user_data_dir() -> Result<PathBuf, PathError> {
    user_data_root()
}

/// Resolves the single operational database path.
///
/// The path is intentionally independent of the configured repository ID: one
/// local database contains rows for every configured repository, and each row
/// is namespaced by `repository_id`.
pub fn database_path() -> Result<PathBuf, PathError> {
    Ok(application_data_dir()?.join(DATABASE_FILE_NAME))
}

/// Compatibility alias for [`database_path`].
pub fn state_database_path() -> Result<PathBuf, PathError> {
    database_path()
}

/// Resolves the database path below an explicitly supplied data root.
///
/// This is primarily useful for isolated tests and portable deployments; it
/// still applies the same `repo-com/state.sqlite3` layout.
pub fn database_path_in(data_root: impl AsRef<Path>) -> PathBuf {
    data_root
        .as_ref()
        .join(APPLICATION_DIRECTORY)
        .join(DATABASE_FILE_NAME)
}

/// Compatibility alias for [`database_path`].
pub fn resolve_database_path() -> Result<PathBuf, PathError> {
    database_path()
}

/// Compatibility alias for [`application_data_dir`].
pub fn resolve_application_data_dir() -> Result<PathBuf, PathError> {
    application_data_dir()
}

/// Describes the filesystem protection model used for a state path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionModel {
    /// POSIX owner-only mode bits are set explicitly.
    PosixModes,
    /// Windows Known Folder ACL inheritance is the platform boundary; Rust's
    /// portable permission API does not create or rewrite Windows ACLs.
    WindowsInheritedAcl,
}

impl PermissionModel {
    /// Returns the model for the current target platform.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::WindowsInheritedAcl
        } else {
            Self::PosixModes
        }
    }
}

/// Result of inspecting state path permissions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionInspection {
    /// The platform permission model.
    pub model: PermissionModel,
    /// Whether the path is protected by that model.
    pub user_only: bool,
}

/// Creates the parent directory and database file with user-only permissions.
///
/// On POSIX this creates directories as `0700` and files as `0600`, including
/// correcting an existing state file that is still owner-readable/writable
/// only.  On Windows the Known Folder ACL inherited by the user profile is
/// retained; this function does not pretend that portable mode bits are an
/// ACL implementation.
pub fn create_user_only_database(path: impl AsRef<Path>) -> Result<(), PathError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| io_error("create-directory", parent, error))?;
        secure_directory(parent)?;
    }

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(PathError::SymbolicLink(path.to_path_buf()));
            }
            if !metadata.is_file() {
                return Err(PathError::NotAFile(path.to_path_buf()));
            }
            secure_file(path)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            #[allow(unused_mut)]
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(_) => secure_file(path)?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(path)
                        .map_err(|error| io_error("inspect-database", path, error))?;
                    if metadata.file_type().is_symlink() {
                        return Err(PathError::SymbolicLink(path.to_path_buf()));
                    }
                    if !metadata.is_file() {
                        return Err(PathError::NotAFile(path.to_path_buf()));
                    }
                    secure_file(path)?;
                }
                Err(error) => return Err(io_error("create-database", path, error)),
            }
        }
        Err(error) => return Err(io_error("inspect-database", path, error)),
    }
    Ok(())
}

/// Verifies that an existing database path is protected for its current user.
pub fn inspect_user_only_database(
    path: impl AsRef<Path>,
) -> Result<PermissionInspection, PathError> {
    let path = path.as_ref();
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("inspect-database", path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(PathError::SymbolicLink(path.to_path_buf()));
    }
    if !metadata.is_file() {
        return Err(PathError::NotAFile(path.to_path_buf()));
    }
    Ok(PermissionInspection {
        model: PermissionModel::current(),
        user_only: is_user_only(path),
    })
}

/// Returns whether the path meets the current platform's user-only boundary.
#[must_use]
pub fn is_user_only(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o077 == 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // Windows Known Folder ACL inheritance is checked by the platform, not
        // by the portable Rust permission bits.
        let _ = path;
        true
    }
}

fn secure_directory(path: &Path) -> Result<(), PathError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|error| io_error("inspect-directory", path, error))?
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)
            .map_err(|error| io_error("secure-directory", path, error))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn secure_file(path: &Path) -> Result<(), PathError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .map_err(|error| io_error("inspect-database", path, error))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .map_err(|error| io_error("secure-database", path, error))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn io_error(operation: &'static str, path: &Path, error: io::Error) -> PathError {
    PathError::Io {
        operation,
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}
