use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

use super::identify::{IdentifiedSource, SourceKind};

#[derive(Clone, Debug)]
pub struct PreparedSource {
    pub session_dir: PathBuf,
    pub content_root: PathBuf,
    pub exact_installer: Option<PathBuf>,
    pub disc_images: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("{path} is not a supported install source")]
    Unsupported { path: PathBuf },
    #[error("cannot create setup directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot inspect setup files under {path}: {source}")]
    Inspect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot start {program}: {source}")]
    Start {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("archive extraction failed with {program}: {message}")]
    Failed { program: PathBuf, message: String },
}

pub fn prepare_source(
    source: &IdentifiedSource,
    session_dir: PathBuf,
    seven_zip: &Path,
) -> Result<PreparedSource, ExtractError> {
    fs::create_dir_all(&session_dir).map_err(|error| ExtractError::CreateDirectory {
        path: session_dir.clone(),
        source: error,
    })?;

    let result = prepare_source_inner(source, session_dir.clone(), seven_zip);
    if result.is_err() {
        let _ = fs::remove_dir_all(&session_dir);
    }
    result
}

fn prepare_source_inner(
    source: &IdentifiedSource,
    session_dir: PathBuf,
    seven_zip: &Path,
) -> Result<PreparedSource, ExtractError> {
    match source.kind {
        SourceKind::Archive(_) => {
            let content_root = session_dir.join("source");
            fs::create_dir_all(&content_root).map_err(|error| ExtractError::CreateDirectory {
                path: content_root.clone(),
                source: error,
            })?;
            extract_with_7z(seven_zip, &source.path, &content_root)?;
            let disc_images = find_disc_images(&content_root)?;
            Ok(PreparedSource {
                session_dir,
                content_root,
                exact_installer: None,
                disc_images,
            })
        }
        SourceKind::Directory => {
            let disc_images = find_disc_images(&source.path)?;
            Ok(PreparedSource {
                session_dir,
                content_root: source.path.clone(),
                exact_installer: None,
                disc_images,
            })
        }
        SourceKind::DiscImage(_) => Ok(PreparedSource {
            session_dir,
            content_root: source
                .path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            exact_installer: None,
            disc_images: vec![source.path.clone()],
        }),
        SourceKind::InstallerExecutable => Ok(PreparedSource {
            session_dir,
            content_root: source
                .path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            exact_installer: Some(source.path.clone()),
            disc_images: Vec::new(),
        }),
        SourceKind::OtherFile => Err(ExtractError::Unsupported {
            path: source.path.clone(),
        }),
    }
}

pub fn extract_with_7z(
    seven_zip: &Path,
    archive: &Path,
    destination: &Path,
) -> Result<(), ExtractError> {
    let mut output_argument = OsString::from("-o");
    output_argument.push(destination.as_os_str());
    let output = Command::new(seven_zip)
        .arg("x")
        .arg("-y")
        .arg(output_argument)
        .arg("--")
        .arg(archive)
        .output()
        .map_err(|source| ExtractError::Start {
            program: seven_zip.to_path_buf(),
            source,
        })?;

    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(ExtractError::Failed {
            program: seven_zip.to_path_buf(),
            message: if message.is_empty() {
                output.status.to_string()
            } else {
                message
            },
        })
    }
}

fn find_disc_images(root: &Path) -> Result<Vec<PathBuf>, ExtractError> {
    let mut images = Vec::new();
    walk_files(root, &mut |path| {
        if has_extension(path, &[b"iso", b"mds"]) {
            images.push(path.to_path_buf());
        }
    })?;
    images.sort();
    Ok(images)
}

pub(crate) fn walk_files(root: &Path, visitor: &mut impl FnMut(&Path)) -> Result<(), ExtractError> {
    let entries = fs::read_dir(root).map_err(|source| ExtractError::Inspect {
        path: root.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ExtractError::Inspect {
            path: root.to_path_buf(),
            source,
        })?;
        let file_type = entry.file_type().map_err(|source| ExtractError::Inspect {
            path: entry.path(),
            source,
        })?;
        if file_type.is_dir() && !file_type.is_symlink() {
            walk_files(&entry.path(), visitor)?;
        } else if file_type.is_file() {
            visitor(&entry.path());
        }
    }
    Ok(())
}

pub(crate) fn has_extension(path: &Path, extensions: &[&[u8]]) -> bool {
    path.extension().map(OsStr::as_bytes).is_some_and(|actual| {
        extensions
            .iter()
            .any(|expected| actual.eq_ignore_ascii_case(expected))
    })
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};

    use tempfile::tempdir;

    use crate::stages::identify::{IdentifiedSource, SourceKind};

    use super::prepare_source;

    #[test]
    fn directory_preparation_finds_nested_disc_images_in_stable_order() {
        let root = tempdir().expect("temporary directory");
        let nested = root.path().join("nested");
        fs::create_dir(&nested).expect("nested directory");
        File::create(nested.join("disc2.MDS")).expect("mds fixture");
        File::create(root.path().join("disc1.iso")).expect("iso fixture");
        File::create(root.path().join("readme.txt")).expect("text fixture");
        let session = root.path().join("session");

        let prepared = prepare_source(
            &IdentifiedSource {
                path: root.path().to_path_buf(),
                kind: SourceKind::Directory,
            },
            session,
            std::path::Path::new("7z"),
        )
        .expect("prepare directory");

        assert_eq!(prepared.disc_images.len(), 2);
        assert!(prepared.disc_images[0].ends_with("disc1.iso"));
        assert!(prepared.disc_images[1].ends_with("disc2.MDS"));
    }

    #[test]
    fn unsupported_files_are_rejected_before_running_tools() {
        let root = tempdir().expect("temporary directory");
        let result = prepare_source(
            &IdentifiedSource {
                path: root.path().join("notes.txt"),
                kind: SourceKind::OtherFile,
            },
            root.path().join("session"),
            std::path::Path::new("7z"),
        );

        assert!(result.is_err());
    }
}
