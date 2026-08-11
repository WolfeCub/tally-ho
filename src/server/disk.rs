//! How much room is left where the data lives.

use std::path::Path;

use crate::shared::dto;

/// Stats the filesystem holding `path`, which is the store root — so in the
/// container this reports the mounted volume and not the image around it.
///
/// The path itself has to exist, so this is asked about the root rather than
/// anything under it: `images/` isn't there until the first upload lands.
pub fn usage(path: &Path) -> std::io::Result<dto::DiskUsage> {
    let stats = fs4::statvfs(path)?;

    Ok(dto::DiskUsage {
        path: path.display().to_string(),
        total_bytes: stats.total_space(),
        available_bytes: stats.available_space(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Against the real filesystem, since the point of this is to describe the
    /// volume the data is actually sitting on.
    #[test]
    fn reports_the_filesystem_the_path_is_on() {
        let usage = super::usage(Path::new(".")).unwrap();

        assert!(usage.total_bytes > 0, "no filesystem behind the cwd");
        assert!(usage.available_bytes <= usage.total_bytes);
    }

    /// An error rather than zeroes: a total of nothing would draw as a disk that
    /// is completely full, which is alarming and wrong.
    #[test]
    fn a_path_that_isnt_there_is_an_error() {
        assert!(super::usage(Path::new("no/such/directory")).is_err());
    }
}
