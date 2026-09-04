//! On-disk storage for the files receipts are read from: photos, and the PDF
//! invoices online vendors send.
//!
//! They live on the filesystem rather than in SQLite: full-resolution phone
//! photos would bloat the database and they are never queried, only fetched by
//! path.

use std::path::{Path, PathBuf};

use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("not a PDF, and not an image we can read: {0}")]
    Format(#[from] image::ImageError),
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn from_env() -> Self {
        Self::new(crate::server::env::string("DATA_DIR", "./data"))
    }

    pub fn absolute(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stores an uploaded file and returns the path to record on the receipt,
    /// relative to the store root.
    ///
    /// The filename is generated, never taken from the upload: no
    /// client-controlled string reaches the filesystem, so there is no path
    /// traversal to defend against. Bytes are written verbatim: the original
    /// is what a human re-reads when the model gets something wrong, so it is
    /// never the downscaled copy.
    ///
    /// The extension is what later says which of the two a receipt came from,
    /// so it is taken from the bytes and not from what the browser claimed.
    pub async fn write_upload(
        &self,
        bytes: &[u8],
        today: jiff::civil::Date,
    ) -> Result<String, StoreError> {
        // Sniffing the bytes is also what validates them: anything neither of
        // these recognizes is not worth storing or sending to a model.
        let (folder, ext) = if super::pdf::is_pdf(bytes) {
            ("documents", "pdf")
        } else {
            let format = image::guess_format(bytes)?;
            let ext = format.extensions_str().first().copied().unwrap_or("bin");
            ("images", ext)
        };

        let relative = format!(
            "{folder}/{:04}/{:02}/{}.{ext}",
            today.year(),
            today.month(),
            Uuid::new_v4(),
        );
        let absolute = self.absolute(&relative);

        if let Some(parent) = absolute.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| StoreError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
        }
        tokio::fs::write(&absolute, bytes)
            .await
            .map_err(|source| StoreError::Io {
                path: absolute.display().to_string(),
                source,
            })?;

        Ok(relative)
    }

    /// Removes a stored image. A file that has already gone is not an error:
    /// the row is on its way out either way, and leaving it behind would strand
    /// the receipt in the list.
    ///
    /// An empty path is a receipt that never had a photo. Joined, it would name
    /// the store root itself, so it stops here.
    pub async fn delete(&self, relative: &str) -> Result<(), StoreError> {
        if relative.is_empty() {
            return Ok(());
        }

        let absolute = self.absolute(relative);
        match tokio::fs::remove_file(&absolute).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io {
                path: absolute.display().to_string(),
                source,
            }),
        }
    }

    pub async fn read(&self, relative: &str) -> Result<Vec<u8>, StoreError> {
        let absolute = self.absolute(relative);
        tokio::fs::read(&absolute)
            .await
            .map_err(|source| StoreError::Io {
                path: absolute.display().to_string(),
                source,
            })
    }
}
