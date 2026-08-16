use std::{
    collections::BTreeSet,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

use super::extract::{ExtractError, has_extension};

#[derive(Debug, Error)]
pub enum LocateError {
    #[error(transparent)]
    Inspect(#[from] ExtractError),
}

pub fn snapshot_executables(prefix: &Path) -> Result<BTreeSet<PathBuf>, LocateError> {
    let drive_c = prefix.join("drive_c");
    let mut executables = BTreeSet::new();
    walk_prefix_files(&drive_c, &drive_c, &mut |path| {
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
    let mut found: Vec<PathBuf> = after
        .difference(before)
        .filter(|path| is_game_candidate(path, &drive_c))
        .cloned()
        .collect();
    found.sort_by(|left, right| {
        candidate_rank(left, &drive_c)
            .cmp(&candidate_rank(right, &drive_c))
            .then_with(|| left.cmp(right))
    });
    Ok(found)
}

fn walk_prefix_files(
    root: &Path,
    drive_c: &Path,
    visitor: &mut impl FnMut(&Path),
) -> Result<(), LocateError> {
    let canonical_drive = drive_c
        .canonicalize()
        .unwrap_or_else(|_| drive_c.to_path_buf());
    let mut visited = BTreeSet::new();
    walk_prefix_files_inner(root, &canonical_drive, &mut visited, visitor)
}

fn walk_prefix_files_inner(
    root: &Path,
    canonical_drive: &Path,
    visited: &mut BTreeSet<PathBuf>,
    visitor: &mut impl FnMut(&Path),
) -> Result<(), LocateError> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if !canonical_root.starts_with(canonical_drive) || !visited.insert(canonical_root) {
        return Ok(());
    }

    let entries = fs::read_dir(root).map_err(|source| ExtractError::Inspect {
        path: root.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ExtractError::Inspect {
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = fs::symlink_metadata(&path)
            .and_then(|metadata| Ok(metadata.file_type()))
            .map_err(|source| ExtractError::Inspect {
                path: path.clone(),
                source,
            })?;
        if file_type.is_dir() || file_type.is_symlink() {
            if file_type.is_symlink() {
                let Ok(target) = path.canonicalize() else {
                    continue;
                };
                if target.is_dir() {
                    walk_prefix_files_inner(&path, canonical_drive, visited, visitor)?;
                } else if target.is_file() && target.starts_with(canonical_drive) {
                    visitor(&path);
                }
            } else {
                walk_prefix_files_inner(&path, canonical_drive, visited, visitor)?;
            }
        } else if file_type.is_file() {
            visitor(&path);
        }
    }
    Ok(())
}

fn is_game_candidate(path: &Path, drive_c: &Path) -> bool {
    let relative = path.strip_prefix(drive_c).unwrap_or(path);
    let in_windows = relative.components().any(|component| {
        matches!(component, Component::Normal(value) if value.as_bytes().eq_ignore_ascii_case(b"windows"))
    });
    !in_windows && !is_support_executable(path)
}

fn is_support_executable(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.as_bytes())
        .unwrap_or_default();
    name.eq_ignore_ascii_case(b"uninstall.exe")
        || name.eq_ignore_ascii_case(b"uninstaller.exe")
        || name
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"unins"))
        || name.eq_ignore_ascii_case(b"setup.exe")
        || name.eq_ignore_ascii_case(b"install.exe")
        || name.eq_ignore_ascii_case(b"bootstrap.exe")
}

fn candidate_rank(path: &Path, drive_c: &Path) -> (u8, usize) {
    let relative = path.strip_prefix(drive_c).unwrap_or(path);
    let in_program_files = relative.components().any(|component| {
        matches!(component, Component::Normal(value) if {
            let bytes = value.as_bytes();
            bytes.eq_ignore_ascii_case(b"Program Files")
                || bytes.eq_ignore_ascii_case(b"Program Files (x86)")
        })
    });
    let support = u8::from(is_support_executable(path));
    let systemish = u8::from(in_program_files);
    (support + systemish, relative.components().count())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs::{self, File},
        os::unix::fs::symlink,
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

    #[test]
    fn locate_prefers_the_game_over_bootstrap_and_follows_in_prefix_links() {
        let root = tempdir().expect("temporary directory");
        let prefix = root.path().join("prefix");
        let documents = prefix.join("drive_c/users/jacob/Documents");
        let japanese_documents = prefix.join("drive_c/users/jacob/ドキュメント");
        let game_dir = documents.join("蒼の彼方のフォーリズムPerfect Edition");
        fs::create_dir_all(&game_dir).expect("game directory");
        fs::create_dir_all(japanese_documents.parent().expect("user directory"))
            .expect("user directory");
        symlink(&documents, &japanese_documents).expect("localized documents link");
        File::create(game_dir.join("BootStrap.exe")).expect("bootstrap fixture");
        File::create(game_dir.join("Uninstaller.exe")).expect("uninstaller fixture");
        let game = game_dir.join("蒼の彼方のフォーリズムPerfect Edition.exe");
        File::create(&game).expect("game fixture");

        let found =
            super::find_new_executables(&prefix, &BTreeSet::new()).expect("locate executables");

        assert_eq!(found, vec![game]);
    }
}
