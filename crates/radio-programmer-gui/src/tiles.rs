//! OpenStreetMap tiles for the Map tab, fetched on worker threads and
//! cached in `$XDG_CACHE_HOME/afik/tiles`.
//!
//! Unlike the station lists, tiles are fetched while the map is shown,
//! without a Sync press (operator decision, 2026-10-17). OpenStreetMap's tile
//! policy is kept: the studio names itself, uses at most two connections,
//! and reads a cached tile before fetching it, refetching only one older
//! than [`STALE_AFTER`].
//!
//! In the browser (`ADR-119`) the page fetches them, two at a time, and the
//! browser's own HTTP cache keeps them: the page's storage is too small.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, SystemTime},
};

use eframe::egui;

/// Edge of a tile in pixels.
pub const TILE_SIZE: f64 = 256.0;
/// The closest zoom OpenStreetMap serves.
pub const MAX_ZOOM: u8 = 18;
/// The farthest zoom the map allows: the whole of Europe and more.
pub const MIN_ZOOM: u8 = 3;
/// The furthest north or south the Web Mercator projection reaches.
pub const MAX_LATITUDE: f64 = 85.051_128;
/// Where tiles are fetched from.
const TILE_URL: &str = "https://tile.openstreetmap.org";
/// What the map must show under it.
pub const ATTRIBUTION: &str = "\u{a9} OpenStreetMap contributors";
/// OpenStreetMap asks for no more than two connections.
const WORKERS: usize = 2;
/// A cached tile older than this is fetched again, keeping the old one if
/// the fetch fails.
const STALE_AFTER: Duration = Duration::from_secs(30 * 24 * 60 * 60);
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const TIMEOUT: Duration = Duration::from_secs(20);
/// No tile is bigger; a larger response is not a tile.
const MAX_TILE_BYTES: u64 = 1 << 20;
/// Tiles kept as textures; the least recently drawn go first.
const MAX_SLOTS: usize = 600;

/// One tile of the Web Mercator pyramid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileId {
    /// Zoom: the world is `2^zoom` tiles across.
    pub zoom: u8,
    /// Column, from 180 degrees west.
    pub x: u32,
    /// Row, from the north.
    pub y: u32,
}

/// The world-pixel position of `latitude`, `longitude` at `zoom`: the
/// world is `TILE_SIZE * 2^zoom` pixels across, from the north-west corner.
pub fn world_point(latitude: f64, longitude: f64, zoom: u8) -> (f64, f64) {
    let size = TILE_SIZE * f64::from(1_u32 << zoom);
    let latitude = latitude.clamp(-MAX_LATITUDE, MAX_LATITUDE).to_radians();
    let x = (longitude + 180.0) / 360.0 * size;
    let y = (1.0 - latitude.tan().asinh() / std::f64::consts::PI) / 2.0 * size;
    (x, y)
}

/// The latitude and longitude at world pixel `x`, `y` at `zoom`, the
/// inverse of [`world_point`].
pub fn lat_lon(x: f64, y: f64, zoom: u8) -> (f64, f64) {
    let size = TILE_SIZE * f64::from(1_u32 << zoom);
    let longitude = x / size * 360.0 - 180.0;
    let latitude = (std::f64::consts::PI * (1.0 - 2.0 * y / size))
        .sinh()
        .atan()
        .to_degrees();
    (latitude, longitude)
}

/// Fetches a tile's PNG bytes.
pub type Fetch = Arc<dyn Fn(TileId) -> Result<Vec<u8>, String> + Send + Sync>;

/// Where one tile is fetched from.
fn tile_url(id: TileId) -> String {
    format!("{TILE_URL}/{}/{}/{}.png", id.zoom, id.x, id.y)
}

/// Fetches one tile from OpenStreetMap.
#[cfg(not(target_arch = "wasm32"))]
fn fetch_tile(id: TileId) -> Result<Vec<u8>, String> {
    let url = tile_url(id);
    log::debug!("fetching {url}");
    crate::net::get(
        &url,
        TIMEOUT,
        MAX_TILE_BYTES,
        concat!(
            "afik-studio/",
            env!("CARGO_PKG_VERSION"),
            " (radio programmer map)"
        ),
    )
}

/// Decodes a tile's PNG into an image egui can draw.
fn decode(bytes: &[u8]) -> Result<egui::ColorImage, String> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| format!("not a PNG tile: {error}"))?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}

/// Reads a tile from the cache, else fetches and caches it.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn load(
    id: TileId,
    dir: Option<&PathBuf>,
    fetch: Option<&Fetch>,
) -> Result<egui::ColorImage, String> {
    let path = dir.map(|dir| {
        dir.join(id.zoom.to_string())
            .join(id.x.to_string())
            .join(format!("{}.png", id.y))
    });
    let cached = path.as_ref().and_then(|path| {
        let age = fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())?;
        Some((fs::read(path).ok()?, age > STALE_AFTER))
    });
    let fetched = match (&cached, fetch) {
        (Some((_, false)), _) | (_, None) => None,
        (_, Some(fetch)) => Some(fetch(id)),
    };
    match (fetched, cached) {
        (Some(Ok(bytes)), _) => {
            let image = decode(&bytes)?;
            if let Some(path) = &path {
                // A tile that cannot be cached is still drawn.
                let partial = path.with_extension("part");
                let written = path
                    .parent()
                    .map_or(Ok(()), fs::create_dir_all)
                    .and_then(|()| fs::write(&partial, &bytes))
                    .and_then(|()| fs::rename(&partial, path));
                if let Err(error) = written {
                    log::warn!("could not cache tile {}: {error}", path.display());
                }
            }
            Ok(image)
        }
        (_, Some((bytes, _))) => decode(&bytes),
        (Some(Err(error)), None) => Err(error),
        (None, None) => Err("not cached".to_owned()),
    }
}

/// A tile asked for.
enum Slot {
    /// On its way.
    Pending,
    /// Ready to draw.
    Ready(egui::TextureHandle),
    /// Neither cached nor fetchable; the map shows a blank square.
    Missing,
}

/// The tiles asked for, loaded on worker threads.
pub struct Tiles {
    slots: HashMap<TileId, (Slot, u64)>,
    /// Counts frames, to drop the tiles drawn least recently.
    frame: u64,
    requests: Option<mpsc::Sender<TileId>>,
    done: mpsc::Receiver<(TileId, Result<egui::ColorImage, String>)>,
    /// The page's fetches, which stand in for the worker threads.
    #[cfg(target_arch = "wasm32")]
    page: Option<PageFetches>,
}

/// Tiles the page fetches itself, at most [`WORKERS`] at once.
#[cfg(target_arch = "wasm32")]
struct PageFetches {
    queue: mpsc::Receiver<TileId>,
    finished: mpsc::Sender<(TileId, Result<egui::ColorImage, String>)>,
    in_flight: usize,
}

#[cfg(target_arch = "wasm32")]
impl PageFetches {
    /// Starts queued fetches while fewer than [`WORKERS`] run.
    fn start(&mut self) {
        while self.in_flight < WORKERS {
            let Ok(id) = self.queue.try_recv() else {
                return;
            };
            self.in_flight += 1;
            let finished = self.finished.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let image = crate::net::fetch(
                    &tile_url(id),
                    usize::try_from(MAX_TILE_BYTES).unwrap_or(usize::MAX),
                )
                .await
                .and_then(|bytes| decode(&bytes));
                let _ignored = finished.send((id, image));
            });
        }
    }
}

impl Tiles {
    /// Tiles from OpenStreetMap, cached under the XDG cache directory.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open() -> Self {
        let dir = crate::paths::cache_dir().map(|dir| dir.join("tiles"));
        let fetch: Fetch = Arc::new(fetch_tile);
        Self::with(dir.as_deref(), Some(&fetch))
    }

    /// Tiles from OpenStreetMap, fetched by the page and kept by the
    /// browser's HTTP cache.
    #[cfg(target_arch = "wasm32")]
    pub fn open() -> Self {
        let (requests, queue) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        Self {
            slots: HashMap::new(),
            frame: 0,
            requests: Some(requests),
            done,
            page: Some(PageFetches {
                queue,
                finished,
                in_flight: 0,
            }),
        }
    }

    /// No tiles at all, for tests: every tile is blank, and nothing is read
    /// or fetched.
    pub fn none() -> Self {
        let (_, done) = mpsc::channel();
        Self {
            slots: HashMap::new(),
            frame: 0,
            requests: None,
            done,
            #[cfg(target_arch = "wasm32")]
            page: None,
        }
    }

    /// Tiles cached in `dir` and fetched with `fetch`, either of which may
    /// be absent.
    pub fn with(dir: Option<&Path>, fetch: Option<&Fetch>) -> Self {
        let (dir, fetch) = (dir.map(Path::to_path_buf), fetch.cloned());
        let (requests, queue) = mpsc::channel::<TileId>();
        let (finished, done) = mpsc::channel();
        let queue = Arc::new(Mutex::new(queue));
        for _ in 0..WORKERS {
            let (queue, finished, dir, fetch) = (
                Arc::clone(&queue),
                finished.clone(),
                dir.clone(),
                fetch.clone(),
            );
            thread::spawn(move || loop {
                // The lock is held only to take the next request.
                let next = queue
                    .lock()
                    .map_err(|_| ())
                    .and_then(|q| q.recv().map_err(|_| ()));
                let Ok(id) = next else {
                    return;
                };
                let image = load(id, dir.as_ref(), fetch.as_ref());
                if finished.send((id, image)).is_err() {
                    return;
                }
            });
        }
        Self {
            slots: HashMap::new(),
            frame: 0,
            requests: Some(requests),
            done,
            #[cfg(target_arch = "wasm32")]
            page: None,
        }
    }

    /// Takes in loaded tiles and starts a new frame; whether any are still
    /// on their way, so the map should be drawn again soon.
    pub fn poll(&mut self, context: &egui::Context) -> bool {
        self.frame += 1;
        while let Ok((id, image)) = self.done.try_recv() {
            #[cfg(target_arch = "wasm32")]
            if let Some(page) = &mut self.page {
                page.in_flight = page.in_flight.saturating_sub(1);
            }
            let slot = match image {
                Ok(image) => {
                    let name = format!("tile-{}-{}-{}", id.zoom, id.x, id.y);
                    Slot::Ready(context.load_texture(name, image, egui::TextureOptions::LINEAR))
                }
                Err(error) => {
                    log::debug!("tile {id:?}: {error}");
                    Slot::Missing
                }
            };
            let used = self.slots.get(&id).map_or(self.frame, |(_, used)| *used);
            self.slots.insert(id, (slot, used));
        }
        if self.slots.len() > MAX_SLOTS {
            let mut drawn: Vec<_> = self
                .slots
                .iter()
                .filter(|(_, (slot, _))| !matches!(slot, Slot::Pending))
                .map(|(id, (_, used))| (*used, *id))
                .collect();
            drawn.sort_unstable_by_key(|(used, _)| *used);
            for (_, id) in drawn.iter().take(self.slots.len() - MAX_SLOTS) {
                self.slots.remove(id);
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(page) = &mut self.page {
            page.start();
        }
        self.slots
            .values()
            .any(|(slot, _)| matches!(slot, Slot::Pending))
    }

    /// The texture for `id` once loaded, asking for it the first time.
    pub fn get(&mut self, id: TileId) -> Option<&egui::TextureHandle> {
        let frame = self.frame;
        let requests = &self.requests;
        let (slot, used) = self.slots.entry(id).or_insert_with(|| {
            let sent = requests
                .as_ref()
                .is_some_and(|requests| requests.send(id).is_ok());
            (if sent { Slot::Pending } else { Slot::Missing }, frame)
        });
        *used = frame;
        match slot {
            Slot::Ready(texture) => Some(texture),
            Slot::Pending | Slot::Missing => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{lat_lon, tile_url, world_point, Fetch, TileId, Tiles, TILE_SIZE};
    use eframe::egui;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn png() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(256, 256, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn positions_go_to_world_pixels_and_back() {
        // London at zoom 10 is in tile 511, 340.
        let (x, y) = world_point(51.5, -0.12, 10);
        assert_eq!(
            ((x / TILE_SIZE).floor(), (y / TILE_SIZE).floor()),
            (511.0, 340.0)
        );
        let (latitude, longitude) = lat_lon(x, y, 10);
        assert!((latitude - 51.5).abs() < 1e-9 && (longitude + 0.12).abs() < 1e-9);
        assert_eq!(world_point(0.0, -180.0, 0), (0.0, TILE_SIZE / 2.0));
    }

    #[test]
    fn desktop_and_browser_fetch_a_tile_from_one_place() {
        let id = TileId {
            zoom: 10,
            x: 511,
            y: 340,
        };
        assert_eq!(
            tile_url(id),
            "https://tile.openstreetmap.org/10/511/340.png"
        );
    }

    #[test]
    fn a_tile_is_fetched_once_then_read_from_the_cache() {
        let dir = std::env::temp_dir().join(format!("afik-tiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let fetches = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&fetches);
        let bytes = png();
        let fetch: Fetch = Arc::new(move |id: TileId| {
            counted.fetch_add(1, Ordering::SeqCst);
            if id.x == 0 {
                Ok(bytes.clone())
            } else {
                Err("offline".to_owned())
            }
        });
        let context = egui::Context::default();
        let id = TileId {
            zoom: 3,
            x: 0,
            y: 1,
        };
        let missing = TileId {
            zoom: 3,
            x: 1,
            y: 1,
        };
        let wait = |tiles: &mut Tiles| {
            for _ in 0..500 {
                if !tiles.poll(&context) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("tiles never loaded");
        };

        let mut tiles = Tiles::with(Some(&dir), Some(&fetch));
        assert!(tiles.get(id).is_none(), "asked for, not yet loaded");
        assert!(tiles.get(missing).is_none());
        wait(&mut tiles);
        assert_eq!(tiles.get(id).unwrap().size(), [256, 256]);
        assert!(tiles.get(missing).is_none(), "a failed fetch stays blank");
        assert!(dir.join("3/0/1.png").exists(), "cached");
        assert_eq!(fetches.load(Ordering::SeqCst), 2);

        // A fresh start reads the cache without fetching.
        let mut tiles = Tiles::with(Some(&dir), Some(&fetch));
        tiles.get(id);
        wait(&mut tiles);
        assert!(tiles.get(id).is_some());
        assert_eq!(fetches.load(Ordering::SeqCst), 2);

        let mut none = Tiles::none();
        assert!(none.get(id).is_none());
        assert!(!none.poll(&context), "nothing is ever pending");
        let _ = std::fs::remove_dir_all(dir);
    }
}
