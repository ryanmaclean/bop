use std::path::PathBuf;

/// Resolve the user home with the same platform semantics as dirs::home_dir.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        directories_next::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
    }

    #[cfg(not(windows))]
    {
        directories_next::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
    }
}
