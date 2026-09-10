//! Process locks use an OS-wide location independent of launcher environment.
//! They remain outside the watched project when its root is removed/recreated.
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// Stable identity before and after creation, including lexical path aliases.
pub(crate) fn canonical_identity(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let normalized: PathBuf = absolute.components().collect();
    let mut ancestor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut identity) => {
                for component in missing.into_iter().rev() {
                    identity.push(component);
                }
                #[cfg(windows)]
                let identity = PathBuf::from(identity.to_string_lossy().to_lowercase());
                return Ok(identity);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = ancestor.file_name().ok_or(error)?;
                missing.push(name.to_os_string());
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| io::Error::other("lock identity has no existing ancestor"))?;
            }
            Err(error) => return Err(error),
        }
    }
}

/// All transaction targets share these locks even across nested project roots.
pub(crate) struct OutputLocks {
    identities: std::collections::BTreeSet<PathBuf>,
    guards: Vec<ProcessLock>,
}

impl OutputLocks {
    pub(crate) fn acquire(root: &Path, paths: &[PathBuf], block: bool) -> io::Result<Option<Self>> {
        let identities = paths
            .iter()
            .map(|path| canonical_identity(&super::safe_project_input_path(root, path)?))
            .collect::<io::Result<std::collections::BTreeSet<_>>>()?;
        let mut guards = Vec::with_capacity(identities.len());
        // Every transaction uses the same order after its root journal lock.
        for identity in &identities {
            let Some(guard) = ProcessLock::acquire("project-output", identity, block)? else {
                return Ok(None);
            };
            guards.push(guard);
        }
        Ok(Some(Self { identities, guards }))
    }

    pub(crate) fn is_current(&self) -> io::Result<bool> {
        for guard in &self.guards {
            if !guard.is_current()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn covers(&self, root: &Path, path: &Path) -> io::Result<bool> {
        Ok(self
            .identities
            .contains(&canonical_identity(&super::safe_project_input_path(
                root, path,
            )?)?))
    }
}

pub(crate) struct ProcessLock {
    file: File,
    path: PathBuf,
}

impl ProcessLock {
    pub(crate) fn acquire(kind: &str, identity: &Path, block: bool) -> io::Result<Option<Self>> {
        let identity = canonical_identity(identity)?;
        #[cfg(windows)]
        let identity = identity.to_string_lossy().to_lowercase();
        #[cfg(windows)]
        let bytes = identity.as_bytes();
        #[cfg(not(windows))]
        let bytes = identity.as_os_str().as_encoded_bytes();
        let digest = Sha256::digest(bytes);
        let name = format!("girder-{kind}-{digest:x}.lock");
        let path = lock_path(&name)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&path)?;
        if !file.metadata()?.is_file() || std::fs::symlink_metadata(&path)?.file_type().is_symlink()
        {
            return Err(io::Error::other("process lock is not a regular file"));
        }
        if block {
            file.lock()?;
        } else {
            match file.try_lock() {
                Ok(()) => (),
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Error(e)) => return Err(e),
            }
        }
        let lock = Self { file, path };
        if !lock.is_current()? {
            return Err(io::Error::other(
                "process lock file was replaced during acquisition",
            ));
        }
        // Never unlink: another process may already have opened this file.
        Ok(Some(lock))
    }

    pub(crate) fn outside(&self, root: &Path) -> bool {
        !self.path.starts_with(root)
    }

    pub(crate) fn is_current(&self) -> io::Result<bool> {
        let current = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        Ok(file_identity(&self.file)? == file_identity(&current)?)
    }
}

#[cfg(unix)]
fn lock_path(name: &str) -> io::Result<PathBuf> {
    // /tmp is the shared OS location, including /private/tmp on macOS. Do not
    // use TMPDIR: two launchers of the same graph can set different values.
    Ok(Path::new("/tmp").canonicalize()?.join(name))
}

#[cfg(windows)]
fn lock_path(name: &str) -> io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath},
    };
    let mut pointer = std::ptr::null_mut();
    // The API allocates a NUL-terminated UTF-16 path; its buffer must be freed
    // with CoTaskMemFree on both success and failure.
    let status = unsafe {
        SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, std::ptr::null_mut(), &mut pointer)
    };
    let result = if status < 0 || pointer.is_null() {
        Err(io::Error::other(format!(
            "cannot locate shared process-lock storage: HRESULT {status:#x}"
        )))
    } else {
        let mut length = 0;
        unsafe {
            while *pointer.add(length) != 0 {
                length += 1;
            }
        }
        let path =
            unsafe { std::ffi::OsString::from_wide(std::slice::from_raw_parts(pointer, length)) };
        Ok(PathBuf::from(path))
    };
    unsafe {
        CoTaskMemFree(pointer.cast());
    }
    // One folder per lock identity avoids a common folder owned by the first
    // user. Existing ACLs are retained; access failures fail closed.
    let directory = result?.join(name);
    std::fs::create_dir_all(&directory)?;
    if std::fs::symlink_metadata(&directory)?
        .file_type()
        .is_symlink()
    {
        return Err(io::Error::other("process lock directory is a symlink"));
    }
    Ok(directory.canonicalize()?.join("lock"))
}

#[cfg(unix)]
fn file_identity(file: &File) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn file_identity(file: &File) -> io::Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}
