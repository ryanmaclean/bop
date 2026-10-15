//! Home-directory lookup for BOP's existing configuration and credential paths.
//!
//! Keep the platform semantics previously selected by `dirs 5.0.1`: Unix HOME
//! precedes the account database, whereas Windows uses the Profile Known Folder
//! without consulting HOME or USERPROFILE. Callers still own their path suffixes.

use std::path::PathBuf;

/// Return the current user's home directory, preserving native path bytes.
///
/// Lookup failure remains `None`; this does not invent a current-directory or
/// environment-variable fallback on platforms that did not previously use one.
pub fn home_dir() -> Option<PathBuf> {
    platform::home_dir()
}

#[cfg(all(unix, not(target_os = "redox"), not(target_arch = "wasm32")))]
mod platform {
    #[cfg(any(
        test,
        not(any(target_os = "android", target_os = "ios", target_os = "emscripten"))
    ))]
    use std::ffi::OsStr;
    use std::ffi::OsString;
    #[cfg(any(
        test,
        not(any(target_os = "android", target_os = "ios", target_os = "emscripten"))
    ))]
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    pub(super) fn home_dir() -> Option<PathBuf> {
        choose_home(std::env::var_os("HOME"), account_home)
    }

    fn choose_home(
        home: Option<OsString>,
        account: impl FnOnce() -> Option<PathBuf>,
    ) -> Option<PathBuf> {
        match home {
            Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
            _ => account(),
        }
    }

    #[cfg(any(
        test,
        not(any(target_os = "android", target_os = "ios", target_os = "emscripten"))
    ))]
    fn account_path(bytes: &[u8]) -> Option<PathBuf> {
        if bytes.is_empty() {
            None
        } else {
            Some(PathBuf::from(OsStr::from_bytes(bytes)))
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios", target_os = "emscripten"))]
    fn account_home() -> Option<PathBuf> {
        None
    }

    #[cfg(not(any(target_os = "android", target_os = "ios", target_os = "emscripten")))]
    fn account_home() -> Option<PathBuf> {
        use std::ffi::CStr;
        use std::mem::MaybeUninit;
        use std::ptr;

        // Keep the previous one-call NSS lookup, including its failure on an
        // insufficient buffer. Growing/retrying would change failure semantics.
        // SAFETY: sysconf takes a constant selector and no caller-owned pointers.
        let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
        let size = if suggested < 0 { 512 } else { suggested as usize };
        let mut storage = Vec::<libc::c_char>::with_capacity(size);
        let mut record = MaybeUninit::<libc::passwd>::zeroed();
        let mut found = ptr::null_mut();
        // SAFETY: record, found and storage provide writable, nonoverlapping
        // buffers for the duration of getpwuid_r. Its pointers are only read
        // after success and the returned path is copied before storage is freed.
        let status = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                record.as_mut_ptr(),
                storage.as_mut_ptr(),
                storage.capacity(),
                &mut found,
            )
        };
        if status != 0 || found.is_null() {
            return None;
        }
        // SAFETY: successful getpwuid_r initialized the returned passwd record.
        let directory = unsafe { (*found).pw_dir };
        if directory.is_null() {
            return None;
        }
        // SAFETY: pw_dir is a NUL-terminated string in the still-live storage.
        account_path(unsafe { CStr::from_ptr(directory) }.to_bytes())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::ffi::OsStringExt;

        #[test]
        fn explicit_home_skips_account_lookup_and_preserves_relative_paths() {
            let home = OsString::from("relative/home");
            assert_eq!(
                choose_home(Some(home.clone()), || panic!("unexpected account lookup")),
                Some(PathBuf::from(home))
            );
        }

        #[test]
        fn missing_and_empty_home_use_the_account_lookup() {
            for home in [None, Some(OsString::new())] {
                assert_eq!(
                    choose_home(home, || Some(PathBuf::from("/account/home"))),
                    Some(PathBuf::from("/account/home"))
                );
            }
        }

        #[test]
        fn failed_account_lookup_does_not_invent_a_path() {
            assert_eq!(choose_home(None, || None), None);
            assert_eq!(choose_home(Some(OsString::new()), || None), None);
        }

        #[test]
        fn empty_passwd_home_is_rejected() {
            assert_eq!(account_path(b""), None);
        }

        #[test]
        fn environment_and_account_paths_preserve_non_utf8_bytes() {
            let bytes = b"/home/\xff".to_vec();
            let home = OsString::from_vec(bytes.clone());
            assert_eq!(
                choose_home(Some(home.clone()), || panic!("unexpected account lookup")),
                Some(PathBuf::from(home.clone()))
            );
            assert_eq!(account_path(&bytes), Some(PathBuf::from(home)));
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::{ptr, slice};
    use windows_sys::Win32::Globalization::lstrlenW;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Profile, SHGetKnownFolderPath};

    struct FolderMemory(*mut u16);

    impl Drop for FolderMemory {
        fn drop(&mut self) {
            // SAFETY: this pointer was initialized to null and passed only to
            // SHGetKnownFolderPath. CoTaskMemFree accepts null and owns cleanup
            // on both success and failure, including during unwinding.
            unsafe { CoTaskMemFree(self.0.cast()) };
        }
    }

    pub(super) fn home_dir() -> Option<PathBuf> {
        let mut folder = FolderMemory(ptr::null_mut());
        // SAFETY: the GUID is static; zero selects current-user/default flags.
        // folder owns any allocation returned through its writable out pointer.
        let status = unsafe { SHGetKnownFolderPath(&FOLDERID_Profile, 0, 0, &mut folder.0) };
        if status != 0 || folder.0.is_null() {
            return None;
        }
        // SAFETY: successful SHGetKnownFolderPath returns a NUL-terminated UTF-16
        // path. The allocation stays live until after the owned OsString copy.
        let wide = unsafe { slice::from_raw_parts(folder.0, lstrlenW(folder.0) as usize) };
        Some(PathBuf::from(OsString::from_wide(wide)))
    }
}

#[cfg(target_os = "redox")]
mod platform {
    use std::path::PathBuf;

    pub(super) fn home_dir() -> Option<PathBuf> {
        use redox_users::{All, AllUsers, Config};

        let uid = redox_users::get_uid().ok()?;
        let accounts = AllUsers::basic(Config::default()).ok()?;
        accounts
            .get_by_id(uid)
            .map(|account| PathBuf::from(account.home.clone()))
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    pub(super) fn home_dir() -> Option<std::path::PathBuf> {
        None
    }
}
