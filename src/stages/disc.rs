use std::{
    fs, io,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

use super::{
    extract::{ExtractError, PreparedSource, extract_with_7z},
    prefix::LocaleEnvironment,
};

#[derive(Clone, Debug)]
pub struct PreparedDisc {
    pub root: PathBuf,
}

#[derive(Debug, Error)]
pub enum DiscError {
    #[error("selected disc {path} is not one of the detected images")]
    UnknownSelection { path: PathBuf },
    #[error("cannot reset extracted disc directory {path}: {source}")]
    ResetDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Extract(#[from] ExtractError),
    #[error("Wine prefix is missing its dosdevices directory at {path}")]
    MissingDosDevices { path: PathBuf },
    #[error("cannot replace Wine drive mapping {path}: {source}")]
    ReplaceMapping {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot map extracted disc {target} as {link}: {source}")]
    Map {
        target: PathBuf,
        link: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot start {program} to mark drive d: as a CD-ROM: {source}")]
    StartWine {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Wine could not mark drive d: as a CD-ROM: {message}")]
    WineFailed { message: String },
}

pub fn prepare_disc(
    source: &PreparedSource,
    selected_image: &Path,
    seven_zip: &Path,
) -> Result<PreparedDisc, DiscError> {
    if !source
        .disc_images
        .iter()
        .any(|candidate| candidate == selected_image)
    {
        return Err(DiscError::UnknownSelection {
            path: selected_image.to_path_buf(),
        });
    }

    let root = source.session_dir.join("disc");
    if root.exists() {
        fs::remove_dir_all(&root).map_err(|source| DiscError::ResetDirectory {
            path: root.clone(),
            source,
        })?;
    }
    fs::create_dir_all(&root).map_err(|source| DiscError::ResetDirectory {
        path: root.clone(),
        source,
    })?;
    extract_with_7z(seven_zip, selected_image, &root)?;

    Ok(PreparedDisc { root })
}

pub fn map_disc(
    disc: &PreparedDisc,
    prefix: &Path,
    locale: &LocaleEnvironment,
    wine: &Path,
) -> Result<(), DiscError> {
    let dosdevices = prefix.join("dosdevices");
    if !dosdevices.is_dir() {
        return Err(DiscError::MissingDosDevices { path: dosdevices });
    }

    let drive = dosdevices.join("d:");
    if drive.symlink_metadata().is_ok() {
        fs::remove_file(&drive).map_err(|source| DiscError::ReplaceMapping {
            path: drive.clone(),
            source,
        })?;
    }
    symlink(&disc.root, &drive).map_err(|source| DiscError::Map {
        target: disc.root.clone(),
        link: drive,
        source,
    })?;

    let mut command = Command::new(wine);
    command
        .arg("reg")
        .arg("add")
        .arg(r"HKCU\Software\Wine\Drives")
        .arg("/v")
        .arg("d:")
        .arg("/d")
        .arg("cdrom")
        .arg("/f")
        .env("WINEPREFIX", prefix);
    locale.apply(&mut command);
    let output = command.output().map_err(|source| DiscError::StartWine {
        program: wine.to_path_buf(),
        source,
    })?;
    if output.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(DiscError::WineFailed {
            message: if message.is_empty() {
                output.status.to_string()
            } else {
                message
            },
        })
    }
}
