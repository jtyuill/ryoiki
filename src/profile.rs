use std::{
    env, fs, io,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use sqlx::{Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrefixArch {
    Win32,
    Win64,
}

impl PrefixArch {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Win32 => "win32",
            Self::Win64 => "win64",
        }
    }

    fn parse(value: &str) -> Result<Self, ProfileError> {
        match value {
            "win32" => Ok(Self::Win32),
            "win64" => Ok(Self::Win64),
            _ => Err(ProfileError::InvalidValue {
                field: "arch",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Runner {
    Wine,
}

impl Runner {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wine => "wine",
        }
    }

    fn parse(value: &str) -> Result<Self, ProfileError> {
        match value {
            "wine" => Ok(Self::Wine),
            _ => Err(ProfileError::InvalidValue {
                field: "runner",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscKind {
    Extracted,
}

impl DiscKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extracted => "extracted",
        }
    }

    fn parse(value: &str) -> Result<Self, ProfileError> {
        match value {
            "extracted" => Ok(Self::Extracted),
            _ => Err(ProfileError::InvalidValue {
                field: "disc_kind",
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscProfile {
    pub path: PathBuf,
    pub drive: String,
    pub kind: DiscKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchProfile {
    pub exe: PathBuf,
    pub prefix: PathBuf,
    pub arch: PrefixArch,
    pub runner: Runner,
    pub locale: String,
    pub disc: Option<DiscProfile>,
    pub winetricks: Vec<String>,
    pub vndb_id: Option<String>,
    pub notes: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredProfile {
    pub id: i64,
    pub title: String,
    pub profile: LaunchProfile,
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("cannot determine the application data directory; set XDG_DATA_HOME or HOME")]
    DataDirectory,
    #[error("cannot create {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot start the profile database runtime: {0}")]
    Runtime(#[source] io::Error),
    #[error("profile database failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("profile field {field} has unsupported value {value:?}")]
    InvalidValue { field: &'static str, value: String },
    #[error("cannot encode winetricks verbs: {0}")]
    EncodeWinetricks(#[source] serde_json::Error),
    #[error("cannot decode winetricks verbs: {0}")]
    DecodeWinetricks(#[source] serde_json::Error),
}

pub fn data_root() -> Result<PathBuf, ProfileError> {
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Ok(path.join("ryoiki"));
        }
    }

    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".local/share/ryoiki"))
        .ok_or(ProfileError::DataDirectory)
}

#[derive(Clone, Debug)]
pub struct ProfileStore {
    database_path: PathBuf,
}

impl ProfileStore {
    pub fn new(data_root: &Path) -> Self {
        Self {
            database_path: data_root.join("profiles.sqlite3"),
        }
    }

    pub fn load(&self) -> Result<Vec<StoredProfile>, ProfileError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(ProfileError::Runtime)?;
        runtime.block_on(self.load_async())
    }

    pub fn save(
        &self,
        title: &str,
        profile: &LaunchProfile,
    ) -> Result<StoredProfile, ProfileError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(ProfileError::Runtime)?;
        runtime.block_on(self.save_async(title, profile))
    }

    pub fn delete(&self, id: i64) -> Result<(), ProfileError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(ProfileError::Runtime)?;
        runtime.block_on(self.delete_async(id))
    }

    pub fn reorder(&self, ids: &[i64]) -> Result<(), ProfileError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(ProfileError::Runtime)?;
        runtime.block_on(self.reorder_async(ids))
    }

    async fn connect(&self) -> Result<SqliteConnection, ProfileError> {
        if let Some(parent) = self.database_path.parent() {
            fs::create_dir_all(parent).map_err(|source| ProfileError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let options = SqliteConnectOptions::new()
            .filename(&self.database_path)
            .create_if_missing(true);
        let mut connection = SqliteConnection::connect_with(&options).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS launch_profiles (\
                id INTEGER PRIMARY KEY,\
                title TEXT NOT NULL,\
                exe BLOB NOT NULL,\
                prefix BLOB NOT NULL,\
                arch TEXT NOT NULL,\
                runner TEXT NOT NULL,\
                locale TEXT NOT NULL,\
                disc_path BLOB,\
                disc_drive TEXT,\
                disc_kind TEXT,\
                winetricks TEXT NOT NULL,\
                vndb_id TEXT,\
                notes TEXT NOT NULL,\
                position INTEGER\
            )",
        )
        .execute(&mut connection)
        .await?;
        let columns = sqlx::query("PRAGMA table_info(launch_profiles)")
            .fetch_all(&mut connection)
            .await?;
        let has_position = columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .is_ok_and(|name| name == "position")
        });
        if !has_position {
            sqlx::query("ALTER TABLE launch_profiles ADD COLUMN position INTEGER")
                .execute(&mut connection)
                .await?;
        }
        sqlx::query("UPDATE launch_profiles SET position = id WHERE position IS NULL")
            .execute(&mut connection)
            .await?;
        Ok(connection)
    }

    async fn load_async(&self) -> Result<Vec<StoredProfile>, ProfileError> {
        let mut connection = self.connect().await?;
        let rows = sqlx::query(
            "SELECT id, title, exe, prefix, arch, runner, locale, disc_path, \
                    disc_drive, disc_kind, winetricks, vndb_id, notes \
             FROM launch_profiles ORDER BY COALESCE(position, id), id",
        )
        .fetch_all(&mut connection)
        .await?;

        rows.into_iter().map(decode_profile).collect()
    }

    async fn delete_async(&self, id: i64) -> Result<(), ProfileError> {
        let mut connection = self.connect().await?;
        sqlx::query("DELETE FROM launch_profiles WHERE id = ?")
            .bind(id)
            .execute(&mut connection)
            .await?;
        Ok(())
    }

    async fn reorder_async(&self, ids: &[i64]) -> Result<(), ProfileError> {
        let mut connection = self.connect().await?;
        let mut transaction = connection.begin().await?;
        for (position, id) in ids.iter().enumerate() {
            sqlx::query("UPDATE launch_profiles SET position = ? WHERE id = ?")
                .bind(position as i64)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn save_async(
        &self,
        title: &str,
        profile: &LaunchProfile,
    ) -> Result<StoredProfile, ProfileError> {
        let mut connection = self.connect().await?;
        let winetricks =
            serde_json::to_string(&profile.winetricks).map_err(ProfileError::EncodeWinetricks)?;
        let (disc_path, disc_drive, disc_kind) = match &profile.disc {
            Some(disc) => (
                Some(path_bytes(&disc.path)),
                Some(disc.drive.as_str()),
                Some(disc.kind.as_str()),
            ),
            None => (None, None, None),
        };
        let position: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(position), -1) + 1 FROM launch_profiles")
                .fetch_one(&mut connection)
                .await?;

        let result = sqlx::query(
            "INSERT INTO launch_profiles (\
                title, exe, prefix, arch, runner, locale, disc_path, disc_drive, \
                disc_kind, winetricks, vndb_id, notes, position\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(title)
        .bind(path_bytes(&profile.exe))
        .bind(path_bytes(&profile.prefix))
        .bind(profile.arch.as_str())
        .bind(profile.runner.as_str())
        .bind(&profile.locale)
        .bind(disc_path)
        .bind(disc_drive)
        .bind(disc_kind)
        .bind(winetricks)
        .bind(&profile.vndb_id)
        .bind(&profile.notes)
        .bind(position)
        .execute(&mut connection)
        .await?;

        Ok(StoredProfile {
            id: result.last_insert_rowid(),
            title: title.to_owned(),
            profile: profile.clone(),
        })
    }
}

fn decode_profile(row: sqlx::sqlite::SqliteRow) -> Result<StoredProfile, ProfileError> {
    let disc_path: Option<Vec<u8>> = row.try_get("disc_path")?;
    let disc_drive: Option<String> = row.try_get("disc_drive")?;
    let disc_kind: Option<String> = row.try_get("disc_kind")?;
    let disc = match (disc_path, disc_drive, disc_kind) {
        (None, None, None) => None,
        (Some(path), Some(drive), Some(kind)) => Some(DiscProfile {
            path: bytes_path(path),
            drive,
            kind: DiscKind::parse(&kind)?,
        }),
        _ => {
            return Err(ProfileError::InvalidValue {
                field: "disc",
                value: "incomplete disc fields".to_owned(),
            });
        }
    };
    let winetricks: String = row.try_get("winetricks")?;

    Ok(StoredProfile {
        id: row.try_get("id")?,
        title: row.try_get("title")?,
        profile: LaunchProfile {
            exe: bytes_path(row.try_get("exe")?),
            prefix: bytes_path(row.try_get("prefix")?),
            arch: PrefixArch::parse(&row.try_get::<String, _>("arch")?)?,
            runner: Runner::parse(&row.try_get::<String, _>("runner")?)?,
            locale: row.try_get("locale")?,
            disc,
            winetricks: serde_json::from_str(&winetricks)
                .map_err(ProfileError::DecodeWinetricks)?,
            vndb_id: row.try_get("vndb_id")?,
            notes: row.try_get("notes")?,
        },
    })
}

fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}

fn bytes_path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

    use tempfile::tempdir;

    use super::{DiscKind, DiscProfile, LaunchProfile, PrefixArch, ProfileStore, Runner};

    #[test]
    fn profile_round_trip_preserves_the_launch_schema_and_linux_paths() {
        let root = tempdir().expect("temporary directory");
        let store = ProfileStore::new(root.path());
        let profile = LaunchProfile {
            exe: PathBuf::from(OsString::from_vec(b"/tmp/game-\x81.exe".to_vec())),
            prefix: root.path().join("prefix"),
            arch: PrefixArch::Win32,
            runner: Runner::Wine,
            locale: "ja_JP.UTF-8".to_owned(),
            disc: Some(DiscProfile {
                path: root.path().join("disc"),
                drive: "d:".to_owned(),
                kind: DiscKind::Extracted,
            }),
            winetricks: vec!["corefonts".to_owned()],
            vndb_id: Some("v1".to_owned()),
            notes: "Keep disc mapped".to_owned(),
        };

        let stored = store.save("Game", &profile).expect("save profile");
        let loaded = store.load().expect("load profiles");

        assert_eq!(loaded, vec![stored.clone()]);
        assert_eq!(loaded[0].profile, profile);

        let second = store.save("Second", &profile).expect("save second profile");
        store
            .reorder(&[second.id, stored.id])
            .expect("reorder profiles");
        assert_eq!(
            store
                .load()
                .expect("load reordered profiles")
                .into_iter()
                .map(|profile| profile.title)
                .collect::<Vec<_>>(),
            vec!["Second", "Game"]
        );

        store.delete(second.id).expect("delete profile");
        assert_eq!(store.load().expect("load remaining profiles"), vec![stored]);
    }
}
