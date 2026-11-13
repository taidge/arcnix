//! Local file operations. Existing destinations are never replaced.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};

use crate::FileEntry;

pub fn list_dir(path: &Path) -> Result<(PathBuf, Vec<FileEntry>)> {
    let path = fs::canonicalize(path).context("Cannot open local directory")?;
    let mut entries = Vec::new();
    for item in fs::read_dir(&path)? {
        let item = item?;
        let metadata = fs::symlink_metadata(item.path())?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Filename is not valid Unicode"))?;
        let entry_path = item
            .path()
            .into_os_string()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Path is not valid Unicode"))?;
        entries.push(FileEntry {
            name,
            path: entry_path,
            is_dir: item.path().is_dir(),
            is_symlink: metadata.is_symlink(),
            size: metadata.len(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|t| t.as_secs()),
            permissions: None,
        });
    }
    sort_entries(&mut entries);
    Ok((path, entries))
}

pub(crate) fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

pub fn mkdir(path: &Path) -> Result<()> {
    fs::create_dir(path).context("Cannot create local directory")
}

pub fn rename(from: &Path, to: &Path) -> Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        bail!("Destination already exists");
    }
    // std::fs::rename replaces destinations on Windows too. Use the native
    // no-replace operation so the final publication cannot race another writer.
    #[cfg(not(windows))]
    if fs::symlink_metadata(from)?.is_file() {
        fs::hard_link(from, to).context("Cannot publish renamed file without overwriting")?;
        fs::remove_file(from)?;
        return Ok(());
    }
    #[cfg(not(windows))]
    bail!("Directory rename without overwrite is currently supported on Windows only");
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut from: Vec<u16> = from.as_os_str().encode_wide().collect();
        let mut to: Vec<u16> = to.as_os_str().encode_wide().collect();
        if from.contains(&0) || to.contains(&0) {
            bail!("Path contains NUL");
        }
        from.push(0);
        to.push(0);
        // SAFETY: both pointers reference live, terminated UTF-16 arrays;
        // flag 0 specifically omits MOVEFILE_REPLACE_EXISTING.
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
        } == 0
        {
            return Err(std::io::Error::last_os_error())
                .context("Cannot rename local entry without overwrite");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_listing_and_no_overwrite() -> Result<()> {
        let root = std::env::temp_dir().join(format!("arcnix-local-{}", crate::new_id()));
        fs::create_dir(&root)?;
        let result = (|| {
            mkdir(&root.join("目录"))?;
            fs::write(root.join("测试.txt"), b"hello")?;
            fs::write(root.join("existing"), b"keep")?;
            let (_, entries) = list_dir(&root)?;
            assert!(entries[0].is_dir);
            assert!(entries.iter().any(|e| e.name == "测试.txt"));
            assert!(rename(&root.join("测试.txt"), &root.join("existing")).is_err());
            assert_eq!(fs::read(root.join("existing"))?, b"keep");
            rename(&root.join("测试.txt"), &root.join("renamed"))?;
            Ok(())
        })();
        fs::remove_dir_all(&root)?;
        result
    }
}
