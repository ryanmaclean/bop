//! Fallible dispatcher directory scans. Filesystem uncertainty is not capacity.
use std::{fs, io, path::{Path, PathBuf}};

fn running_entries(entries: impl IntoIterator<Item = io::Result<PathBuf>>) -> io::Result<usize> {
    let mut count = 0usize;
    for entry in entries {
        let path = entry?;
        if path.extension().is_some_and(|extension| extension == "bop") {
            count = count.checked_add(1).ok_or_else(|| io::Error::other("running card count overflow"))?;
        }
    }
    Ok(count)
}

pub(crate) fn running_count(dir: &Path) -> io::Result<usize> {
    running_entries(fs::read_dir(dir)?.map(|entry| entry.map(|entry| entry.path())))
}

fn pending_entries(entries: impl IntoIterator<Item = io::Result<PathBuf>>) -> io::Result<Vec<PathBuf>> {
    let mut cards = Vec::new();
    for entry in entries {
        let path = entry?;
        if path.extension().is_some_and(|extension| extension == "bop") && fs::metadata(&path)?.is_dir() {
            cards.push(path);
        }
    }
    Ok(cards)
}

pub(crate) fn pending_cards(dir: &Path) -> io::Result<Vec<PathBuf>> {
    pending_entries(fs::read_dir(dir)?.map(|entry| entry.map(|entry| entry.path())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_scan_counts_bop_entries_and_propagates_iteration_error() {
        let paths = [Ok(PathBuf::from("a.bop")), Ok(PathBuf::from("notes"))];
        assert_eq!(running_entries(paths).unwrap(), 1);
        let paths = [Ok(PathBuf::from("a.bop")), Err(io::Error::other("injected iteration error"))];
        assert!(running_entries(paths).is_err());
    }

    #[test]
    fn both_scans_propagate_directory_open_and_pending_iteration_errors() {
        let absent = Path::new("/dev/null/bop-dispatcher-scan-test");
        assert!(running_count(absent).is_err());
        assert!(pending_cards(absent).is_err());
        let paths = [Err(io::Error::other("injected iteration error"))];
        assert!(pending_entries(paths).is_err());
        let paths = [Ok(absent.join("missing.bop"))];
        assert!(pending_entries(paths).is_err());
    }

    #[test]
    fn pending_scan_selects_directories_and_discards_partial_results_on_error() {
        let unique = format!("bop-scan-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        let root = std::env::temp_dir().join(unique);
        fs::create_dir(&root).unwrap();
        let card = root.join("one.bop");
        let file = root.join("not-a-card.bop");
        fs::create_dir(&card).unwrap();
        fs::write(&file, b"x").unwrap();
        assert_eq!(pending_cards(&root).unwrap(), vec![card.clone()]);
        let paths = [Ok(card.clone()), Err(io::Error::other("injected iteration error"))];
        assert!(pending_entries(paths).is_err());
        fs::remove_file(file).unwrap();
        fs::remove_dir(card).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
