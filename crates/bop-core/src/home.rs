//! User home discovery shared by configuration and credential paths.
//!
//! Standard-library platform lookup avoids an additional directory dependency.
//! An empty passwd home must never turn credential paths into cwd-relative paths.
use std::path::PathBuf;

pub fn directory() -> Option<PathBuf> {
    non_empty(std::env::home_dir())
}

fn non_empty(home: Option<PathBuf>) -> Option<PathBuf> {
    home.filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_home_stays_unknown() {
        assert_eq!(non_empty(None), None);
    }

    #[test]
    fn empty_passwd_home_cannot_make_relative_credentials() {
        assert_eq!(non_empty(Some(PathBuf::new())), None);
    }

    #[test]
    fn home_with_spaces_and_unicode_is_preserved() {
        let home = PathBuf::from("/users/éva smith");
        assert_eq!(non_empty(Some(home.clone())), Some(home));
    }
}
