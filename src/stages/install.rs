use std::{
    fs, io,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::profile::{DiscKind, DiscProfile, LaunchProfile, PrefixArch, Runner};

use super::{
    disc::{DiscError, PreparedDisc, map_disc, prepare_disc},
    extract::{ExtractError, PreparedSource, prepare_source},
    identify::IdentifiedSource,
    locate::{LocateError, find_new_executables, snapshot_executables},
    prefix::{LocaleEnvironment, PrefixError, create_prefix, prepare_locale},
    setup::{SetupError, find_installers, run_installer},
};

#[derive(Clone, Debug)]
pub struct RuntimeCommands {
    pub seven_zip: PathBuf,
    pub wine: PathBuf,
    pub wineboot: PathBuf,
    pub locale: PathBuf,
    pub localedef: PathBuf,
}

impl Default for RuntimeCommands {
    fn default() -> Self {
        Self {
            seven_zip: PathBuf::from("7z"),
            wine: PathBuf::from("wine"),
            wineboot: PathBuf::from("wineboot"),
            locale: PathBuf::from("locale"),
            localedef: PathBuf::from("localedef"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub title: String,
    pub source: IdentifiedSource,
    pub arch: PrefixArch,
    pub locale: String,
    pub data_root: PathBuf,
    pub commands: RuntimeCommands,
}

#[derive(Clone, Debug)]
pub struct PreparedInstallSource {
    pub request: InstallRequest,
    pub source: PreparedSource,
}

#[derive(Clone, Debug)]
pub struct PreparedInstall {
    pub request: InstallRequest,
    pub source: PreparedSource,
    pub disc: Option<PreparedDisc>,
    pub installers: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct InstallOutcome {
    pub session_dir: PathBuf,
    pub prefix: PathBuf,
    pub arch: PrefixArch,
    pub locale: String,
    pub disc: Option<PreparedDisc>,
    pub executables: Vec<PathBuf>,
}

impl InstallOutcome {
    pub fn launch_profile(&self, executable: PathBuf) -> Result<LaunchProfile, InstallError> {
        validate_executable(&self.prefix, &executable)?;
        Ok(LaunchProfile {
            exe: executable,
            prefix: self.prefix.clone(),
            arch: self.arch,
            runner: Runner::Wine,
            locale: self.locale.clone(),
            disc: self.disc.as_ref().map(|disc| DiscProfile {
                path: disc.root.clone(),
                drive: "d:".to_owned(),
                kind: DiscKind::Extracted,
            }),
            winetricks: Vec::new(),
            vndb_id: None,
            notes: if self.disc.is_some() {
                "Keep the extracted disc mapped as Wine drive d:.".to_owned()
            } else {
                String::new()
            },
        })
    }

    pub fn cleanup_after_save(&self) {
        if self.disc.is_some() {
            let _ = fs::remove_dir_all(self.session_dir.join("source"));
        } else {
            let _ = fs::remove_dir_all(&self.session_dir);
        }
    }

    pub fn discard(&self) {
        let _ = fs::remove_dir_all(&self.session_dir);
    }
}

impl PreparedInstallSource {
    pub fn discard(&self) {
        let _ = fs::remove_dir_all(&self.source.session_dir);
    }
}

impl PreparedInstall {
    pub fn discard(&self) {
        let _ = fs::remove_dir_all(&self.source.session_dir);
    }
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("cannot create setup workspace under {path}: {source}")]
    Workspace {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Extract(#[from] ExtractError),
    #[error("choose one of the detected disc images before continuing")]
    DiscSelectionRequired,
    #[error(transparent)]
    Disc(#[from] DiscError),
    #[error(transparent)]
    Setup(#[from] SetupError),
    #[error(transparent)]
    Prefix(#[from] PrefixError),
    #[error(transparent)]
    Locate(#[from] LocateError),
    #[error("selected executable does not exist as a file: {path}")]
    MissingExecutable { path: PathBuf },
    #[error("selected executable is outside this prefix's drive_c: {path}")]
    ExecutableOutsidePrefix { path: PathBuf },
    #[error("selected file is not a Windows executable: {path}")]
    InvalidExecutable { path: PathBuf },
}

pub fn prepare_install_source(
    request: InstallRequest,
) -> Result<PreparedInstallSource, InstallError> {
    let sessions = request.data_root.join("setups");
    let session_dir = create_unique_directory(&sessions, &slug(&request.title))?;
    let source = prepare_source(&request.source, session_dir, &request.commands.seven_zip)?;
    Ok(PreparedInstallSource { request, source })
}

pub fn inspect_install_source(
    prepared: PreparedInstallSource,
    selected_disc: Option<PathBuf>,
) -> Result<PreparedInstall, InstallError> {
    let disc = match (prepared.source.disc_images.len(), selected_disc) {
        (0, _) => None,
        (_, Some(path)) => Some(prepare_disc(
            &prepared.source,
            &path,
            &prepared.request.commands.seven_zip,
        )?),
        _ => return Err(InstallError::DiscSelectionRequired),
    };
    let root = disc
        .as_ref()
        .map(|disc| disc.root.as_path())
        .unwrap_or(&prepared.source.content_root);
    let exact_installer = if disc.is_some() {
        None
    } else {
        prepared.source.exact_installer.as_deref()
    };
    let installers = find_installers(root, exact_installer)?;

    Ok(PreparedInstall {
        request: prepared.request,
        source: prepared.source,
        disc,
        installers,
    })
}

pub fn execute_install(
    prepared: PreparedInstall,
    installer: PathBuf,
) -> Result<InstallOutcome, InstallError> {
    let prefixes = prepared.request.data_root.join("prefixes");
    fs::create_dir_all(&prefixes).map_err(|source| InstallError::Workspace {
        path: prefixes.clone(),
        source,
    })?;
    let prefix = next_available_path(&prefixes, &slug(&prepared.request.title));
    let locale = prepare_locale(
        &prepared.request.locale,
        &prepared.request.data_root.join("locales"),
        &prepared.request.commands.locale,
        &prepared.request.commands.localedef,
    )?;
    create_prefix(
        &prefix,
        prepared.request.arch,
        &locale,
        &prepared.request.commands.wineboot,
        &prepared.request.commands.wine,
    )?;
    let result = execute_with_prefix(&prepared, &installer, &prefix, &locale);
    match result {
        Ok(executables) => Ok(InstallOutcome {
            session_dir: prepared.source.session_dir,
            prefix,
            arch: prepared.request.arch,
            locale: prepared.request.locale,
            disc: prepared.disc,
            executables,
        }),
        Err(error) => {
            let _ = fs::remove_dir_all(&prefix);
            Err(error)
        }
    }
}

fn execute_with_prefix(
    prepared: &PreparedInstall,
    installer: &Path,
    prefix: &Path,
    locale: &LocaleEnvironment,
) -> Result<Vec<PathBuf>, InstallError> {
    if let Some(disc) = &prepared.disc {
        map_disc(disc, prefix, locale, &prepared.request.commands.wine)?;
    }
    let before = snapshot_executables(prefix)?;
    run_installer(
        installer,
        &prepared.installers,
        prepared.disc.as_ref(),
        prefix,
        locale,
        &prepared.request.commands.wine,
    )?;
    Ok(find_new_executables(prefix, &before)?)
}

fn validate_executable(prefix: &Path, executable: &Path) -> Result<(), InstallError> {
    if !executable.is_file() {
        return Err(InstallError::MissingExecutable {
            path: executable.to_path_buf(),
        });
    }
    if !super::extract::has_extension(executable, &[b"exe"]) {
        return Err(InstallError::InvalidExecutable {
            path: executable.to_path_buf(),
        });
    }
    let canonical_drive = prefix.join("drive_c").canonicalize().map_err(|_| {
        InstallError::ExecutableOutsidePrefix {
            path: executable.to_path_buf(),
        }
    })?;
    let canonical_executable =
        executable
            .canonicalize()
            .map_err(|_| InstallError::MissingExecutable {
                path: executable.to_path_buf(),
            })?;
    if !canonical_executable.starts_with(canonical_drive) {
        return Err(InstallError::ExecutableOutsidePrefix {
            path: executable.to_path_buf(),
        });
    }
    Ok(())
}

fn create_unique_directory(parent: &Path, stem: &str) -> Result<PathBuf, InstallError> {
    fs::create_dir_all(parent).map_err(|source| InstallError::Workspace {
        path: parent.to_path_buf(),
        source,
    })?;
    for suffix in 1..=10_000 {
        let candidate = numbered_path(parent, stem, suffix);
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(InstallError::Workspace {
                    path: candidate,
                    source,
                });
            }
        }
    }
    Err(InstallError::Workspace {
        path: parent.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no unused setup directory name",
        ),
    })
}

fn next_available_path(parent: &Path, stem: &str) -> PathBuf {
    (1..=10_000)
        .map(|suffix| numbered_path(parent, stem, suffix))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| parent.join(format!("{stem}-overflow")))
}

fn numbered_path(parent: &Path, stem: &str, suffix: usize) -> PathBuf {
    if suffix == 1 {
        parent.join(stem)
    } else {
        parent.join(format!("{stem}-{suffix}"))
    }
}

fn slug(title: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else if !slug.is_empty() && !separator {
            slug.push('-');
            separator = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "game".to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        os::unix::fs::PermissionsExt,
        path::Path,
    };

    use tempfile::tempdir;

    use super::{
        InstallOutcome, InstallRequest, RuntimeCommands, execute_install, inspect_install_source,
        prepare_install_source, slug,
    };
    use crate::{
        profile::PrefixArch,
        stages::identify::{ArchiveFormat, IdentifiedSource, SourceKind},
    };

    #[test]
    fn title_slug_produces_safe_stable_directory_names() {
        assert_eq!(slug("Subarashiki Hibi!"), "subarashiki-hibi");
        assert_eq!(slug("素晴らしき日々"), "game");
    }

    #[test]
    fn launch_profile_accepts_only_executables_inside_drive_c() {
        let root = tempdir().expect("temporary directory");
        let prefix = root.path().join("prefix");
        let game_dir = prefix.join("drive_c/Game");
        fs::create_dir_all(&game_dir).expect("game directory");
        let executable = game_dir.join("game.exe");
        File::create(&executable).expect("game executable");
        let outside = root.path().join("outside.exe");
        File::create(&outside).expect("outside executable");
        let outcome = InstallOutcome {
            session_dir: root.path().join("session"),
            prefix,
            arch: PrefixArch::Win32,
            locale: "ja_JP.UTF-8".to_owned(),
            disc: None,
            executables: vec![executable.clone()],
        };

        assert_eq!(
            outcome
                .launch_profile(executable.clone())
                .expect("valid profile")
                .exe,
            executable
        );
        assert!(outcome.launch_profile(outside).is_err());
    }

    #[test]
    fn archive_disc_pipeline_runs_mapped_installer_and_locates_game() {
        let root = tempdir().expect("temporary directory");
        let tools = root.path().join("tools");
        fs::create_dir(&tools).expect("tools directory");
        let seven_zip = tools.join("7z");
        let wineboot = tools.join("wineboot");
        let wine = tools.join("wine");
        let locale = tools.join("locale");
        let localedef = tools.join("localedef");
        write_executable(
            &seven_zip,
            r#"#!/bin/sh
out=
archive=
for arg in "$@"; do
    case "$arg" in
        -o*) out=${arg#-o} ;;
        --) ;;
        *) archive=$arg ;;
    esac
done
mkdir -p "$out"
case "$archive" in
    *.7z) : > "$out/disc.iso" ;;
    *.iso) : > "$out/setup.exe" ;;
    *) exit 2 ;;
esac
"#,
        );
        write_executable(
            &wineboot,
            r#"#!/bin/sh
mkdir -p "$WINEPREFIX/drive_c/windows" "$WINEPREFIX/drive_c/users/test" "$WINEPREFIX/dosdevices"
"#,
        );
        write_executable(
            &wine,
            r#"#!/bin/sh
if [ "$1" = "reg" ]; then
    exit 0
fi
mkdir -p "$WINEPREFIX/drive_c/Game"
printf '%s' "$1" > "$WINEPREFIX/installer-argument"
: > "$WINEPREFIX/drive_c/Game/game.exe"
"#,
        );
        write_executable(&locale, "#!/bin/sh\nprintf 'ja_JP.utf8\\n'\n");
        write_executable(&localedef, "#!/bin/sh\nexit 99\n");
        let archive = root.path().join("game.7z");
        File::create(&archive).expect("archive fixture");
        let request = InstallRequest {
            title: "Game".to_owned(),
            source: IdentifiedSource {
                path: archive,
                kind: SourceKind::Archive(ArchiveFormat::SevenZip),
            },
            arch: PrefixArch::Win64,
            locale: "ja_JP.UTF-8".to_owned(),
            data_root: root.path().join("data"),
            commands: RuntimeCommands {
                seven_zip,
                wine,
                wineboot,
                locale,
                localedef,
            },
        };

        let source = prepare_install_source(request).expect("prepare archive");
        let disc = source.source.disc_images[0].clone();
        let prepared = inspect_install_source(source, Some(disc)).expect("inspect disc");
        let installer = prepared.installers[0].clone();
        let outcome = execute_install(prepared, installer).expect("execute installer");

        assert_eq!(outcome.executables.len(), 1);
        assert_eq!(
            fs::read_to_string(outcome.prefix.join("installer-argument"))
                .expect("installer argument"),
            "D:\\setup.exe"
        );
        assert_eq!(
            fs::read_link(outcome.prefix.join("dosdevices/d:")).expect("d drive mapping"),
            outcome.disc.as_ref().expect("disc profile").root
        );
        let profile = outcome
            .launch_profile(outcome.executables[0].clone())
            .expect("launch profile");
        assert!(profile.disc.is_some());
        outcome.discard();
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
