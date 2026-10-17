//! The interactive map on the Map tab: OpenStreetMap tiles, the locator,
//! the saved locators, and the sites each generator would add, with their
//! radius.
//!
//! Drag pans, the wheel or the buttons zoom, and a click picks the locator
//! under the pointer.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::tiles::{self, TileId, Tiles, MAX_ZOOM, MIN_ZOOM, TILE_SIZE};

/// The Earth's circumference at the equator, in kilometres.
const EQUATOR_KM: f64 = 40_075.017;
/// How near the pointer a site must be, in points, to be named.
const HOVER_POINTS: f32 = 10.0;
/// The most names a tooltip lists before saying how many more.
const HOVER_NAMES: usize = 12;

/// Something a generator would add, where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    /// What it is called, such as a callsign or channel name.
    pub name: String,
    /// Latitude in degrees north.
    pub latitude: f64,
    /// Longitude in degrees east.
    pub longitude: f64,
    /// Whether it is singled out, such as an airport given a bank of its
    /// own; drawn as a disc filled in its layer's colour.
    pub highlight: bool,
}

/// One generator's sites, drawn in one colour.
#[derive(Clone, Debug)]
pub struct Layer {
    /// The generator, for the tooltip.
    pub label: &'static str,
    /// The sites' colour.
    pub color: Color32,
    /// The glyph each site is marked with, such as an aeroplane.
    pub icon: &'static str,
    /// The sites the bank would take.
    pub sites: Vec<Site>,
    /// The radius the generator takes from, drawn about the locator.
    pub radius_km: Option<f64>,
}

/// A named point drawn on the map, such as a saved locator.
#[derive(Clone, Debug)]
pub struct Pin {
    /// Its label.
    pub label: String,
    /// Latitude and longitude in degrees.
    pub position: (f64, f64),
}

/// Where the map looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapView {
    /// Latitude and longitude at the middle of the map.
    pub centre: (f64, f64),
    /// Tile zoom.
    pub zoom: u8,
}

impl Default for MapView {
    /// The British Isles.
    fn default() -> Self {
        Self {
            centre: (54.0, -2.5),
            zoom: 5,
        }
    }
}

/// The six-character Maidenhead locator holding `latitude`, `longitude`.
pub fn locator_at(latitude: f64, longitude: f64) -> String {
    // Kept just inside the grid, so the poles and the antimeridian map to
    // its last square rather than past it.
    let longitude = (longitude + 180.0).clamp(0.0, 359.999_999);
    let latitude = (latitude + 90.0).clamp(0.0, 179.999_999);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let digit = |value: f64, base: u8| char::from(base + value.floor() as u8);
    [
        digit(longitude / 20.0, b'A'),
        digit(latitude / 10.0, b'A'),
        digit(longitude % 20.0 / 2.0, b'0'),
        digit(latitude % 10.0, b'0'),
        digit(longitude % 2.0 * 12.0, b'A'),
        digit(latitude % 1.0 * 24.0, b'A'),
    ]
    .into_iter()
    .collect()
}

/// The map's mapping between screen points and world pixels.
struct Projection {
    rect: Rect,
    zoom: u8,
    centre: (f64, f64),
}

impl Projection {
    fn new(rect: Rect, view: &MapView) -> Self {
        Self {
            rect,
            zoom: view.zoom,
            centre: tiles::world_point(view.centre.0, view.centre.1, view.zoom),
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn screen(&self, (latitude, longitude): (f64, f64)) -> Pos2 {
        let (x, y) = tiles::world_point(latitude, longitude, self.zoom);
        self.rect.center() + Vec2::new((x - self.centre.0) as f32, (y - self.centre.1) as f32)
    }

    fn position(&self, point: Pos2) -> (f64, f64) {
        let offset = point - self.rect.center();
        tiles::lat_lon(
            self.centre.0 + f64::from(offset.x),
            self.centre.1 + f64::from(offset.y),
            self.zoom,
        )
    }

    /// Points per kilometre at `latitude`.
    fn points_per_km(&self, latitude: f64) -> f64 {
        TILE_SIZE * f64::from(1_u32 << self.zoom) / (EQUATOR_KM * latitude.to_radians().cos())
    }
}

/// Moves `view` so the world pixel under `point` stays put as the zoom
/// changes to `zoom`.
fn zoom_about(view: &mut MapView, rect: Rect, point: Pos2, zoom: u8) {
    let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    if zoom == view.zoom {
        return;
    }
    let anchor = Projection::new(rect, view).position(point);
    view.zoom = zoom;
    // Put the anchor back under the pointer.
    let projection = Projection::new(rect, view);
    let drift = projection.screen(anchor) - point;
    view.centre = projection.position(rect.center() + drift);
}

/// Draws the map filling `ui` and handles panning and zooming; the position
/// clicked, if any.
#[allow(clippy::too_many_lines)]
pub fn show(
    ui: &mut egui::Ui,
    view: &mut MapView,
    tiles: &mut Tiles,
    home: Option<(f64, f64)>,
    pins: &[Pin],
    layers: &[Layer],
) -> Option<(f64, f64)> {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    if response.dragged() {
        let projection = Projection::new(rect, view);
        view.centre = projection.position(rect.center() - response.drag_delta());
    }
    if let Some(pointer) = response.hover_pos() {
        let scroll = ui.input(|input| input.raw_scroll_delta.y);
        if scroll > 0.0 {
            zoom_about(view, rect, pointer, view.zoom.saturating_add(1));
        } else if scroll < 0.0 {
            zoom_about(view, rect, pointer, view.zoom.saturating_sub(1));
        }
    }
    view.centre.0 = view
        .centre
        .0
        .clamp(-tiles::MAX_LATITUDE, tiles::MAX_LATITUDE);
    let projection = Projection::new(rect, view);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_gray(0xD8));

    // Tiles, wrapping east and west.
    let count = 1_i64 << view.zoom;
    #[allow(clippy::cast_possible_truncation)]
    let size = TILE_SIZE as f32;
    let top_left = projection.position(rect.left_top());
    let (left, top) = tiles::world_point(top_left.0, top_left.1, view.zoom);
    #[allow(clippy::cast_possible_truncation)]
    let first = (
        (left / TILE_SIZE).floor() as i64,
        (top / TILE_SIZE).floor() as i64,
    );
    #[allow(clippy::cast_possible_truncation)]
    let (across, down) = (
        (rect.width() / size).ceil() as i64 + 1,
        (rect.height() / size).ceil() as i64 + 1,
    );
    for row in first.1..=first.1 + down {
        if !(0..count).contains(&row) {
            continue;
        }
        for column in first.0..=first.0 + across {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let id = TileId {
                zoom: view.zoom,
                x: column.rem_euclid(count) as u32,
                y: row as u32,
            };
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let corner = rect.center()
                + Vec2::new(
                    (column as f64 * TILE_SIZE - projection.centre.0) as f32,
                    (row as f64 * TILE_SIZE - projection.centre.1) as f32,
                );
            let tile = Rect::from_min_size(corner, Vec2::splat(size));
            if let Some(texture) = tiles.get(id) {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(texture.id(), tile, uv, Color32::WHITE);
            } else {
                painter.rect_stroke(tile, 0.0, Stroke::new(1.0_f32, Color32::from_gray(0xC0)));
            }
        }
    }

    // Each generator's radius, then its sites.
    if let Some(home) = home {
        for layer in layers {
            if let Some(km) = layer.radius_km {
                #[allow(clippy::cast_possible_truncation)]
                let radius = (km * projection.points_per_km(home.0)) as f32;
                painter.circle(
                    projection.screen(home),
                    radius,
                    layer.color.gamma_multiply(0.08),
                    Stroke::new(2.0_f32, layer.color),
                );
            }
        }
    }
    // Each site is its layer's glyph on a white disc ringed in its colour,
    // or white on a disc of its colour when singled out, drawn on top.
    let icon_font = egui::FontId::proportional(13.0);
    for highlight in [false, true] {
        for layer in layers {
            for site in layer
                .sites
                .iter()
                .filter(|site| site.highlight == highlight)
            {
                let at = projection.screen((site.latitude, site.longitude));
                let (disc, glyph) = if highlight {
                    (layer.color, Color32::WHITE)
                } else {
                    (Color32::WHITE, layer.color)
                };
                painter.circle(at, 9.0, disc, Stroke::new(1.5_f32, layer.color));
                painter.text(
                    at,
                    egui::Align2::CENTER_CENTER,
                    layer.icon,
                    icon_font.clone(),
                    glyph,
                );
            }
        }
    }

    // Saved locators, then the locator itself on top.
    let text = |at: Pos2, label: &str| {
        let font = egui::FontId::proportional(13.0);
        let galley = painter.layout_no_wrap(label.to_owned(), font, Color32::BLACK);
        let place = Rect::from_min_size(at + Vec2::new(8.0, -8.0), galley.size()).expand(2.0);
        painter.rect_filled(place, 2.0, Color32::from_white_alpha(0xD0));
        painter.galley(place.min + Vec2::splat(2.0), galley, Color32::BLACK);
    };
    for pin in pins {
        let at = projection.screen(pin.position);
        painter.rect(
            Rect::from_center_size(at, Vec2::splat(9.0)),
            1.0,
            Color32::from_rgb(0x1F, 0x5F, 0xBF),
            Stroke::new(1.5_f32, Color32::WHITE),
        );
        text(at, &pin.label);
    }
    if let Some(home) = home {
        let at = projection.screen(home);
        painter.circle(
            at,
            7.0,
            Color32::from_rgb(0xD0, 0x20, 0x20),
            Stroke::new(2.0_f32, Color32::WHITE),
        );
    }

    let attribution = painter.layout_no_wrap(
        tiles::ATTRIBUTION.to_owned(),
        egui::FontId::proportional(11.0),
        Color32::BLACK,
    );
    let place = Rect::from_min_size(
        rect.right_bottom() - attribution.size() - Vec2::splat(4.0),
        attribution.size(),
    )
    .expand(2.0);
    painter.rect_filled(place, 0.0, Color32::from_white_alpha(0xC0));
    painter.galley(place.min + Vec2::splat(2.0), attribution, Color32::BLACK);

    // Names under the pointer, and the locator there.
    if let Some(pointer) = response.hover_pos() {
        let mut names: Vec<String> = layers
            .iter()
            .flat_map(|layer| layer.sites.iter().map(move |site| (layer, site)))
            .filter(|(_, site)| {
                projection
                    .screen((site.latitude, site.longitude))
                    .distance(pointer)
                    <= HOVER_POINTS
            })
            .map(|(layer, site)| format!("{}: {}", layer.label, site.name))
            .collect();
        names.dedup();
        let more = names.len().saturating_sub(HOVER_NAMES);
        names.truncate(HOVER_NAMES);
        if more > 0 {
            names.push(format!("and {more} more"));
        }
        let (latitude, longitude) = projection.position(pointer);
        names.push(format!(
            "{} ({latitude:.3}, {longitude:.3}); click to choose",
            locator_at(latitude, longitude)
        ));
        // Each line is kept whole: egui sizes a pointer tooltip from the
        // space it had last frame, which wraps short lines towards the right.
        response.clone().on_hover_ui_at_pointer(|ui| {
            for name in &names {
                ui.add(egui::Label::new(name).extend());
            }
        });
    }
    if response.clicked() {
        return response
            .interact_pointer_pos()
            .map(|pointer| projection.position(pointer));
    }
    None
}

/// Zooms `view` in or out by one about the middle of the map.
pub fn zoom_by(view: &mut MapView, steps: i8) {
    let zoom = if steps > 0 {
        view.zoom.saturating_add(steps.unsigned_abs())
    } else {
        view.zoom.saturating_sub(steps.unsigned_abs())
    };
    view.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
}

#[cfg(test)]
mod tests {
    use super::{locator_at, zoom_about, MapView, Projection};
    use crate::repeaters::locator_centre;
    use eframe::egui::{Pos2, Rect};

    #[test]
    fn a_position_gives_the_locator_holding_it() {
        assert_eq!(locator_at(51.52, -0.12), "IO91WM");
        assert_eq!(locator_at(48.86, 2.35), "JN18EU");
        assert_eq!(locator_at(-33.87, 151.21), "QF56OD");
        assert_eq!(
            locator_at(90.0, 180.0),
            "RR99XX",
            "the far corner stays on the grid"
        );
        for locator in ["IO91WM", "JN18EU", "QF56OD", "AA00AA"] {
            let (latitude, longitude) = locator_centre(locator).unwrap();
            assert_eq!(locator_at(latitude, longitude), locator);
        }
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
        let mut view = MapView {
            centre: (51.5, -0.1),
            zoom: 8,
        };
        let pointer = Pos2::new(600.0, 150.0);
        let before = Projection::new(rect, &view).position(pointer);
        zoom_about(&mut view, rect, pointer, 9);
        assert_eq!(view.zoom, 9);
        let after = Projection::new(rect, &view).position(pointer);
        assert!((before.0 - after.0).abs() < 1e-3 && (before.1 - after.1).abs() < 1e-3);
        zoom_about(&mut view, rect, pointer, 30);
        assert_eq!(view.zoom, super::MAX_ZOOM, "clamped");
    }
}
