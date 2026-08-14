use std::{fmt, fs, io, path::PathBuf};

use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveFormat {
    SevenZip,
    Rar,
    Zip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscImageFormat {
    Iso,
    Mds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    Directory,
    Archive(ArchiveFormat),
    DiscImage(DiscImageFormat),
    InstallerExecutable,
    OtherFile,
}

impl fmt::Display for SourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::Directory => "Folder",
            Self::Archive(ArchiveFormat::SevenZip) => "7-Zip archive",
            Self::Archive(ArchiveFormat::Rar) => "RAR archive",
            Self::Archive(ArchiveFormat::Zip) => "ZIP archive",
            Self::DiscImage(DiscImageFormat::Iso) => "ISO disc image",
            Self::DiscImage(DiscImageFormat::Mds) => "MDS disc descriptor",
            Self::InstallerExecutable => "Windows executable",
            Self::OtherFile => "Unrecognized file",
        };

        formatter.write_str(description)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentifiedSource {
    pub path: PathBuf,
    pub kind: SourceKind,
}

#[derive(Debug, Error)]
pub enum IdentifyError {
    #[error("cannot inspect {path}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn identify_source(path: PathBuf) -> Result<IdentifiedSource, IdentifyError> {
    let metadata = fs::metadata(&path).map_err(|source| IdentifyError::Metadata {
        path: path.clone(),
        source,
    })?;

    let kind = if metadata.is_dir() {
        SourceKind::Directory
    } else {
        classify_file(&path)
    };

    Ok(IdentifiedSource { path, kind })
}

fn classify_file(path: &std::path::Path) -> SourceKind {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("7z") => SourceKind::Archive(ArchiveFormat::SevenZip),
        Some("rar") => SourceKind::Archive(ArchiveFormat::Rar),
        Some("zip") => SourceKind::Archive(ArchiveFormat::Zip),
        Some("iso") => SourceKind::DiscImage(DiscImageFormat::Iso),
        Some("mds") => SourceKind::DiscImage(DiscImageFormat::Mds),
        Some("exe") => SourceKind::InstallerExecutable,
        _ => SourceKind::OtherFile,
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::tempdir;

    use super::{ArchiveFormat, SourceKind, identify_source};

    #[test]
    fn identifies_directories_and_case_insensitive_archive_extensions() {
        let directory = tempdir().expect("temporary directory should be created");
        let archive_path = directory.path().join("GAME.7Z");
        File::create(&archive_path).expect("archive fixture should be created");

        let identified_directory = identify_source(directory.path().to_path_buf())
            .expect("directory should be identified");
        let identified_archive =
            identify_source(archive_path).expect("archive should be identified");

        assert_eq!(identified_directory.kind, SourceKind::Directory);
        assert_eq!(
            identified_archive.kind,
            SourceKind::Archive(ArchiveFormat::SevenZip)
        );
    }
}
