use std::{
    collections::BTreeSet,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

use super::extract::{ExtractError, has_extension, walk_files};

#[derive(Debug, Error)]
pub enum LocateError {
    #[error(transparent)]
    Inspect(#[from] ExtractError),
}

pub fn snapshot_executables(prefix: &Path) -> Result<BTreeSet<PathBuf>, LocateError> {
    let drive_c = prefix.join("drive_c");
    let mut executables = BTreeSet::new();
    walk_files(&drive_c, &mut |path| {
        if has_extension(path, &[b"exe"]) {
            executables.insert(path.to_path_buf());
        }
    })?;
    Ok(executables)
}

pub fn find_new_executables(
    prefix: &Path,
    before: &BTreeSet<PathBuf>,
) -> Result<Vec<PathBuf>, LocateError> {
    let after = snapshot_executables(prefix)?;
    let drive_c = prefix.join("drive_c");
    Ok(after
        .difference(before)
        .filter(|path| is_game_candidate(path, &drive_c))
        .cloned()
        .collect())
}

fn is_game_candidate(path: &Path, drive_c: &Path) -> bool {
    let relative = path.strip_prefix(drive_c).unwrap_or(path);
    let in_windows = relative.components().any(|component| {
        matches!(component, Component::Normal(value) if value.as_bytes().eq_ignore_ascii_case(b"windows"))
    });
    let name = path
        .file_name()
        .map(|name| name.as_bytes())
        .unwrap_or_default();
    !in_windows
        && !name.eq_ignore_ascii_case(b"uninstall.exe")
        && !name
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"unins"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs::{self, File},
    };

    use tempfile::tempdir;

    #[test]
    fn locate_returns_only_new_non_system_executables() {
        let root = tempdir().expect("temporary directory");
        let prefix = root.path().join("prefix");
        let game_dir = prefix.join("drive_c/Game");
        let windows_dir = prefix.join("drive_c/windows/system32");
        fs::create_dir_all(&game_dir).expect("game directory");
        fs::create_dir_all(&windows_dir).expect("windows directory");
        let old = game_dir.join("old.exe");
        File::create(&old).expect("old fixture");
        let before = BTreeSet::from([old]);
        File::create(game_dir.join("game.exe")).expect("game fixture");
        File::create(game_dir.join("unins000.exe")).expect("uninstaller fixture");
        File::create(windows_dir.join("helper.exe")).expect("system fixture");

        let found = super::find_new_executables(&prefix, &before).expect("locate executables");

        assert_eq!(found, vec![game_dir.join("game.exe")]);
    }
}
