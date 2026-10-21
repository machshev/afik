//! Where Studio keeps the operator's own files: projects, the last project,
//! locators, and view and option settings (`ADR-119`).
//!
//! On the desktop these are the XDG files themselves. The browser has no
//! file system, so there each file is one entry in the page's
//! `localStorage`, keyed by its path and held as hex; directories are
//! implied by the keys. The calls mirror the `std::fs` ones Studio uses.
//! The browser takes files in through its file picker ([`upload`]) or from
//! beside the page ([`fetch`]), and gives them out as downloads
//! ([`download`]).

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
pub use std::fs::{create_dir_all, read, read_to_string, remove_file, rename, write};

/// Whether `path` names a file.
#[cfg(not(target_arch = "wasm32"))]
pub fn exists(path: &Path) -> bool {
    path.exists()
}

/// Whether `path` names a regular file.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// Changes whenever the file at `path` does, without reading it; `None`
/// when there is no file.
#[cfg(not(target_arch = "wasm32"))]
pub fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path).ok().map(|metadata| {
        (
            metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            metadata.len(),
        )
    })
}

/// What [`stamp`] returns.
pub type Stamp = Option<(std::time::SystemTime, u64)>;

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
pub use web::{
    create_dir_all, download, exists, fetch, is_file, list, read, read_to_string, remove_file,
    rename, stamp, upload, write, Upload,
};

#[cfg(target_arch = "wasm32")]
mod web {
    use std::{
        cell::{Cell, RefCell},
        io,
        path::{Path, PathBuf},
        rc::Rc,
        time::{Duration, SystemTime},
    };

    use js_sys::{Array, Uint8Array};
    use wasm_bindgen::{closure::Closure, JsCast, JsValue};
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, HtmlInputElement, Url};

    /// Keys of Studio's files, so other entries on the origin are left alone.
    const PREFIX: &str = "afik:";

    thread_local! {
        /// Counts writes and removals, which is what [`stamp`] reports.
        static WRITES: Cell<u64> = const { Cell::new(0) };
    }

    fn written() {
        WRITES.set(WRITES.get().wrapping_add(1));
    }

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

    /// Every stored entry is a file.
    pub fn is_file(path: &Path) -> bool {
        exists(path)
    }

    /// Changes with every write to the page's storage: stored files carry
    /// no time of their own.
    pub fn stamp(path: &Path) -> super::Stamp {
        exists(path).then(|| {
            let writes = WRITES.get();
            (SystemTime::UNIX_EPOCH + Duration::from_secs(writes), writes)
        })
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
        written();
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
        written();
        storage()?
            .remove_item(&key(path))
            .map_err(|error| refused(&error))
    }

    /// A file on its way into the page's storage.
    pub struct Upload {
        outcome: Rc<RefCell<Option<Result<PathBuf, String>>>>,
    }

    impl Upload {
        /// Where the chosen file was stored, or why it was not, once known.
        pub fn poll(&self) -> Option<Result<PathBuf, String>> {
            self.outcome.borrow_mut().take()
        }
    }

    /// Shows the browser's file picker, so this must follow a click, and
    /// stores the chosen file under `dir` by its own name. A file over
    /// `maximum` bytes is refused unread.
    pub fn upload(dir: PathBuf, accept: &str, maximum: usize) -> Result<Upload, String> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("no browser document")?;
        let input = document
            .create_element("input")
            .map_err(|error| format!("{error:?}"))?
            .unchecked_into::<HtmlInputElement>();
        input.set_type("file");
        input.set_accept(accept);
        let outcome = Rc::new(RefCell::new(None));
        let chosen = {
            let (input, outcome) = (input.clone(), Rc::clone(&outcome));
            Closure::once_into_js(move || {
                let Some(file) = input.files().and_then(|files| files.get(0)) else {
                    outcome.replace(Some(Err("no file chosen".to_owned())));
                    return;
                };
                wasm_bindgen_futures::spawn_local(async move {
                    let stored = store(&dir, &file, maximum).await;
                    outcome.replace(Some(stored));
                });
            })
        };
        let cancelled = {
            let outcome = Rc::clone(&outcome);
            Closure::once_into_js(move || {
                outcome.replace(Some(Err("no file chosen".to_owned())));
            })
        };
        input.set_onchange(Some(chosen.unchecked_ref()));
        input.set_oncancel(Some(cancelled.unchecked_ref()));
        input.click();
        Ok(Upload { outcome })
    }

    async fn store(dir: &Path, file: &web_sys::File, maximum: usize) -> Result<PathBuf, String> {
        let name = file.name();
        // A bare name only: the picker's name is the operator's, not a path.
        let name = Path::new(&name)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{name:?} is not a file name"))?;
        #[allow(clippy::cast_precision_loss)]
        if file.size() > maximum as f64 {
            return Err(format!("{name} exceeds {maximum} bytes"));
        }
        let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
            .await
            .map_err(|error| format!("could not read {name}: {error:?}"))?;
        let path = dir.join(name);
        write(&path, Uint8Array::new(&buffer).to_vec()).map_err(|error| error.to_string())?;
        log::info!("stored {} from the browser", path.display());
        Ok(path)
    }

    /// Fetches `url`, relative to the page, into `path`. A response over
    /// `maximum` bytes is refused.
    pub fn fetch(url: String, path: PathBuf, maximum: usize) -> Upload {
        let outcome = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&outcome);
        wasm_bindgen_futures::spawn_local(async move {
            let stored = crate::net::fetch(&url, maximum)
                .await
                .and_then(|bytes| write(&path, bytes).map_err(|error| error.to_string()))
                .map(|()| path);
            slot.replace(Some(stored));
        });
        Upload { outcome }
    }

    /// Gives `bytes` to the operator as a download named `name`.
    pub fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
        let failed = |error: JsValue| format!("could not offer {name} as a download: {error:?}");
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or("no browser document")?;
        let body = document.body().ok_or("no document body")?;
        let options = BlobPropertyBag::new();
        options.set_type("application/octet-stream");
        let parts = Array::of1(&Uint8Array::from(bytes));
        let blob =
            Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(failed)?;
        let url = Url::create_object_url_with_blob(&blob).map_err(failed)?;
        let anchor = document
            .create_element("a")
            .map_err(failed)?
            .unchecked_into::<HtmlAnchorElement>();
        anchor.set_href(&url);
        anchor.set_download(name);
        // Firefox follows only a link in the document.
        body.append_child(&anchor).map_err(failed)?;
        anchor.click();
        anchor.remove();
        // Revoked once the browser has had time to start the download.
        let revoke = Closure::once_into_js(move || {
            let _ignored = Url::revoke_object_url(&url);
        });
        if let Some(window) = web_sys::window() {
            let _id = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                revoke.unchecked_ref(),
                60_000,
            );
        }
        Ok(())
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
    fn a_stamp_follows_the_file_and_a_directory_is_not_a_file() {
        let dir = std::env::temp_dir().join(format!("afik-stamp-{}", std::process::id()));
        let file = dir.join("image.bin");
        std::fs::create_dir_all(&dir).unwrap();
        let absent = super::stamp(&file);
        std::fs::write(&file, [1, 2, 3]).unwrap();
        let (file_is_file, dir_is_file) = (super::is_file(&file), super::is_file(&dir));
        let present = super::stamp(&file);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(absent, None);
        assert_eq!(present.map(|(_, length)| length), Some(3));
        assert!(file_is_file && !dir_is_file);
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
