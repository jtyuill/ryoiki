use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

use crate::profile::PrefixArch;

#[derive(Clone, Debug)]
pub struct LocaleEnvironment {
    name: String,
    search_path: Option<PathBuf>,
}

impl LocaleEnvironment {
    pub fn apply(&self, command: &mut Command) {
        command.env("LANG", &self.name).env("LC_ALL", &self.name);
        if let Some(search_path) = &self.search_path {
            command.env("LOCPATH", search_path);
        }
    }
}

#[derive(Debug, Error)]
pub enum PrefixError {
    #[error("cannot create prefix parent {path}: {source}")]
    CreateParent {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot isolate Wine user folder {path}: {source}")]
    IsolateUserFolder {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot record Japanese font configuration at {path}: {source}")]
    RecordFontConfiguration {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot create private locale directory {path}: {source}")]
    CreateLocaleDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("locale {locale} is unavailable and cannot be generated safely")]
    UnsupportedLocale { locale: String },
    #[error("cannot start {program} to generate locale {locale}: {source}")]
    StartLocaleCompiler {
        program: PathBuf,
        locale: String,
        #[source]
        source: io::Error,
    },
    #[error("cannot generate locale {locale} with {program}: {message}")]
    LocaleCompilerFailed {
        program: PathBuf,
        locale: String,
        message: String,
    },
    #[error("prefix destination already exists: {path}")]
    AlreadyExists { path: PathBuf },
    #[error("cannot start {program}: {source}")]
    Start {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "this Wine build cannot create 32-bit-only prefixes. Choose the 64-bit WoW64 prefix; it also runs 32-bit games"
    )]
    UnsupportedArchitecture,
    #[error("wineboot failed for {path}: {message}")]
    Failed { path: PathBuf, message: String },
    #[error("cannot configure Japanese UI fonts for prefix {path}: {message}")]
    FontConfiguration { path: PathBuf, message: String },
}

pub fn prepare_locale(
    locale: &str,
    private_root: &Path,
    locale_program: &Path,
    localedef: &Path,
) -> Result<LocaleEnvironment, PrefixError> {
    if system_locale_available(locale, locale_program) {
        return Ok(LocaleEnvironment {
            name: locale.to_owned(),
            search_path: None,
        });
    }

    let destination = private_root.join(locale);
    if destination.is_dir() {
        return Ok(LocaleEnvironment {
            name: locale.to_owned(),
            search_path: Some(private_root.to_path_buf()),
        });
    }
    let (input, charmap) =
        locale_definition(locale).ok_or_else(|| PrefixError::UnsupportedLocale {
            locale: locale.to_owned(),
        })?;
    fs::create_dir_all(private_root).map_err(|source| PrefixError::CreateLocaleDirectory {
        path: private_root.to_path_buf(),
        source,
    })?;

    let output = Command::new(localedef)
        .arg("--no-archive")
        .arg("--inputfile")
        .arg(input)
        .arg("--charmap")
        .arg(charmap)
        .arg(&destination)
        .output()
        .map_err(|source| PrefixError::StartLocaleCompiler {
            program: localedef.to_path_buf(),
            locale: locale.to_owned(),
            source,
        })?;
    if !output.status.success() || !destination.is_dir() {
        let _ = fs::remove_dir_all(&destination);
        let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(PrefixError::LocaleCompilerFailed {
            program: localedef.to_path_buf(),
            locale: locale.to_owned(),
            message: if message.is_empty() {
                output.status.to_string()
            } else {
                message
            },
        });
    }

    Ok(LocaleEnvironment {
        name: locale.to_owned(),
        search_path: Some(private_root.to_path_buf()),
    })
}

pub fn create_prefix(
    path: &Path,
    arch: PrefixArch,
    locale: &LocaleEnvironment,
    wineboot: &Path,
    wine: &Path,
) -> Result<(), PrefixError> {
    if path.exists() {
        return Err(PrefixError::AlreadyExists {
            path: path.to_path_buf(),
        });
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| PrefixError::CreateParent {
        path: parent.to_path_buf(),
        source,
    })?;

    let mut command = Command::new(wineboot);
    command
        .arg("-u")
        .env("WINEPREFIX", path)
        .env("WINEARCH", arch.as_str());
    locale.apply(&mut command);
    let output = command.output().map_err(|source| PrefixError::Start {
        program: wineboot.to_path_buf(),
        source,
    })?;
    if output.status.success() {
        if let Err(error) =
            isolate_user_folders(path).and_then(|()| ensure_japanese_ui_font(path, locale, wine))
        {
            let _ = fs::remove_dir_all(path);
            return Err(error);
        }
        return Ok(());
    }

    let _ = fs::remove_dir_all(path);
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if arch == PrefixArch::Win32
        && message
            .to_ascii_lowercase()
            .contains("not supported in wow64 mode")
    {
        Err(PrefixError::UnsupportedArchitecture)
    } else {
        Err(PrefixError::Failed {
            path: path.to_path_buf(),
            message: if message.is_empty() {
                output.status.to_string()
            } else {
                message
            },
        })
    }
}

pub fn ensure_japanese_ui_font(
    prefix: &Path,
    locale: &LocaleEnvironment,
    wine: &Path,
) -> Result<(), PrefixError> {
    const MARKER: &str = ".ryoiki-japanese-ui-font";
    const FONT: &str = "Noto Sans CJK JP";
    const SUBSTITUTES: [&str; 5] = [
        "MS Shell Dlg",
        "MS Shell Dlg 2",
        "MS UI Gothic",
        "Meiryo UI",
        "Yu Gothic UI",
    ];

    let marker = prefix.join(MARKER);
    if marker.is_file() {
        return Ok(());
    }

    for name in SUBSTITUTES {
        let mut command = Command::new(wine);
        command
            .args([
                "reg",
                "add",
                r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\FontSubstitutes",
                "/v",
                name,
                "/t",
                "REG_SZ",
                "/d",
                FONT,
                "/f",
            ])
            .env("WINEPREFIX", prefix);
        locale.apply(&mut command);
        let output = command.output().map_err(|source| PrefixError::Start {
            program: wine.to_path_buf(),
            source,
        })?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(PrefixError::FontConfiguration {
                path: prefix.to_path_buf(),
                message: if message.is_empty() {
                    output.status.to_string()
                } else {
                    message
                },
            });
        }
    }

    fs::write(&marker, FONT).map_err(|source| PrefixError::RecordFontConfiguration {
        path: marker,
        source,
    })
}

fn isolate_user_folders(prefix: &Path) -> Result<(), PrefixError> {
    const HOST_FOLDER_LINKS: [&str; 7] = [
        "Desktop",
        "Documents",
        "Downloads",
        "Music",
        "My Documents",
        "Pictures",
        "Videos",
    ];

    let users = prefix.join("drive_c/users");
    let entries = fs::read_dir(&users).map_err(|source| PrefixError::IsolateUserFolder {
        path: users.clone(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| PrefixError::IsolateUserFolder {
            path: users.clone(),
            source,
        })?;
        let file_type = entry
            .file_type()
            .map_err(|source| PrefixError::IsolateUserFolder {
                path: entry.path(),
                source,
            })?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }

        for folder_name in HOST_FOLDER_LINKS {
            let folder = entry.path().join(folder_name);
            match fs::symlink_metadata(&folder) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    fs::remove_file(&folder).map_err(|source| PrefixError::IsolateUserFolder {
                        path: folder.clone(),
                        source,
                    })?;
                    fs::create_dir(&folder).map_err(|source| PrefixError::IsolateUserFolder {
                        path: folder.clone(),
                        source,
                    })?;
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(PrefixError::IsolateUserFolder {
                        path: folder,
                        source,
                    });
                }
            }
        }
    }
    Ok(())
}

fn system_locale_available(requested: &str, locale_program: &Path) -> bool {
    let Ok(output) = Command::new(locale_program).arg("-a").output() else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|available| normalize_locale(available) == normalize_locale(requested))
}

fn locale_definition(locale: &str) -> Option<(&str, &str)> {
    let (input, charmap) = locale.split_once('.')?;
    (normalize_locale(charmap) == "utf8").then_some((input, "UTF-8"))
}

fn normalize_locale(locale: &str) -> String {
    locale
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    use tempfile::tempdir;

    use crate::profile::PrefixArch;

    #[test]
    fn missing_japanese_locale_is_generated_in_private_search_path() {
        let root = tempdir().expect("temporary directory");
        let locale = root.path().join("locale");
        let localedef = root.path().join("localedef");
        write_executable(&locale, "#!/bin/sh\nprintf 'C\\nC.utf8\\n'\n");
        write_executable(
            &localedef,
            "#!/bin/sh\nfor argument in \"$@\"; do destination=$argument; done\nmkdir -p \"$destination\"\n",
        );

        let environment = super::prepare_locale(
            "ja_JP.UTF-8",
            &root.path().join("generated"),
            &locale,
            &localedef,
        )
        .expect("generate private locale");

        assert_eq!(environment.name, "ja_JP.UTF-8");
        assert_eq!(environment.search_path, Some(root.path().join("generated")));
    }

    #[test]
    fn wow64_failure_explains_that_the_win64_prefix_runs_32_bit_games() {
        let root = tempdir().expect("temporary directory");
        let wineboot = root.path().join("wineboot");
        write_executable(
            &wineboot,
            "#!/bin/sh\nprintf '%s\\n' \"wine: WINEARCH is set to 'win32' but this is not supported in wow64 mode.\" >&2\nexit 1\n",
        );
        let locale = super::LocaleEnvironment {
            name: "C.UTF-8".to_owned(),
            search_path: None,
        };

        let error = super::create_prefix(
            &root.path().join("prefix"),
            PrefixArch::Win32,
            &locale,
            &wineboot,
            &wineboot,
        )
        .expect_err("win32 prefix should fail");

        assert!(matches!(error, super::PrefixError::UnsupportedArchitecture));
        assert!(error.to_string().contains("also runs 32-bit games"));
    }

    #[test]
    fn prefix_replaces_host_shell_folder_links_with_private_directories() {
        let root = tempdir().expect("temporary directory");
        let wineboot = root.path().join("wineboot");
        let wine = root.path().join("wine");
        write_executable(
            &wine,
            "#!/bin/sh\nprintf '%s=%s\\n' \"$5\" \"$9\" >> \"$WINEPREFIX/font-substitutes\"\n",
        );
        write_executable(
            &wineboot,
            "#!/bin/sh\nuser=\"$WINEPREFIX/drive_c/users/test\"\nmkdir -p \"$user/AppData\"\nln -s /tmp/host-documents \"$user/Documents\"\nln -s /tmp/host-desktop \"$user/Desktop\"\n",
        );
        let locale = super::LocaleEnvironment {
            name: "C.UTF-8".to_owned(),
            search_path: None,
        };
        let prefix = root.path().join("prefix");

        super::create_prefix(&prefix, PrefixArch::Win64, &locale, &wineboot, &wine)
            .expect("create isolated prefix");

        for folder in ["Documents", "Desktop"] {
            let path = prefix.join("drive_c/users/test").join(folder);
            assert!(path.is_dir());
            assert!(
                !fs::symlink_metadata(path)
                    .expect("isolated folder metadata")
                    .file_type()
                    .is_symlink()
            );
        }
        assert!(prefix.join("drive_c/users/test/AppData").is_dir());
        let substitutions =
            fs::read_to_string(prefix.join("font-substitutes")).expect("font substitutions");
        assert!(substitutions.contains("MS Shell Dlg=Noto Sans CJK JP"));
        assert!(substitutions.contains("MS Shell Dlg 2=Noto Sans CJK JP"));
        assert!(prefix.join(".ryoiki-japanese-ui-font").is_file());
    }

    #[test]
    fn locale_name_matching_accepts_common_utf8_spellings() {
        assert_eq!(
            super::normalize_locale("ja_JP.utf8"),
            super::normalize_locale("ja_JP.UTF-8")
        );
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
