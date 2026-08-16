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

    fn parse(value: &str) -> Result<Self, LibraryError> {
        match value {
            "win32" => Ok(Self::Win32),
            "win64" => Ok(Self::Win64),
            _ => Err(LibraryError::InvalidValue {
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

    fn parse(value: &str) -> Result<Self, LibraryError> {
        match value {
            "wine" => Ok(Self::Wine),
            _ => Err(LibraryError::InvalidValue {
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

    fn parse(value: &str) -> Result<Self, LibraryError> {
        match value {
            "extracted" => Ok(Self::Extracted),
            _ => Err(LibraryError::InvalidValue {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameMetadata {
    pub title: String,
    pub alttitle: Option<String>,
    pub released: Option<String>,
    pub vndb_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredGame {
    pub id: i64,
    pub metadata: GameMetadata,
    pub thumbnail: Option<Vec<u8>>,
    pub profile: Option<StoredProfile>,
    pub playtime_seconds: i64,
    pub last_played_at: Option<i64>,
}

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("cannot determine the application data directory; set XDG_DATA_HOME or HOME")]
    DataDirectory,
    #[error("cannot create {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot start the library database runtime: {0}")]
    Runtime(#[source] io::Error),
    #[error("library database failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("library field {field} has unsupported value {value:?}")]
    InvalidValue { field: &'static str, value: String },
    #[error("library game {id} does not exist")]
    MissingGame { id: i64 },
    #[error("library game {id} already has installed files")]
    AlreadyInstalled { id: i64 },
    #[error("cannot encode winetricks verbs: {0}")]
    EncodeWinetricks(#[source] serde_json::Error),
    #[error("cannot decode winetricks verbs: {0}")]
    DecodeWinetricks(#[source] serde_json::Error),
}

pub fn data_root() -> Result<PathBuf, LibraryError> {
    if let Some(path) = env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return Ok(path.join("ryoiki"));
        }
    }

    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".local/share/ryoiki"))
        .ok_or(LibraryError::DataDirectory)
}

#[derive(Clone, Debug)]
pub struct LibraryStore {
    database_path: PathBuf,
}

impl LibraryStore {
    pub fn new(data_root: &Path) -> Self {
        Self {
            database_path: data_root.join("profiles.sqlite3"),
        }
    }

    pub fn load(&self) -> Result<Vec<StoredGame>, LibraryError> {
        self.runtime()?.block_on(self.load_async())
    }

    pub fn add_game(
        &self,
        metadata: &GameMetadata,
        thumbnail: Option<&[u8]>,
    ) -> Result<StoredGame, LibraryError> {
        self.runtime()?
            .block_on(self.add_game_async(metadata, thumbnail))
    }

    pub fn attach_profile(
        &self,
        game_id: i64,
        profile: &LaunchProfile,
    ) -> Result<StoredProfile, LibraryError> {
        self.runtime()?
            .block_on(self.attach_profile_async(game_id, profile))
    }

    pub fn update_metadata(
        &self,
        game_id: i64,
        metadata: &GameMetadata,
        thumbnail: Option<&[u8]>,
    ) -> Result<(), LibraryError> {
        self.runtime()?
            .block_on(self.update_metadata_async(game_id, metadata, thumbnail))
    }

    pub fn delete(&self, game_id: i64) -> Result<(), LibraryError> {
        self.runtime()?.block_on(self.delete_async(game_id))
    }

    pub fn reorder(&self, game_ids: &[i64]) -> Result<(), LibraryError> {
        self.runtime()?.block_on(self.reorder_async(game_ids))
    }

    pub fn record_session(
        &self,
        game_id: i64,
        playtime_seconds: i64,
        last_played_at: i64,
    ) -> Result<(), LibraryError> {
        self.runtime()?.block_on(self.record_session_async(
            game_id,
            playtime_seconds,
            last_played_at,
        ))
    }

    fn runtime(&self) -> Result<tokio::runtime::Runtime, LibraryError> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(LibraryError::Runtime)
    }

    async fn connect(&self) -> Result<SqliteConnection, LibraryError> {
        if let Some(parent) = self.database_path.parent() {
            fs::create_dir_all(parent).map_err(|source| LibraryError::CreateDirectory {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let options = SqliteConnectOptions::new()
            .filename(&self.database_path)
            .create_if_missing(true)
            .foreign_keys(true);
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
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS library_games (\
                id INTEGER PRIMARY KEY,\
                title TEXT NOT NULL,\
                alttitle TEXT,\
                released TEXT,\
                vndb_id TEXT,\
                thumbnail BLOB,\
                profile_id INTEGER UNIQUE REFERENCES launch_profiles(id),\
                position INTEGER NOT NULL,\
                playtime_seconds INTEGER NOT NULL DEFAULT 0,\
                last_played_at INTEGER\
            )",
        )
        .execute(&mut connection)
        .await?;
        let game_columns = sqlx::query("PRAGMA table_info(library_games)")
            .fetch_all(&mut connection)
            .await?;
        let has_playtime_seconds = game_columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .is_ok_and(|name| name == "playtime_seconds")
        });
        if !has_playtime_seconds {
            sqlx::query(
                "ALTER TABLE library_games ADD COLUMN playtime_seconds INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&mut connection)
            .await?;
        }
        let has_last_played_at = game_columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .is_ok_and(|name| name == "last_played_at")
        });
        if !has_last_played_at {
            sqlx::query("ALTER TABLE library_games ADD COLUMN last_played_at INTEGER")
                .execute(&mut connection)
                .await?;
        }
        sqlx::query(
            "INSERT INTO library_games (title, vndb_id, profile_id, position) \
             SELECT profile.title, profile.vndb_id, profile.id, \
                    COALESCE(profile.position, profile.id) \
             FROM launch_profiles AS profile \
             WHERE NOT EXISTS (\
                 SELECT 1 FROM library_games AS game WHERE game.profile_id = profile.id\
             )",
        )
        .execute(&mut connection)
        .await?;
        Ok(connection)
    }

    async fn load_async(&self) -> Result<Vec<StoredGame>, LibraryError> {
        let mut connection = self.connect().await?;
        let rows = sqlx::query(
            "SELECT game.id AS game_id, game.title AS game_title, game.alttitle, \
                    game.released, game.vndb_id AS game_vndb_id, game.thumbnail, \
                    game.playtime_seconds, game.last_played_at, \
                    profile.id AS profile_id, profile.exe, profile.prefix, profile.arch, \
                    profile.runner, profile.locale, profile.disc_path, profile.disc_drive, \
                    profile.disc_kind, profile.winetricks, profile.vndb_id AS profile_vndb_id, \
                    profile.notes \
             FROM library_games AS game \
             LEFT JOIN launch_profiles AS profile ON profile.id = game.profile_id \
             ORDER BY game.position, game.id",
        )
        .fetch_all(&mut connection)
        .await?;
        rows.into_iter().map(decode_game).collect()
    }

    async fn add_game_async(
        &self,
        metadata: &GameMetadata,
        thumbnail: Option<&[u8]>,
    ) -> Result<StoredGame, LibraryError> {
        let mut connection = self.connect().await?;
        let position: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(position), -1) + 1 FROM library_games")
                .fetch_one(&mut connection)
                .await?;
        let result = sqlx::query(
            "INSERT INTO library_games (\
                title, alttitle, released, vndb_id, thumbnail, position\
             ) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&metadata.title)
        .bind(&metadata.alttitle)
        .bind(&metadata.released)
        .bind(&metadata.vndb_id)
        .bind(thumbnail)
        .bind(position)
        .execute(&mut connection)
        .await?;
        Ok(StoredGame {
            id: result.last_insert_rowid(),
            metadata: metadata.clone(),
            thumbnail: thumbnail.map(ToOwned::to_owned),
            profile: None,
            playtime_seconds: 0,
            last_played_at: None,
        })
    }

    async fn attach_profile_async(
        &self,
        game_id: i64,
        profile: &LaunchProfile,
    ) -> Result<StoredProfile, LibraryError> {
        let mut connection = self.connect().await?;
        let mut transaction = connection.begin().await?;
        let game = sqlx::query("SELECT title, vndb_id, profile_id FROM library_games WHERE id = ?")
            .bind(game_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(LibraryError::MissingGame { id: game_id })?;
        if game.try_get::<Option<i64>, _>("profile_id")?.is_some() {
            return Err(LibraryError::AlreadyInstalled { id: game_id });
        }
        let title: String = game.try_get("title")?;
        let vndb_id: Option<String> = game.try_get("vndb_id")?;
        let winetricks =
            serde_json::to_string(&profile.winetricks).map_err(LibraryError::EncodeWinetricks)?;
        let (disc_path, disc_drive, disc_kind) = match &profile.disc {
            Some(disc) => (
                Some(path_bytes(&disc.path)),
                Some(disc.drive.as_str()),
                Some(disc.kind.as_str()),
            ),
            None => (None, None, None),
        };
        let result = sqlx::query(
            "INSERT INTO launch_profiles (\
                title, exe, prefix, arch, runner, locale, disc_path, disc_drive, \
                disc_kind, winetricks, vndb_id, notes, position\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&title)
        .bind(path_bytes(&profile.exe))
        .bind(path_bytes(&profile.prefix))
        .bind(profile.arch.as_str())
        .bind(profile.runner.as_str())
        .bind(&profile.locale)
        .bind(disc_path)
        .bind(disc_drive)
        .bind(disc_kind)
        .bind(winetricks)
        .bind(vndb_id)
        .bind(&profile.notes)
        .bind(game_id)
        .execute(&mut *transaction)
        .await?;
        let profile_id = result.last_insert_rowid();
        sqlx::query("UPDATE library_games SET profile_id = ? WHERE id = ?")
            .bind(profile_id)
            .bind(game_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;

        Ok(StoredProfile {
            id: profile_id,
            title,
            profile: profile.clone(),
        })
    }

    async fn update_metadata_async(
        &self,
        game_id: i64,
        metadata: &GameMetadata,
        thumbnail: Option<&[u8]>,
    ) -> Result<(), LibraryError> {
        let mut connection = self.connect().await?;
        let mut transaction = connection.begin().await?;
        let result = sqlx::query(
            "UPDATE library_games SET \
                title = ?, alttitle = ?, released = ?, vndb_id = ?, thumbnail = ? \
             WHERE id = ?",
        )
        .bind(&metadata.title)
        .bind(&metadata.alttitle)
        .bind(&metadata.released)
        .bind(&metadata.vndb_id)
        .bind(thumbnail)
        .bind(game_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(LibraryError::MissingGame { id: game_id });
        }
        sqlx::query(
            "UPDATE launch_profiles SET title = ?, vndb_id = ? \
             WHERE id = (SELECT profile_id FROM library_games WHERE id = ?)",
        )
        .bind(&metadata.title)
        .bind(&metadata.vndb_id)
        .bind(game_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn delete_async(&self, game_id: i64) -> Result<(), LibraryError> {
        let mut connection = self.connect().await?;
        let mut transaction = connection.begin().await?;
        let profile_id: Option<i64> =
            sqlx::query_scalar("SELECT profile_id FROM library_games WHERE id = ?")
                .bind(game_id)
                .fetch_optional(&mut *transaction)
                .await?
                .flatten();
        let result = sqlx::query("DELETE FROM library_games WHERE id = ?")
            .bind(game_id)
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() == 0 {
            return Err(LibraryError::MissingGame { id: game_id });
        }
        if let Some(profile_id) = profile_id {
            sqlx::query("DELETE FROM launch_profiles WHERE id = ?")
                .bind(profile_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn reorder_async(&self, game_ids: &[i64]) -> Result<(), LibraryError> {
        let mut connection = self.connect().await?;
        let mut transaction = connection.begin().await?;
        for (position, id) in game_ids.iter().enumerate() {
            sqlx::query("UPDATE library_games SET position = ? WHERE id = ?")
                .bind(position as i64)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn record_session_async(
        &self,
        game_id: i64,
        playtime_seconds: i64,
        last_played_at: i64,
    ) -> Result<(), LibraryError> {
        let mut connection = self.connect().await?;
        let result = sqlx::query(
            "UPDATE library_games SET \
                playtime_seconds = CASE \
                    WHEN playtime_seconds > ? THEN 9223372036854775807 \
                    ELSE playtime_seconds + ? \
                END, \
                last_played_at = MAX(COALESCE(last_played_at, ?), ?) \
             WHERE id = ?",
        )
        .bind(i64::MAX.saturating_sub(playtime_seconds))
        .bind(playtime_seconds)
        .bind(last_played_at)
        .bind(last_played_at)
        .bind(game_id)
        .execute(&mut connection)
        .await?;
        if result.rows_affected() == 0 {
            return Err(LibraryError::MissingGame { id: game_id });
        }
        Ok(())
    }
}

fn decode_game(row: sqlx::sqlite::SqliteRow) -> Result<StoredGame, LibraryError> {
    let title: String = row.try_get("game_title")?;
    let profile_id: Option<i64> = row.try_get("profile_id")?;
    let profile = match profile_id {
        Some(id) => Some(StoredProfile {
            id,
            title: title.clone(),
            profile: decode_profile(&row)?,
        }),
        None => None,
    };
    Ok(StoredGame {
        id: row.try_get("game_id")?,
        metadata: GameMetadata {
            title,
            alttitle: row.try_get("alttitle")?,
            released: row.try_get("released")?,
            vndb_id: row.try_get("game_vndb_id")?,
        },
        thumbnail: row.try_get("thumbnail")?,
        profile,
        playtime_seconds: row.try_get("playtime_seconds")?,
        last_played_at: row.try_get("last_played_at")?,
    })
}

fn decode_profile(row: &sqlx::sqlite::SqliteRow) -> Result<LaunchProfile, LibraryError> {
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
            return Err(LibraryError::InvalidValue {
                field: "disc",
                value: "incomplete disc fields".to_owned(),
            });
        }
    };
    let winetricks: String = row.try_get("winetricks")?;
    Ok(LaunchProfile {
        exe: bytes_path(row.try_get("exe")?),
        prefix: bytes_path(row.try_get("prefix")?),
        arch: PrefixArch::parse(&row.try_get::<String, _>("arch")?)?,
        runner: Runner::parse(&row.try_get::<String, _>("runner")?)?,
        locale: row.try_get("locale")?,
        disc,
        winetricks: serde_json::from_str(&winetricks).map_err(LibraryError::DecodeWinetricks)?,
        vndb_id: row.try_get("profile_vndb_id")?,
        notes: row.try_get("notes")?,
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

    use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
    use tempfile::tempdir;

    use super::{
        DiscKind, DiscProfile, GameMetadata, LaunchProfile, LibraryStore, PrefixArch, Runner,
    };

    #[test]
    fn metadata_profile_attachment_order_and_removal_round_trip() {
        let root = tempdir().expect("temporary directory");
        let store = LibraryStore::new(root.path());
        let first_metadata = GameMetadata {
            title: "Game".to_owned(),
            alttitle: Some("原題".to_owned()),
            released: Some("2025-01-01".to_owned()),
            vndb_id: Some("v1".to_owned()),
        };
        let first = store
            .add_game(&first_metadata, Some(b"cover"))
            .expect("add metadata-only game");
        let second = store
            .add_game(
                &GameMetadata {
                    title: "Second".to_owned(),
                    alttitle: None,
                    released: None,
                    vndb_id: Some("v2".to_owned()),
                },
                None,
            )
            .expect("add second game");
        assert!(first.profile.is_none());

        let profile = LaunchProfile {
            exe: PathBuf::from(OsString::from_vec(b"/tmp/game-\x81.exe".to_vec())),
            prefix: root.path().join("prefix"),
            arch: PrefixArch::Win64,
            runner: Runner::Wine,
            locale: "ja_JP.UTF-8".to_owned(),
            disc: Some(DiscProfile {
                path: root.path().join("disc"),
                drive: "d:".to_owned(),
                kind: DiscKind::Extracted,
            }),
            winetricks: Vec::new(),
            vndb_id: Some("v1".to_owned()),
            notes: "Keep disc mapped".to_owned(),
        };
        store
            .attach_profile(first.id, &profile)
            .expect("attach launch profile");
        store
            .reorder(&[second.id, first.id])
            .expect("reorder games");
        let loaded = store.load().expect("load games");
        assert_eq!(loaded[0].metadata.title, "Second");
        assert_eq!(loaded[1].thumbnail.as_deref(), Some(b"cover".as_slice()));
        assert_eq!(
            loaded[1].profile.as_ref().expect("profile").profile,
            profile
        );

        store
            .record_session(first.id, 30, 1_700_000_000)
            .expect("record first session");
        store
            .record_session(first.id, 45, 1_699_999_999)
            .expect("record second session");
        let played = store.load().expect("load recorded sessions");
        assert_eq!(played[1].playtime_seconds, 75);
        assert_eq!(played[1].last_played_at, Some(1_700_000_000));

        let replacement = GameMetadata {
            title: "Updated".to_owned(),
            alttitle: None,
            released: Some("2026".to_owned()),
            vndb_id: Some("v3".to_owned()),
        };
        store
            .update_metadata(first.id, &replacement, Some(b"new cover"))
            .expect("update metadata");
        let updated = store.load().expect("load updated game");
        assert_eq!(updated[1].metadata, replacement);
        assert_eq!(
            updated[1].profile.as_ref().expect("profile").title,
            "Updated"
        );

        store.delete(first.id).expect("remove game");
        assert_eq!(store.load().expect("load remaining games"), vec![second]);
    }

    #[test]
    fn existing_launch_profiles_migrate_to_library_games() {
        let root = tempdir().expect("temporary directory");
        let database = root.path().join("profiles.sqlite3");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("database runtime");
        runtime.block_on(async {
            let options = SqliteConnectOptions::new()
                .filename(&database)
                .create_if_missing(true);
            let mut connection = SqliteConnection::connect_with(&options)
                .await
                .expect("open legacy database");
            sqlx::query(
                "CREATE TABLE launch_profiles (
                    id INTEGER PRIMARY KEY,
                    title TEXT NOT NULL,
                    exe BLOB NOT NULL,
                    prefix BLOB NOT NULL,
                    arch TEXT NOT NULL,
                    runner TEXT NOT NULL,
                    locale TEXT NOT NULL,
                    disc_path BLOB,
                    disc_drive TEXT,
                    disc_kind TEXT,
                    winetricks TEXT NOT NULL,
                    vndb_id TEXT,
                    notes TEXT NOT NULL,
                    position INTEGER
                )",
            )
            .execute(&mut connection)
            .await
            .expect("create legacy schema");
            sqlx::query(
                "CREATE TABLE library_games (
                    id INTEGER PRIMARY KEY,
                    title TEXT NOT NULL,
                    alttitle TEXT,
                    released TEXT,
                    vndb_id TEXT,
                    thumbnail BLOB,
                    profile_id INTEGER UNIQUE REFERENCES launch_profiles(id),
                    position INTEGER NOT NULL
                )",
            )
            .execute(&mut connection)
            .await
            .expect("create legacy library schema");
            sqlx::query(
                "INSERT INTO launch_profiles
                 (title, exe, prefix, arch, runner, locale, winetricks, vndb_id, notes, position)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind("Sanoba Witch")
            .bind(b"/prefix/drive_c/game.exe".as_slice())
            .bind(b"/prefix".as_slice())
            .bind("win64")
            .bind("wine")
            .bind("ja_JP.UTF-8")
            .bind("[]")
            .bind("v16044")
            .bind("")
            .bind(1_i64)
            .execute(&mut connection)
            .await
            .expect("insert legacy profile");
        });

        let games = LibraryStore::new(root.path())
            .load()
            .expect("migrate legacy profile");
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].metadata.title, "Sanoba Witch");
        assert_eq!(games[0].metadata.vndb_id.as_deref(), Some("v16044"));
        assert!(games[0].profile.is_some());
        assert_eq!(games[0].playtime_seconds, 0);
        assert_eq!(games[0].last_played_at, None);
    }
}
