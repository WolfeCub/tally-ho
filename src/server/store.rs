//! On-disk storage for receipt images.
//!
//! Images live on the filesystem rather than in SQLite: full-resolution phone
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
    #[error("unrecognized image format: {0}")]
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
        Self::new(std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".to_string()))
    }

    pub fn absolute(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Stores an uploaded image and returns the path to record on the receipt,
    /// relative to the store root.
    ///
    /// The filename is generated, never taken from the upload: no
    /// client-controlled string reaches the filesystem, so there is no path
    /// traversal to defend against. Bytes are written verbatim — the original
    /// is what a human re-reads when the model gets something wrong, so it is
    /// never the downscaled copy.
    pub async fn write_upload(
        &self,
        bytes: &[u8],
        today: jiff::civil::Date,
    ) -> Result<String, StoreError> {
        // Also serves as validation: anything `image` cannot identify is not
        // worth storing or sending to a model.
        let format = image::guess_format(bytes)?;
        let ext = format.extensions_str().first().copied().unwrap_or("bin");

        let relative = format!(
            "images/{:04}/{:02}/{}.{}",
            today.year(),
            today.month(),
            Uuid::new_v4(),
            ext
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
