use std::{
    ffi::OsString,
    io,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf},
    process::Command,
};

use thiserror::Error;

use super::{
    disc::PreparedDisc,
    extract::{ExtractError, has_extension, walk_files},
    prefix::LocaleEnvironment,
};

#[derive(Debug, Error)]
pub enum SetupError {
    #[error(transparent)]
    Inspect(#[from] ExtractError),
    #[error("no setup.exe or install.exe was found; this may be a portable folder")]
    NoInstaller,
    #[error("selected installer {path} is not one of the detected setup programs")]
    UnknownInstaller { path: PathBuf },
    #[error("cannot convert disc installer path {path} to a Wine drive path")]
    DiscPath { path: PathBuf },
    #[error("cannot start installer with {program}: {source}")]
    Start {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the installer exited unsuccessfully: {0}")]
    Failed(String),
}

pub fn find_installers(
    root: &Path,
    exact_installer: Option<&Path>,
) -> Result<Vec<PathBuf>, SetupError> {
    if let Some(installer) = exact_installer {
        return Ok(vec![installer.to_path_buf()]);
    }

    let mut installers = Vec::new();
    walk_files(root, &mut |path| {
        let name = path
            .file_name()
            .map(|name| name.as_bytes())
            .unwrap_or_default();
        if has_extension(path, &[b"exe"])
            && (name.eq_ignore_ascii_case(b"setup.exe")
                || name.eq_ignore_ascii_case(b"install.exe"))
        {
            installers.push(path.to_path_buf());
        }
    })?;
    installers.sort();
    if installers.is_empty() {
        Err(SetupError::NoInstaller)
    } else {
        Ok(installers)
    }
}

pub fn run_installer(
    installer: &Path,
    candidates: &[PathBuf],
    disc: Option<&PreparedDisc>,
    prefix: &Path,
    locale: &LocaleEnvironment,
    wine: &Path,
) -> Result<(), SetupError> {
    if !candidates.iter().any(|candidate| candidate == installer) {
        return Err(SetupError::UnknownInstaller {
            path: installer.to_path_buf(),
        });
    }

    let argument = match disc {
        Some(disc) if installer.starts_with(&disc.root) => {
            disc_windows_path(installer, &disc.root)?
        }
        _ => installer.as_os_str().to_os_string(),
    };
    let mut command = Command::new(wine);
    command
        .arg(argument)
        .current_dir(installer.parent().unwrap_or_else(|| Path::new(".")))
        .env("WINEPREFIX", prefix);
    locale.apply(&mut command);
    let status = command.status().map_err(|source| SetupError::Start {
        program: wine.to_path_buf(),
        source,
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(SetupError::Failed(status.to_string()))
    }
}

fn disc_windows_path(installer: &Path, disc_root: &Path) -> Result<OsString, SetupError> {
    let relative = installer
        .strip_prefix(disc_root)
        .map_err(|_| SetupError::DiscPath {
            path: installer.to_path_buf(),
        })?;
    let mut bytes = b"D:\\".to_vec();
    let mut first = true;
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(SetupError::DiscPath {
                path: installer.to_path_buf(),
            });
        };
        if !first {
            bytes.push(b'\\');
        }
        bytes.extend_from_slice(value.as_bytes());
        first = false;
    }
    Ok(OsString::from_vec(bytes))
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        os::unix::ffi::OsStrExt,
    };

    use tempfile::tempdir;

    #[test]
    fn installer_discovery_finds_only_conventional_setup_names() {
        let root = tempdir().expect("temporary directory");
        fs::create_dir(root.path().join("nested")).expect("nested directory");
        File::create(root.path().join("nested/SETUP.EXE")).expect("setup fixture");
        File::create(root.path().join("game.exe")).expect("game fixture");

        let installers = super::find_installers(root.path(), None).expect("find installer");

        assert_eq!(installers, vec![root.path().join("nested/SETUP.EXE")]);
    }

    #[test]
    fn disc_installer_uses_the_mapped_d_drive() {
        let root = tempdir().expect("temporary directory");
        let argument = super::disc_windows_path(&root.path().join("folder/setup.exe"), root.path())
            .expect("disc path");

        assert_eq!(argument.as_bytes(), b"D:\\folder\\setup.exe");
    }
}
