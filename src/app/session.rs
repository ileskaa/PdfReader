use std::path::{Path, PathBuf};

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

fn session_file() -> PathBuf {
    glib::user_state_dir().join("pdf-reader").join("session")
}

/// Documents open at the end of the last session, in window order. Missing or unreadable
/// state yields an empty session.
pub fn load() -> Vec<PathBuf> {
    std::fs::read_to_string(session_file())
        .map(|text| decode(&text))
        .unwrap_or_default()
}

pub fn save(paths: &[PathBuf]) {
    if let Err(err) = write_atomically(&session_file(), &encode(paths)) {
        glib::g_warning!("pdf-reader", "cannot save session: {err}");
    }
}

fn write_atomically(target: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = target.with_extension("tmp");
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, target)
}

// One file URI per line: URIs are percent-encoded, so they hold no newlines and carry
// non-UTF-8 path bytes losslessly.
fn encode(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| format!("{}\n", gio::File::for_path(path).uri()))
        .collect()
}

fn decode(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter(|line| line.starts_with("file://"))
        .filter_map(|line| gio::File::for_uri(line).path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn session_round_trips_unusual_paths() {
        let paths = vec![
            PathBuf::from("/home/u/My Papers/a b.pdf"),
            PathBuf::from("/tmp/line\nbreak.pdf"),
            PathBuf::from(OsStr::from_bytes(b"/tmp/latin1-\xe9.pdf")),
        ];
        assert_eq!(decode(&encode(&paths)), paths);
    }

    #[test]
    fn garbage_session_decodes_to_nothing() {
        assert!(decode("").is_empty());
        assert!(decode("not a uri\nhttps://example.com/a.pdf\n").is_empty());
    }

    #[test]
    fn write_replaces_previous_session() {
        let dir = std::env::temp_dir().join(format!("pdf-reader-session-{}", std::process::id()));
        let target = dir.join("nested").join("session");
        write_atomically(&target, "first\n").unwrap();
        write_atomically(&target, "second\n").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "second\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
