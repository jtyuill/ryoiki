use std::{
    ffi::OsString,
    io,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Component, Path, PathBuf},
    process::{Child, Command},
};

use thiserror::Error;

use crate::profile::{LaunchProfile, Runner};

use super::{
    install::RuntimeCommands,
    prefix::{PrefixError, ensure_japanese_ui_font, prepare_locale},
};

#[derive(Debug, Error)]
pub enum LaunchError {
    #[error("saved prefix does not exist: {path}")]
    MissingPrefix { path: PathBuf },
    #[error("saved game executable does not exist: {path}")]
    MissingExecutable { path: PathBuf },
    #[error("saved executable is outside the prefix drive_c: {path}")]
    ExecutableOutsidePrefix { path: PathBuf },
    #[error(transparent)]
    Locale(#[from] PrefixError),
    #[error("cannot start game with {program}: {source}")]
    Start {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot wait for the game process: {0}")]
    Wait(#[source] io::Error),
    #[error("game exited unsuccessfully: {0}")]
    Exited(std::process::ExitStatus),
}

pub fn launch_game(
    profile: &LaunchProfile,
    data_root: &Path,
    commands: &RuntimeCommands,
) -> Result<Child, LaunchError> {
    if !profile.prefix.is_dir() {
        return Err(LaunchError::MissingPrefix {
            path: profile.prefix.clone(),
        });
    }
    if !profile.exe.is_file() {
        return Err(LaunchError::MissingExecutable {
            path: profile.exe.clone(),
        });
    }

    match profile.runner {
        Runner::Wine => launch_wine(profile, data_root, commands),
    }
}

pub fn run_game(
    profile: &LaunchProfile,
    data_root: &Path,
    commands: &RuntimeCommands,
) -> Result<(), LaunchError> {
    let mut child = launch_game(profile, data_root, commands)?;
    let status = child.wait().map_err(LaunchError::Wait)?;
    if status.success() {
        Ok(())
    } else {
        Err(LaunchError::Exited(status))
    }
}

fn launch_wine(
    profile: &LaunchProfile,
    data_root: &Path,
    commands: &RuntimeCommands,
) -> Result<Child, LaunchError> {
    let executable = windows_executable_path(&profile.exe, &profile.prefix)?;
    let locale = prepare_locale(
        &profile.locale,
        &data_root.join("locales"),
        &commands.locale,
        &commands.localedef,
    )?;
    ensure_japanese_ui_font(&profile.prefix, &locale, &commands.wine)?;
    let mut command = Command::new(&commands.wine);
    command
        .arg(executable)
        .current_dir(profile.exe.parent().unwrap_or_else(|| Path::new(".")))
        .env("WINEPREFIX", &profile.prefix);
    locale.apply(&mut command);
    command.spawn().map_err(|source| LaunchError::Start {
        program: commands.wine.clone(),
        source,
    })
}

fn windows_executable_path(executable: &Path, prefix: &Path) -> Result<OsString, LaunchError> {
    let drive_c =
        prefix
            .join("drive_c")
            .canonicalize()
            .map_err(|_| LaunchError::MissingPrefix {
                path: prefix.to_path_buf(),
            })?;
    let canonical_executable =
        executable
            .canonicalize()
            .map_err(|_| LaunchError::MissingExecutable {
                path: executable.to_path_buf(),
            })?;
    let relative = canonical_executable.strip_prefix(&drive_c).map_err(|_| {
        LaunchError::ExecutableOutsidePrefix {
            path: executable.to_path_buf(),
        }
    })?;
    let mut bytes = b"C:\\".to_vec();
    let mut first = true;
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(LaunchError::ExecutableOutsidePrefix {
                path: executable.to_path_buf(),
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
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use tempfile::tempdir;

    use crate::{
        profile::{LaunchProfile, PrefixArch, Runner},
        stages::install::RuntimeCommands,
    };

    #[test]
    fn launch_uses_c_drive_path_prefix_and_saved_locale() {
        let root = tempdir().expect("temporary directory");
        let prefix = root.path().join("prefix");
        let game_dir = prefix.join("drive_c/Game");
        fs::create_dir_all(&game_dir).expect("game directory");
        let executable = game_dir.join("game.exe");
        fs::write(&executable, []).expect("game executable");
        let tools = root.path().join("tools");
        fs::create_dir(&tools).expect("tools directory");
        let wine = tools.join("wine");
        let locale = tools.join("locale");
        write_executable(
            &wine,
            "#!/bin/sh\nif [ \"$1\" = reg ]; then exit 0; fi\nprintf '%s\\n%s\\n%s' \"$1\" \"$WINEPREFIX\" \"$LC_ALL\" > \"$WINEPREFIX/launch\"\n",
        );
        write_executable(&locale, "#!/bin/sh\nprintf 'ja_JP.utf8\\n'\n");
        let profile = LaunchProfile {
            exe: executable,
            prefix: prefix.clone(),
            arch: PrefixArch::Win64,
            runner: Runner::Wine,
            locale: "ja_JP.UTF-8".to_owned(),
            disc: None,
            winetricks: Vec::new(),
            vndb_id: None,
            notes: String::new(),
        };
        let commands = RuntimeCommands {
            seven_zip: PathBuf::from("unused"),
            wine,
            wineboot: PathBuf::from("unused"),
            locale,
            localedef: PathBuf::from("unused"),
        };

        let mut child = super::launch_game(&profile, root.path(), &commands).expect("launch game");
        assert!(child.wait().expect("wait for game").success());
        assert_eq!(
            fs::read_to_string(prefix.join("launch")).expect("launch record"),
            format!("C:\\Game\\game.exe\n{}\nja_JP.UTF-8", prefix.display())
        );
        assert!(prefix.join(".ryoiki-japanese-ui-font").is_file());
    }

    fn write_executable(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write fake tool");
        let mut permissions = fs::metadata(path)
            .expect("fake tool metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("mark fake tool executable");
    }
}
