//! Where Studio keeps the operator's own files: projects, the last project,
//! locators, and view and option settings (`ADR-119`).
//!
//! On the desktop these are the XDG files themselves. The browser has no
//! file system, so there each file is one entry in the page's
//! `localStorage`, keyed by its path and held as hex; directories are
//! implied by the keys. The calls mirror the `std::fs` ones Studio uses.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
pub use std::fs::{create_dir_all, read, read_to_string, remove_file, rename, write};

/// Whether `path` names a file.
#[cfg(not(target_arch = "wasm32"))]
pub fn exists(path: &Path) -> bool {
    path.exists()
}

/// The files directly in `dir`, unsorted; none when it cannot be read.
#[cfg(not(target_arch = "wasm32"))]
pub fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect()
}

#[cfg(target_arch = "wasm32")]
pub use web::{create_dir_all, exists, list, read, read_to_string, remove_file, rename, write};

#[cfg(target_arch = "wasm32")]
mod web {
    use std::{
        io,
        path::{Path, PathBuf},
    };

    /// Keys of Studio's files, so other entries on the origin are left alone.
    const PREFIX: &str = "afik:";

    fn storage() -> io::Result<web_sys::Storage> {
        web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "no browser storage"))
    }

    fn key(path: &Path) -> String {
        format!("{PREFIX}{}", path.display())
    }

    fn refused(error: &wasm_bindgen::JsValue) -> io::Error {
        io::Error::other(format!("browser storage refused the write: {error:?}"))
    }

    /// Directories are implied by the keys of the files in them.
    #[allow(clippy::unnecessary_wraps)]
    pub fn create_dir_all<P: AsRef<Path>>(_path: P) -> io::Result<()> {
        Ok(())
    }

    /// Whether `path` names a file.
    pub fn exists(path: &Path) -> bool {
        storage().is_ok_and(|storage| storage.get_item(&key(path)).ok().flatten().is_some())
    }

    /// The files directly in `dir`, unsorted.
    pub fn list(dir: &Path) -> Vec<PathBuf> {
        let Ok(storage) = storage() else {
            return Vec::new();
        };
        let count = storage.length().unwrap_or(0);
        (0..count)
            .filter_map(|index| storage.key(index).ok().flatten())
            .filter_map(|key| key.strip_prefix(PREFIX).map(PathBuf::from))
            .filter(|path| path.parent() == Some(dir))
            .collect()
    }

    /// Reads a whole file.
    pub fn read<P: AsRef<Path>>(path: P) -> io::Result<Vec<u8>> {
        let path = path.as_ref();
        let text = storage()?
            .get_item(&key(path))
            .ok()
            .flatten()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))?;
        super::from_hex(&text)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "damaged stored file"))
    }

    /// Reads a whole file as UTF-8 text.
    pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
        String::from_utf8(read(path)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Replaces a whole file.
    pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
        storage()?
            .set_item(&key(path.as_ref()), &super::to_hex(contents.as_ref()))
            .map_err(|error| refused(&error))
    }

    /// Moves a file, replacing whatever `to` held.
    pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> io::Result<()> {
        let bytes = read(&from)?;
        write(to, bytes)?;
        remove_file(from)
    }

    /// Deletes a file.
    pub fn remove_file<P: AsRef<Path>>(path: P) -> io::Result<()> {
        let path = path.as_ref();
        if !exists(path) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
        }
        storage()?
            .remove_item(&key(path))
            .map_err(|error| refused(&error))
    }
}

/// Lower-case hex, two digits a byte.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// The bytes [`to_hex`] wrote, or `None` for anything else.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            let digit = |c: u8| char::from(c).to_digit(16);
            u8::try_from(digit(pair[0])? << 4 | digit(pair[1])?).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{from_hex, to_hex};

    #[test]
    fn hex_holds_every_byte_and_refuses_anything_else() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(from_hex(&to_hex(&bytes)).unwrap(), bytes);
        assert_eq!(to_hex(&[0x0f, 0xa0]), "0fa0");
        assert_eq!(from_hex("0"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn a_listing_holds_only_the_files_directly_in_the_directory() {
        let dir = std::env::temp_dir().join(format!("afik-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        std::fs::write(dir.join("a.afik"), [1]).unwrap();
        std::fs::write(dir.join("inner").join("b.afik"), [2]).unwrap();
        let listed = super::list(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(listed, [dir.join("a.afik")]);
        assert!(!super::exists(&dir.join("a.afik")));
    }
}
