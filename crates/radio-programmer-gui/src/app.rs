//! The egui application drawing the validated project model.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use crate::files as fs;

use eframe::egui::{
    self, collapsing_header::CollapsingState, ComboBox, Grid, RichText, ScrollArea, TextEdit,
};
use radio_channel_plan::{
    BankSource, SharedSettings, MAX_BANKS, MAX_BANK_NAME_LEN, MAX_GENERATED_CHANNELS, MAX_GROUPS,
};
use radio_domain::{
    Bandwidth, Modulation, PowerLevel, ScanResume, TxClass, BACKLIGHT_ALWAYS_ON,
    MAX_BATTERY_SAVE_RATIO, MAX_SQUELCH_LEVEL, TX_POWER_LEVELS, TX_TIMEOUT_CHOICES,
};
use radio_flasher::ImageWriteReport;
use radio_platform::operator_display::tone_label;
use radio_platform::operator_shell::{
    tone_at_row, TONE_ROWS, TX_CLASSES, TX_POWER_NOMINAL_MW, VFO_STEPS_HZ,
};
use radio_tx_policy::MAX_TX_RANGES;
use web_time::Instant;

use radio_programmer_serial::SUPPORTED_BAUDS;

use crate::{
    airfields::{self, AirfieldArea},
    airports::{
        self, AirportArea, AirportCache, AirportFrequency, AirportList, AirportOptions,
        AirportSize, Service,
    },
    device::{DeviceChoice, DeviceChooser},
    flash_tab::FlashTab,
    locators::{SavedLocator, SavedLocators},
    map::{self, Layer, MapView, Pin, Site, SiteSize},
    marine::{self, MarineArea, MarineCache, MarineList},
    model::{
        move_to, trailing_locator, BankDraft, BankKind, BankRole, ChannelDraft, GroupDraft,
        ModelError, Origin, ProjectModel, StartDraft, StorageSummary, ToneDraft, ToneKind,
        TxRangeDraft, VfoDraft, AIRBAND_GROUP, COMMON_GROUP, MARINE_GROUP, MAX_PROJECT_IMAGE_BYTES,
        RADIO_IMAGE_BYTES,
    },
    presets::{satellite_bank, PRESETS, SATELLITE_PRESET, TX_RANGE_PRESETS},
    projects::{ProjectStore, PROJECT_EXTENSION},
    receiver::Receiver,
    repeaters::{
        self, AmateurBand, BandFilter, Coverage, Repeater, RepeaterArea, RepeaterCache,
        RepeaterList, RepeaterReach,
    },
    satellites::{SatelliteCache, SatelliteList, SyncJob},
    session::{DeviceJob, DeviceOutcome, DeviceSession, JobKind},
    stations::{self, StationArea, StationCache, StationKind, StationList},
    tiles::Tiles,
    world::{self, WorldCache, WorldList},
    DeviceSelector, Options,
};

/// The grip a bank is dragged by.
const DRAG_HANDLE: &str = "\u{2630}";

/// Columns in a bank row's header.
const BANK_COLUMNS: usize = 4;

/// The least space after each bank header column, in points.
const MIN_COLUMN_GAP: f32 = 16.0;

/// Channels of one generated plan the editor lists before summarising the rest.
const EXPANSION_PREVIEW_ROWS: usize = 32;
/// Colour used for advisory text which is not a validation failure.
const WARNING_COLOUR: egui::Color32 = egui::Color32::from_rgb(0xB7, 0x6E, 0x00);

/// What the radio-wide squelch level actually governs, said once.
const SQUELCH_HINT: &str = "Applies to every channel on the radio. The operator \
can change it from the handset settings menu, and the radio stores what they \
chose.";

/// What the TX power and timeout settings govern, said once.
const TX_HINT: &str = "The default TX power is a VFO's when the VFO section \
sets none of its own; a channel transmits at its own or its bank's. The \
operator can override it for every channel from the handset settings menu \
until the radio is next switched off.";

/// What programming a VFO does, said once.
const VFO_HINT: &str = "Each slot's start and VFO are written as the place the \
radio keeps over a power cycle: the radio goes there now and after a power \
cycle, until the operator moves it. A slot with no start begins as a radio \
nobody has used does, A on every channel and B on its VFO. The operator can \
change a VFO from the handset's settings menu, and the radio keeps those \
changes until the next write that programs it.";

/// What the TX ranges do, said once.
const TX_RANGES_HINT: &str = "The only frequencies this radio may transmit on, \
in channel and VFO mode alike. Each range names its class, which the handset \
can switch off to lend the radio, and the most power it allows; a higher power \
level is refused there. With no range the radio never transmits.";

/// What the backlight timeout does, said once.
const BACKLIGHT_HINT: &str = "How long the display stays lit after a key or PTT \
press, or after a received signal ends. 0 keeps it off; 255 keeps it on.";

/// What battery save does, said once.
const BATTERY_SAVE_HINT: &str = "After 10 s with no key, signal or scan, the \
receiver sleeps this many times 100 ms, then listens for 100 ms. 0 is off. \
Off while dual watch runs.";

/// Which editor tab is visible.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Tab {
    Projects,
    #[default]
    Banks,
    Map,
    Radio,
    Device,
    Flash,
}

/// Which bank generator the Banks tab shows below the shared locator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Generator {
    #[default]
    Repeaters,
    Gateways,
    Beacons,
    Stations,
    Airports,
    Airfields,
    Marine,
}

/// Draws one generator tab: rounded on top, outlined, and filled like the
/// panel below when chosen, faint otherwise.
fn generator_tab(
    ui: &mut egui::Ui,
    generator: Generator,
    chosen: Generator,
    panel_fill: egui::Color32,
) -> egui::Response {
    let selected = generator == chosen;
    let text = egui::WidgetText::from(generator.label()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let padding = egui::vec2(10.0, 5.0);
    let (rect, response) =
        ui.allocate_exact_size(text.size() + 2.0 * padding, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            generator.label(),
        )
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let fill = if selected {
            panel_fill
        } else if response.hovered() {
            visuals.widgets.hovered.weak_bg_fill
        } else {
            visuals.faint_bg_color
        };
        let stroke = if response.has_focus() {
            visuals.selection.stroke
        } else {
            visuals.widgets.noninteractive.bg_stroke
        };
        let rounding = egui::CornerRadius {
            nw: 4,
            ne: 4,
            sw: 0,
            se: 0,
        };
        ui.painter()
            .rect(rect, rounding, fill, stroke, egui::StrokeKind::Middle);
        let colour = if selected {
            // The theme's accent.
            visuals.selection.stroke.color
        } else {
            visuals.text_color()
        };
        ui.painter().galley(rect.min + padding, text, colour);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The state file the map's shown generators and all-locations choice are
/// kept in.
const MAP_VIEW_FILE: &str = "map-view";

/// Where the map's view is kept, and every filter as last written, so each
/// change is kept for the next start and nothing is written while none
/// changes.
#[derive(Default)]
struct RememberedFilters {
    map_view_file: Option<PathBuf>,
    /// Where the repeaters' reach is kept.
    repeater_reach_file: Option<PathBuf>,
    /// The filters as last written; none until first looked at.
    last: Option<String>,
}

/// The title of the panel holding the bank generators.
const GENERATORS_TITLE: &str = "Banks near your locator";
/// The saved-locator dropdown's entry showing every saved locator on the map.
const ALL_LOCATIONS: &str = "All locations";

impl Generator {
    const ALL: [Self; 7] = [
        Self::Repeaters,
        Self::Gateways,
        Self::Beacons,
        Self::Stations,
        Self::Airports,
        Self::Airfields,
        Self::Marine,
    ];

    /// The word the map-view file keeps it as.
    const fn key(self) -> &'static str {
        match self {
            Self::Repeaters => "repeaters",
            Self::Gateways => "gateways",
            Self::Beacons => "beacons",
            Self::Stations => "stations",
            Self::Airports => "airports",
            Self::Airfields => "airfields",
            Self::Marine => "marine",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Repeaters => "Repeaters",
            Self::Gateways => "UK gateways",
            Self::Beacons => "UK beacons",
            Self::Stations => "FM stations",
            Self::Airports => "Airports",
            Self::Airfields => "Airfields",
            Self::Marine => "Marine",
        }
    }

    /// The glyph its sites are marked with on the map, from egui's own
    /// fonts.
    const fn icon(self) -> &'static str {
        match self {
            Self::Repeaters => "\u{1F4E1}",
            Self::Gateways => "\u{1F5A7}",
            Self::Beacons => "\u{1F5FC}",
            Self::Stations => "\u{1F4FB}",
            Self::Airports | Self::Airfields => "\u{2708}",
            Self::Marine => "\u{2693}",
        }
    }

    /// The colour its sites and radius are drawn in on the map.
    const fn color(self) -> egui::Color32 {
        match self {
            Self::Repeaters => egui::Color32::from_rgb(0xE0, 0x6C, 0x00),
            Self::Gateways => egui::Color32::from_rgb(0x00, 0x90, 0x90),
            Self::Beacons => egui::Color32::from_rgb(0xC0, 0x9C, 0x00),
            Self::Stations => egui::Color32::from_rgb(0x20, 0x60, 0xD0),
            Self::Airports => egui::Color32::from_rgb(0x30, 0x30, 0x30),
            Self::Airfields => egui::Color32::from_rgb(0x2E, 0x9E, 0x40),
            Self::Marine => egui::Color32::from_rgb(0x00, 0x50, 0x80),
        }
    }
}

/// The ETCC systems other than repeaters a bank is built from, out of the
/// repeaters' list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EtccKind {
    Gateways,
    Beacons,
}

impl EtccKind {
    /// The generator its bank is recorded as made by.
    const fn source(self) -> BankSource {
        match self {
            Self::Gateways => BankSource::Gateways,
            Self::Beacons => BankSource::Beacons,
        }
    }

    /// What the bank holds, as the status line names them.
    const fn noun(self) -> &'static str {
        match self {
            Self::Gateways => "gateways",
            Self::Beacons => "beacons",
        }
    }

    fn systems(self, list: &RepeaterList) -> &[Repeater] {
        match self {
            Self::Gateways => &list.gateways,
            Self::Beacons => &list.beacons,
        }
    }

    fn bank(self, locator: &str, near: &[(&Repeater, f64)]) -> BankDraft {
        match self {
            Self::Gateways => repeaters::gateway_bank(locator, near),
            Self::Beacons => repeaters::beacon_bank(locator, near),
        }
    }
}

/// Which group of radio settings the Radio tab shows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum RadioSection {
    #[default]
    Receive,
    Transmit,
    Vfo,
    Scan,
    General,
}

impl RadioSection {
    const ALL: [Self; 5] = [
        Self::Receive,
        Self::Transmit,
        Self::Vfo,
        Self::Scan,
        Self::General,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Receive => "Receive",
            Self::Transmit => "Transmit",
            Self::Vfo => "VFO",
            Self::Scan => "Scan",
            Self::General => "General",
        }
    }
}

/// The complete native editor.
pub struct StudioApp {
    tab: Tab,
    /// The bank generator tab chosen on the Banks tab.
    generator: Generator,
    radio_section: RadioSection,
    project: ProjectModel,
    /// The project as last loaded or saved, to tell whether it has changed.
    saved: ProjectModel,
    projects: ProjectStore,
    new_project: String,
    /// Whether Delete was pressed and waits for Yes or Cancel.
    confirm_delete: bool,
    status: String,
    /// The status line as last logged at debug level, so `-v` shows each
    /// one once.
    logged_status: String,
    errors: Vec<ModelError>,
    session: Option<DeviceSession>,
    chooser: DeviceChooser,
    last_write: Option<ImageWriteReport>,
    flash: FlashTab,
    preset: usize,
    tx_preset: usize,
    /// A write or read running on a worker, which holds the session meanwhile.
    job: Option<DeviceJob>,
    /// A browser serial port being chosen and connected.
    #[cfg(target_arch = "wasm32")]
    connecting: Option<crate::session::ConnectJob>,
    /// A project file being chosen in the browser's file picker.
    #[cfg(target_arch = "wasm32")]
    project_upload: Option<crate::files::Upload>,
    /// Which banks the Banks tab lists, and a bank to open in it.
    banks_tab: BanksTab,
    /// Where SatNOGS responses are cached, and what was last read from it.
    satellite_cache: SatelliteCache,
    satellites: Option<SatelliteList>,
    /// A SatNOGS fetch running on a worker.
    satellite_sync: Option<SyncJob>,
    /// Where the ETCC repeater list is cached, and what was last read from it.
    repeater_cache: RepeaterCache,
    repeaters: Option<RepeaterList>,
    /// An ETCC fetch running on a worker.
    repeater_sync: Option<repeaters::SyncJob>,
    /// The locator and radius repeaters are added for, and the state file
    /// they are remembered in.
    repeater_area: RepeaterArea,
    repeater_area_file: Option<PathBuf>,
    /// The amateur bands repeaters are taken from, 2 m and 70 cm by
    /// default, and their state file.
    repeater_bands: BandFilter,
    repeater_bands_file: Option<PathBuf>,
    /// How far off a standard and a local repeater are heard.
    repeater_reach: RepeaterReach,
    /// Where Ofcom's and the FCC's FM transmitter lists are cached, and what
    /// was last read.
    station_cache: StationCache,
    stations: Option<StationList>,
    /// An Ofcom and FCC fetch running on a worker.
    station_sync: Option<stations::SyncJob>,
    /// How many stations are added, at the repeaters' locator, and the state
    /// file it is remembered in.
    station_area: StationArea,
    station_area_file: Option<PathBuf>,
    /// Where the `OurAirports` lists are cached, and what was last read.
    airport_cache: AirportCache,
    airports: Option<AirportList>,
    /// An `OurAirports` fetch running on a worker.
    airport_sync: Option<airports::SyncJob>,
    /// The radius and count airport frequencies are added for, at the
    /// repeaters' locator, and the state file they are remembered in.
    airport_area: AirportArea,
    airport_area_file: Option<PathBuf>,
    /// Which services the airport banks take, and whether each airport gets
    /// its own.
    airport_options: AirportOptions,
    airport_options_file: Option<PathBuf>,
    /// Where the `OpenStreetMap` marine channels are cached, and what was
    /// last read.
    marine_cache: MarineCache,
    marine: Option<MarineList>,
    /// An Overpass query running on a worker.
    marine_sync: Option<marine::SyncJob>,
    /// The radius and count local marine channels are added for, at the
    /// repeaters' locator, and the state file they are remembered in.
    marine_area: MarineArea,
    marine_area_file: Option<PathBuf>,
    /// The radius and count gateways are added for, at the repeaters'
    /// locator, and the state file they are remembered in.
    gateway_area: AirportArea,
    gateway_area_file: Option<PathBuf>,
    /// The amateur bands gateways are taken from, and their state file.
    gateway_bands: BandFilter,
    gateway_bands_file: Option<PathBuf>,
    /// The same for beacons.
    beacon_bands: BandFilter,
    beacon_bands_file: Option<PathBuf>,
    /// The same for beacons.
    beacon_area: AirportArea,
    beacon_area_file: Option<PathBuf>,
    /// The same for the small airfields, from the airports' list.
    airfield_area: AirfieldArea,
    airfield_area_file: Option<PathBuf>,
    /// Where hearham's worldwide repeater list is cached, and what was last
    /// read from it; the repeaters outside the UK.
    world_cache: WorldCache,
    world: Option<WorldList>,
    /// A hearham fetch running on a worker.
    world_sync: Option<world::SyncJob>,
    /// The locators older studios kept beside the projects (`ADR-120`),
    /// which a project with none of its own takes, and the description
    /// being typed for the current locator.
    legacy_locators: SavedLocators,
    locator_description: String,
    /// What the radio listens with, and its state file.
    receiver: Receiver,
    receiver_file: Option<PathBuf>,
    /// Where the Map tab looks, the locator it last centred on, and the
    /// generators it shows, by their place in [`Generator::ALL`].
    map_view: MapView,
    map_locator: String,
    map_shown: [bool; Generator::ALL.len()],
    /// Whether the map shows what the generators would add at every saved
    /// locator at once, rather than at the operator's alone.
    map_all_locations: bool,
    /// Where the map's view is kept, and the filters as last written.
    remembered: RememberedFilters,
    /// Whether a generator's add takes its banks even past the radio's
    /// memory, as "Add all shown" does so the operator can trim after.
    overfill: bool,
    /// The layers last drawn and the settings they were built from, so the
    /// previews behind them are rebuilt only when a setting changes.
    map_layers: Option<(String, Vec<Layer>)>,
    tiles: Tiles,
}

impl StudioApp {
    /// Builds the editor from start-up options, opening the last project.
    pub fn new(options: &Options) -> Self {
        let mut app = Self::with_projects(options, ProjectStore::open());
        app.use_satellite_cache(SatelliteCache::open());
        app.tiles = Tiles::open();
        app.use_repeater_cache(RepeaterCache::open());
        // The receiver comes first: it sets the radii no file has saved.
        app.receiver_file = Receiver::file();
        if let Some(file) = &app.receiver_file {
            app.receiver = Receiver::load(file);
        }
        let radii = app.receiver.radii();
        app.repeater_area_file = RepeaterArea::file();
        if let Some(file) = &app.repeater_area_file {
            app.repeater_area = RepeaterArea::load(file);
            if !file.exists() {
                app.repeater_area.radius_km = radii.repeaters;
            }
        }
        if let Some(file) = SavedLocators::file() {
            app.legacy_locators = SavedLocators::load(&file);
        }
        // The project opened before the older studios' locators were read.
        app.adopt_legacy_locations();
        app.locator_description = app.saved_description();
        app.use_station_cache(StationCache::open());
        app.station_area_file = StationArea::file();
        if let Some(file) = &app.station_area_file {
            app.station_area = StationArea::load(file);
        }
        app.use_airport_cache(AirportCache::open());
        app.airport_area_file = AirportArea::file();
        if let Some(file) = &app.airport_area_file {
            app.airport_area = AirportArea::load_or(file, &area_within(radii.airports));
        }
        app.airport_options_file = AirportOptions::file();
        if let Some(file) = &app.airport_options_file {
            app.airport_options = AirportOptions::load(file);
        }
        app.use_marine_cache(MarineCache::open());
        app.marine_area_file = marine::area_file();
        if let Some(file) = &app.marine_area_file {
            app.marine_area = MarineArea::load_or(file, &area_within(radii.marine));
        }
        app.gateway_area_file = repeaters::gateway_area_file();
        if let Some(file) = &app.gateway_area_file {
            app.gateway_area = AirportArea::load_or(file, &area_within(radii.gateways));
        }
        app.remembered.repeater_reach_file = RepeaterReach::file();
        if let Some(file) = &app.remembered.repeater_reach_file {
            app.repeater_reach = RepeaterReach::load(file);
        }
        app.repeater_bands_file = repeaters::repeater_bands_file();
        if let Some(file) = &app.repeater_bands_file {
            app.repeater_bands = BandFilter::load(file);
        }
        app.gateway_bands_file = repeaters::gateway_bands_file();
        if let Some(file) = &app.gateway_bands_file {
            app.gateway_bands = BandFilter::load(file);
        }
        app.beacon_bands_file = repeaters::beacon_bands_file();
        if let Some(file) = &app.beacon_bands_file {
            app.beacon_bands = BandFilter::load(file);
        }
        app.beacon_area_file = repeaters::beacon_area_file();
        if let Some(file) = &app.beacon_area_file {
            app.beacon_area = AirportArea::load_or(file, &area_within(radii.beacons));
        }
        app.use_world_cache(WorldCache::open());
        app.airfield_area_file = airfields::area_file();
        if let Some(file) = &app.airfield_area_file {
            app.airfield_area = AirfieldArea::load_or(file, &area_within(radii.airfields));
        }
        app.remembered.map_view_file = crate::paths::state_dir().map(|dir| dir.join(MAP_VIEW_FILE));
        if let Some(file) = app.remembered.map_view_file.clone() {
            app.load_map_view(&file);
        }
        app.remembered.last = Some(app.filter_state());
        // A dataset never synced is fetched once on its own; the browser
        // keeps none, so it reads the whole mirror at every start.
        if app.datasets(never).iter().any(|(_, wanted)| *wanted) {
            app.sync_datasets(never);
        }
        app
    }

    /// Reads which generators the map shows and whether every location is
    /// shown; with no file, every generator at the one locator.
    fn load_map_view(&mut self, file: &Path) {
        let Ok(text) = fs::read_to_string(file) else {
            return;
        };
        let words: Vec<&str> = text.split_whitespace().collect();
        for (generator, shown) in Generator::ALL.into_iter().zip(&mut self.map_shown) {
            *shown = words.contains(&generator.key());
        }
        self.map_all_locations = words.contains(&"all-locations");
    }

    /// Every filter the generators and the map take, as one string.
    fn filter_state(&self) -> String {
        format!(
            "{:?}",
            (
                (&self.repeater_area, &self.repeater_bands),
                &self.repeater_reach,
                (&self.gateway_area, &self.gateway_bands),
                (&self.beacon_area, &self.beacon_bands, &self.station_area),
                (
                    &self.airport_area,
                    &self.airport_options,
                    &self.airfield_area
                ),
                (&self.marine_area, self.map_shown, self.map_all_locations),
            )
        )
    }

    /// Writes every filter and the map's view once one has changed, so the
    /// next start opens as this one was left, Add pressed or not.
    fn remember_filters(&mut self) {
        let state = self.filter_state();
        // The first look only notes how the filters start.
        let last = self.remembered.last.replace(state.clone());
        if last.is_none_or(|last| last == state) {
            return;
        }
        let save = |file: &Option<PathBuf>, write: &dyn Fn(&Path) -> Result<(), String>| {
            file.as_deref().map(write)
        };
        let mut view: Vec<&str> = Generator::ALL
            .into_iter()
            .zip(self.map_shown)
            .filter(|(_, shown)| *shown)
            .map(|(generator, _)| generator.key())
            .collect();
        if self.map_all_locations {
            view.push("all-locations");
        }

        let view = view.join(" ") + "\n";
        let saved = [
            save(&self.repeater_area_file, &|f| self.repeater_area.save(f)),
            save(&self.repeater_bands_file, &|f| self.repeater_bands.save(f)),
            save(&self.remembered.repeater_reach_file, &|f| {
                self.repeater_reach.save(f)
            }),
            save(&self.gateway_area_file, &|f| self.gateway_area.save(f)),
            save(&self.gateway_bands_file, &|f| self.gateway_bands.save(f)),
            save(&self.beacon_area_file, &|f| self.beacon_area.save(f)),
            save(&self.beacon_bands_file, &|f| self.beacon_bands.save(f)),
            save(&self.station_area_file, &|f| self.station_area.save(f)),
            save(&self.airport_area_file, &|f| self.airport_area.save(f)),
            save(&self.airport_options_file, &|f| {
                self.airport_options.save(f)
            }),
            save(&self.airfield_area_file, &|f| self.airfield_area.save(f)),
            save(&self.marine_area_file, &|f| self.marine_area.save(f)),
            save(&self.remembered.map_view_file, &|f| {
                f.parent()
                    .map_or(Ok(()), fs::create_dir_all)
                    .and_then(|()| fs::write(f, &view))
                    .map_err(|error| format!("{}: {error}", f.display()))
            }),
        ];
        for error in saved.into_iter().flatten().filter_map(Result::err) {
            log::warn!("could not remember the filters: {error}");
        }
    }

    /// Reads the marine channel list from `cache`, which a sync then writes.
    ///
    /// Reading never fetches; see [`Self::sync_datasets`] for what does.
    pub fn use_marine_cache(&mut self, cache: MarineCache) {
        self.marine = match cache.load() {
            Ok(list) => list,
            Err(error) => {
                log::warn!("reading the marine cache: {error}");
                None
            }
        };
        self.marine_cache = cache;
    }

    /// Takes in a finished marine sync.
    fn poll_marine_sync(&mut self) {
        let Some(outcome) = self.marine_sync.as_ref().and_then(marine::SyncJob::poll) else {
            return;
        };
        self.marine_sync = None;
        match outcome {
            Ok(list) => {
                self.status = format!(
                    "Synced {} places with marine VHF channels from OpenStreetMap.",
                    list.places.len()
                );
                log::info!("{}", self.status);
                self.marine = Some(list);
            }
            // The cache is untouched, so the previous list still applies.
            Err(error) => self.fail(
                "syncing marine channels",
                format!("Marine sync failed: {error}"),
            ),
        }
    }

    /// Builds the marine bank the area would add, and the project it would
    /// make, as [`Self::repeater_preview`] does. Without a sync the bank
    /// holds the core channels alone; synced with none in range, the
    /// location needs no core.
    fn marine_preview(&self) -> Result<AreaPreview, String> {
        let locator = &self.repeater_area.locator;
        let mut near = if let Some(list) = &self.marine {
            marine::nearby(&list.places, locator, self.marine_area.radius_km)?
        } else {
            repeaters::locator_centre(locator)?;
            Vec::new()
        };
        let in_range = near.len();
        near.truncate(usize::from(self.marine_area.max_channels));
        let (project, bank_bytes) = self.with_core(
            BankSource::Marine,
            marine::core_bank(),
            marine::marine_bank(locator, &near),
            marine::is_core,
            self.marine.is_some(),
        );
        Ok(AreaPreview {
            in_range,
            taken: near.len(),
            furthest_km: near.iter().map(|local| local.km).fold(0.0, f64::max),
            sites: near
                .iter()
                .map(|local| site(&local.channel.designator, local.position))
                .collect(),
            bank_bytes,
            project,
        })
    }

    /// Describes the marine list, what the area would add, and its cost.
    fn marine_source(&self) -> String {
        if self.marine_sync.is_some() {
            return "Syncing...".to_owned();
        }
        let synced = self.marine.as_ref().map_or_else(
            || "Not synced: the core channels only.".to_owned(),
            |list| {
                format!(
                    "{} places from OpenStreetMap (ODbL), synced {}.",
                    list.places.len(),
                    age(list.synced)
                )
            },
        );
        let Ok(preview) = self.marine_preview() else {
            return synced;
        };
        if self.marine.is_some() && preview.taken == 0 {
            return format!(
                "No local channels within {} km, so no core. {synced}",
                self.marine_area.radius_km
            );
        }
        let taking = format!(
            "{} core and nearest {} of {} local channels, furthest {:.0} km",
            marine::CORE_CHANNELS,
            preview.taken,
            preview.in_range,
            preview.furthest_km
        );
        format!("{taking}{}. {synced}", cost(&preview))
    }

    /// Adds a bank of the core marine channels and those used near the
    /// locator.
    fn add_marine(&mut self) {
        let preview = match self.marine_preview() {
            Ok(preview) => preview,
            Err(error) => {
                self.refuse(&format!("Cannot add marine channels: {error}."));
                return;
            }
        };
        let locator = self.repeater_area.locator.trim().to_ascii_uppercase();
        let area = self.marine_area.clone();
        if self.marine.is_some() && preview.taken == 0 {
            // The project no longer files the core here.
            self.project = preview.project;
            self.status = format!(
                "No marine channels within {} km of {locator}, so {} is not filed there.",
                area.radius_km,
                marine::CORE_BANK
            );
            return;
        }
        if preview.project.banks.len() > usize::from(MAX_BANKS) {
            self.status =
                format!("A radio holds {MAX_BANKS} banks and the project already has them.");
            return;
        }
        if !self.overfill && too_big(&preview.project) {
            self.refuse(&format!(
                "{} marine channels would not fit the radio; take fewer.",
                marine::CORE_CHANNELS + preview.taken
            ));
            return;
        }
        self.take_generated(preview.project, false);
        // The core is shared, so it may be unchanged; it is filed here too.
        self.file_here(marine::CORE_BANK, false);
        self.status = format!(
            "Added {} core and {} local marine channels near {locator} (furthest {:.0} km). \
             Local channels are OpenStreetMap data (ODbL), mapped by volunteers; check \
             before relying on them. Without a marine TX range the radio listens to both \
             sides of a duplex channel.",
            marine::CORE_CHANNELS,
            preview.taken,
            preview.furthest_km
        );
        if let Some(file) = &self.marine_area_file {
            if let Err(error) = area.save(file) {
                log::warn!("could not remember the marine area: {error}");
            }
        }
    }

    /// Builds the airfield bank the area would add, and the project it would
    /// make, as [`Self::repeater_preview`] does. Without the airports' list
    /// the bank holds the sporting core alone; synced with none in range,
    /// the location needs no core.
    fn airfield_preview(&self) -> Result<AreaPreview, String> {
        let locator = &self.repeater_area.locator;
        let mut near = if let Some(list) = &self.airports {
            airfields::nearby(&list.frequencies, locator, self.airfield_area.radius_km)?
        } else {
            repeaters::locator_centre(locator)?;
            Vec::new()
        };
        let in_range = near.len();
        near.truncate(usize::from(self.airfield_area.max_channels));
        let (project, bank_bytes) = self.with_core(
            BankSource::Airfields,
            airfields::core_bank(),
            airfields::airfield_bank(locator, &near),
            airfields::is_core,
            self.airports.is_some(),
        );
        Ok(AreaPreview {
            in_range,
            taken: near.len(),
            furthest_km: near.last().map_or(0.0, |(_, km)| *km),
            sites: near
                .iter()
                .map(|(f, _)| site(&f.name, (f.latitude, f.longitude)))
                .collect(),
            bank_bytes,
            project,
        })
    }

    /// What the shared [`airports::MAJOR_BANK`] holds: the large airports'
    /// approach `plan` hears here, then that of every other location the
    /// bank is filed under, each carrier once.
    fn major_frequencies<'a>(
        &self,
        frequencies: &'a [AirportFrequency],
        plan: &airports::AirportPlan<'a>,
    ) -> Vec<(&'a AirportFrequency, f64)> {
        let mut held = plan.major.clone();
        let here = self.repeater_area.locator.trim().to_ascii_uppercase();
        let Some(bank) = self
            .project
            .banks
            .iter()
            .find(|bank| bank.name == airports::MAJOR_BANK)
        else {
            return held;
        };
        let elsewhere = bank
            .groups
            .iter()
            .filter_map(|group| self.project.groups.get(*group))
            .map(|group| group.locator.clone())
            .filter(|locator| !locator.is_empty() && *locator != here);
        for locator in elsewhere {
            let Ok(there) = airports::plan(
                frequencies,
                &locator,
                &self.airport_area,
                &self.airport_options,
                self.receiver,
            ) else {
                continue;
            };
            for frequency in there.major {
                if held
                    .iter()
                    .all(|kept| kept.0.frequency_hz != frequency.0.frequency_hz)
                {
                    held.push(frequency);
                }
            }
        }
        held
    }

    /// Describes the airfield list, what the area would add, and its cost.
    fn airfield_source(&self) -> String {
        if self.airport_sync.is_some() {
            return "Syncing...".to_owned();
        }
        let synced = self.airports.as_ref().map_or_else(
            || "Not synced: the sporting frequencies only.".to_owned(),
            |list| {
                format!(
                    "{} small-field frequencies from OurAirports, synced {}.",
                    list.frequencies.iter().filter(|f| f.is_small()).count(),
                    age(list.synced)
                )
            },
        );
        let Ok(preview) = self.airfield_preview() else {
            return synced;
        };
        if self.airports.is_some() && preview.taken == 0 {
            return format!(
                "No local frequencies within {} km, so no sporting set. {synced}",
                self.airfield_area.radius_km
            );
        }
        let taking = format!(
            "{} sporting and nearest {} of {} local frequencies, furthest {:.0} km",
            airfields::CORE_CHANNELS,
            preview.taken,
            preview.in_range,
            preview.furthest_km
        );
        format!("{taking}{}. {synced}", cost(&preview))
    }

    /// Adds a bank of the UK sporting frequencies and those of the small
    /// fields near the locator.
    fn add_airfields(&mut self) {
        let preview = match self.airfield_preview() {
            Ok(preview) => preview,
            Err(error) => {
                self.refuse(&format!("Cannot add airfields: {error}."));
                return;
            }
        };
        let locator = self.repeater_area.locator.trim().to_ascii_uppercase();
        let area = self.airfield_area.clone();
        if self.airports.is_some() && preview.taken == 0 {
            // The project no longer files the core here.
            self.project = preview.project;
            self.status = format!(
                "No airfield frequencies within {} km of {locator}, so {} is not filed \
                 there.",
                area.radius_km,
                airfields::CORE_BANK
            );
            return;
        }
        if preview.project.banks.len() > usize::from(MAX_BANKS) {
            self.status =
                format!("A radio holds {MAX_BANKS} banks and the project already has them.");
            return;
        }
        if !self.overfill && too_big(&preview.project) {
            self.refuse(&format!(
                "{} airfield frequencies would not fit the radio; take fewer.",
                airfields::CORE_CHANNELS + preview.taken
            ));
            return;
        }
        self.take_generated(preview.project, true);
        // The core is shared, so it may be unchanged; it is filed here too.
        self.file_here(airfields::CORE_BANK, true);
        self.status = format!(
            "Added {} sporting and {} local airfield frequencies near {locator} (furthest \
             {:.0} km), receive only. The sporting set is CAP 1606's, which the CAA has \
             withdrawn; a field's own frequency comes first.",
            airfields::CORE_CHANNELS,
            preview.taken,
            preview.furthest_km
        );
        if let Some(file) = &self.airfield_area_file {
            if let Err(error) = area.save(file) {
                log::warn!("could not remember the airfield area: {error}");
            }
        }
    }

    /// Reads the airport frequency list from `cache`, which a sync then
    /// writes.
    ///
    /// Reading never fetches; see [`Self::sync_datasets`] for what does.
    pub fn use_airport_cache(&mut self, cache: AirportCache) {
        self.airports = match cache.load() {
            Ok(list) => list,
            Err(error) => {
                log::warn!("reading the airport cache: {error}");
                None
            }
        };
        self.airport_cache = cache;
    }

    /// Takes in a finished airport sync.
    fn poll_airport_sync(&mut self) {
        let Some(outcome) = self.airport_sync.as_ref().and_then(airports::SyncJob::poll) else {
            return;
        };
        self.airport_sync = None;
        match outcome {
            Ok(list) => {
                self.status = format!(
                    "Synced {} airband frequencies from OurAirports.",
                    list.frequencies.len()
                );
                log::info!("{}", self.status);
                self.airports = Some(list);
            }
            // The cache is untouched, so the previous list still applies.
            Err(error) => self.fail("syncing airports", format!("Airport sync failed: {error}")),
        }
    }

    /// Builds the airport banks the area and options would add, and the
    /// project they would make, as [`Self::repeater_preview`] does:
    /// `in_range` counts the hearable frequencies for the area bank, `taken`
    /// every channel added.
    fn airport_preview(&self) -> Result<(AreaPreview, usize), String> {
        let list = self
            .airports
            .as_ref()
            .ok_or("sync the airport list from OurAirports first")?;
        let locator = &self.repeater_area.locator;
        let plan = airports::plan(
            &list.frequencies,
            locator,
            &self.airport_area,
            &self.airport_options,
            self.receiver,
        )?;
        let major = self.major_frequencies(&list.frequencies, &plan);
        let mut banks = Vec::new();
        if !major.is_empty() {
            let mut bank = airports::major_bank(&major);
            bank.origin = Some(Origin::new(BankSource::Airports, ""));
            banks.push(bank);
        }
        banks.extend(airports::airport_banks(locator, &plan));
        let bank_count = banks.len();
        let (project, bank_bytes) = self.with_banks(BankSource::Airports, banks);
        let furthest_km = plan
            .area
            .iter()
            .map(|(_, km)| *km)
            .chain(plan.airports.iter().map(|group| group.km))
            .fold(0.0, f64::max);
        Ok((
            AreaPreview {
                in_range: plan.hearable,
                taken: plan.channels(),
                furthest_km,
                sites: plan
                    .area
                    .iter()
                    .map(|(f, _)| *f)
                    .map(|f| airport_site(f, false))
                    .chain(plan.major.iter().map(|(f, _)| airport_site(f, false)))
                    .chain(
                        plan.airports
                            .iter()
                            .flat_map(|group| group.frequencies.iter())
                            .map(|f| airport_site(f, true)),
                    )
                    .collect(),
                bank_bytes,
                project,
            },
            bank_count,
        ))
    }

    /// Describes the airport list, what the area would add, and its cost.
    fn airport_source(&self) -> String {
        if self.airport_sync.is_some() {
            return "Syncing...".to_owned();
        }
        let Some(list) = &self.airports else {
            return "Not synced.".to_owned();
        };
        let synced = format!(
            "{} frequencies from OurAirports, synced {}.",
            list.frequencies.len(),
            age(list.synced)
        );
        let Ok((preview, banks)) = self.airport_preview() else {
            return synced;
        };
        let taking = format!(
            "{} channels in {banks} bank{} ({} hearable for the area bank), furthest \
             airport {:.0} km",
            preview.taken,
            if banks == 1 { "" } else { "s" },
            preview.in_range,
            preview.furthest_km
        );
        format!("{taking}{}. {synced}", cost(&preview))
    }

    /// Adds the banks of the hearable frequencies of the airports near the
    /// locator.
    fn add_airports(&mut self) {
        let (preview, banks) = match self.airport_preview() {
            Ok(preview) => preview,
            Err(error) => {
                self.refuse(&format!("Cannot add airports: {error}."));
                return;
            }
        };
        let locator = self.repeater_area.locator.trim().to_ascii_uppercase();
        let area = self.airport_area.clone();
        if preview.taken == 0 {
            self.status = format!(
                "No hearable airport frequencies of the chosen services within {} km of \
                 {locator}.",
                area.radius_km
            );
            return;
        }
        if preview.project.banks.len() > usize::from(MAX_BANKS) {
            self.status = format!(
                "A radio holds {MAX_BANKS} banks and these would make {}; take fewer \
                 airport banks.",
                preview.project.banks.len()
            );
            return;
        }
        if !self.overfill && too_big(&preview.project) {
            self.refuse(&format!(
                "{} airport frequencies would not fit the radio; take fewer.",
                preview.taken
            ));
            return;
        }
        self.take_generated(preview.project, true);
        // The large airports' bank is shared, so it may be unchanged; it is
        // filed here too.
        if self
            .project
            .banks
            .iter()
            .any(|bank| bank.name == airports::MAJOR_BANK)
        {
            self.file_here(airports::MAJOR_BANK, true);
        }
        self.status = format!(
            "Added {} airport frequencies near {locator} in {banks} bank{}, most hearable \
             first (furthest airport {:.0} km). Hearing ranges are estimates by service; \
             OurAirports is maintained by volunteers, check before relying on it.",
            preview.taken,
            if banks == 1 { "" } else { "s" },
            preview.furthest_km
        );
        if let Some(file) = &self.airport_area_file {
            if let Err(error) = area.save(file) {
                log::warn!("could not remember the airport area: {error}");
            }
        }
        if let Some(file) = &self.airport_options_file {
            if let Err(error) = self.airport_options.save(file) {
                log::warn!("could not remember the airport options: {error}");
            }
        }
    }

    /// Reads the FM station list from `cache`, which a sync then writes.
    ///
    /// Reading never fetches; see [`Self::sync_datasets`] for what does.
    pub fn use_station_cache(&mut self, cache: StationCache) {
        self.stations = match cache.load() {
            Ok(list) => list,
            Err(error) => {
                log::warn!("reading the station cache: {error}");
                None
            }
        };
        self.station_cache = cache;
    }

    /// Takes in a finished station sync.
    fn poll_station_sync(&mut self) {
        let Some(outcome) = self.station_sync.as_ref().and_then(stations::SyncJob::poll) else {
            return;
        };
        self.station_sync = None;
        match outcome {
            Ok(list) => {
                self.status = format!(
                    "Synced {} FM transmitters from Ofcom and the FCC.",
                    list.stations.len()
                );
                log::info!("{}", self.status);
                self.stations = Some(list);
            }
            // A source that failed kept its cache, and one that synced was
            // cached, so the caches hold the newest of each.
            Err(error) => {
                self.use_station_cache(self.station_cache.clone());
                self.fail("syncing stations", format!("Station sync failed: {error}"));
            }
        }
    }

    /// Builds the station bank the area would add, and the project it would
    /// make, as [`Self::repeater_preview`] does.
    fn station_preview(&self) -> Result<AreaPreview, String> {
        let list = self
            .stations
            .as_ref()
            .ok_or("sync the FM station lists from Ofcom and the FCC first")?;
        let locator = &self.repeater_area.locator;
        // Kinds are left out before ranking, so a station sharing a frequency
        // with one left out can still take it.
        let taken = list.stations.iter().filter(|s| self.station_area.takes(s));
        let mut near = stations::strongest(taken, locator, self.receiver)?;
        let in_range = near.len();
        near.truncate(usize::from(self.station_area.max_channels));
        // One bank holds the stations of every location it is filed under,
        // and of each one-location bank an older studio made, rebuilt here
        // with each frequency once, this locator's first.
        let stations_bank = |bank: &BankDraft| {
            bank.origin
                .as_ref()
                .is_some_and(|origin| origin.source == BankSource::Stations)
        };
        let located = |bank: &BankDraft| {
            stations_bank(bank) && bank.origin.as_ref().is_some_and(|o| !o.locator.is_empty())
        };
        let mut locators = vec![locator.trim().to_ascii_uppercase()];
        for bank in self.project.banks.iter().filter(|bank| stations_bank(bank)) {
            let filed = bank
                .groups
                .iter()
                .filter_map(|group| self.project.groups.get(*group))
                .map(|group| group.locator.clone());
            let made_for = bank.origin.iter().map(|origin| origin.locator.clone());
            for other in filed.chain(made_for) {
                if !other.is_empty() && !locators.contains(&other) {
                    locators.push(other);
                }
            }
        }
        let mut held = near.clone();
        for other in &locators[1..] {
            let taken = list.stations.iter().filter(|s| self.station_area.takes(s));
            let Ok(mut there) = stations::strongest(taken, other, self.receiver) else {
                continue;
            };
            there.truncate(usize::from(self.station_area.max_channels));
            for station in there {
                if held
                    .iter()
                    .all(|kept| kept.0.frequency_hz != station.0.frequency_hz)
                {
                    held.push(station);
                }
            }
        }
        let mut bank = stations::station_bank(&held);
        bank.origin = Some(Origin::new(BankSource::Stations, ""));
        let (mut project, bank_bytes) = self.with_bank(BankSource::Stations, bank);
        // The shared bank takes the place of each one-location bank, and is
        // filed where each was.
        let filed: Vec<usize> = project
            .banks
            .iter()
            .filter(|old| located(old))
            .flat_map(|old| old.groups.iter().copied())
            .collect();
        project.banks.retain(|old| !located(old));
        if let Some(bank) = project
            .banks
            .iter_mut()
            .find(|bank| bank.name == stations::STATION_BANK)
        {
            for group in filed {
                if !bank.groups.contains(&group) {
                    bank.groups.push(group);
                }
            }
        }
        Ok(AreaPreview {
            in_range,
            taken: near.len(),
            furthest_km: near.iter().map(|(_, km)| *km).fold(0.0, f64::max),
            sites: near
                .iter()
                .map(|(s, _)| site(&s.name, (s.latitude, s.longitude)))
                .collect(),
            bank_bytes,
            project,
        })
    }

    /// Describes the station list, what the area would add, and its cost.
    fn station_source(&self) -> String {
        if self.station_sync.is_some() {
            return "Syncing...".to_owned();
        }
        let Some(list) = &self.stations else {
            return "Not synced.".to_owned();
        };
        let synced = format!(
            "{} transmitters from Ofcom and the FCC, synced {}.",
            list.stations.len(),
            age(list.synced)
        );
        let Ok(preview) = self.station_preview() else {
            return synced;
        };
        let taking = format!(
            "Strongest {} of {} stations, furthest transmitter {:.0} km",
            preview.taken, preview.in_range, preview.furthest_km
        );
        format!("{taking}{}. {synced}", cost(&preview))
    }

    /// Adds a bank of the strongest stations near the locator.
    fn add_stations(&mut self) {
        let preview = match self.station_preview() {
            Ok(preview) => preview,
            Err(error) => {
                self.refuse(&format!("Cannot add stations: {error}."));
                return;
            }
        };
        let locator = self.repeater_area.locator.trim().to_ascii_uppercase();
        let area = self.station_area.clone();
        if preview.taken == 0 {
            self.status = format!("No FM stations for {locator}.");
            return;
        }
        if preview.project.banks.len() > usize::from(MAX_BANKS) {
            self.status =
                format!("A radio holds {MAX_BANKS} banks and the project already has them.");
            return;
        }
        if !self.overfill && too_big(&preview.project) {
            self.refuse(&format!(
                "{} stations would not fit the radio; take fewer.",
                preview.taken
            ));
            return;
        }
        self.take_generated(preview.project, false);
        self.status = format!(
            "Added the {} strongest FM stations at {locator} to {}, in frequency order \
             (furthest transmitter {:.0} km).",
            preview.taken,
            stations::STATION_BANK,
            preview.furthest_km
        );
        if let Some(file) = &self.station_area_file {
            if let Err(error) = area.save(file) {
                log::warn!("could not remember the station area: {error}");
            }
        }
    }

    /// The name the location at `locator` goes by: the saved locator's
    /// description, cut to a group name, else the locator itself.
    fn location_name(&self, locator: &str) -> String {
        self.project
            .locations
            .find(locator)
            .and_then(|saved| repeaters::sanitise(&saved.description, MAX_BANK_NAME_LEN))
            .unwrap_or_else(|| locator.trim().to_ascii_uppercase())
    }

    /// Takes a project a generator built, filing each bank it added or
    /// changed under the location of the current locator (`ADR-116`): named
    /// by the saved locator's description, else the locator itself. Airband
    /// banks go under AIRBAND too.
    fn take_generated(&mut self, project: ProjectModel, airband: bool) {
        let locator = self.repeater_area.locator.trim().to_ascii_uppercase();
        // Another location's bank the add changed stays filed where it is.
        let generated: Vec<String> = project
            .banks
            .iter()
            .filter(|bank| !self.project.banks.contains(bank))
            .filter(|bank| {
                bank.origin
                    .as_ref()
                    .is_none_or(|origin| origin.locator.is_empty() || origin.locator == locator)
            })
            // A bank locations share stays with those it was shared by.
            .filter(|bank| !is_shared_location_bank(bank))
            .map(|bank| bank.name.clone())
            .collect();
        self.project = project;
        for bank in &generated {
            self.file_here(bank, airband);
        }
        share_overlapping(&mut self.project);
    }

    /// Files the bank named `bank` under the location of the locator, under
    /// the airband group when `airband`, and under the marine group when it
    /// holds marine channels.
    fn file_here(&mut self, bank: &str, airband: bool) {
        self.name_locations();
        let locator = self.repeater_area.locator.trim().to_ascii_uppercase();
        let name = self.location_name(&locator);
        let mut filed = self.project.file_bank(bank, &name, &locator, airband);
        let marine = self
            .project
            .banks
            .iter()
            .any(|b| b.name == *bank && b.is_marine());
        if marine {
            filed &= self.project.file_in(bank, MARINE_GROUP);
        }
        if !filed {
            log::warn!("a radio holds {MAX_GROUPS} groups; some generated banks are not filed");
        }
    }

    /// The project with `source`'s `core` bank, which every location
    /// shares, and this location's `local` bank of the rest, left out when
    /// it holds nothing; and the bytes they add. Each location's bank from
    /// `source` loses the core channels an older studio put in it, a bank
    /// left empty is dropped and the core filed where it was. When
    /// `synced` and this location has no local channels it needs no core:
    /// the core is taken from it, and dropped once no location has it.
    fn with_core(
        &self,
        source: BankSource,
        mut core: BankDraft,
        local: BankDraft,
        is_core: fn(&ChannelDraft) -> bool,
        synced: bool,
    ) -> (ProjectModel, Option<usize>) {
        core.origin = Some(Origin::new(source, ""));
        let core_name = core.name.clone();
        let local_name = local.name.clone();
        let empty = local.channels.is_empty();
        let bare = empty && synced;
        let mut banks = Vec::new();
        if !bare {
            banks.push(core);
        }
        if !empty {
            banks.push(local);
        }
        let (mut project, bank_bytes) = self.with_banks(source, banks);
        let located = |bank: &BankDraft| {
            bank.origin
                .as_ref()
                .is_some_and(|origin| origin.source == source && !origin.locator.is_empty())
        };
        let mut filed = Vec::new();
        for bank in &mut project.banks {
            if !located(bank) {
                continue;
            }
            bank.channels.retain(|channel| !is_core(channel));
            if empty && bank.name == local_name {
                bank.channels.clear();
            }
            if bank.channels.is_empty() {
                filed.extend(bank.groups.iter().copied());
            }
        }
        project
            .banks
            .retain(|bank| !(located(bank) && bank.channels.is_empty()));
        let here = project.location(&self.repeater_area.locator);
        if let Some(core) = project.banks.iter_mut().find(|bank| bank.name == core_name) {
            for group in filed {
                if !core.groups.contains(&group) {
                    core.groups.push(group);
                }
            }
            if bare {
                core.groups.retain(|group| Some(*group) != here);
            }
        }
        if let Some(row) = project.banks.iter().position(|bank| bank.name == core_name) {
            if bare && locations_of(&project, &project.banks[row]).is_empty() {
                project.banks.remove(row);
            }
        }
        (project, bank_bytes)
    }

    /// The project with `bank`, made by `source`, replacing one of the same
    /// name, else after the rest, and the bytes the bank alone adds to a
    /// radio image.
    fn with_bank(&self, source: BankSource, bank: BankDraft) -> (ProjectModel, Option<usize>) {
        self.with_banks(source, vec![bank])
    }

    /// The project with each bank added or replaced as [`Self::with_bank`]
    /// does, and the bytes they add together. A bank with no origin yet is
    /// recorded as made by `source` for the locator (`ADR-117`).
    fn with_banks(
        &self,
        source: BankSource,
        mut banks: Vec<BankDraft>,
    ) -> (ProjectModel, Option<usize>) {
        for bank in &mut banks {
            bank.origin
                .get_or_insert_with(|| Origin::new(source, &self.repeater_area.locator));
        }
        let mut project = self.project.clone();
        // A location's bank made again leaves any bank it shared.
        for origin in banks.iter().filter_map(|bank| bank.origin.as_ref()) {
            if !origin.locator.is_empty() {
                unshare(&mut project, origin.source, &origin.locator);
            }
        }
        for bank in &banks {
            if let Some(existing) = project.banks.iter().position(|b| b.name == bank.name) {
                // A regenerated bank stays in the groups it was in.
                let groups = core::mem::take(&mut project.banks[existing].groups);
                project.banks[existing] = BankDraft {
                    groups,
                    ..bank.clone()
                };
            } else {
                project.banks.push(bank.clone());
            }
        }
        // What the banks alone cost: their image less an empty project's.
        let bank_only = ProjectModel {
            banks,
            ..ProjectModel::new()
        };
        let bank_bytes = match (
            bank_only.radio_image_len(),
            ProjectModel::new().radio_image_len(),
        ) {
            (Some(with), Some(without)) => Some(with.saturating_sub(without)),
            _ => None,
        };
        (project, bank_bytes)
    }

    /// Reads the repeater list from `cache`, which a sync then writes.
    ///
    /// Reading never fetches; see [`Self::sync_datasets`] for what does.
    pub fn use_repeater_cache(&mut self, cache: RepeaterCache) {
        self.repeaters = match cache.load() {
            Ok(list) => list,
            Err(error) => {
                log::warn!("reading the repeater cache: {error}");
                None
            }
        };
        self.repeater_cache = cache;
    }

    /// Every repeater synced: the ETCC's in the UK, then hearham's outside
    /// it; none before either list is synced.
    fn all_repeaters(&self) -> Option<impl Iterator<Item = &Repeater> + Clone> {
        if self.repeaters.is_none() && self.world.is_none() {
            return None;
        }
        let uk = self.repeaters.iter().flat_map(|list| &list.repeaters);
        let world = self.world.iter().flat_map(|list| &list.repeaters);
        Some(uk.chain(world))
    }

    /// Takes in a finished repeater sync.
    fn poll_repeater_sync(&mut self) {
        let Some(outcome) = self
            .repeater_sync
            .as_ref()
            .and_then(repeaters::SyncJob::poll)
        else {
            return;
        };
        self.repeater_sync = None;
        match outcome {
            Ok(list) => {
                self.status = format!(
                    "Synced {} FM repeaters from the ETCC.",
                    list.repeaters.len()
                );
                log::info!("{}", self.status);
                self.repeaters = Some(list);
            }
            // The cache is untouched, so the previous list still applies.
            Err(error) => self.fail(
                "syncing repeaters",
                format!("Repeater sync failed: {error}"),
            ),
        }
    }

    /// Builds the repeater bank the area would add, and the project it would
    /// make: the bank replaces one of the same name, else follows the rest.
    fn repeater_preview(&self) -> Result<AreaPreview, String> {
        let all = self
            .all_repeaters()
            .ok_or("sync the repeater lists first")?;
        let area = &self.repeater_area;
        let mut near = repeaters::hearable(
            all,
            &area.locator,
            area.radius_km,
            &self.repeater_reach,
            self.receiver,
        )?;
        near.retain(|(repeater, _)| self.repeater_bands.takes(repeater.output_hz));
        let in_range = near.len();
        near.truncate(usize::from(area.max_channels));
        let (project, bank_bytes) = self.with_bank(
            BankSource::Repeaters,
            repeaters::repeater_bank(&area.locator, &near),
        );
        Ok(AreaPreview {
            in_range,
            taken: near.len(),
            furthest_km: near.iter().map(|(_, km)| *km).fold(0.0, f64::max),
            sites: repeater_sites(&near),
            bank_bytes,
            project,
        })
    }

    /// Describes the repeater list, what the area would add, and its cost.
    fn repeater_source(&self) -> String {
        if self.repeater_sync.is_some() || self.world_sync.is_some() {
            return "Syncing...".to_owned();
        }
        if self.repeaters.is_none() && self.world.is_none() {
            return "Not synced.".to_owned();
        }
        let listed = |count: Option<(usize, std::time::SystemTime)>, source: &str, place: &str| {
            count.map_or_else(
                || format!("{place} not synced"),
                |(count, synced)| format!("{count} from {source} {place}, synced {}", age(synced)),
            )
        };
        let synced = format!(
            "{}; {}.",
            listed(
                self.repeaters
                    .as_ref()
                    .map(|list| (list.repeaters.len(), list.synced)),
                "the ETCC",
                "in the UK"
            ),
            listed(
                self.world
                    .as_ref()
                    .map(|list| (list.repeaters.len(), list.synced)),
                "hearham",
                "elsewhere"
            ),
        );
        let Ok(preview) = self.repeater_preview() else {
            return synced;
        };
        let radius = self.repeater_area.radius_km;
        let taking = if preview.taken < preview.in_range {
            format!(
                "{} heard within {radius} km, most hearable {} taken",
                preview.in_range, preview.taken
            )
        } else {
            format!("{} heard within {radius} km", preview.in_range)
        };
        format!("{taking}{}. {synced}", cost(&preview))
    }

    /// Adds a bank of the repeaters near the locator, nearest first.
    fn add_repeaters(&mut self) {
        let preview = match self.repeater_preview() {
            Ok(preview) => preview,
            Err(error) => {
                self.refuse(&format!("Cannot add repeaters: {error}."));
                return;
            }
        };
        let area = self.repeater_area.clone();
        if preview.taken == 0 {
            self.status = format!(
                "No FM repeaters within {} km of {}.",
                area.radius_km, area.locator
            );
            return;
        }
        if preview.project.banks.len() > usize::from(MAX_BANKS) {
            self.status =
                format!("A radio holds {MAX_BANKS} banks and the project already has them.");
            return;
        }
        if !self.overfill && too_big(&preview.project) {
            self.refuse(&format!(
                "{} repeaters would not fit the radio; take fewer.",
                preview.taken
            ));
            return;
        }
        self.take_generated(preview.project, false);
        let (count, furthest) = (preview.taken, preview.furthest_km);
        self.status = format!(
            "Added {count} repeaters within {} km of {}, nearest first (furthest {furthest:.0} \
             km). Confirm each against a national list before use.",
            area.radius_km,
            area.locator.trim().to_ascii_uppercase()
        );
        if let Some(file) = &self.repeater_area_file {
            if let Err(error) = area.save(file) {
                log::warn!("could not remember the repeater area: {error}");
            }
        }
        if let Some(file) = &self.repeater_bands_file {
            if let Err(error) = self.repeater_bands.save(file) {
                log::warn!("could not remember the repeater bands: {error}");
            }
        }
        if let Some(file) = &self.remembered.repeater_reach_file {
            if let Err(error) = self.repeater_reach.save(file) {
                log::warn!("could not remember the repeater reach: {error}");
            }
        }
    }

    /// Reads the worldwide repeater list from `cache`, which a sync then
    /// writes.
    ///
    /// Reading never fetches; see [`Self::sync_datasets`] for what does.
    pub fn use_world_cache(&mut self, cache: WorldCache) {
        self.world = match cache.load() {
            Ok(list) => list,
            Err(error) => {
                log::warn!("reading the hearham cache: {error}");
                None
            }
        };
        self.world_cache = cache;
    }

    /// Takes in a finished hearham sync.
    fn poll_world_sync(&mut self) {
        let Some(outcome) = self.world_sync.as_ref().and_then(world::SyncJob::poll) else {
            return;
        };
        self.world_sync = None;
        match outcome {
            Ok(list) => {
                self.status = format!("Synced {} FM repeaters from hearham.", list.repeaters.len());
                log::info!("{}", self.status);
                self.world = Some(list);
            }
            // The cache is untouched, so the previous list still applies.
            Err(error) => self.fail(
                "syncing worldwide repeaters",
                format!("Worldwide repeater sync failed: {error}"),
            ),
        }
    }

    /// The radius and count chosen for `kind`.
    const fn etcc_area(&self, kind: EtccKind) -> &AirportArea {
        match kind {
            EtccKind::Gateways => &self.gateway_area,
            EtccKind::Beacons => &self.beacon_area,
        }
    }

    /// The bands chosen for `kind`.
    const fn etcc_bands(&self, kind: EtccKind) -> &BandFilter {
        match kind {
            EtccKind::Gateways => &self.gateway_bands,
            EtccKind::Beacons => &self.beacon_bands,
        }
    }

    /// Builds the gateway or beacon bank the area would add, and the project
    /// it would make, as [`Self::repeater_preview`] does.
    fn etcc_preview(&self, kind: EtccKind) -> Result<AreaPreview, String> {
        let list = self
            .repeaters
            .as_ref()
            .ok_or("sync the repeater list from the ETCC first")?;
        let locator = &self.repeater_area.locator;
        let area = self.etcc_area(kind);
        let mut near = repeaters::nearby(kind.systems(list), locator, area.radius_km)?;
        let bands = self.etcc_bands(kind);
        near.retain(|(system, _)| bands.takes(system.output_hz));
        let in_range = near.len();
        near.truncate(usize::from(area.max_channels));
        let (project, bank_bytes) = self.with_bank(kind.source(), kind.bank(locator, &near));
        Ok(AreaPreview {
            in_range,
            taken: near.len(),
            furthest_km: near.last().map_or(0.0, |(_, km)| *km),
            sites: repeater_sites(&near),
            bank_bytes,
            project,
        })
    }

    /// Describes the gateways or beacons, what the area would add, and its
    /// cost.
    fn etcc_source(&self, kind: EtccKind) -> String {
        if self.repeater_sync.is_some() {
            return "Syncing...".to_owned();
        }
        let Some(list) = &self.repeaters else {
            return "Not synced.".to_owned();
        };
        let synced = format!(
            "{} {} from the ETCC, synced {}.",
            kind.systems(list).len(),
            kind.noun(),
            age(list.synced)
        );
        let Ok(preview) = self.etcc_preview(kind) else {
            return synced;
        };
        let radius = self.etcc_area(kind).radius_km;
        let taking = if preview.taken < preview.in_range {
            format!(
                "{} within {radius} km, nearest {} taken",
                preview.in_range, preview.taken
            )
        } else {
            format!("{} within {radius} km", preview.in_range)
        };
        format!("{taking}{}. {synced}", cost(&preview))
    }

    /// Adds a bank of the gateways or beacons near the locator, nearest
    /// first.
    fn add_etcc(&mut self, kind: EtccKind) {
        let noun = kind.noun();
        let preview = match self.etcc_preview(kind) {
            Ok(preview) => preview,
            Err(error) => {
                self.refuse(&format!("Cannot add {noun}: {error}."));
                return;
            }
        };
        let area = self.etcc_area(kind).clone();
        let locator = self.repeater_area.locator.trim().to_ascii_uppercase();
        if preview.taken == 0 {
            self.status = format!("No {noun} within {} km of {locator}.", area.radius_km);
            return;
        }
        if preview.project.banks.len() > usize::from(MAX_BANKS) {
            self.status =
                format!("A radio holds {MAX_BANKS} banks and the project already has them.");
            return;
        }
        if !self.overfill && too_big(&preview.project) {
            self.refuse(&format!(
                "{} {noun} would not fit the radio; take fewer.",
                preview.taken
            ));
            return;
        }
        self.take_generated(preview.project, false);
        let (count, furthest) = (preview.taken, preview.furthest_km);
        let use_note = match kind {
            EtccKind::Gateways => "Confirm each against the ETCC list before use.",
            EtccKind::Beacons => {
                "Each listens on USB just below the carrier, so the beacon is heard as a \
                 tone; it never transmits."
            }
        };
        self.status = format!(
            "Added {count} {noun} within {} km of {locator}, nearest first (furthest \
             {furthest:.0} km). {use_note}",
            area.radius_km
        );
        let file = match kind {
            EtccKind::Gateways => &self.gateway_area_file,
            EtccKind::Beacons => &self.beacon_area_file,
        };
        if let Some(file) = file {
            if let Err(error) = area.save(file) {
                log::warn!("could not remember the {noun} area: {error}");
            }
        }
        let file = match kind {
            EtccKind::Gateways => &self.gateway_bands_file,
            EtccKind::Beacons => &self.beacon_bands_file,
        };
        if let Some(file) = file {
            if let Err(error) = self.etcc_bands(kind).save(file) {
                log::warn!("could not remember the {noun} bands: {error}");
            }
        }
    }

    /// Reads the satellite list from `cache`, which a sync then writes.
    ///
    /// Reading never fetches; see [`Self::sync_datasets`] for what does.
    pub fn use_satellite_cache(&mut self, cache: SatelliteCache) {
        self.satellites = match cache.load() {
            Ok(list) => list,
            Err(error) => {
                log::warn!("reading the satellite cache: {error}");
                None
            }
        };
        self.satellite_cache = cache;
    }

    /// Takes in a finished satellite sync.
    fn poll_satellite_sync(&mut self) {
        let Some(outcome) = self.satellite_sync.as_ref().and_then(SyncJob::poll) else {
            return;
        };
        self.satellite_sync = None;
        match outcome {
            Ok(list) => {
                self.status = format!(
                    "Synced {} satellite channels from SatNOGS. Add the {SATELLITE_PRESET} set \
                     to use them.",
                    list.channels.len()
                );
                log::info!("{}", self.status);
                self.satellites = Some(list);
            }
            // The cache is untouched, so the previous list still applies.
            Err(error) => self.fail(
                "syncing satellites",
                format!("Satellite sync failed: {error}"),
            ),
        }
    }

    /// Describes where the satellite set's channels come from.
    fn satellite_source(&self) -> String {
        if self.satellite_sync.is_some() {
            return "Syncing...".to_owned();
        }
        match &self.satellites {
            Some(list) => format!(
                "{} channels from SatNOGS, synced {}.",
                list.channels.len(),
                age(list.synced)
            ),
            None => "Built-in list; not synced.".to_owned(),
        }
    }

    /// Builds the editor over an explicit project store.
    pub fn with_projects(options: &Options, projects: ProjectStore) -> Self {
        let mut app = Self {
            tab: Tab::default(),
            generator: Generator::default(),
            radio_section: RadioSection::default(),
            project: ProjectModel::new(),
            saved: ProjectModel::new(),
            projects,
            new_project: String::new(),
            confirm_delete: false,
            status: "Ready.".to_owned(),
            logged_status: String::new(),
            errors: Vec::new(),
            session: None,
            chooser: DeviceChooser::new(),
            last_write: None,
            flash: FlashTab::default(),
            preset: 0,
            tx_preset: 0,
            banks_tab: BanksTab::default(),
            job: None,
            #[cfg(target_arch = "wasm32")]
            connecting: None,
            #[cfg(target_arch = "wasm32")]
            project_upload: None,
            satellite_cache: SatelliteCache::none(),
            satellites: None,
            satellite_sync: None,
            repeater_cache: RepeaterCache::none(),
            repeaters: None,
            repeater_sync: None,
            repeater_area: RepeaterArea::default(),
            repeater_area_file: None,
            repeater_bands: BandFilter::default(),
            repeater_bands_file: None,
            repeater_reach: RepeaterReach::default(),
            station_cache: StationCache::none(),
            stations: None,
            station_sync: None,
            station_area: StationArea::default(),
            station_area_file: None,
            airport_cache: AirportCache::none(),
            airports: None,
            airport_sync: None,
            airport_area: AirportArea::default(),
            airport_area_file: None,
            airport_options: AirportOptions::default(),
            airport_options_file: None,
            marine_cache: MarineCache::none(),
            marine: None,
            marine_sync: None,
            marine_area: area_within(Receiver::Handheld.radii().marine),
            marine_area_file: None,
            gateway_area: area_within(Receiver::Handheld.radii().gateways),
            gateway_area_file: None,
            gateway_bands: BandFilter::default(),
            gateway_bands_file: None,
            beacon_bands: BandFilter::default(),
            beacon_bands_file: None,
            beacon_area: area_within(Receiver::Handheld.radii().beacons),
            beacon_area_file: None,
            airfield_area: area_within(Receiver::Handheld.radii().airfields),
            airfield_area_file: None,
            world_cache: WorldCache::none(),
            world: None,
            world_sync: None,
            legacy_locators: SavedLocators::default(),
            locator_description: String::new(),
            receiver: Receiver::Handheld,
            receiver_file: None,
            map_view: MapView::default(),
            map_locator: String::new(),
            map_shown: [true; Generator::ALL.len()],
            map_all_locations: false,
            overfill: false,
            map_layers: None,
            remembered: RememberedFilters::default(),
            tiles: Tiles::none(),
        };
        if let Some(baud) = options.baud {
            app.chooser.baud = baud;
        }
        if let Some(path) = &options.project {
            app.projects.adopt(path);
        }
        app.load_project();
        app.connect_at_start(options);
        app
    }

    /// Connects the simulator or the device `options` name, or otherwise
    /// only detects what is plugged in.
    fn connect_at_start(&mut self, options: &Options) {
        if options.simulator {
            self.connect_simulator();
            return;
        }
        // Detection runs at start-up so the Device tab already knows what is
        // plugged in, but only an explicit request connects without a click.
        let detection = self.chooser.detect();
        match &options.device {
            Some(DeviceSelector::Auto) => self.connect_serial(),
            Some(DeviceSelector::Explicit(path)) => {
                self.chooser.choice = DeviceChoice::Manual;
                self.chooser.manual_path = path.display().to_string();
                self.connect_serial();
            }
            None => self.status = detection,
        }
    }

    /// Shows a failure on the status line and logs it with what was being done.
    fn fail(&mut self, doing: &str, message: String) {
        log::error!("{doing}: {message}");
        self.status = message;
    }

    /// Shows a request the editor refused before doing anything, logged as a warning.
    fn refuse(&mut self, message: &str) {
        log::warn!("{message}");
        message.clone_into(&mut self.status);
    }

    /// Refuses an action because the project does not validate, logging each field.
    fn refuse_invalid(&mut self, action: &str, errors: Vec<ModelError>) {
        log::warn!("{action} refused: {} fields do not validate", errors.len());
        for error in &errors {
            log::warn!("  {error}");
        }
        self.errors = errors;
        self.status = format!("Fix the highlighted fields before {action}.");
    }

    /// Refuses a connection change or a second job while one is running:
    /// the worker owns the session until it ends.
    fn refuse_while_busy(&mut self) -> bool {
        if self.is_connecting() {
            self.refuse("Wait until the studio has finished connecting the radio.");
            return true;
        }
        let Some(job) = &self.job else {
            return false;
        };
        let doing = match job.kind() {
            JobKind::Write => "writing to",
            JobKind::Read => "reading from",
            JobKind::LinkTest => "testing the link to",
        };
        let message = format!(
            "Wait until the studio has finished {doing} {}.",
            job.description()
        );
        self.refuse(&message);
        true
    }

    fn connect_simulator(&mut self) {
        if self.refuse_while_busy() {
            return;
        }
        match DeviceSession::connect_simulator() {
            Ok(session) => self.install_session(session),
            Err(error) => self.fail("connecting the simulator", error.to_string()),
        }
    }

    /// Whether a browser serial port is being chosen and connected.
    #[cfg(target_arch = "wasm32")]
    fn is_connecting(&self) -> bool {
        self.connecting.is_some()
    }

    /// Whether a browser serial port is being chosen and connected.
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::unused_self)]
    const fn is_connecting(&self) -> bool {
        false
    }

    /// Asks the browser for the radio's port and connects it in the
    /// background; [`Self::poll_connect`] installs the session.
    #[cfg(target_arch = "wasm32")]
    fn connect_serial(&mut self) {
        if self.refuse_while_busy() {
            return;
        }
        let baud = self.chooser.baud;
        if !radio_programmer_serial::is_supported_baud(baud) {
            self.refuse(&format!("Baud {baud} is not one of {SUPPORTED_BAUDS:?}."));
            return;
        }
        log::info!("asking the browser for a serial port at {baud} baud");
        "Choose the radio's serial port in the browser…".clone_into(&mut self.status);
        self.connecting = Some(DeviceSession::start_connect_web_serial(baud));
    }

    /// Installs the session once the browser's port has connected.
    #[cfg(target_arch = "wasm32")]
    fn poll_connect(&mut self) {
        let Some(finished) = self
            .connecting
            .as_ref()
            .and_then(crate::session::ConnectJob::poll)
        else {
            return;
        };
        self.connecting = None;
        match finished {
            Ok(session) => self.install_session(session),
            Err(error) => self.fail("connecting a browser serial port", error.to_string()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn connect_serial(&mut self) {
        if self.refuse_while_busy() {
            return;
        }
        let (path, baud) = match self.chooser.resolve() {
            Ok(resolved) => resolved,
            Err(reason) => {
                self.refuse(&reason);
                return;
            }
        };
        match DeviceSession::connect_serial(&path, baud) {
            Ok(session) => self.install_session(session),
            Err(error) => self.fail(
                &format!("connecting {} at {baud} baud", path.display()),
                error.to_string(),
            ),
        }
    }

    /// Replaces the connected radio, forgetting what was written to the last.
    fn install_session(&mut self, session: DeviceSession) {
        log::info!(
            "connected to {} running {}",
            session.description(),
            session.firmware()
        );
        self.status = format!(
            "Connected to {}, running {}.",
            session.description(),
            session.firmware()
        );
        self.session = Some(session);
        self.last_write = None;
    }

    fn disconnect(&mut self) {
        if self.refuse_while_busy() {
            return;
        }
        if let Some(session) = self.session.take() {
            log::info!("disconnected from {}", session.description());
            self.status = format!("Disconnected from {}.", session.description());
            self.last_write = None;
        }
    }

    /// Starts writing the project to the radio on a worker thread.
    fn write_project(&mut self) {
        if self.refuse_while_busy() {
            return;
        }
        let shared = share_overlapping(&mut self.project);
        let project = match self.project.validate_for_radio() {
            Ok(project) => project,
            Err(errors) => {
                self.refuse_invalid("writing", errors);
                return;
            }
        };
        self.errors.clear();
        let Some(session) = self.session.take() else {
            self.refuse("Connect a radio first.");
            return;
        };
        let device = session.description().to_owned();
        match session.start_write(project) {
            Ok(job) => {
                log::info!("writing the project to {device}");
                if shared > 0 {
                    log::info!("shared {shared} location banks before writing");
                }
                self.status = format!("Writing the project to {device}…");
                self.job = Some(job);
            }
            Err(error) => self.fail(&format!("starting a write to {device}"), error.to_string()),
        }
    }

    /// Starts reading the project from the radio on a worker thread.
    fn read_project(&mut self) {
        if self.refuse_while_busy() {
            return;
        }
        let Some(session) = self.session.take() else {
            self.refuse("Connect a radio first.");
            return;
        };
        let device = session.description().to_owned();
        match session.start_read() {
            Ok(job) => {
                log::info!("reading the project from {device}");
                self.status = format!("Reading the project from {device}…");
                self.job = Some(job);
            }
            Err(error) => self.fail(&format!("starting a read from {device}"), error.to_string()),
        }
    }

    /// Starts a link test on a worker thread.
    fn test_link(&mut self) {
        if self.refuse_while_busy() {
            return;
        }
        let Some(session) = self.session.take() else {
            self.refuse("Connect a radio first.");
            return;
        };
        let device = session.description().to_owned();
        match session.start_link_test() {
            Ok(job) => {
                log::info!("testing the link to {device}");
                self.status = format!("Testing the link to {device}…");
                self.job = Some(job);
            }
            Err(error) => self.fail(
                &format!("starting a link test on {device}"),
                error.to_string(),
            ),
        }
    }

    /// Takes back the session and reports the outcome once the job has ended.
    fn poll_job(&mut self) {
        let Some(finished) = self.job.as_ref().and_then(DeviceJob::poll) else {
            return;
        };
        let device = self
            .job
            .take()
            .map(|job| job.description().to_owned())
            .unwrap_or_default();
        match finished {
            Ok((session, outcome)) => {
                self.session = Some(session);
                self.finish_job(&device, outcome);
            }
            Err(()) => self.fail(
                &format!("talking to {device}"),
                format!("The connection to {device} was lost; connect it again."),
            ),
        }
    }

    fn finish_job(&mut self, device: &str, outcome: DeviceOutcome) {
        match outcome {
            DeviceOutcome::Wrote(Ok(report)) => {
                log::info!(
                    "wrote and verified a {}-byte image on {device} running {}: {} blocks changed",
                    report.image_bytes,
                    report.firmware.version(),
                    report.changed_blocks
                );
                self.status = format!(
                    "Wrote and verified {} bytes ({} blocks changed). The radio loads \
                     the new channels a second after the last write.",
                    report.image_bytes, report.changed_blocks
                );
                self.last_write = Some(report);
            }
            DeviceOutcome::Wrote(Err(error)) => self.fail(
                &format!("writing the project to {device}"),
                error.to_string(),
            ),
            DeviceOutcome::Read(Ok(project)) => {
                self.project = ProjectModel::from_project(&project);
                self.errors.clear();
                let (banks, channels) = (self.project.banks.len(), self.project.channel_count());
                log::info!("read {channels} channels in {banks} banks from {device}");
                self.tab = Tab::Banks;
                self.status = format!(
                    "Loaded {channels} channels in {banks} banks from the radio into {}. \
                     Save to keep them.",
                    self.projects.current().name
                );
            }
            DeviceOutcome::Read(Err(error)) => self.fail(
                &format!("reading the project from {device}"),
                error.to_string(),
            ),
            DeviceOutcome::LinkTested(Ok(report)) => {
                let (answered, sent) = (report.answered_count(), report.answered.len());
                log::info!(
                    "link test on {device} running {}: {answered} of {sent} back-to-back reads \
                     answered ({})",
                    report.firmware.version(),
                    report.pattern()
                );
                self.status = if report.is_clean() {
                    format!("The link is clean: all {sent} back-to-back reads were answered.")
                } else {
                    format!(
                        "The radio missed {} of {sent} back-to-back reads ({}: . answered, \
                         X missed). A write sends a missed request again, but slowly.",
                        sent - answered,
                        report.pattern()
                    )
                };
            }
            DeviceOutcome::LinkTested(Err(error)) => {
                self.fail(&format!("testing the link to {device}"), error.to_string());
            }
        }
    }

    /// Appends a bank of `kind` and opens it for editing; other banks stay
    /// as they are, collapsed unless the operator opened them.
    fn add_bank(&mut self, kind: BankKind) {
        match kind {
            BankKind::Named => self.project.add_bank(),
            BankKind::Generated => self.project.add_generated_bank(),
        }
        self.banks_tab.expand = Some(self.project.banks.len() - 1);
    }

    /// Whether the project differs from what was last loaded or saved.
    fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    fn save_project(&mut self) {
        let entry = self.projects.current().clone();
        let shared = share_overlapping(&mut self.project);
        let image = match self.project.to_image() {
            Ok(image) => image,
            Err(errors) => {
                self.refuse_invalid("saving", errors);
                return;
            }
        };
        let written = entry
            .path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&entry.path, image));
        match written {
            Ok(()) => {
                log::info!("saved project {} to {}", entry.name, entry.path.display());
                self.errors.clear();
                self.saved = self.project.clone();
                self.status = format!("Saved {}.{}", entry.name, shared_note(shared));
            }
            Err(error) => self.fail(
                "saving the project",
                format!("Could not save {}: {error}", entry.path.display()),
            ),
        }
    }

    /// Loads the open project from its file; a project not saved yet is empty.
    fn load_project(&mut self) {
        let entry = self.projects.current().clone();
        let path = entry.path.as_path();
        let (loaded, converted) = if fs::exists(path) {
            match read_project_file(path) {
                Ok(project) => project,
                Err(message) => {
                    self.fail(&format!("loading project {}", entry.name), message);
                    return;
                }
            }
        } else {
            (ProjectModel::new(), false)
        };
        log::info!("opened project {} from {}", entry.name, path.display());
        self.status = if fs::exists(path) {
            format!(
                "Opened {}: {} channels in {} banks.{}",
                entry.name,
                loaded.channel_count(),
                loaded.banks.len(),
                if converted {
                    " Converted from an older project format; saving writes the new one, \
                     which older studios cannot open."
                } else {
                    ""
                }
            )
        } else {
            format!(
                "Opened {}, which is saved to {} once it is first saved.",
                entry.name,
                path.display()
            )
        };
        self.project = loaded.clone();
        self.saved = loaded;
        self.errors.clear();
        self.adopt_legacy_locations();
        let recorded = self.record_origins();
        if recorded > 0 {
            self.status = format!(
                "{} Recorded which generator made {recorded} bank{}; save to keep it.",
                self.status,
                if recorded == 1 { "" } else { "s" }
            );
        }
        let renamed = self.name_locations();
        if renamed > 0 {
            self.status = format!(
                "{} Named {renamed} location group{} after the saved locator; save to keep it.",
                self.status,
                if renamed == 1 { "" } else { "s" }
            );
        }
        let shared = share_overlapping(&mut self.project);
        if shared > 0 {
            self.status = format!("{}{} Save to keep it.", self.status, shared_note(shared));
        }
    }

    /// Gives each bank of a project saved before origins were kept the
    /// origin its name shows (`ADR-117`); a project with any origin is left
    /// as it is. Returns how many banks were given one.
    fn record_origins(&mut self) -> usize {
        if self.project.banks.iter().any(|bank| bank.origin.is_some()) {
            return 0;
        }
        let origins: Vec<_> = self
            .project
            .banks
            .iter()
            .map(|bank| infer_origin(&self.project, bank))
            .collect();
        let mut recorded = 0;
        for (bank, origin) in self.project.banks.iter_mut().zip(origins) {
            recorded += usize::from(origin.is_some());
            bank.origin = origin;
        }
        recorded
    }

    /// Names each location group still called by its bare locator after the
    /// description saved for that locator, as the locator dropdown shows it;
    /// a name the operator gave it is kept. Returns how many were renamed.
    fn name_locations(&mut self) -> usize {
        let mut renamed = 0;
        for row in 0..self.project.groups.len() {
            let locator = self.project.groups[row].locator.clone();
            if locator.is_empty() || !self.project.groups[row].name.eq_ignore_ascii_case(&locator) {
                continue;
            }
            let name = self.location_name(&locator);
            if self.project.groups[row].name != name {
                self.project.groups[row].name = name;
                renamed += 1;
            }
        }
        renamed
    }

    /// Opens another project, unless this one has changes it would lose.
    fn open_project(&mut self, index: usize) {
        if index == self.projects.current_index() {
            return;
        }
        if self.is_dirty() {
            self.refuse(&format!(
                "Save or revert {} before opening another project.",
                self.projects.current().name
            ));
            return;
        }
        self.projects.select(index);
        self.load_project();
    }

    /// Creates an empty project with the typed name and opens it.
    fn create_project(&mut self) {
        if self.is_dirty() {
            self.refuse(&format!(
                "Save or revert {} before creating another project.",
                self.projects.current().name
            ));
            return;
        }
        let name = self.new_project.clone();
        if let Err(reason) = self.projects.create(&name) {
            self.refuse(&reason);
            return;
        }
        self.new_project.clear();
        self.project = ProjectModel::new();
        self.save_project();
    }

    /// Renames the open project to the typed name; its changes, saved or
    /// not, go with it.
    fn rename_project(&mut self) {
        let old = self.projects.current().name.clone();
        let name = self.new_project.clone();
        if let Err(reason) = self.projects.rename_current(&name) {
            self.refuse(&reason);
            return;
        }
        self.new_project.clear();
        self.status = format!("Renamed {old} to {}.", self.projects.current().name);
        log::info!("{}", self.status);
    }

    /// Adds the project file at `staged`, such as one the browser took in,
    /// as a new project and opens it. The staged copy is removed whatever
    /// happens; a file that does not load as a project is refused.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn import_project(&mut self, staged: &Path) {
        let bytes = fs::read(staged);
        let loads = read_project_file(staged);
        if let Err(error) = fs::remove_file(staged) {
            log::warn!("removing the staged {}: {error}", staged.display());
        }
        if self.is_dirty() {
            self.refuse(&format!(
                "Save or revert {} before importing a project.",
                self.projects.current().name
            ));
            return;
        }
        let (bytes, name) = match (bytes, loads) {
            (Ok(bytes), Ok(_)) => {
                let stem = staged
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (bytes, self.projects.free_name(&stem))
            }
            (Err(error), _) => {
                self.fail("importing a project", error.to_string());
                return;
            }
            (_, Err(message)) => {
                self.fail(
                    "importing a project",
                    format!("Not an AFIK project: {message}"),
                );
                return;
            }
        };
        let path = match self.projects.create(&name) {
            Ok(entry) => entry.path.clone(),
            Err(reason) => {
                self.refuse(&reason);
                return;
            }
        };
        let written = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&path, &bytes));
        if let Err(error) = written {
            self.fail(
                "importing a project",
                format!("Could not write {}: {error}", path.display()),
            );
            return;
        }
        log::info!("imported {} as project {name}", staged.display());
        self.load_project();
        self.status = format!("Imported {name}. {}", self.status);
    }

    /// The open project's file, named for the project, to give out; refused
    /// while it has changes the file does not hold.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn project_export(&mut self) -> Option<(String, Vec<u8>)> {
        let entry = self.projects.current().clone();
        if self.is_dirty() {
            self.refuse(&format!("Save {} before exporting it.", entry.name));
            return None;
        }
        match fs::read(&entry.path) {
            Ok(bytes) => Some((format!("{}.{PROJECT_EXTENSION}", entry.name), bytes)),
            Err(error) => {
                self.fail(
                    "exporting the project",
                    format!("Save {} before exporting it ({error}).", entry.name),
                );
                None
            }
        }
    }

    /// Gives the open project's file to the operator as a download.
    #[cfg(target_arch = "wasm32")]
    fn export_project(&mut self) {
        let Some((name, bytes)) = self.project_export() else {
            return;
        };
        match fs::download(&name, &bytes) {
            Ok(()) => self.status = format!("Exported {name}."),
            Err(error) => self.fail("exporting the project", error),
        }
    }

    /// Shows the browser's file picker for a project to import.
    #[cfg(target_arch = "wasm32")]
    fn choose_import(&mut self) {
        let staged = crate::paths::data_dir().unwrap_or_default().join("imports");
        match fs::upload(staged, ".afik", MAX_PROJECT_IMAGE_BYTES) {
            Ok(upload) => self.project_upload = Some(upload),
            Err(error) => self.fail("importing a project", error),
        }
    }

    /// Imports the project the browser's file picker took in, once it has.
    #[cfg(target_arch = "wasm32")]
    fn poll_import(&mut self) {
        let Some(outcome) = self
            .project_upload
            .as_ref()
            .and_then(crate::files::Upload::poll)
        else {
            return;
        };
        self.project_upload = None;
        match outcome {
            Ok(staged) => self.import_project(&staged),
            Err(error) => self.refuse(&format!("No project imported: {error}.")),
        }
    }

    /// Deletes the open project, once confirmed, and opens the next.
    fn delete_project(&mut self) {
        self.confirm_delete = false;
        match self.projects.delete_current() {
            Ok(name) => {
                log::info!("deleted project {name}");
                self.load_project();
                self.status = format!("Deleted {name}. {}", self.status);
            }
            Err(reason) => self.fail("deleting the project", reason),
        }
    }

    /// Appends one TX range preset's ranges after the operator's own.
    ///
    /// A range the project already has is skipped, and a preset that would take
    /// the project past the ranges a radio holds is refused whole.
    fn add_tx_preset(&mut self) {
        let Some(preset) = TX_RANGE_PRESETS.get(self.tx_preset) else {
            return;
        };
        let new: Vec<_> = preset
            .drafts()
            .into_iter()
            .filter(|range| !self.project.tx_ranges.iter().any(|r| r.same_span(range)))
            .collect();
        if new.is_empty() {
            self.status = format!(
                "The {} TX ranges are already in the project.",
                preset.name()
            );
            return;
        }
        if self.project.tx_ranges.len() + new.len() > MAX_TX_RANGES {
            self.status = format!(
                "{} needs {} more TX ranges; a radio holds {MAX_TX_RANGES} and the \
                 project has {}.",
                preset.name(),
                new.len(),
                self.project.tx_ranges.len()
            );
            return;
        }
        let added = new.len();
        self.project.tx_ranges.extend(new);
        self.status = format!(
            "Added {added} {} TX ranges. Confirm each against your own licence \
             before writing the radio.",
            preset.name()
        );
    }

    /// Appends one preset default set's banks after the operator's own.
    ///
    /// A bank whose name the project already has is skipped, so adding a set
    /// twice does not duplicate it, and a set that would take the project past
    /// the banks a radio holds is refused whole.
    fn add_preset(&mut self) {
        let Some(preset) = PRESETS.get(self.preset) else {
            return;
        };
        let mut banks = preset.build().banks;
        // A synced satellite list replaces the built-in one.
        if preset.name() == SATELLITE_PRESET {
            if let Some(list) = self.satellites.as_ref().filter(|l| !l.channels.is_empty()) {
                banks = vec![satellite_bank(&list.channels)];
            }
        }
        // A default set is the same wherever the operator is, so its banks
        // go in COMMON, which puts them in every group on the radio.
        let names: Vec<String> = banks.iter().map(|bank| bank.name.clone()).collect();
        let new: Vec<_> = banks
            .into_iter()
            .filter(|bank| !self.project.banks.iter().any(|b| b.name == bank.name))
            .collect();
        if new.is_empty() {
            let filed = self.file_common(&names);
            self.status = format!(
                "{} is already in the project{}.",
                preset.name(),
                if filed { ", in COMMON" } else { "" }
            );
            return;
        }
        if self.project.banks.len() + new.len() > usize::from(MAX_BANKS) {
            self.status = format!(
                "{} needs {} more banks; a radio holds {MAX_BANKS} and the project has {}.",
                preset.name(),
                new.len(),
                self.project.banks.len()
            );
            return;
        }
        let added = new.len();
        self.project.banks.extend(new);
        let filed = self.file_common(&names);
        self.status = format!(
            "Added {}: {added} banks after the existing ones{}. \
             Confirm every frequency against your own band plan before use.",
            preset.name(),
            if filed {
                ", in COMMON so every group has them"
            } else {
                ""
            }
        );
    }

    /// Files each bank named in `names` under [`COMMON_GROUP`]; false when
    /// the radio's groups are all taken.
    fn file_common(&mut self, names: &[String]) -> bool {
        let mut filed = true;
        for name in names {
            filed &= self.project.file_in(name, COMMON_GROUP);
        }
        if !filed {
            log::warn!("a radio holds {MAX_GROUPS} groups; a default set is not in COMMON");
        }
        filed
    }
}

/// The repeater or station bank an area would add, and what it would cost.
struct AreaPreview {
    /// Repeaters within the radius, or every station ranked.
    in_range: usize,
    /// How many of them, nearest or strongest first, the bank takes.
    taken: usize,
    furthest_km: f64,
    /// Where what the bank takes is, for the map.
    sites: Vec<Site>,
    /// Bytes the bank adds to a radio image, when it compiles.
    bank_bytes: Option<usize>,
    /// The project with the bank added or replaced.
    project: ProjectModel,
}

/// One site for the map.
fn site(name: &str, (latitude, longitude): (f64, f64)) -> Site {
    Site {
        name: name.to_owned(),
        latitude,
        longitude,
        highlight: false,
        size: SiteSize::Medium,
    }
}

/// Where an older studio's generator made `bank`, read from its name as it
/// named them (`RPT JO02EH`), the banks every location shares, and an
/// airport's own receive-only airband bank filed under a location; none for
/// the operator's own. A project saved before origins were kept (`ADR-117`)
/// is given them once from this.
fn infer_origin(project: &ProjectModel, bank: &BankDraft) -> Option<Origin> {
    const PREFIXES: [(&str, BankSource); 8] = [
        ("RPT", BankSource::Repeaters),
        ("WRPT", BankSource::WorldRepeaters),
        ("GW", BankSource::Gateways),
        ("BCN", BankSource::Beacons),
        ("AIR", BankSource::Airports),
        ("FLD", BankSource::Airfields),
        ("SEA", BankSource::Marine),
        ("FM", BankSource::Stations),
    ];
    if bank.kind != BankKind::Named {
        return None;
    }
    let shared = [
        (stations::STATION_BANK, BankSource::Stations),
        (marine::CORE_BANK, BankSource::Marine),
        (airfields::CORE_BANK, BankSource::Airfields),
        (airports::MAJOR_BANK, BankSource::Airports),
    ];
    if let Some((_, source)) = shared.iter().find(|(name, _)| bank.name == *name) {
        return Some(Origin::new(*source, ""));
    }
    if let Some(locator) = trailing_locator(&bank.name) {
        let prefix = bank.name.split_whitespace().next().unwrap_or_default();
        return PREFIXES
            .iter()
            .find(|(known, _)| *known == prefix)
            .map(|(_, source)| Origin::new(*source, &locator));
    }
    let airband = project.group_named(AIRBAND_GROUP)?;
    let location = bank.groups.iter().find_map(|group| {
        project
            .groups
            .get(*group)
            .filter(|group| !group.locator.is_empty())
    })?;
    let receive_only_am = !bank.channels.is_empty()
        && bank
            .channels
            .iter()
            .all(|channel| channel.modulation == Modulation::Am && channel.transmit_mhz.is_empty());
    (bank.groups.contains(&airband) && receive_only_am)
        .then(|| Origin::new(BankSource::Airports, &location.locator))
}

/// One airport frequency for the map, its mark sized by the airport's.
fn airport_site(frequency: &AirportFrequency, highlight: bool) -> Site {
    Site {
        highlight,
        size: match frequency.size {
            AirportSize::Large => SiteSize::Large,
            AirportSize::Medium => SiteSize::Medium,
            AirportSize::Small => SiteSize::Small,
        },
        ..site(&frequency.name, (frequency.latitude, frequency.longitude))
    }
}

/// The banks every location shares by design, whatever they hold.
const CORE_SHARED: [&str; 4] = [
    stations::STATION_BANK,
    marine::CORE_BANK,
    airfields::CORE_BANK,
    airports::MAJOR_BANK,
];

/// Whether `bank` is the one bank several locations' identical generated
/// banks became: made for no one locator, and not a bank shared by design.
fn is_shared_location_bank(bank: &BankDraft) -> bool {
    bank.origin
        .as_ref()
        .is_some_and(|origin| origin.locator.is_empty())
        && !CORE_SHARED.contains(&bank.name.as_str())
}

/// The location groups `bank` is filed under.
fn locations_of<'a>(project: &'a ProjectModel, bank: &BankDraft) -> Vec<&'a GroupDraft> {
    bank.groups
        .iter()
        .filter_map(|group| project.groups.get(*group))
        .filter(|group| !group.locator.is_empty())
        .collect()
}

/// A generated bank's name: its prefix, such as `RPT`, then `rest`, cut to
/// what a radio holds.
fn prefixed_name(bank: &BankDraft, rest: &str) -> String {
    let prefix = bank.name.split_whitespace().next().unwrap_or_default();
    let mut name = format!("{prefix} {rest}");
    name.truncate(MAX_BANK_NAME_LEN);
    name
}

/// Takes the location at `locator` out of each bank `source`'s banks of
/// several locations became, as its own is being made again; one left with
/// a single location becomes that location's own again.
fn unshare(project: &mut ProjectModel, source: BankSource, locator: &str) {
    let Some(here) = project.location(locator) else {
        return;
    };
    for row in 0..project.banks.len() {
        let bank = &project.banks[row];
        let ours = bank
            .origin
            .as_ref()
            .is_some_and(|origin| origin.source == source);
        if !ours || !is_shared_location_bank(bank) || !bank.groups.contains(&here) {
            continue;
        }
        project.banks[row].groups.retain(|group| *group != here);
        let bank = &project.banks[row];
        if let [only] = locations_of(project, bank)[..] {
            let locator = only.locator.clone();
            let name = prefixed_name(bank, &locator);
            let bank = &mut project.banks[row];
            bank.name = name;
            bank.origin = Some(Origin::new(source, &locator));
        }
    }
}

/// How much of two locations' banks must be common, as a percentage of
/// every channel either holds, for them to share one bank holding both.
const SHARE_OVERLAP_PERCENT: usize = 75;

/// Whether `first` and `second`, alike but for their channels, hold enough
/// of the same channels to share one bank: [`SHARE_OVERLAP_PERCENT`] of all
/// either holds, in any order, since each location puts its most hearable
/// first.
fn overlap_enough(first: &BankDraft, second: &BankDraft) -> bool {
    let common = first
        .channels
        .iter()
        .filter(|channel| second.channels.contains(channel))
        .count();
    let either = first.channels.len() + second.channels.len() - common;
    common * 100 >= SHARE_OVERLAP_PERCENT * either
}

/// Makes the generated banks of different locations which hold mostly the
/// same channels one bank of all of them, filed under every one of the
/// locations and named for them, such as `RPT HOME+WORK`; each location
/// then also holds the few the other hears. Returns how many banks went.
fn share_overlapping(project: &mut ProjectModel) -> usize {
    // Alike in everything but their channels and where they are filed.
    let settings = |bank: &BankDraft| BankDraft {
        name: String::new(),
        groups: Vec::new(),
        origin: None,
        channels: Vec::new(),
        ..bank.clone()
    };
    // A location's own bank, named for its locator, or one already
    // shared; an airport's own bank is named for the airport instead.
    let shareable = |bank: &BankDraft| {
        let own = bank.origin.as_ref().is_some_and(|origin| {
            !origin.locator.is_empty()
                && trailing_locator(&bank.name).as_deref() == Some(origin.locator.as_str())
        });
        !bank.channels.is_empty() && (own || is_shared_location_bank(bank))
    };
    let mut gone = 0;
    let mut row = 0;
    while row < project.banks.len() {
        let mut merged = false;
        let mut other = row + 1;
        while other < project.banks.len() {
            let (first, second) = (&project.banks[row], &project.banks[other]);
            let share = shareable(first)
                && shareable(second)
                && first.origin.as_ref().map(|o| o.source)
                    == second.origin.as_ref().map(|o| o.source)
                && settings(first) == settings(second)
                && overlap_enough(first, second);
            if share {
                let second = project.banks.remove(other);
                let first = &mut project.banks[row];
                for channel in second.channels {
                    if !first.channels.contains(&channel) {
                        first.channels.push(channel);
                    }
                }
                for group in second.groups {
                    if !first.groups.contains(&group) {
                        first.groups.push(group);
                    }
                }
                merged = true;
                gone += 1;
                // What it now holds may share with a bank passed over.
                other = row + 1;
            } else {
                other += 1;
            }
        }
        if merged {
            // Named in the order the locations were made, whichever bank
            // came first.
            project.banks[row].groups.sort_unstable();
            let bank = &project.banks[row];
            let names: Vec<_> = locations_of(project, bank)
                .iter()
                .map(|group| group.name.clone())
                .collect();
            let name = prefixed_name(bank, &names.join("+"));
            let bank = &mut project.banks[row];
            bank.name = name;
            if let Some(origin) = &mut bank.origin {
                origin.locator.clear();
            }
        }
        row += 1;
    }
    gone
}

/// Says how many banks sharing took away, for a status line; nothing when
/// none.
fn shared_note(gone: usize) -> String {
    match gone {
        0 => String::new(),
        1 => " Shared 1 bank between locations hearing mostly the same.".to_owned(),
        _ => format!(" Shared {gone} banks between locations hearing mostly the same."),
    }
}

/// Adds `more` to `sites`, a site already there taken once and singled out
/// when either is.
fn merge_sites(sites: &mut Vec<Site>, more: Vec<Site>) {
    for site in more {
        let same = sites.iter_mut().find(|known| {
            known.name == site.name
                && known.latitude.to_bits() == site.latitude.to_bits()
                && known.longitude.to_bits() == site.longitude.to_bits()
        });
        match same {
            Some(known) => known.highlight |= site.highlight,
            None => sites.push(site),
        }
    }
}

/// The map sites of repeaters, gateways or beacons.
fn repeater_sites(near: &[(&Repeater, f64)]) -> Vec<Site> {
    near.iter()
        .map(|(r, _)| Site {
            size: match r.coverage {
                Coverage::WideArea => SiteSize::Large,
                Coverage::Standard => SiteSize::Medium,
                Coverage::Local => SiteSize::Small,
            },
            ..site(&r.callsign, (r.latitude, r.longitude))
        })
        .collect()
}

/// A generator's area before the operator chooses: its own radius, about
/// how far what it lists is heard on a handheld, and the usual count.
fn area_within(radius_km: u16) -> AirportArea {
    AirportArea {
        radius_km,
        ..AirportArea::default()
    }
}

/// How many of `systems` within `radius_km` of `locator` are on each band,
/// whatever the band filter; none when the locator does not read.
fn band_counts<'a>(
    systems: impl IntoIterator<Item = &'a Repeater>,
    locator: &str,
    radius_km: u16,
) -> Option<[usize; AmateurBand::ALL.len()]> {
    let near = repeaters::nearby(systems, locator, radius_km).ok()?;
    let mut counts = [0; AmateurBand::ALL.len()];
    for band in near
        .iter()
        .filter_map(|(system, _)| AmateurBand::of(system.output_hz))
    {
        counts[band.index()] += 1;
    }
    Some(counts)
}

/// A filter's label with how many it would take, when known.
fn counted(label: &str, count: Option<usize>) -> String {
    count.map_or_else(|| label.to_owned(), |count| format!("{label} ({count})"))
}

/// The row of amateur-band checkboxes under a generator, for `noun` such
/// as `repeaters`, each with how many are in range on it.
fn band_row(
    ui: &mut egui::Ui,
    bands: &mut BandFilter,
    noun: &str,
    counts: Option<[usize; AmateurBand::ALL.len()]>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Bands");
        for band in AmateurBand::ALL {
            let label = counted(band.label(), counts.map(|counts| counts[band.index()]));
            ui.checkbox(&mut bands.bands[band.index()], label)
                .on_hover_text(format!("Takes the {noun} on {}", band.label()));
        }
    });
}

/// What a previewed bank costs, and the project's use after adding it.
fn cost(preview: &AreaPreview) -> String {
    let bank = preview.bank_bytes.map_or_else(
        || ", bank does not compile".to_owned(),
        |bytes| {
            format!(
                ", bank {}",
                memory_percent((bytes * 1_000).div_ceil(RADIO_IMAGE_BYTES))
            )
        },
    );
    let total = match preview.project.radio_image_len() {
        Some(bytes) if bytes > RADIO_IMAGE_BYTES => format!(
            ", project {} after adding, too big for the radio",
            memory_percent((bytes * 1_000).div_ceil(RADIO_IMAGE_BYTES))
        ),
        Some(bytes) => format!(
            ", project {} after adding",
            memory_percent((bytes * 1_000).div_ceil(RADIO_IMAGE_BYTES))
        ),
        // Another bank does not validate; the usage bar already says so.
        None => String::new(),
    };
    format!("{bank}{total}")
}

/// Whether a project validates but does not fit a radio's image, which is
/// the one way its compile fails once validation has passed.
fn too_big(project: &ProjectModel) -> bool {
    matches!(project.to_radio_image(), Err(errors) if errors.iter().any(|e| e.field == "radio image"))
}

/// How long a synced dataset stays fresh: Sync fetches only those synced
/// longer ago, or never.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const SYNC_AFTER_SECS: u64 = 24 * 3600;

/// Seconds since `written`; none for a time in the future.
fn seconds_since(written: std::time::SystemTime) -> u64 {
    // `SystemTime::elapsed` panics in the browser; `web_time` does not.
    let now = web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .unwrap_or_default();
    let then = written
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.saturating_sub(then).as_secs()
}

/// Whether a dataset last synced at `synced`, if ever, is due a sync.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn due(synced: Option<std::time::SystemTime>) -> bool {
    synced.is_none_or(|synced| seconds_since(synced) >= SYNC_AFTER_SECS)
}

/// Whether a dataset last synced at `synced`, if ever, was never synced.
const fn never(synced: Option<std::time::SystemTime>) -> bool {
    synced.is_none()
}

/// Says how long ago a cache was written.
fn age(written: std::time::SystemTime) -> String {
    let age = seconds_since(written);
    match age {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", age / 60),
        3600..=86_399 => format!("{} h ago", age / 3600),
        _ => format!("{} days ago", age / 86_400),
    }
}

/// Reads one project file, and whether it was in an older format.
fn read_project_file(path: &Path) -> Result<(ProjectModel, bool), String> {
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if bytes.len() > MAX_PROJECT_IMAGE_BYTES {
        return Err(format!(
            "{} exceeds {MAX_PROJECT_IMAGE_BYTES} bytes.",
            path.display()
        ));
    }
    ProjectModel::from_image(&bytes)
        .map(|project| (project, ProjectModel::is_older_project(&bytes)))
        .map_err(|error| error.to_string())
}

impl StudioApp {
    /// Logs the status line at debug level when it has changed, so `-v`
    /// records what every action reported, including the ones that succeed
    /// quietly.
    fn log_status(&mut self) {
        if self.status != self.logged_status {
            log::debug!("status: {}", self.status);
            self.logged_status.clone_from(&self.status);
        }
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        // A write or read runs on a worker; the window keeps drawing and
        // checks for its end, wherever the operator is looking.
        self.poll_job();
        #[cfg(target_arch = "wasm32")]
        {
            self.poll_connect();
            self.poll_import();
        }
        self.poll_satellite_sync();
        self.poll_repeater_sync();
        self.poll_station_sync();
        self.poll_airport_sync();
        self.poll_marine_sync();
        self.poll_world_sync();
        self.remember_filters();
        self.log_status();
        if self.job.is_some()
            || self.is_connecting()
            || self.satellite_sync.is_some()
            || self.repeater_sync.is_some()
            || self.station_sync.is_some()
            || self.airport_sync.is_some()
            || self.marine_sync.is_some()
            || self.world_sync.is_some()
        {
            context.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Tiles load only while the map is shown.
        if self.tab == Tab::Map && self.tiles.poll(context) {
            context.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // The Flash tab's automatic flow only runs while it is shown, so a
        // radio is never probed from a tab the operator is not looking at.
        if self.tab == Tab::Flash {
            self.flash.tick(Instant::now());
            if self.flash.busy() {
                context.request_repaint();
            } else {
                context.request_repaint_after(std::time::Duration::from_millis(500));
            }
        }

        egui::TopBottomPanel::top("tabs").show(context, |ui| self.tab_bar(ui));

        egui::TopBottomPanel::bottom("status").show(context, |ui| {
            usage_bar(
                ui,
                &self.project.storage_summary(),
                self.project.role_bytes(),
            );
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&self.status).strong());
            });
            for error in &self.errors {
                ui.colored_label(egui::Color32::from_rgb(0xC0, 0x39, 0x2B), error.to_string());
            }
        });

        egui::CentralPanel::default().show(context, |ui| match self.tab {
            Tab::Projects => self.projects_tab(ui),
            Tab::Banks => self.banks_tab(ui),
            Tab::Map => self.map_tab(ui),
            Tab::Radio => self.radio_tab(ui),
            Tab::Device => self.device_tab(ui),
            Tab::Flash => self.flash.ui(ui),
        });
    }
}

impl StudioApp {
    /// The datasets, each named and whether it is `wanted`, given when it
    /// was last synced, if ever, and not syncing now.
    fn datasets(
        &self,
        wanted: fn(Option<std::time::SystemTime>) -> bool,
    ) -> [(&'static str, bool); 6] {
        [
            (
                "the ETCC's UK repeaters",
                self.repeater_sync.is_none() && wanted(self.repeaters.as_ref().map(|l| l.synced)),
            ),
            (
                "hearham's worldwide repeaters",
                self.world_sync.is_none() && wanted(self.world.as_ref().map(|l| l.synced)),
            ),
            (
                "Ofcom's and the FCC's FM stations",
                self.station_sync.is_none()
                    && wanted(
                        // A source never synced is due now.
                        self.stations
                            .as_ref()
                            .filter(|l| l.complete)
                            .map(|l| l.synced),
                    ),
            ),
            (
                "OurAirports' airports",
                self.airport_sync.is_none() && wanted(self.airports.as_ref().map(|l| l.synced)),
            ),
            (
                "OpenStreetMap's marine channels",
                self.marine_sync.is_none() && wanted(self.marine.as_ref().map(|l| l.synced)),
            ),
            (
                "SatNOGS' satellites",
                self.satellite_sync.is_none() && wanted(self.satellites.as_ref().map(|l| l.synced)),
            ),
        ]
    }

    /// Starts syncing every dataset that is `wanted`, as [`Self::datasets`]
    /// says: on the desktop from each source into the cache, in the browser
    /// from the Pages build's mirror.
    fn sync_datasets(&mut self, wanted: fn(Option<std::time::SystemTime>) -> bool) {
        if self.repeater_cache.dir().is_none() {
            self.refuse("Cannot sync: no XDG cache directory or HOME is set.");
            return;
        }
        let datasets = self.datasets(wanted);
        let [etcc, hearham, ofcom, ourairports, marine, satnogs] = datasets.map(|(_, due)| due);
        if etcc {
            self.repeater_sync = Some(repeaters::SyncJob::start(self.repeater_cache.clone()));
        }
        if hearham {
            self.world_sync = Some(world::SyncJob::start(self.world_cache.clone()));
        }
        if ofcom {
            self.station_sync = Some(stations::SyncJob::start(self.station_cache.clone()));
        }
        if ourairports {
            self.airport_sync = Some(airports::SyncJob::start(self.airport_cache.clone()));
        }
        if marine {
            self.marine_sync = Some(marine::SyncJob::start(self.marine_cache.clone()));
        }
        if satnogs {
            self.satellite_sync = Some(SyncJob::start(self.satellite_cache.clone()));
        }
        let names: Vec<_> = datasets
            .iter()
            .filter(|(_, due)| *due)
            .map(|(name, _)| *name)
            .collect();
        self.status = if names.is_empty() {
            "Every dataset was synced within the last day.".to_owned()
        } else if cfg!(target_arch = "wasm32") {
            format!("Loading {} from the Studio's mirror...", names.join(", "))
        } else {
            format!("Syncing {}...", names.join(", "))
        };
        log::info!("{}", self.status);
    }

    /// The one Sync button, for every dataset due a sync.
    #[cfg(not(target_arch = "wasm32"))]
    fn sync_button(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let names: Vec<_> = self
            .datasets(due)
            .into_iter()
            .filter(|(_, due)| *due)
            .map(|(name, _)| name)
            .collect();
        let help = if names.is_empty() {
            "Every dataset was synced within the last day, or is syncing now".to_owned()
        } else {
            format!(
                "Fetches and caches the lists not synced within the last day: {}. \
                 Studio fetches on its own only a list it has never synced",
                names.join(", ")
            )
        };
        if ui
            .add_enabled(!names.is_empty(), egui::Button::new("Sync"))
            .on_hover_text(&help)
            .on_disabled_hover_text(&help)
            .clicked()
        {
            self.sync_datasets(due);
        }
    }

    /// The main tabs, with the open project's name, Save and Revert at the
    /// right, so they are at hand on every tab; choosing and managing
    /// projects is on the Projects tab.
    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Projects, "Projects");
            ui.selectable_value(&mut self.tab, Tab::Banks, "Banks");
            ui.selectable_value(&mut self.tab, Tab::Map, "Map");
            ui.selectable_value(&mut self.tab, Tab::Radio, "Radio");
            ui.selectable_value(&mut self.tab, Tab::Device, "Device");
            ui.selectable_value(&mut self.tab, Tab::Flash, "Flash");
            // The browser loads the Pages build's mirror on its own.
            #[cfg(not(target_arch = "wasm32"))]
            self.sync_button(ui);
            // Laid out from the right edge, so in reverse.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = self.is_dirty();
                if ui
                    .add_enabled(dirty, egui::Button::new("Revert"))
                    .on_hover_text("Discard the changes since the project was last saved")
                    .clicked()
                {
                    self.load_project();
                }
                if ui.button("Save").clicked() {
                    self.save_project();
                }
                ui.label(RichText::new(&self.projects.current().name).strong())
                    .on_hover_text(self.projects.current().path.display().to_string());
                ui.label("Project:");
                if dirty {
                    ui.colored_label(WARNING_COLOUR, "unsaved changes");
                }
            });
        });
    }

    /// The Projects tab: every project to open, and New, Rename and Delete.
    fn projects_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.new_project)
                    .hint_text("project name")
                    .desired_width(200.0),
            )
            .on_hover_text("The name for New project or Rename");
            if ui
                .button("New project")
                .on_hover_text("Creates an empty project with this name")
                .clicked()
            {
                self.create_project();
            }
            if ui
                .button("Rename")
                .on_hover_text("Renames the open project, and its file, to this name")
                .clicked()
            {
                self.rename_project();
            }
            if self.confirm_delete {
                ui.colored_label(
                    WARNING_COLOUR,
                    format!("Delete {}?", self.projects.current().name),
                );
                if ui
                    .button("Yes, delete")
                    .on_hover_text("Deletes the project file; this cannot be undone")
                    .clicked()
                {
                    self.delete_project();
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_delete = false;
                }
            } else if ui
                .button("Delete project")
                .on_hover_text("Deletes the open project, after asking")
                .clicked()
            {
                self.confirm_delete = true;
            }
            // The browser keeps projects in the page: these move them in
            // and out as files.
            #[cfg(target_arch = "wasm32")]
            {
                ui.separator();
                if ui
                    .add_enabled(self.project_upload.is_none(), egui::Button::new("Import…"))
                    .on_hover_text("Adds a .afik project file as a new project")
                    .clicked()
                {
                    self.choose_import();
                }
                if ui
                    .button("Export")
                    .on_hover_text("Downloads the open project's saved file")
                    .clicked()
                {
                    self.export_project();
                }
            }
        });
        ui.separator();
        let mut chosen = None;
        ScrollArea::vertical().show(ui, |ui| {
            for (index, entry) in self.projects.entries().iter().enumerate() {
                let open = index == self.projects.current_index();
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(open, RichText::new(&entry.name).strong())
                        .on_hover_text("Opens this project")
                        .clicked()
                    {
                        chosen = Some(index);
                    }
                    ui.weak(entry.path.display().to_string());
                });
            }
        });
        if let Some(index) = chosen {
            self.open_project(index);
        }
    }

    /// Default sets add banks, so they are offered only on the Banks tab.
    fn default_set_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Default set:");
            let selected = PRESETS
                .get(self.preset)
                .map_or("none", |preset| preset.name());
            ComboBox::from_id_salt("preset")
                .selected_text(selected)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (index, preset) in PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut self.preset, index, preset.name())
                            .on_hover_text(preset.detail());
                    }
                });
            // Choosing a set changes nothing until it is added.
            if ui
                .button("Add")
                .on_hover_text("Adds the set's banks after the existing ones")
                .clicked()
            {
                self.add_preset();
            }
            if let Some(preset) = PRESETS.get(self.preset) {
                ui.label(preset.detail());
                if preset.name() == SATELLITE_PRESET {
                    ui.end_row();
                    ui.label(self.satellite_source());
                }
            }
        });
        ui.separator();
    }

    /// The bordered box of the locator and the generators that use it, at
    /// the top of the Map tab.
    fn generators_box(&mut self, ui: &mut egui::Ui) {
        // One bordered panel holds the locator and the generators that use
        // it; the chosen generator's row sits in a second panel joined to
        // its tab, so the tabs read as belonging to it.
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong(GENERATORS_TITLE);
            self.locator_bar(ui);
            ui.add_space(4.0);
            let fill = ui.visuals().extreme_bg_color;
            let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
            let selected_tab = ui
                .scope(|ui| {
                    // The tabs touch the panel and nearly each other.
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                    let tabs = ui.horizontal(|ui| {
                        let mut selected = None;
                        for generator in Generator::ALL {
                            let tab = generator_tab(ui, generator, self.generator, fill);
                            if tab.clicked() {
                                self.generator = generator;
                            }
                            if generator == self.generator {
                                selected = Some(tab.rect);
                            }
                        }
                        selected
                    });
                    let panel = egui::Frame::NONE
                        .fill(fill)
                        .stroke(stroke)
                        .corner_radius(egui::CornerRadius {
                            nw: 0,
                            ne: 4,
                            sw: 4,
                            se: 4,
                        })
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
                            ui.set_width(ui.available_width());
                            self.generator_row(ui);
                        });
                    tabs.inner.map(|tab| (tab, panel.response.rect.top()))
                })
                .inner;
            // Open the panel's top edge under the chosen tab, so the two
            // read as one shape.
            if let Some((tab, top)) = selected_tab {
                let inset = stroke.width.max(1.0);
                ui.painter().line_segment(
                    [
                        egui::pos2(tab.left() + inset, top),
                        egui::pos2(tab.right() - inset, top),
                    ],
                    egui::Stroke::new(inset + 1.0, fill),
                );
            }
        });
    }

    /// The chosen generator's row.
    fn generator_row(&mut self, ui: &mut egui::Ui) {
        match self.generator {
            Generator::Repeaters => self.repeater_bar(ui),
            Generator::Gateways => self.etcc_bar(ui, EtccKind::Gateways),
            Generator::Beacons => self.etcc_bar(ui, EtccKind::Beacons),
            Generator::Stations => self.station_bar(ui),
            Generator::Airports => self.airport_bar(ui),
            Generator::Airfields => self.airfield_bar(ui),
            Generator::Marine => self.marine_bar(ui),
        }
    }

    /// The row that adds the repeaters near the shared locator.
    fn repeater_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Repeaters within");
            ComboBox::from_id_salt("repeater-radius")
                .selected_text(format!("{} km", self.repeater_area.radius_km))
                .show_ui(ui, |ui| {
                    for km in repeaters::RADII_KM {
                        ui.selectable_value(
                            &mut self.repeater_area.radius_km,
                            km,
                            format!("{km} km"),
                        );
                    }
                });
            ui.label("nearest");
            ui.add(
                egui::DragValue::new(&mut self.repeater_area.max_channels)
                    .range(1..=repeaters::MAX_CHANNELS_LIMIT),
            )
            .on_hover_text("How many repeaters the bank takes, most hearable first");
            // Always enabled: a disabled button here cuts the Tab order
            // short in egui 0.29, and Add explains itself when unsynced.
            if ui
                .button("Add")
                .on_hover_text(
                    "Adds the FM repeaters heard here as one bank, most hearable first, \
                     replacing one for the same locator; each receives the output and \
                     transmits on the input with its CTCSS tone",
                )
                .clicked()
            {
                self.add_repeaters();
            }
            ui.end_row();
            ui.label(self.repeater_source());
        });
        let counts = self.all_repeaters().and_then(|all| {
            let area = &self.repeater_area;
            band_counts(all, &area.locator, area.radius_km)
        });
        band_row(ui, &mut self.repeater_bands, "repeaters", counts);
        self.repeater_reach_controls(ui);
    }

    /// How far off a standard and a local repeater are heard; a wide-area
    /// one across the radius.
    fn repeater_reach_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Heard to");
            for (coverage, km) in Coverage::CHOSEN
                .into_iter()
                .zip(&mut self.repeater_reach.reach_km)
            {
                ui.label(coverage.label());
                let reach = self.receiver.reach_km(*km);
                ui.add(
                    egui::DragValue::new(km)
                        .range(1..=repeaters::MAX_RADIUS_KM)
                        .suffix(" km"),
                )
                .on_hover_text(format!(
                    "How far off a handheld hears {}; a {} hears it to about {reach} km. \
                     Drawn faintly about each locator on the map",
                    coverage.help(),
                    self.receiver.label().to_lowercase()
                ));
            }
            if ui
                .button("Defaults")
                .on_hover_text("Puts each repeater's reach back to its estimate")
                .clicked()
            {
                self.repeater_reach = RepeaterReach::default();
            }
            ui.label(format!(
                "Wide-area: the radius, {} km",
                self.repeater_area.radius_km
            ))
            .on_hover_text(format!(
                "A full-power repeater with its aerial at least {} m up is heard across the \
                 radius; one under 10 dBW or with its aerial under {} m is local. The ETCC \
                 does not list the site's own height, so a hilltop with a short mast reads \
                 as standard",
                repeaters::WIDE_AREA_HEIGHT_M,
                repeaters::LOCAL_HEIGHT_M
            ));
        });
    }

    /// The row that adds the UK gateways or beacons near the shared locator,
    /// from the repeaters' ETCC list.
    fn etcc_bar(&mut self, ui: &mut egui::Ui, kind: EtccKind) {
        ui.horizontal_wrapped(|ui| {
            let (label, salt, add_help) = match kind {
                EtccKind::Gateways => (
                    "UK gateways within",
                    "gateway-radius",
                    "Adds the nearest analogue FM gateways (simplex internet nodes) in range \
                     as one bank, replacing one for the same locator; each is simplex on the \
                     node's frequency with its CTCSS tone",
                ),
                EtccKind::Beacons => (
                    "UK beacons within",
                    "beacon-radius",
                    "Adds the nearest beacons in range as one receive-only USB bank, \
                     replacing one for the same locator; each is tuned 700 Hz below the \
                     carrier so the beacon is heard as a tone",
                ),
            };
            let area = match kind {
                EtccKind::Gateways => &mut self.gateway_area,
                EtccKind::Beacons => &mut self.beacon_area,
            };
            ui.label(label);
            ComboBox::from_id_salt(salt)
                .selected_text(format!("{} km", area.radius_km))
                .show_ui(ui, |ui| {
                    for km in repeaters::RADII_KM {
                        ui.selectable_value(&mut area.radius_km, km, format!("{km} km"));
                    }
                });
            ui.label("nearest");
            ui.add(
                egui::DragValue::new(&mut area.max_channels)
                    .range(1..=airports::MAX_CHANNELS_LIMIT),
            )
            .on_hover_text(format!(
                "How many {} the bank takes, nearest first",
                kind.noun()
            ));
            if ui.button("Add").on_hover_text(add_help).clicked() {
                self.add_etcc(kind);
            }
            ui.end_row();
            ui.label(self.etcc_source(kind));
        });
        let counts = self.repeaters.as_ref().and_then(|list| {
            let locator = &self.repeater_area.locator;
            band_counts(kind.systems(list), locator, self.etcc_area(kind).radius_km)
        });
        let bands = match kind {
            EtccKind::Gateways => &mut self.gateway_bands,
            EtccKind::Beacons => &mut self.beacon_bands,
        };
        band_row(ui, bands, kind.noun(), counts);
    }

    /// The sites `generator` would add at the locator, and the radius it
    /// takes them from; none when it cannot preview, such as before a sync.
    fn generator_sites(&self, generator: Generator) -> Option<(Vec<Site>, Option<u16>)> {
        let radius_km = match generator {
            Generator::Repeaters => Some(self.repeater_area.radius_km),
            Generator::Gateways => Some(self.gateway_area.radius_km),
            Generator::Beacons => Some(self.beacon_area.radius_km),
            Generator::Stations => None,
            Generator::Airports => Some(self.airport_area.radius_km),
            Generator::Airfields => Some(self.airfield_area.radius_km),
            Generator::Marine => Some(self.marine_area.radius_km),
        };
        let preview = self.generator_preview(generator).ok()?;
        Some((preview.sites, radius_km))
    }

    /// What `generator` would add here, or why it cannot.
    fn generator_preview(&self, generator: Generator) -> Result<AreaPreview, String> {
        match generator {
            Generator::Repeaters => self.repeater_preview(),
            Generator::Gateways => self.etcc_preview(EtccKind::Gateways),
            Generator::Beacons => self.etcc_preview(EtccKind::Beacons),
            Generator::Stations => self.station_preview(),
            Generator::Airports => self.airport_preview().map(|(preview, _)| preview),
            Generator::Airfields => self.airfield_preview(),
            Generator::Marine => self.marine_preview(),
        }
    }

    /// The faint ranges drawn about each centre of `generator`'s layer: how
    /// far off each size of airport's approach is heard, when taken.
    fn generator_rings(&self, generator: Generator) -> Vec<(f64, String)> {
        match generator {
            Generator::Airports if self.airport_options.takes(Service::Approach) => {
                approach_rings(&self.airport_options, self.receiver)
            }
            Generator::Repeaters => repeater_rings(&self.repeater_reach, self.receiver),
            _ => Vec::new(),
        }
    }

    /// Everything the map's layers are built from: the locator, every
    /// generator's area and filters, which are shown, and when each list
    /// was synced.
    fn map_settings(&self) -> String {
        format!(
            "{:?}",
            (
                (&self.repeater_area, &self.repeater_bands),
                &self.repeater_reach,
                (&self.gateway_area, &self.gateway_bands),
                (&self.beacon_area, &self.beacon_bands, &self.station_area),
                (
                    &self.airport_area,
                    &self.airport_options,
                    &self.airfield_area
                ),
                (&self.marine_area, self.map_shown, self.receiver),
                (
                    self.map_all_locations,
                    &self.project.locations.list,
                    &self.repeater_area.locator,
                ),
                (
                    self.repeaters.as_ref().map(|list| list.synced),
                    self.world.as_ref().map(|list| list.synced),
                    self.stations.as_ref().map(|list| list.synced),
                    self.airports.as_ref().map(|list| list.synced),
                    self.marine.as_ref().map(|list| list.synced),
                ),
            )
        )
    }

    /// The locators the map shows and "Add all shown" adds for: the
    /// operator's, then with every location shown each saved one.
    fn map_locators(&self) -> Vec<String> {
        let mut locators = vec![self.repeater_area.locator.clone()];
        if self.map_all_locations {
            for saved in &self.project.locations.list {
                if !locators.contains(&saved.locator) {
                    locators.push(saved.locator.clone());
                }
            }
        }
        locators
    }

    /// The map's layers for the generators shown, rebuilt whenever a setting
    /// behind them changes, so the map follows the filters as they are set.
    fn map_layers(&mut self) -> Vec<Layer> {
        let settings = self.map_settings();
        if let Some((built_from, layers)) = &self.map_layers {
            if *built_from == settings {
                return layers.clone();
            }
        }
        let locators = self.map_locators();
        // Each locator's previews read the shared locator, so it is set to
        // each in turn and put back.
        let own = self.repeater_area.locator.clone();
        let mut layers: Vec<Layer> = Vec::new();
        for locator in &locators {
            let Ok(centre) = repeaters::locator_centre(locator) else {
                continue;
            };
            locator.clone_into(&mut self.repeater_area.locator);
            for (generator, shown) in Generator::ALL.into_iter().zip(self.map_shown) {
                if !shown {
                    continue;
                }
                let Some((sites, radius_km)) = self.generator_sites(generator) else {
                    continue;
                };
                let label = generator.label();
                let layer = if let Some(layer) = layers.iter_mut().find(|l| l.label == label) {
                    layer
                } else {
                    layers.push(Layer {
                        label,
                        color: generator.color(),
                        icon: generator.icon(),
                        sites: Vec::new(),
                        radius_km: radius_km.map(f64::from),
                        centres: Vec::new(),
                        rings: self.generator_rings(generator),
                    });
                    layers.last_mut().expect("just pushed")
                };
                // A radius with nothing in it is not drawn.
                if !sites.is_empty() {
                    layer.centres.push(centre);
                }
                merge_sites(&mut layer.sites, sites);
            }
        }
        self.repeater_area.locator = own;
        // Layers keep the order of [`Generator::ALL`].
        layers.sort_by_key(|layer| {
            Generator::ALL
                .iter()
                .position(|generator| generator.label() == layer.label)
        });
        self.map_layers = Some((settings, layers.clone()));
        layers
    }

    /// Runs `generator`'s Add.
    fn add_generator(&mut self, generator: Generator) {
        match generator {
            Generator::Repeaters => self.add_repeaters(),
            Generator::Gateways => self.add_etcc(EtccKind::Gateways),
            Generator::Beacons => self.add_etcc(EtccKind::Beacons),
            Generator::Stations => self.add_stations(),
            Generator::Airports => self.add_airports(),
            Generator::Airfields => self.add_airfields(),
            Generator::Marine => self.add_marine(),
        }
    }

    /// Adds or updates the bank of every generator shown on the map, and
    /// says which did and why the rest did not, or how many across every
    /// location.
    fn add_all_shown(&mut self) {
        let mut added = Vec::new();
        let mut skipped = Vec::new();
        // With every location shown, what the generators made before is
        // dropped and rewritten, so a filter that now leaves a bank out
        // takes it away.
        let mut dropped = 0;
        if self.map_all_locations {
            let before = self.project.banks.len();
            self.project.banks.retain(|bank| bank.origin.is_none());
            dropped = before - self.project.banks.len();
        }
        let locators = self.map_locators();
        let own = self.repeater_area.locator.clone();
        for locator in &locators {
            // With every location shown, each is named in what it added.
            let at = |label: &str| {
                if locators.len() > 1 {
                    format!("{label} at {locator}")
                } else {
                    label.to_owned()
                }
            };
            locator.clone_into(&mut self.repeater_area.locator);
            for (generator, shown) in Generator::ALL.into_iter().zip(self.map_shown) {
                if !shown {
                    continue;
                }
                let preview = match self.generator_preview(generator) {
                    Ok(preview) => preview,
                    Err(reason) => {
                        skipped.push(format!("{} ({reason})", at(generator.label())));
                        continue;
                    }
                };
                // Unsynced, airfields and marine still have their core;
                // synced with none in range, the add takes the core from
                // here.
                let cored = match generator {
                    Generator::Airfields => self.airports.is_none(),
                    Generator::Marine => self.marine.is_none(),
                    _ => false,
                };
                if preview.taken == 0 && !cored {
                    if matches!(generator, Generator::Airfields | Generator::Marine) {
                        self.add_generator(generator);
                    }
                    skipped.push(format!("{} (none in range)", at(generator.label())));
                } else if preview.project.banks.len() > usize::from(MAX_BANKS) {
                    skipped.push(format!(
                        "{} (more than {MAX_BANKS} banks)",
                        at(generator.label())
                    ));
                } else {
                    // Added past the radio's memory too; the usage bar shows
                    // by how much, and the operator drops what to after.
                    self.overfill = true;
                    self.add_generator(generator);
                    self.overfill = false;
                    added.push(at(generator.label()));
                }
            }
        }
        self.repeater_area.locator = own;
        // An add keeps the locator it ran at; the operator's is kept instead.
        if locators.len() > 1 {
            self.remember_locator();
        }
        // Across every location the names run long, so the status counts
        // them and the log keeps each one.
        let summary = locators.len() > 1;
        if summary {
            if !added.is_empty() {
                log::info!("added or updated {}", added.join(", "));
            }
            if !skipped.is_empty() {
                log::info!("skipped {}", skipped.join("; "));
            }
        }
        let what = if summary {
            format!("{} generators at {} locations", added.len(), locators.len())
        } else {
            added.join(", ")
        };
        let added = match (added.is_empty(), dropped) {
            (true, 0) => "Nothing added".to_owned(),
            (true, _) => format!("Dropped {dropped} location banks and added nothing"),
            (false, 0) => format!("Added or updated {what}"),
            (false, _) => format!("Dropped {dropped} location banks and rewrote {what}"),
        };
        self.status = match (skipped.len(), summary) {
            (0, _) => format!("{added}."),
            (count, true) => format!("{added}; skipped {count}."),
            (_, false) => format!("{added}. Skipped {}.", skipped.join("; ")),
        };
        if let Some(per_mille) = self
            .project
            .storage_summary()
            .used_per_mille()
            .filter(|per_mille| *per_mille > 1_000)
        {
            self.status = format!(
                "{} The project is now {} of the radio's memory; remove banks before Write.",
                self.status,
                memory_percent(per_mille)
            );
        }
        log::info!("{}", self.status);
    }

    /// Makes the locator holding a point clicked on the map the current one.
    fn pick_on_map(&mut self, (latitude, longitude): (f64, f64)) {
        let locator = map::locator_at(latitude, longitude);
        // The map stays where it is rather than jumping to the new locator.
        locator.clone_into(&mut self.map_locator);
        self.choose_locator(&locator);
        self.status = format!("Chose {locator} on the map.");
    }

    /// The Map tab: the generators and their locator, which to show, and
    /// the map, which follows every change to them.
    fn map_tab(&mut self, ui: &mut egui::Ui) {
        self.generators_box(ui);
        ui.horizontal_wrapped(|ui| {
            ui.label("Show");
            for (generator, shown) in Generator::ALL.into_iter().zip(&mut self.map_shown) {
                let label = RichText::new(format!("{} {}", generator.icon(), generator.label()))
                    .color(generator.color());
                ui.checkbox(shown, label)
                    .on_hover_text("Shows what this generator would add, and its radius");
            }
            ui.separator();
            self.per_airport_controls(ui);
            ui.separator();
            if ui.button("\u{2212}").on_hover_text("Zooms out").clicked() {
                map::zoom_by(&mut self.map_view, -1);
            }
            if ui.button("+").on_hover_text("Zooms in").clicked() {
                map::zoom_by(&mut self.map_view, 1);
            }
            if ui
                .button("Centre")
                .on_hover_text("Centres the map on your locator")
                .clicked()
            {
                self.map_locator.clear();
            }
            ui.separator();
            if ui
                .button("Add all shown")
                .on_hover_text(
                    "Adds or updates the bank of every generator shown, for this locator \
                     or with All locations for every saved one, each filed under its \
                     location",
                )
                .clicked()
            {
                self.add_all_shown();
            }
        });
        let home = repeaters::locator_centre(&self.repeater_area.locator).ok();
        // A locator typed or chosen from the dropdown brings the map to it.
        if self.map_locator != self.repeater_area.locator {
            if let Some(home) = home {
                self.map_view.centre = home;
                self.map_view.zoom = self.map_view.zoom.max(8);
            }
            self.map_locator.clone_from(&self.repeater_area.locator);
        }
        let pins: Vec<Pin> = self
            .project
            .locations
            .list
            .iter()
            .filter_map(|saved| {
                Some(Pin {
                    label: saved.label(),
                    position: repeaters::locator_centre(&saved.locator).ok()?,
                })
            })
            .collect();
        let layers = self.map_layers();
        if let Some(clicked) = map::show(
            ui,
            &mut self.map_view,
            &mut self.tiles,
            home,
            &pins,
            &layers,
        ) {
            self.pick_on_map(clicked);
        }
    }

    /// The row holding the locator the repeaters, stations and airports share.
    fn locator_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Your locator");
            let edited = ui
                .add(
                    egui::TextEdit::singleline(&mut self.repeater_area.locator)
                        .hint_text("IO91WM")
                        .char_limit(8)
                        .desired_width(70.0),
                )
                .on_hover_text(
                    "Your Maidenhead locator, 4 to 8 characters, for every generator below",
                )
                .changed();
            if edited {
                if self
                    .project
                    .locations
                    .find(&self.repeater_area.locator)
                    .is_some()
                {
                    self.locator_description = self.saved_description();
                }
                self.remember_locator();
            }
            let current = self.project.locations.find(&self.repeater_area.locator);
            let mut chosen = None;
            let mut all = self.map_all_locations;
            let selected_text = if all {
                ALL_LOCATIONS.to_owned()
            } else {
                current.map_or_else(|| "Saved...".to_owned(), SavedLocator::label)
            };
            ComboBox::from_id_salt("saved-locators")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut all, true, ALL_LOCATIONS)
                        .on_hover_text(
                            "Shows every saved locator on the map at once, and Add all \
                             shown adds for each",
                        );
                    for saved in &self.project.locations.list {
                        let selected = !all && current.is_some_and(|c| c.locator == saved.locator);
                        if ui.selectable_label(selected, saved.label()).clicked() {
                            chosen = Some(saved.locator.clone());
                        }
                    }
                })
                .response
                .on_hover_text("Chooses a saved locator");
            self.map_all_locations = all;
            if let Some(locator) = chosen {
                self.choose_locator(&locator);
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.locator_description)
                    .hint_text("Description, e.g. Home")
                    .desired_width(140.0),
            );
            if ui
                .button("Save")
                .on_hover_text("Keeps this locator and its description in the dropdown")
                .clicked()
            {
                self.save_locator();
            }
            // Always enabled: a disabled button cuts the Tab order short in
            // egui 0.29, and Forget explains itself when nothing is saved.
            if ui
                .button("Forget")
                .on_hover_text("Removes this locator from the dropdown")
                .clicked()
            {
                self.forget_locator();
            }
            ui.separator();
            ui.label("Receiver");
            let mut chosen = None;
            ComboBox::from_id_salt("receiver")
                .selected_text(self.receiver.label())
                .show_ui(ui, |ui| {
                    for receiver in Receiver::ALL {
                        if ui
                            .selectable_label(receiver == self.receiver, receiver.label())
                            .on_hover_text(receiver.description())
                            .clicked()
                        {
                            chosen = Some(receiver);
                        }
                    }
                })
                .response
                .on_hover_text(
                    "What you listen with; choosing one sets every generator's radius to \
                     about how far it hears, and how far the airband services carry",
                );
            if let Some(receiver) = chosen {
                self.choose_receiver(receiver);
            }
        });
    }

    /// The description kept for the current locator, else none.
    fn saved_description(&self) -> String {
        self.project
            .locations
            .find(&self.repeater_area.locator)
            .map(|saved| saved.description.clone())
            .unwrap_or_default()
    }

    /// Makes a kept locator the current one.
    fn choose_locator(&mut self, locator: &str) {
        self.map_all_locations = false;
        locator.clone_into(&mut self.repeater_area.locator);
        self.locator_description = self.saved_description();
        self.remember_locator();
    }

    /// Listens with `receiver`: every generator's radius becomes its own,
    /// and all of them are kept for the next start.
    fn choose_receiver(&mut self, receiver: Receiver) {
        self.receiver = receiver;
        let radii = receiver.radii();
        self.repeater_area.radius_km = radii.repeaters;
        self.gateway_area.radius_km = radii.gateways;
        self.beacon_area.radius_km = radii.beacons;
        self.airport_area.radius_km = radii.airports;
        self.airfield_area.radius_km = radii.airfields;
        self.marine_area.radius_km = radii.marine;
        self.status = format!(
            "Radii set for a {}: repeaters {} km, gateways {} km, beacons {} km, airports {} \
             km, airfields {} km, marine {} km.",
            receiver.label().to_lowercase(),
            radii.repeaters,
            radii.gateways,
            radii.beacons,
            radii.airports,
            radii.airfields,
            radii.marine
        );
        let saved = [
            self.receiver_file.as_ref().map(|file| receiver.save(file)),
            self.repeater_area_file
                .as_ref()
                .map(|file| self.repeater_area.save(file)),
            self.gateway_area_file
                .as_ref()
                .map(|file| self.gateway_area.save(file)),
            self.beacon_area_file
                .as_ref()
                .map(|file| self.beacon_area.save(file)),
            self.airport_area_file
                .as_ref()
                .map(|file| self.airport_area.save(file)),
            self.airfield_area_file
                .as_ref()
                .map(|file| self.airfield_area.save(file)),
            self.marine_area_file
                .as_ref()
                .map(|file| self.marine_area.save(file)),
        ];
        for error in saved.into_iter().flatten().filter_map(Result::err) {
            log::warn!("could not remember the receiver's radii: {error}");
        }
    }

    /// Keeps the current locator and its description in the dropdown.
    fn save_locator(&mut self) {
        let (locator, description) = (&self.repeater_area.locator, &self.locator_description);
        if let Err(error) = self.project.locations.put(locator, description) {
            self.refuse(&format!("Cannot save the locator: {error}."));
            return;
        }
        let saved = self
            .project
            .locations
            .find(locator)
            .map(SavedLocator::label);
        self.status = format!(
            "Saved {} in the project; save the project to keep it.",
            saved.unwrap_or_default()
        );
        // The location's group follows its description.
        let locator = self.repeater_area.locator.clone();
        if let Some(group) = self.project.location(&locator) {
            let name = self.location_name(&locator);
            if self.project.groups[group].name != name {
                self.status = format!(
                    "{} Renamed its group {} to {name}.",
                    self.status, self.project.groups[group].name
                );
                self.project.groups[group].name = name;
            }
        }
    }

    /// Forgets the current locator from the dropdown; the field keeps it.
    fn forget_locator(&mut self) {
        let locator = self.repeater_area.locator.trim().to_ascii_uppercase();
        if self.project.locations.remove(&locator) {
            self.status = format!("Forgot {locator}; save the project to keep that.");
        } else {
            self.status = format!("{locator} is not saved.");
        }
    }

    /// Gives a project that keeps no locations the ones older studios kept
    /// beside the projects (`ADR-120`), as an unsaved change, so saving the
    /// project keeps them in it.
    fn adopt_legacy_locations(&mut self) {
        if !self.project.locations.list.is_empty() || self.legacy_locators.list.is_empty() {
            return;
        }
        self.project.locations = self.legacy_locators.clone();
        let count = self.project.locations.list.len();
        log::info!("took {count} saved locations from the older studios' file");
        self.status = format!(
            "{} Took {count} saved location{} from before they were kept in projects; \
             save to keep them in this one.",
            self.status,
            if count == 1 { "" } else { "s" }
        );
    }

    /// Keeps a locator edit for the next start once it reads as one, so the
    /// repeaters and the stations always share what was last typed.
    fn remember_locator(&self) {
        if repeaters::locator_centre(&self.repeater_area.locator).is_err() {
            return;
        }
        if let Some(file) = &self.repeater_area_file {
            if let Err(error) = self.repeater_area.save(file) {
                log::warn!("could not remember the locator: {error}");
            }
        }
    }

    /// The row that adds the UK or US FM stations near the shared locator.
    fn station_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("FM stations (UK, US), strongest");
            ui.add(
                egui::DragValue::new(&mut self.station_area.max_channels)
                    .range(1..=stations::MAX_CHANNELS_LIMIT),
            )
            .on_hover_text("How many stations the bank takes, strongest signal first");
            if ui
                .button("Add")
                .on_hover_text(
                    "Adds the stations with the strongest estimated signal here (transmitter \
                     power over distance squared) to the one FM STATIONS bank every location \
                     shares, in frequency order with each frequency once",
                )
                .clicked()
            {
                self.add_stations();
            }
            ui.end_row();
            ui.label(self.station_source());
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Licences");
            let counts = self.station_kind_counts();
            // Once synced, only the kinds offered here: the UK's or the US's.
            let offered = |kind: StationKind| counts.is_none_or(|counts| counts[kind.index()] > 0);
            for kind in StationKind::ALL.into_iter().filter(|&kind| offered(kind)) {
                let label = counted(kind.label(), counts.map(|counts| counts[kind.index()]));
                ui.checkbox(&mut self.station_area.kinds[kind.index()], label)
                    .on_hover_text(format!("Takes the {}", kind.description()));
            }
        });
    }

    /// How many stations of each licence kind the ranking would offer at
    /// the locator, whatever the kinds ticked.
    fn station_kind_counts(&self) -> Option<[usize; StationKind::ALL.len()]> {
        let list = self.stations.as_ref()?;
        let ranked =
            stations::strongest(&list.stations, &self.repeater_area.locator, self.receiver).ok()?;
        let mut counts = [0; StationKind::ALL.len()];
        for (station, _) in ranked {
            counts[station.kind.index()] += 1;
        }
        Some(counts)
    }

    /// How many frequencies of each service are within the airport radius
    /// and heard from the locator, whatever the services ticked.
    fn airport_service_counts(&self) -> Option<[usize; Service::ALL.len()]> {
        let list = self.airports.as_ref()?;
        let home = repeaters::locator_centre(&self.repeater_area.locator).ok()?;
        let radius = f64::from(self.airport_area.radius_km);
        let mut counts = [0; Service::ALL.len()];
        for frequency in &list.frequencies {
            let km = repeaters::distance_km(home, (frequency.latitude, frequency.longitude));
            if km <= radius
                && km
                    <= f64::from(self.airport_options.reach_km(
                        frequency,
                        self.receiver,
                        self.airport_area.radius_km,
                    ))
            {
                counts[frequency.service.index()] += 1;
            }
        }
        Some(counts)
    }

    /// The row that adds the airport frequencies near the shared locator.
    fn airport_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Airports within");
            ComboBox::from_id_salt("airport-radius")
                .selected_text(format!("{} km", self.airport_area.radius_km))
                .show_ui(ui, |ui| {
                    for km in repeaters::RADII_KM {
                        ui.selectable_value(
                            &mut self.airport_area.radius_km,
                            km,
                            format!("{km} km"),
                        );
                    }
                });
            ui.label("nearest");
            ui.add(
                egui::DragValue::new(&mut self.airport_area.max_channels)
                    .range(1..=airports::MAX_CHANNELS_LIMIT),
            )
            .on_hover_text("How many frequencies the area bank takes, most hearable first");
            if ui
                .button("Add")
                .on_hover_text(
                    "Adds the airband frequencies of the chosen services at the airports in \
                     range that can be heard here as one receive-only AM bank, most hearable \
                     first, replacing one for the same locator; with a bank per airport, the \
                     nearest airports' tower, ground and ATIS go in banks of their own",
                )
                .clicked()
            {
                self.add_airports();
            }
            ui.end_row();
            ui.label(self.airport_source());
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Take");
            let counts = self.airport_service_counts();
            for service in Service::ALL {
                let help = match service {
                    Service::Approach => {
                        "Approach, departure, radar and area control: aircraft at height"
                    }
                    Service::Tower => "Tower, AFIS, air/ground, CTAF and UNICOM: the circuit",
                    Service::Ground => "Ground, delivery, apron and operations: taxiing",
                    Service::Broadcast => {
                        "ATIS and other recorded broadcasts, which never stop and so hold a \
                         scan"
                    }
                    Service::Other => "Anything else OurAirports lists",
                };
                ui.checkbox(
                    &mut self.airport_options.services[service.index()],
                    counted(
                        service.label(),
                        counts.map(|counts| counts[service.index()]),
                    ),
                )
                .on_hover_text(heard_to(
                    service,
                    help,
                    &self.airport_options,
                    self.receiver,
                    self.airport_area.radius_km,
                ));
            }
        });
        self.approach_range_controls(ui);
    }

    /// How far off each size of airport's approach is heard, set while
    /// approach is taken.
    fn approach_range_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(self.airport_options.takes(Service::Approach), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Approach heard to");
                for (size, km) in AirportSize::CHOSEN
                    .into_iter()
                    .zip(&mut self.airport_options.approach_km)
                {
                    ui.label(size.label());
                    let reach = self.receiver.reach_km(*km);
                    ui.add(
                        egui::DragValue::new(km)
                            .range(1..=airports::MAX_APPROACH_KM)
                            .suffix(" km"),
                    )
                    .on_hover_text(format!(
                        "How far off a handheld hears approach and radar at {}; a {} hears \
                         it to about {reach} km. Drawn faintly about each locator on the map",
                        size.help(),
                        self.receiver.label().to_lowercase()
                    ));
                }
                if ui
                    .button("Defaults")
                    .on_hover_text("Puts each size's approach range back to its estimate")
                    .clicked()
                {
                    self.airport_options.approach_km = AirportOptions::default().approach_km;
                }
                ui.label(format!(
                    "Large: the airport radius, {} km",
                    self.airport_area.radius_km
                ))
                .on_hover_text(
                    "Airliners into a large airport are high and heard from far off, so its \
                     approach is taken anywhere within the airports radius",
                );
            });
        });
    }

    /// Whether the nearest airports each get a bank of their own, and how
    /// many, on the map's Show row since every location's airports take it.
    fn per_airport_controls(&mut self, ui: &mut egui::Ui) {
        let km = airports::own_bank_km(self.receiver);
        ui.checkbox(&mut self.airport_options.per_airport, "Bank per airport")
            .on_hover_text(format!(
                "Gives each of the nearest airports within {km} km a bank of its own tower, \
                 ground and ATIS, for when you are there; the area bank keeps the rest"
            ));
        ui.add_enabled(
            self.airport_options.per_airport,
            egui::DragValue::new(&mut self.airport_options.airports)
                .range(1..=MAX_BANKS)
                .suffix(" airports"),
        )
        .on_hover_text(format!(
            "How many airports within {km} km, nearest first, get a bank of their own"
        ));
    }

    /// The row that adds the gliding and small-airfield frequencies near the
    /// shared locator.
    fn airfield_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Airfields within");
            ComboBox::from_id_salt("airfield-radius")
                .selected_text(format!("{} km", self.airfield_area.radius_km))
                .show_ui(ui, |ui| {
                    for km in repeaters::RADII_KM {
                        ui.selectable_value(
                            &mut self.airfield_area.radius_km,
                            km,
                            format!("{km} km"),
                        );
                    }
                });
            ui.label("nearest");
            ui.add(
                egui::DragValue::new(&mut self.airfield_area.max_channels)
                    .range(1..=airports::MAX_CHANNELS_LIMIT),
            )
            .on_hover_text("How many local frequencies the bank takes, nearest field first");
            if ui
                .button("Add")
                .on_hover_text(
                    "Adds one receive-only AM bank: SafetyCom and the UK gliding, \
                     microlight, balloon and parachuting frequencies, then those of the \
                     airstrips, gliding sites and heliports in range, replacing one for the \
                     same locator",
                )
                .clicked()
            {
                self.add_airfields();
            }
            ui.end_row();
            ui.label(self.airfield_source());
        });
    }

    /// The row that adds the marine channels near the shared locator.
    fn marine_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Marine within");
            ComboBox::from_id_salt("marine-radius")
                .selected_text(format!("{} km", self.marine_area.radius_km))
                .show_ui(ui, |ui| {
                    for km in repeaters::RADII_KM {
                        ui.selectable_value(
                            &mut self.marine_area.radius_km,
                            km,
                            format!("{km} km"),
                        );
                    }
                });
            ui.label("nearest");
            ui.add(
                egui::DragValue::new(&mut self.marine_area.max_channels)
                    .range(1..=airports::MAX_CHANNELS_LIMIT),
            )
            .on_hover_text("How many local channels the bank takes, nearest place first");
            if ui
                .button("Add")
                .on_hover_text(
                    "Adds one FM bank: 16, 67, 6, 8, 72, 77, M and M2, then the channels the \
                     harbours, marinas, locks and lookouts in range list on OpenStreetMap, \
                     each named after its nearest place, replacing one for the same locator. \
                     A duplex channel keeps both frequencies; without a marine TX range the \
                     radio listens to both sides",
                )
                .clicked()
            {
                self.add_marine();
            }
            ui.end_row();
            ui.label(self.marine_source());
        });
    }

    /// Add group, and the buttons that file banks by locator, airband and
    /// marine.
    fn group_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Group by locator")
                .on_hover_text(
                    "Files every bank named for a locator, as the generators name \
                     them (RPT JO02EH), under a location for that locator",
                )
                .clicked()
            {
                let filed = self.project.group_by_locator();
                self.status = format!("Filed {filed} banks under their locations.");
            }
            if ui
                .button("Group airband")
                .on_hover_text(
                    "Files every bank whose channels are all AM in the VHF airband \
                     under AIRBAND",
                )
                .clicked()
            {
                let filed = self.project.group_airband();
                self.status = format!("Filed {filed} airband banks under AIRBAND.");
            }
            if ui
                .button("Group marine")
                .on_hover_text(
                    "Files every bank whose channels are all FM in the marine VHF \
                     band under MARINE",
                )
                .clicked()
            {
                let filed = self.project.group_marine();
                self.status = format!("Filed {filed} marine banks under MARINE.");
            }
            if ui
                .button("Add common group")
                .on_hover_text(
                    "Adds COMMON: a bank in it, such as PMR446, is in every group on the \
                     radio, wherever you are",
                )
                .clicked()
            {
                self.status = if self.project.ensure_group(COMMON_GROUP, None).is_some() {
                    format!(
                        "{COMMON_GROUP} is a group: tick a bank into it and the radio has it in \
                         every group."
                    )
                } else {
                    format!("A radio holds {MAX_GROUPS} groups.")
                };
            }
        });
    }

    /// The Banks tab's row of adds and how the list is shown.
    fn banks_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let kinds = [
                ("Add channel bank", BankKind::Named),
                ("Add generated plan", BankKind::Generated),
            ];
            for (label, kind) in kinds {
                if ui.button(label).clicked() {
                    self.add_bank(kind);
                    // Added in a group's tab, the bank joins that group.
                    if let BankView::Group(group) = self.banks_tab.view {
                        let row = self.project.banks.len() - 1;
                        self.project.set_member(row, group, true);
                    }
                }
            }
        });
    }

    /// One tab for every bank, one for each group, one for the banks in no
    /// group, and one that adds a group.
    fn bank_view_tabs(&mut self, ui: &mut egui::Ui) {
        if let BankView::Group(group) = self.banks_tab.view {
            if group >= self.project.groups.len() {
                self.banks_tab.view = BankView::All;
            }
        }
        ui.horizontal_wrapped(|ui| {
            let mut tabs = vec![(
                BankView::All,
                format!("All banks ({})", self.project.banks.len()),
            )];
            for (group, draft) in self.project.groups.iter().enumerate() {
                let view = BankView::Group(group);
                tabs.push((
                    view,
                    format!("{} ({})", draft.name, bank_rows(&self.project, view).len()),
                ));
            }
            let loose = bank_rows(&self.project, BankView::Loose).len();
            tabs.push((BankView::Loose, format!("Not in a group ({loose})")));
            for (view, label) in tabs {
                ui.selectable_value(&mut self.banks_tab.view, view, label);
            }
            if ui
                .button("+ Group")
                .on_hover_text("Adds a group the radio can narrow its list to, and opens it")
                .clicked()
            {
                let number = self.project.groups.len() + 1;
                match self.project.ensure_group(&format!("GROUP {number}"), None) {
                    Some(group) => self.banks_tab.view = BankView::Group(group),
                    None => self.refuse(&format!("A radio holds {MAX_GROUPS} groups.")),
                }
            }
        });
    }

    /// The open group's name, locator, members and Remove.
    fn group_editor(&mut self, ui: &mut egui::Ui, group: usize) {
        let mut remove = false;
        let mut membership = None;
        let members = bank_rows(&self.project, BankView::Group(group));
        let common = self.project.groups[group]
            .name
            .eq_ignore_ascii_case(COMMON_GROUP);
        ui.horizontal_wrapped(|ui| {
            let draft = &mut self.project.groups[group];
            ui.label("Name");
            ui.add(
                TextEdit::singleline(&mut draft.name)
                    .char_limit(16)
                    .desired_width(130.0),
            )
            .on_hover_text("The name the radio shows, up to 16 characters");
            ui.label("Locator");
            ui.add(
                TextEdit::singleline(&mut draft.locator)
                    .char_limit(8)
                    .hint_text("none")
                    .desired_width(70.0),
            )
            .on_hover_text(
                "A locator makes the group a location: the generators file the banks they \
                 make there under it. It stays in the project and is never sent to the radio",
            );
            ui.menu_button(format!("Banks ({})", members.len()), |ui| {
                ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for (bank, draft) in self.project.banks.iter().enumerate() {
                        let mut member = members.contains(&bank);
                        if ui.checkbox(&mut member, &draft.name).changed() {
                            membership = Some((bank, member));
                        }
                    }
                });
            })
            .response
            .on_hover_text("Ticks which banks are in the group");
            if ui
                .button("Remove group")
                .on_hover_text("Its banks stay")
                .clicked()
            {
                remove = true;
            }
        });
        if common {
            ui.label("Every bank here is in every group on the radio.");
        }
        if let Some((bank, member)) = membership {
            self.project.set_member(bank, group, member);
        }
        if remove {
            self.project.remove_group(group);
            self.banks_tab.view = BankView::All;
        }
    }

    /// Does what a bank row asked for.
    fn apply_bank_action(&mut self, row: usize, action: BankAction) {
        match action {
            BankAction::MoveTo(slot) => self.project.move_bank_to(row, slot),
            BankAction::Remove => {
                self.project.banks.remove(row);
            }
            BankAction::AddChannel => self.project.add_channel(row),
            BankAction::MakeChannels => {
                let name = self.project.banks.get(row).map(|bank| bank.name.clone());
                self.status = match (self.project.make_channels_editable(row), name) {
                    (Ok(count), Some(name)) => format!(
                        "{name} is now a channel bank of {count} channels; each \
                         channel has its own tones."
                    ),
                    (Ok(_), None) => String::new(),
                    (Err(reason), name) => {
                        format!("{} stays a plan: {reason}.", name.unwrap_or_default())
                    }
                };
            }
            BankAction::Channel(channel, RowAction::Duplicate) => {
                self.project.duplicate_channel(row, channel);
            }
            BankAction::Channel(channel, RowAction::Remove) => {
                self.project.banks[row].channels.remove(channel);
            }
        }
    }

    fn banks_tab(&mut self, ui: &mut egui::Ui) {
        self.default_set_bar(ui);
        self.banks_toolbar(ui);
        self.bank_view_tabs(ui);
        ui.separator();
        match self.banks_tab.view {
            BankView::All => self.group_buttons(ui),
            BankView::Group(group) => self.group_editor(ui, group),
            BankView::Loose => {}
        }
        ui.separator();
        // Only every bank at once can be dragged into a new order.
        let grouped = self.banks_tab.view != BankView::All;
        let rows = bank_rows(&self.project, self.banks_tab.view);

        let mut action = None;
        let mut toggle = None;
        let mut ids = Vec::with_capacity(self.project.banks.len());
        let expand = self.banks_tab.expand.take();
        let headers: Vec<_> = self
            .project
            .banks
            .iter()
            .enumerate()
            .map(|(row, bank)| bank_row_columns(row, bank))
            .collect();
        let widths = column_widths(ui, &headers);
        let buttons_width = button_width(ui, "Delete") + button_width(ui, DRAG_HANDLE);
        let banks = &mut self.project.banks;
        // One bank's header and editor; only the list of every bank can be
        // dragged.
        let mut show_bank = |ui: &mut egui::Ui, row: usize| {
            ui.push_id(row, |ui| {
                // Banks are keyed by row, not by their label: the label shows
                // what is being edited, and a key that changed with it would
                // drop focus after one keystroke.
                let id = ui.make_persistent_id("bank");
                if !grouped {
                    ids.push(id);
                }
                let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
                if expand == Some(row) {
                    state.set_open(true);
                }
                state
                    .show_header(ui, |ui| {
                        if !grouped {
                            bank_drag_handle(ui, id.with("drag"), row, &headers[row][0]);
                        }
                        let gap = column_gap(ui.available_width() - buttons_width, &widths);
                        for (text, width) in headers[row].iter().zip(widths) {
                            let label = egui::Label::new(RichText::new(text).strong())
                                .sense(egui::Sense::click());
                            let response = ui.add(label);
                            if response.clicked() {
                                toggle = Some(id);
                            }
                            ui.add_space((width - response.rect.width()).max(0.0) + gap);
                        }
                        // The header runs the full width, its buttons at the right.
                        if let Some(requested) = bank_header_buttons(ui) {
                            action = Some((row, requested));
                        }
                    })
                    .body(|ui| {
                        if let Some(requested) = bank_row_editor(ui, &mut banks[row]) {
                            action = Some((row, requested));
                        }
                    });
                ui.separator();
            });
        };
        let mut dropped = None;
        ScrollArea::vertical().show(ui, |ui| {
            // Each bank row's top and bottom, for where a dragged one lands.
            let mut spans = Vec::with_capacity(headers.len());
            for &row in &rows {
                let top = ui.cursor().top();
                show_bank(ui, row);
                spans.push((top, ui.cursor().top()));
            }
            if !grouped {
                dropped = drop_bank(ui, &spans);
            }
        });
        if dropped.is_some() {
            action = dropped;
        }
        let context = ui.ctx().clone();
        if let Some(id) = toggle {
            let mut state = CollapsingState::load_with_default_open(&context, id, false);
            state.toggle(ui);
            state.store(&context);
        }
        if let Some((row, change)) = action {
            if !grouped {
                keep_open_state(&context, &ids, row, change);
            }
            self.apply_bank_action(row, change);
        }
    }

    fn radio_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for section in RadioSection::ALL {
                ui.selectable_value(&mut self.radio_section, section, section.label());
            }
        });
        ui.separator();
        match self.radio_section {
            RadioSection::Receive => self.receive_settings(ui),
            RadioSection::Transmit => {
                self.transmit_settings(ui);
                ui.separator();
                self.tx_ranges_editor(ui);
            }
            RadioSection::Vfo => self.vfo_settings(ui),
            RadioSection::Scan => self.scan_settings(ui),
            RadioSection::General => self.general_settings(ui),
        }
    }

    /// Each slot's column: where it starts, then its VFO.
    fn vfo_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(VFO_HINT);
        ui.add_space(4.0);
        let groups: Vec<String> = self.project.groups.iter().map(|g| g.name.clone()).collect();
        let ProjectModel {
            banks,
            starts,
            vfos,
            ..
        } = &mut self.project;
        ui.columns(2, |columns| {
            for (slot, ((ui, start), vfo)) in columns.iter_mut().zip(starts).zip(vfos).enumerate() {
                start_slot_editor(ui, slot, start, &groups, banks);
                ui.separator();
                vfo_slot_editor(ui, slot, vfo);
            }
        });
    }

    fn receive_settings(&mut self, ui: &mut egui::Ui) {
        let config = &mut self.project.config;
        Grid::new("radio-receive").num_columns(2).show(ui, |ui| {
            ui.label("Squelch level").on_hover_text(SQUELCH_HINT);
            ui.add(egui::DragValue::new(&mut config.squelch).range(0..=MAX_SQUELCH_LEVEL))
                .on_hover_text(SQUELCH_HINT);
            ui.end_row();

            ui.label("Options");
            ui.vertical(|ui| {
                ui.checkbox(&mut config.dual_watch, "Dual watch");
                ui.checkbox(&mut config.am_fix, "AM gain compensation");
                ui.checkbox(&mut config.tone_tail_elimination, "Tone tail elimination");
            });
            ui.end_row();
        });
    }

    fn transmit_settings(&mut self, ui: &mut egui::Ui) {
        let config = &mut self.project.config;
        Grid::new("radio-transmit").num_columns(2).show(ui, |ui| {
            ui.label("Default TX power").on_hover_text(TX_HINT);
            ComboBox::from_id_salt("tx-power")
                .selected_text(tx_power_label(config.tx_power))
                .show_ui(ui, |ui| {
                    for level in 0..TX_POWER_LEVELS {
                        ui.selectable_value(&mut config.tx_power, level, tx_power_label(level));
                    }
                });
            ui.end_row();

            ui.label("TX timeout").on_hover_text(TX_HINT);
            ComboBox::from_id_salt("tx-timeout")
                .selected_text(tx_timeout_label(config.tx_timeout))
                .show_ui(ui, |ui| {
                    for choice in 0..TX_TIMEOUT_CHOICES {
                        ui.selectable_value(
                            &mut config.tx_timeout,
                            choice,
                            tx_timeout_label(choice),
                        );
                    }
                });
            ui.end_row();

            ui.label("Options");
            ui.checkbox(&mut config.busy_lockout_default, "Busy lockout by default");
            ui.end_row();
        });
    }

    fn scan_settings(&mut self, ui: &mut egui::Ui) {
        let config = &mut self.project.config;
        Grid::new("radio-scan").num_columns(2).show(ui, |ui| {
            ui.label("Scan resume");
            ComboBox::from_id_salt("scan-resume")
                .selected_text(scan_resume_label(config.scan_resume))
                .show_ui(ui, |ui| {
                    for resume in [ScanResume::TimeOut, ScanResume::Carrier, ScanResume::Stop] {
                        ui.selectable_value(
                            &mut config.scan_resume,
                            resume,
                            scan_resume_label(resume),
                        );
                    }
                });
            ui.end_row();

            ui.label("Scan dwell ms");
            ui.add(egui::DragValue::new(&mut config.scan_dwell_ms).range(1..=60_000));
            ui.end_row();

            ui.label("Scan hold ms");
            ui.add(egui::DragValue::new(&mut config.scan_hold_ms).range(1..=600_000));
            ui.end_row();
        });
    }

    fn general_settings(&mut self, ui: &mut egui::Ui) {
        let config = &mut self.project.config;
        Grid::new("radio-general").num_columns(2).show(ui, |ui| {
            ui.label("Backlight seconds").on_hover_text(BACKLIGHT_HINT);
            ui.add(
                egui::DragValue::new(&mut config.backlight_seconds).range(0..=BACKLIGHT_ALWAYS_ON),
            )
            .on_hover_text(BACKLIGHT_HINT);
            ui.end_row();

            ui.label("Battery save ratio")
                .on_hover_text(BATTERY_SAVE_HINT);
            ui.add(
                egui::DragValue::new(&mut config.battery_save_ratio)
                    .range(0..=MAX_BATTERY_SAVE_RATIO)
                    .prefix("1:"),
            )
            .on_hover_text(BATTERY_SAVE_HINT);
            ui.end_row();
        });
    }

    fn tx_ranges_editor(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("TX ranges").strong())
            .on_hover_text(TX_RANGES_HINT);
        ui.horizontal_wrapped(|ui| {
            let selected = TX_RANGE_PRESETS
                .get(self.tx_preset)
                .map_or("none", |preset| preset.name());
            ComboBox::from_id_salt("tx-preset")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (index, preset) in TX_RANGE_PRESETS.iter().enumerate() {
                        ui.selectable_value(&mut self.tx_preset, index, preset.name())
                            .on_hover_text(preset.detail());
                    }
                });
            if ui
                .button("Add preset")
                .on_hover_text("Adds the preset's ranges after the existing ones")
                .clicked()
            {
                self.add_tx_preset();
            }
            if ui.button("Add range").clicked() && self.project.tx_ranges.len() < MAX_TX_RANGES {
                self.project.tx_ranges.push(TxRangeDraft::default());
            }
            if let Some(preset) = TX_RANGE_PRESETS.get(self.tx_preset) {
                ui.label(preset.detail());
            }
        });
        if self.project.tx_ranges.is_empty() {
            ui.label("No TX range: this radio will never transmit.");
            return;
        }
        let mut remove = None;
        Grid::new("tx-ranges")
            .num_columns(7)
            .striped(true)
            .show(ui, |ui| {
                for heading in [
                    "Class",
                    "First MHz",
                    "Last MHz",
                    "Max power",
                    "Wide",
                    "Raster Hz",
                    "",
                ] {
                    ui.label(RichText::new(heading).strong());
                }
                ui.end_row();
                for (row, range) in self.project.tx_ranges.iter_mut().enumerate() {
                    ui.push_id(row, |ui| {
                        tx_class_editor(ui, &mut range.class);
                    });
                    ui.add(TextEdit::singleline(&mut range.first_mhz).desired_width(90.0));
                    ui.add(TextEdit::singleline(&mut range.last_mhz).desired_width(90.0));
                    ui.add(
                        egui::DragValue::new(&mut range.max_mw)
                            .range(1..=u16::MAX)
                            .suffix(" mW"),
                    );
                    ui.checkbox(&mut range.wide_fm, "");
                    ui.add(egui::DragValue::new(&mut range.raster_hz))
                        .on_hover_text("Channel spacing from the first frequency; 0 allows any");
                    if ui.button("Delete").clicked() {
                        remove = Some(row);
                    }
                    ui.end_row();
                }
            });
        if let Some(row) = remove {
            self.project.tx_ranges.remove(row);
        }
    }

    fn device_tab(&mut self, ui: &mut egui::Ui) {
        self.device_picker(ui);
        ui.separator();

        if let Some(job) = &self.job {
            ui.horizontal(|ui| {
                ui.spinner();
                match (job.kind(), job.progress()) {
                    (JobKind::Write, (done, total)) if total > 0 => {
                        ui.add(
                            egui::ProgressBar::new(f32::from(done) / f32::from(total))
                                .desired_width(240.0)
                                .text(format!("{done} of {total} blocks")),
                        );
                        ui.label(format!("Writing to {}", job.description()));
                    }
                    (JobKind::Write, _) => {
                        ui.label(format!("Comparing with what {} holds", job.description()));
                    }
                    (JobKind::Read, _) => {
                        ui.label(format!("Reading from {}", job.description()));
                    }
                    (JobKind::LinkTest, _) => {
                        ui.label(format!("Testing the link to {}", job.description()));
                    }
                }
            });
        } else if let Some(session) = self.session.as_ref() {
            ui.label(format!(
                "Connected to {}, running {}.",
                session.description(),
                session.firmware()
            ));
        } else {
            ui.label("No radio is connected.");
        }
        ui.label(
            "The project is compiled into the image the radio runs, written into its \
             memory and read back. The radio checks it again before it uses it.",
        );

        let idle = self.job.is_none();
        ui.add_enabled_ui(idle && self.session.is_some(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Read project from radio").clicked() {
                    self.read_project();
                }
                if ui.button("Write project to radio").clicked() {
                    self.write_project();
                }
                if ui
                    .button("Test link")
                    .on_hover_text(
                        "Sends sixteen reads back to back, each once, and reports how many \
                         the radio answered. Nothing is written.",
                    )
                    .clicked()
                {
                    self.test_link();
                }
            });
        });

        if let Some(report) = &self.last_write {
            ui.label(format!(
                "Last verified write: {} bytes, {} blocks changed, on {}.",
                report.image_bytes,
                report.changed_blocks,
                report.firmware.version()
            ));
        }
    }

    /// Draws detection, selection, and the connection controls.
    fn device_picker(&mut self, ui: &mut egui::Ui) {
        // The browser lists its ports itself, when the page connects.
        let browser = cfg!(target_arch = "wasm32");
        ui.horizontal_wrapped(|ui| {
            if !browser && ui.button("Detect devices").clicked() {
                self.status = self.chooser.detect();
            }
            let idle = self.job.is_none() && !self.is_connecting();
            if ui.add_enabled(idle, egui::Button::new("Connect")).clicked() {
                self.connect_serial();
            }
            ui.add_enabled_ui(idle && self.session.is_some(), |ui| {
                if ui.button("Disconnect").clicked() {
                    self.disconnect();
                }
            });
            ui.separator();
            if ui
                .add_enabled(idle, egui::Button::new("Connect simulator"))
                .clicked()
            {
                self.connect_simulator();
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Baud:");
            ComboBox::from_id_salt("device-baud")
                .selected_text(self.chooser.baud.to_string())
                .width(90.0)
                .show_ui(ui, |ui| {
                    for baud in SUPPORTED_BAUDS {
                        ui.selectable_value(&mut self.chooser.baud, baud, baud.to_string());
                    }
                });
        });

        if browser {
            ui.label(
                "Connect asks the browser for the radio's serial port; \
                 Chrome, Edge and recent Firefox offer one.",
            );
            return;
        }
        if !self.chooser.detected {
            ui.label("Serial devices have not been scanned for yet.");
        } else if self.chooser.candidates.is_empty() {
            ui.label("No USB serial device was detected.");
        }
        for (index, candidate) in self.chooser.candidates.iter().enumerate() {
            ui.radio_value(
                &mut self.chooser.choice,
                DeviceChoice::Detected(index),
                candidate.label(),
            )
            .on_hover_text(candidate.path.display().to_string());
        }
        // A manual path is always available: an unusual port must stay reachable.
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(&mut self.chooser.choice, DeviceChoice::Manual, "Path:");
            if ui
                .add(
                    TextEdit::singleline(&mut self.chooser.manual_path)
                        .hint_text("/dev/ttyUSB0")
                        .desired_width(260.0),
                )
                .changed()
            {
                self.chooser.choice = DeviceChoice::Manual;
            }
        });
    }
}

fn tone_editor(ui: &mut egui::Ui, id: &str, tone: &mut ToneDraft) {
    ui.horizontal(|ui| {
        let before = tone.kind;
        ComboBox::from_id_salt(id)
            .selected_text(tone.kind.label())
            .width(110.0)
            .show_ui(ui, |ui| {
                for kind in ToneKind::all() {
                    ui.selectable_value(&mut tone.kind, kind, kind.label());
                }
            });
        let values = tone.kind.standard_values();
        if tone.kind != before {
            // A CTCSS frequency means nothing as a DCS code, nor the
            // reverse, so a new kind starts on its first standard value.
            if !values.contains(&tone.value) {
                tone.value = values.first().cloned().unwrap_or_default();
            }
        }
        if values.is_empty() {
            return;
        }
        // Only the standard tones are offered, as on the handset; one an
        // older project holds outside them is kept and shown as such.
        let standard = values.contains(&tone.value);
        let shown = if standard || tone.value.is_empty() {
            tone.value.clone()
        } else {
            format!("{} (non-standard)", tone.value)
        };
        ComboBox::from_id_salt((id, "value"))
            .selected_text(shown)
            .width(70.0)
            .height(320.0)
            .show_ui(ui, |ui| {
                for value in values {
                    let label = value.clone();
                    ui.selectable_value(&mut tone.value, value, label);
                }
            });
    });
}

/// What one row asked the editor to do with itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowAction {
    /// Append a copy of this row after it.
    Duplicate,
    /// Delete this row.
    Remove,
}

/// The bank row being dragged by its handle.
struct BankDrag(usize);

/// Where a bank dropped at `pointer_y` lands: before the first bank whose
/// middle is below the pointer, or last. `spans` are each bank row's top and
/// bottom.
fn drop_slot(spans: &[(f32, f32)], pointer_y: f32) -> usize {
    spans
        .iter()
        .take_while(|(top, bottom)| pointer_y > (top + bottom) / 2.0)
        .count()
}

/// What one bank asked the editor to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BankAction {
    /// Move the bank to just before the bank now at this row, or last at the
    /// bank count: where it was dragged to.
    MoveTo(usize),
    /// Delete the bank and its channels.
    Remove,
    /// Append a channel to the bank.
    AddChannel,
    /// Turn a generated plan into a channel bank of the channels it expands to.
    MakeChannels,
    /// Act on one of the bank's channels.
    Channel(usize, RowAction),
}

/// Draws the grip bank `row` is dragged by; while dragged it carries the
/// bank's `name`.
fn bank_drag_handle(ui: &mut egui::Ui, id: egui::Id, row: usize, name: &str) {
    let dragged = ui.ctx().is_being_dragged(id);
    ui.dnd_drag_source(id, BankDrag(row), |ui| {
        if dragged {
            ui.label(RichText::new(format!("{DRAG_HANDLE} {name}")).strong());
        } else {
            ui.label(DRAG_HANDLE);
        }
    })
    .response
    .on_hover_text("Drag to move this bank");
}

/// The Banks tab's own state.
#[derive(Default)]
struct BanksTab {
    /// Which banks it lists.
    view: BankView,
    /// A bank just added, to open on the next frame.
    expand: Option<usize>,
}

/// Which banks the Banks tab lists.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum BankView {
    /// Every bank, in the radio's order, which can be dragged.
    #[default]
    All,
    /// The banks in one group, by its place in the project's list.
    Group(usize),
    /// The banks in no group.
    Loose,
}

/// The rows of the banks `view` lists, in the radio's order.
fn bank_rows(project: &ProjectModel, view: BankView) -> Vec<usize> {
    (0..project.banks.len())
        .filter(|row| {
            let groups = &project.banks[*row].groups;
            match view {
                BankView::All => true,
                BankView::Group(group) => groups.contains(&group),
                BankView::Loose => groups.is_empty(),
            }
        })
        .collect()
}

/// Shows where a dragged bank would land and, when it is released over the
/// bank list, the move it asks for. Escape cancels a drag.
fn drop_bank(ui: &egui::Ui, spans: &[(f32, f32)]) -> Option<(usize, BankAction)> {
    let dragged = egui::DragAndDrop::payload::<BankDrag>(ui.ctx())?;
    let pointer = ui.ctx().pointer_interact_pos()?;
    if !ui.clip_rect().contains(pointer) {
        return None;
    }
    let slot = drop_slot(spans, pointer.y);
    let y = spans.get(slot).map_or_else(
        || spans.last().map_or(pointer.y, |span| span.1),
        |span| span.0,
    );
    ui.painter().hline(
        ui.max_rect().x_range(),
        y,
        egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
    );
    if !ui.input(|input| input.pointer.any_released()) {
        return None;
    }
    egui::DragAndDrop::clear_payload(ui.ctx());
    Some((dragged.0, BankAction::MoveTo(slot)))
}

/// Keeps open and collapsed banks so when `change` on `row` moves the rows
/// whose headers are `ids`.
fn keep_open_state(context: &egui::Context, ids: &[egui::Id], row: usize, change: BankAction) {
    let mut open: Vec<bool> = ids
        .iter()
        .map(|id| CollapsingState::load_with_default_open(context, *id, false).is_open())
        .collect();
    reorder_open(&mut open, row, change);
    for (id, open) in ids.iter().zip(open) {
        let mut state = CollapsingState::load_with_default_open(context, *id, false);
        state.set_open(open);
        state.store(context);
    }
}

/// Moves the open flags of bank rows the way `action` on `row` moves the rows.
fn reorder_open(open: &mut [bool], row: usize, action: BankAction) {
    match action {
        BankAction::MoveTo(slot) => move_to(open, row, slot),
        BankAction::Remove if row < open.len() => {
            open[row..].rotate_left(1);
            if let Some(last) = open.last_mut() {
                *last = false;
            }
        }
        _ => {}
    }
}

/// Draws one channel row, returning the action its buttons requested.
/// Draws one channel; a setting its bank shares is shown as the bank's.
fn channel_row_editor(
    ui: &mut egui::Ui,
    channel: &mut ChannelDraft,
    shared: SharedSettings,
    bank_modulation: Modulation,
) -> Option<RowAction> {
    // Only an FM channel transmits; any other only receives, so its transmit
    // settings, tones and power are not shown, nor a broadcast one's
    // bandwidth.
    let modulation = if shared.contains(SharedSettings::MODULATION) {
        bank_modulation
    } else {
        channel.modulation
    };
    let receive_only = modulation.is_receive_only();
    let broadcast = modulation == Modulation::Broadcast;
    let own = |ui: &mut egui::Ui, setting: u8, editor: &mut dyn FnMut(&mut egui::Ui)| {
        if shared.contains(setting) {
            ui.weak("from bank");
        } else {
            editor(ui);
        }
    };
    Grid::new("channel").num_columns(4).show(ui, |ui| {
        ui.label("Name");
        ui.add(TextEdit::singleline(&mut channel.name).desired_width(120.0));
        ui.end_row();

        ui.label(if receive_only { "MHz" } else { "Receive MHz" });
        ui.add(TextEdit::singleline(&mut channel.receive_mhz).desired_width(120.0));
        if !receive_only {
            ui.label("Transmit MHz");
            own(ui, SharedSettings::OFFSET, &mut |ui| {
                ui.add(TextEdit::singleline(&mut channel.transmit_mhz).desired_width(120.0));
            });
        }
        ui.end_row();

        if !receive_only {
            ui.label("RX tone");
            own(ui, SharedSettings::RX_TONE, &mut |ui| {
                tone_editor(ui, "rx", &mut channel.rx_tone);
            });
            ui.label("TX tone");
            own(ui, SharedSettings::TX_TONE, &mut |ui| {
                tone_editor(ui, "tx", &mut channel.tx_tone);
            });
            ui.end_row();
        }

        ui.label("Modulation");
        own(ui, SharedSettings::MODULATION, &mut |ui| {
            modulation_editor(ui, &mut channel.modulation);
        });
        if !broadcast {
            ui.label("Bandwidth");
            own(ui, SharedSettings::BANDWIDTH, &mut |ui| {
                bandwidth_editor(ui, &mut channel.bandwidth);
            });
        }
        ui.end_row();

        if !receive_only {
            ui.label("Power");
            own(ui, SharedSettings::POWER, &mut |ui| {
                power_editor(ui, &mut channel.power);
            });
        }
        ui.label("Step Hz");
        own(ui, SharedSettings::STEP, &mut |ui| {
            ui.add(egui::DragValue::new(&mut channel.step_hz).speed(125.0));
        });
        ui.end_row();

        ui.label("Flags");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut channel.scan_skip, "Scan skip");
            ui.checkbox(&mut channel.calling, "Calling")
                .on_hover_text("The band's calling channel, which the radio can reach directly");
            if !receive_only {
                ui.checkbox(&mut channel.busy_lockout, "Busy lockout");
                ui.checkbox(&mut channel.reverse, "Reverse");
                ui.checkbox(&mut channel.compander, "Compander");
            }
        });
        ui.end_row();
    });
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui.button("Duplicate channel").clicked() {
            action = Some(RowAction::Duplicate);
        }
        if ui.button("Remove channel").clicked() {
            action = Some(RowAction::Remove);
        }
    });
    action
}

/// Draws a bank header's button at its right edge, so a collapsed bank can
/// be deleted; its handle moves it.
fn bank_header_buttons(ui: &mut egui::Ui) -> Option<BankAction> {
    let mut action = None;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .button("Delete")
            .on_hover_text("Delete this bank and every channel in it")
            .clicked()
        {
            action = Some(BankAction::Remove);
        }
    });
    action
}

/// Draws one bank and, for a channel bank, its channels.
fn bank_row_editor(ui: &mut egui::Ui, bank: &mut BankDraft) -> Option<BankAction> {
    let mut action = None;
    Grid::new("bank").num_columns(4).show(ui, |ui| {
        ui.label("Name");
        ui.add(TextEdit::singleline(&mut bank.name).desired_width(160.0));
        ui.label("Kind");
        bank_kind_editor(ui, &mut bank.kind);
        ui.end_row();

        ui.label("");
        ui.label(bank.kind.detail());
        ui.end_row();

        match bank.kind {
            BankKind::Named => {
                ui.label("Scanning");
                ui.checkbox(&mut bank.scan_enabled, "Scan enabled");
                ui.end_row();
                shared_settings_editor(ui, bank);
            }
            BankKind::Generated => generated_plan_editor(ui, bank),
        }
    });
    match bank.kind {
        BankKind::Generated => {
            generated_expansion(ui, bank);
            if ui
                .button("Make channels editable")
                .on_hover_text(
                    "Turns the plan into a channel bank of the channels it expands \
                     to, so each can have its own tones. Each channel then costs \
                     radio memory, and base, spacing and count are no longer a plan.",
                )
                .clicked()
            {
                action = Some(BankAction::MakeChannels);
            }
            if !bank.channels.is_empty() {
                ui.colored_label(
                    WARNING_COLOUR,
                    format!(
                        "{} channel rows are kept from when this was a channel bank; \
                         switch back to Channels, or remove them, to write it.",
                        bank.channels.len()
                    ),
                );
            }
        }
        BankKind::Named => {
            let shared = bank.shared;
            let bank_modulation = bank.modulation;
            ui.indent("channels", |ui| {
                for (row, channel) in bank.channels.iter_mut().enumerate() {
                    ui.push_id(row, |ui| {
                        egui::CollapsingHeader::new(channel_row_label(row, channel))
                            .id_salt(row)
                            .default_open(false)
                            .show(ui, |ui| {
                                if let Some(requested) =
                                    channel_row_editor(ui, channel, shared, bank_modulation)
                                {
                                    action = Some(BankAction::Channel(row, requested));
                                }
                            });
                    });
                }
                if ui.button("Add channel").clicked() {
                    action = Some(BankAction::AddChannel);
                }
            });
        }
    }
    action
}

/// Draws the settings a channel bank can share, each with its own switch.
///
/// A shared setting is entered here once and every channel takes it; one the
/// bank does not share is entered on each channel instead.
fn shared_settings_editor(ui: &mut egui::Ui, bank: &mut BankDraft) {
    ui.label("Shared");
    ui.weak("Ticked settings apply to every channel in the bank.");
    ui.end_row();

    if share_toggle(
        ui,
        &mut bank.shared,
        SharedSettings::MODULATION,
        "Modulation",
    ) {
        modulation_editor(ui, &mut bank.modulation);
    } else {
        ui.weak("per channel");
    }
    // A broadcast bank has one bandwidth, and a receive-only one no transmit
    // side; the rest would be noise.
    if bank.modulation != Modulation::Broadcast || !bank.shared.contains(SharedSettings::MODULATION)
    {
        if share_toggle(ui, &mut bank.shared, SharedSettings::BANDWIDTH, "Bandwidth") {
            bandwidth_editor(ui, &mut bank.bandwidth);
        } else {
            ui.weak("per channel");
        }
    }
    ui.end_row();
    if bank.is_receive_only() {
        if share_toggle(ui, &mut bank.shared, SharedSettings::STEP, "Step Hz") {
            ui.add(egui::DragValue::new(&mut bank.step_hz).speed(125.0));
        } else {
            ui.weak("per channel");
        }
        ui.end_row();
        return;
    }

    if share_toggle(ui, &mut bank.shared, SharedSettings::POWER, "Power") {
        power_editor(ui, &mut bank.power);
    } else {
        ui.weak("per channel");
    }
    if share_toggle(ui, &mut bank.shared, SharedSettings::STEP, "Step Hz") {
        ui.add(egui::DragValue::new(&mut bank.step_hz).speed(125.0));
    } else {
        ui.weak("per channel");
    }
    ui.end_row();

    if share_toggle(ui, &mut bank.shared, SharedSettings::OFFSET, "TX offset Hz") {
        ui.add(
            egui::DragValue::new(&mut bank.offset_hz)
                .speed(1_000.0)
                .range(-50_000_000..=50_000_000),
        )
        .on_hover_text("Zero is simplex; each channel transmits this far from where it receives.");
    } else {
        ui.weak("per channel");
    }
    ui.end_row();

    if share_toggle(ui, &mut bank.shared, SharedSettings::RX_TONE, "RX tone") {
        tone_editor(ui, "bank-rx", &mut bank.rx_tone);
    } else {
        ui.weak("per channel");
    }
    if share_toggle(ui, &mut bank.shared, SharedSettings::TX_TONE, "TX tone") {
        tone_editor(ui, "bank-tx", &mut bank.tx_tone);
    } else {
        ui.weak("per channel");
    }
    ui.end_row();
}

/// Draws one share switch and reports whether the setting is now shared.
fn share_toggle(ui: &mut egui::Ui, shared: &mut SharedSettings, setting: u8, label: &str) -> bool {
    let mut on = shared.contains(setting);
    ui.checkbox(&mut on, label);
    *shared = shared.with(setting, on);
    on
}

/// Draws the editor for one generated plan.
///
/// A plan carries far more than a named bank does — its arithmetic, its
/// numbering, its calling channel, and the template every expanded channel
/// shares — so it is drawn here rather than inline in the bank row.
fn generated_plan_editor(ui: &mut egui::Ui, bank: &mut BankDraft) {
    ui.label("Base MHz");
    ui.add(TextEdit::singleline(&mut bank.base_mhz).desired_width(120.0));
    ui.label("Spacing Hz");
    ui.add(
        egui::DragValue::new(&mut bank.spacing_hz)
            .speed(125.0)
            .range(1..=1_000_000),
    );
    ui.end_row();

    ui.label("Channels");
    ui.add(egui::DragValue::new(&mut bank.channel_count).range(1..=MAX_GENERATED_CHANNELS));
    ui.end_row();

    plan_naming_editor(ui, bank);
    let receive_only = bank.is_receive_only();
    let broadcast = bank.modulation == Modulation::Broadcast;

    if !receive_only {
        ui.label("TX offset Hz");
        ui.add(
            egui::DragValue::new(&mut bank.offset_hz)
                .speed(1_000.0)
                .range(-50_000_000..=50_000_000),
        )
        .on_hover_text(
            "Zero is simplex. A repeater bank transmits a fixed distance from \
                 where it receives, such as -600000 for UK 2 m repeaters.",
        );
        ui.label(if bank.offset_hz == 0 {
            "simplex".to_owned()
        } else {
            format!("repeater, {:+} kHz", bank.offset_hz / 1_000)
        });
        ui.end_row();
    }

    ui.label("Span");
    match bank.generated_span() {
        Some((first, last)) => ui.label(format!(
            "{first} to {last} MHz, {} channels",
            bank.channel_count
        )),
        None => ui.colored_label(WARNING_COLOUR, "the plan is incomplete or out of range"),
    };
    ui.end_row();

    // Every expanded channel shares these settings: they are stored
    // once with the plan, not once per channel. Only an FM plan transmits;
    // any other has no transmit settings, tones or power shown, nor a
    // broadcast plan its bandwidth.
    if !receive_only {
        ui.label("RX tone");
        tone_editor(ui, "plan-rx", &mut bank.rx_tone);
        ui.label("TX tone");
        tone_editor(ui, "plan-tx", &mut bank.tx_tone);
        ui.end_row();
    }

    ui.label("Modulation");
    modulation_editor(ui, &mut bank.modulation);
    if !broadcast {
        ui.label("Bandwidth");
        bandwidth_editor(ui, &mut bank.bandwidth);
    }
    ui.end_row();

    if !receive_only {
        ui.label("Power");
        power_editor(ui, &mut bank.power);
    }
    ui.label("Step Hz");
    ui.add(egui::DragValue::new(&mut bank.step_hz).speed(125.0));
    ui.end_row();

    ui.label("Flags");
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut bank.scan_skip, "Scan skip");
        if !receive_only {
            ui.checkbox(&mut bank.busy_lockout, "Busy lockout");
            ui.checkbox(&mut bank.compander, "Compander");
        }
    });
    ui.end_row();
}

/// Draws how a plan names and numbers the channels it expands.
///
/// The plan name is this editor's label for the bank; the designator and the
/// first number are what the radio puts on a channel, which is what an operator
/// matches against a published band plan.
fn plan_naming_editor(ui: &mut egui::Ui, bank: &mut BankDraft) {
    // The plan name above is this editor's label for the bank. The
    // designator is what the radio shows on a channel, which an
    // operator matches against a published band plan.
    ui.label("Designator");
    ui.add(
        TextEdit::singleline(&mut bank.designator)
            .hint_text("S")
            .desired_width(60.0),
    )
    .on_hover_text(
        "Prefix the radio names channels with, up to four characters. \
                 A trailing space separates the number, as in \"PMR 1\". \
                 Left empty the radio derives one from the plan name.",
    );
    ui.label("First number");
    ui.add(egui::DragValue::new(&mut bank.first_number).range(0..=u16::MAX))
        .on_hover_text(
            "Number the first channel carries. UK 2 m simplex starts at 8, \
                     so the plan expands to S8 upwards.",
        );
    ui.end_row();

    // The checkbox, the channel and the name it gets share one cell, so
    // ticking the box puts the choice beside it rather than a column away.
    ui.label("Calling channel");
    ui.horizontal(|ui| {
        let mut has_calling = bank.calling_index.is_some();
        if ui
            .checkbox(&mut has_calling, "Mark one")
            .on_hover_text(
                "Mark one channel as the band's calling channel. It is named \
                 for its purpose as well as its number and the radio can reach \
                 it directly.",
            )
            .changed()
        {
            bank.calling_index = has_calling.then_some(0);
        }
        if let Some(index) = bank.calling_index.as_mut() {
            // The operator picks the number the channel carries, as a band
            // plan names it, not its position in the plan.
            let (first, last) = calling_numbers(bank.first_number, bank.channel_count);
            let mut number = calling_number(bank.first_number, *index);
            ui.label("channel");
            if ui
                .add(egui::DragValue::new(&mut number).range(first..=last))
                .changed()
            {
                *index = calling_index_of(bank.first_number, number);
            }
            ui.label(calling_label(
                bank.designator.as_str(),
                bank.first_number,
                *index,
            ));
        }
    });
    ui.end_row();
}

/// Returns the first and last numbers a plan's channels carry.
fn calling_numbers(first_number: u16, channel_count: u16) -> (u32, u32) {
    let first = u32::from(first_number);
    (first, first + u32::from(channel_count.max(1)) - 1)
}

/// Returns the number the channel at `index` carries.
fn calling_number(first_number: u16, index: u16) -> u32 {
    u32::from(first_number) + u32::from(index)
}

/// Returns the plan index of the channel carrying `number`.
fn calling_index_of(first_number: u16, number: u32) -> u16 {
    u16::try_from(number.saturating_sub(u32::from(first_number))).unwrap_or(u16::MAX)
}

/// Returns the name the marked calling channel will carry on the radio.
///
/// The operator is choosing an index, so the editor shows the name that index
/// produces rather than leaving them to work it out.
fn calling_label(designator: &str, first_number: u16, index: u16) -> String {
    let number = first_number.saturating_add(index);
    if designator.trim().is_empty() {
        format!("channel {number}, named CALL")
    } else {
        format!("{designator}{number} CALL")
    }
}

/// Shows the channels one stored plan becomes on the radio.
///
/// The plan is the only thing written, so this is the operator's view of what
/// the radio will build from it: the same names, order, and frequencies the
/// channel list will show.
fn generated_expansion(ui: &mut egui::Ui, bank: &BankDraft) {
    let expansion = bank.expansion(EXPANSION_PREVIEW_ROWS);
    if expansion.is_empty() {
        ui.colored_label(
            WARNING_COLOUR,
            "this plan does not yet describe any channel the radio could expand",
        );
        return;
    }
    egui::CollapsingHeader::new(format!(
        "Expands to {} channels on the radio",
        bank.channel_count
    ))
    .default_open(false)
    .show(ui, |ui| {
        Grid::new("expansion").num_columns(2).show(ui, |ui| {
            for (name, receive) in &expansion {
                ui.label(name);
                ui.label(format!("{receive} MHz"));
                ui.end_row();
            }
        });
        let shown = u16::try_from(expansion.len()).unwrap_or(u16::MAX);
        if bank.channel_count > shown {
            ui.label(format!(
                "{} further channels follow the same plan.",
                bank.channel_count - shown
            ));
        }
    });
}

/// Shows what the project gives and costs a radio, on every tab: its banks
/// and channels, and how much of the radio's memory the image takes, split
/// by what the banks are for. Kept short so it fits one line; the detail is
/// on hover.
fn usage_bar(ui: &mut egui::Ui, summary: &StorageSummary, roles: Option<[usize; 3]>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "{} banks · {} ch",
            summary.banks,
            summary.selectable_channels()
        ))
        .on_hover_text(format!(
            "{} stored, {} from {} plans",
            summary.stored_channels, summary.expanded_channels, summary.plans,
        ));
        ui.separator();
        let (Some(bytes), Some(per_mille)) = (summary.image_bytes, summary.used_per_mille()) else {
            ui.colored_label(
                WARNING_COLOUR,
                "Memory: the project does not compile into a radio image yet; \
                 Save or Write lists what to fix.",
            );
            return;
        };
        let over = bytes > RADIO_IMAGE_BYTES;
        let roles = roles.unwrap_or_default();
        let mut detail = format!("{bytes} of {RADIO_IMAGE_BYTES} bytes");
        for (role, role_bytes) in BankRole::ALL.into_iter().zip(roles) {
            let _ = write!(detail, "\n{}: {role_bytes} bytes", role.label());
        }
        let shared = bytes.saturating_sub(roles.iter().sum());
        let _ = write!(detail, "\nGroups and settings: {shared} bytes");
        memory_segments(ui, bytes, roles, over).on_hover_text(detail);
        let text = if over {
            format!(
                "{} · {} B over: remove banks before Write",
                memory_percent(per_mille),
                bytes - RADIO_IMAGE_BYTES
            )
        } else {
            format!(
                "{} · {} free",
                memory_percent(per_mille),
                kilobytes(RADIO_IMAGE_BYTES - bytes)
            )
        };
        if over {
            ui.colored_label(WARNING_COLOUR, text);
        } else {
            ui.label(text);
        }
        for (role, role_bytes) in BankRole::ALL.into_iter().zip(roles) {
            if role_bytes > 0 {
                swatch(ui, role_colour(role));
                ui.label(format!("{} {}", role.label(), kilobytes(role_bytes)));
            }
        }
    });
}

/// The width of the memory bar.
const MEMORY_BAR_WIDTH: f32 = 220.0;

/// Draws the radio's memory as one bar, each role's banks in its colour
/// and the groups and settings after them; a project past the memory fills
/// the bar to scale and is outlined in the warning colour.
fn memory_segments(
    ui: &mut egui::Ui,
    bytes: usize,
    roles: [usize; 3],
    over: bool,
) -> egui::Response {
    let height = ui.spacing().interact_size.y * 0.6;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(MEMORY_BAR_WIDTH, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    // Square segments sit in it, so the bar is barely rounded.
    let rounding = 2.0_f32;
    painter.rect_filled(rect, rounding, visuals.extreme_bg_color);
    let whole = bytes.max(RADIO_IMAGE_BYTES);
    let shared = bytes.saturating_sub(roles.iter().sum());
    let mut left = rect.left();
    let segments = BankRole::ALL
        .into_iter()
        .zip(roles)
        .map(|(role, role_bytes)| (role_colour(role), role_bytes))
        .chain([(visuals.widgets.inactive.bg_fill, shared)]);
    for (colour, segment_bytes) in segments {
        let per_mille = u16::try_from(segment_bytes * 1_000 / whole).unwrap_or(1_000);
        let width = MEMORY_BAR_WIDTH * f32::from(per_mille) / 1_000.0;
        let segment = egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2((left + width).min(rect.right()), rect.bottom()),
        );
        painter.rect_filled(segment, 0.0, colour);
        left = segment.right();
    }
    let outline = if over {
        egui::Stroke::new(2.0_f32, WARNING_COLOUR)
    } else {
        egui::Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color)
    };
    painter.rect_stroke(
        rect.shrink(0.5),
        rounding,
        outline,
        egui::StrokeKind::Middle,
    );
    response
}

/// The colour a role's banks are drawn in on the memory bar.
const fn role_colour(role: BankRole) -> egui::Color32 {
    match role {
        BankRole::Location => egui::Color32::from_rgb(0x26, 0xA2, 0x69),
        BankRole::DefaultSet => egui::Color32::from_rgb(0x35, 0x84, 0xE4),
        BankRole::Other => egui::Color32::from_rgb(0x91, 0x41, 0xAC),
    }
}

/// A small square of `colour`, keying the memory bar.
fn swatch(ui: &mut egui::Ui, colour: egui::Color32) {
    let size = ui.spacing().interact_size.y * 0.45;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, colour);
}

/// How far off `receiver` hears each chosen size of airport's approach,
/// labelled, for the map's faint rings; a large airport's is the radius,
/// drawn already.
fn approach_rings(options: &AirportOptions, receiver: Receiver) -> Vec<(f64, String)> {
    AirportSize::CHOSEN
        .into_iter()
        .zip(options.approach_km)
        .map(|(size, km)| {
            let km = receiver.reach_km(km);
            (f64::from(km), format!("{} APP {km} km", size.label()))
        })
        .collect()
}

/// How far off `receiver` hears a standard and a local repeater, labelled,
/// for the map's faint rings; a wide-area one's is the radius, drawn
/// already.
fn repeater_rings(reach: &RepeaterReach, receiver: Receiver) -> Vec<(f64, String)> {
    Coverage::CHOSEN
        .into_iter()
        .zip(reach.reach_km)
        .map(|(coverage, km)| {
            let km = receiver.reach_km(km);
            (f64::from(km), format!("{} RPT {km} km", coverage.label()))
        })
        .collect()
}

/// The hover text of an airport service: what it is and how far off
/// `receiver` hears it, approach by the size of the airport.
fn heard_to(
    service: Service,
    help: &str,
    options: &AirportOptions,
    receiver: Receiver,
    radius_km: u16,
) -> String {
    let on = receiver.label().to_lowercase();
    if service == Service::Approach {
        let [medium, small] = options.approach_km.map(|km| receiver.reach_km(km));
        format!(
            "{help}; a large airport's is taken across the airport radius, {radius_km} km; on \
             a {on}, a regional airport's or military field's is heard to about {medium} km \
             and a small field's {small} km"
        )
    } else {
        format!(
            "{help}; heard to about {} km on a {on}",
            service.reach_km(receiver)
        )
    }
}

/// Formats bytes as kilobytes to a tenth, such as `12.3 kB`.
fn kilobytes(bytes: usize) -> String {
    let tenths = (bytes + 50) / 100;
    format!("{}.{} kB", tenths / 10, tenths % 10)
}

/// Formats tenths of a percent, such as `12.5%`.
fn memory_percent(per_mille: usize) -> String {
    format!("{}.{}%", per_mille / 10, per_mille % 10)
}

fn bank_kind_editor(ui: &mut egui::Ui, kind: &mut BankKind) {
    ComboBox::from_id_salt("bank-kind")
        .selected_text(kind.label())
        .width(160.0)
        .show_ui(ui, |ui| {
            for candidate in BankKind::all() {
                ui.selectable_value(kind, candidate, candidate.label());
            }
        });
}

fn modulation_editor(ui: &mut egui::Ui, modulation: &mut Modulation) {
    ComboBox::from_id_salt("modulation")
        .selected_text(match modulation {
            Modulation::Fm => "FM",
            Modulation::Am => "AM",
            Modulation::Usb => "USB",
            Modulation::Broadcast => "Broadcast FM",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(modulation, Modulation::Fm, "FM");
            ui.selectable_value(modulation, Modulation::Am, "AM");
            ui.selectable_value(modulation, Modulation::Usb, "USB");
            ui.selectable_value(modulation, Modulation::Broadcast, "Broadcast FM");
        });
}

fn bandwidth_editor(ui: &mut egui::Ui, bandwidth: &mut Bandwidth) {
    ComboBox::from_id_salt("bandwidth")
        .selected_text(match bandwidth {
            Bandwidth::Narrow => "Narrow",
            Bandwidth::Wide => "Wide",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(bandwidth, Bandwidth::Narrow, "Narrow");
            ui.selectable_value(bandwidth, Bandwidth::Wide, "Wide");
        });
}

fn tx_class_editor(ui: &mut egui::Ui, class: &mut TxClass) {
    ComboBox::from_id_salt("tx-class")
        .selected_text(tx_class_label(*class))
        .show_ui(ui, |ui| {
            for candidate in TX_CLASSES {
                ui.selectable_value(class, candidate, tx_class_label(candidate));
            }
        });
}

const fn tx_class_label(class: TxClass) -> &'static str {
    match class {
        TxClass::LicenceFreePlan => "Licence free",
        TxClass::Amateur => "Amateur",
        TxClass::Marine => "Marine",
        TxClass::Aeronautical => "Aeronautical",
        TxClass::Business => "Business",
        TxClass::Experimental => "Experimental",
        TxClass::CitizensBand => "CB",
    }
}

/// One TX power level as the handset lists it: its nominal output.
/// Where one slot starts (`ADR-116`): a switch, then the group, the bank and
/// the channel in it. The radio has one group, so both slots name the same.
fn start_slot_editor(
    ui: &mut egui::Ui,
    slot: usize,
    start: &mut Option<StartDraft>,
    groups: &[String],
    banks: &[BankDraft],
) {
    let letter = if slot == 0 { 'A' } else { 'B' };
    let mut on = start.is_some();
    if ui
        .checkbox(&mut on, format!("Start {letter} on a channel"))
        .on_hover_text(
            "Writes where this slot starts with the image: it goes there now and after \
             a power cycle, until you move it",
        )
        .changed()
    {
        *start = on.then(StartDraft::default);
    }
    let Some(start) = start else {
        ui.weak(if slot == 0 {
            "Starts on every channel."
        } else {
            "Starts on its VFO."
        });
        return;
    };
    Grid::new(("radio-start", slot))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Group");
            ComboBox::from_id_salt(("start-group", slot))
                .selected_text(if start.group.is_empty() {
                    "every bank".to_owned()
                } else {
                    start.group.clone()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut start.group, String::new(), "every bank");
                    for name in groups {
                        ui.selectable_value(&mut start.group, name.clone(), name);
                    }
                });
            ui.end_row();

            let group = groups
                .iter()
                .position(|name| name.eq_ignore_ascii_case(&start.group));
            ui.label("Bank");
            ComboBox::from_id_salt(("start-bank", slot))
                .selected_text(if start.bank.is_empty() {
                    "all channels".to_owned()
                } else {
                    start.bank.clone()
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut start.bank, String::new(), "all channels");
                    for bank in banks {
                        if group.is_none_or(|group| bank.groups.contains(&group)) {
                            ui.selectable_value(&mut start.bank, bank.name.clone(), &bank.name);
                        }
                    }
                });
            ui.end_row();

            let bank = banks
                .iter()
                .find(|bank| bank.name.trim() == start.bank.trim());
            if let Some(bank) = bank {
                ui.label("Channel");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut start.channel).range(0..=u16::MAX))
                        .on_hover_text("The channel in that bank, from 1; 0 is its first");
                    // The name the radio shows, so the number reads as a
                    // channel; zero is the first, as the radio takes it.
                    let position = usize::from(start.channel.saturating_sub(1));
                    match bank.channel_name(position) {
                        Some(name) => ui.label(name),
                        None => ui.colored_label(WARNING_COLOUR, "past the bank's end"),
                    };
                });
                ui.end_row();
            }
        });
}

/// One slot's VFO on the Radio tab: a switch to program it, then its
/// frequency, step, tones and power.
fn vfo_slot_editor(ui: &mut egui::Ui, slot: usize, vfo: &mut Option<VfoDraft>) {
    let letter = if slot == 0 { 'A' } else { 'B' };
    let mut programmed = vfo.is_some();
    if ui
        .checkbox(&mut programmed, format!("Program VFO {letter}"))
        .changed()
    {
        *vfo = programmed.then(VfoDraft::default);
    }
    let Some(vfo) = vfo else {
        ui.weak("The radio keeps its own.");
        return;
    };
    Grid::new(("radio-vfo", slot))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Frequency MHz");
            ui.add(egui::TextEdit::singleline(&mut vfo.mhz).desired_width(90.0));
            ui.end_row();

            ui.label("Step");
            ComboBox::from_id_salt(("vfo-step", slot))
                .selected_text(vfo_step_label(vfo.step_hz))
                .show_ui(ui, |ui| {
                    for step in VFO_STEPS_HZ {
                        ui.selectable_value(&mut vfo.step_hz, step, vfo_step_label(step));
                    }
                });
            ui.end_row();

            for (label, salt, tone) in [
                ("RX tone", "vfo-rx-tone", &mut vfo.rx_tone),
                ("TX tone", "vfo-tx-tone", &mut vfo.tx_tone),
            ] {
                ui.label(label);
                ComboBox::from_id_salt((salt, slot))
                    .selected_text(vfo_tone_label(*tone))
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        for each in (0..TONE_ROWS).filter_map(tone_at_row) {
                            ui.selectable_value(tone, each, vfo_tone_label(each));
                        }
                    });
                ui.end_row();
            }

            ui.label("TX power").on_hover_text(TX_HINT);
            ComboBox::from_id_salt(("vfo-power", slot))
                .selected_text(
                    vfo.power
                        .map_or_else(|| "Default".to_owned(), |power| tx_power_label(power as u8)),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut vfo.power, None, "Default");
                    for level in 0..TX_POWER_LEVELS {
                        if let Ok(power) = PowerLevel::try_from(level) {
                            ui.selectable_value(&mut vfo.power, Some(power), tx_power_label(level));
                        }
                    }
                });
            ui.end_row();
        });
}

/// A VFO tone as the handset shows it, such as `88.5` or `D023N`, or `Off`.
fn vfo_tone_label(tone: radio_domain::Tone) -> String {
    if tone.is_some() {
        String::from_utf8_lossy(&tone_label(tone)).trim().to_owned()
    } else {
        "Off".to_owned()
    }
}

/// A VFO tuning step, such as `6.25 kHz` or `1 MHz`.
fn vfo_step_label(step_hz: u32) -> String {
    if step_hz >= 1_000_000 {
        format!("{} MHz", step_hz / 1_000_000)
    } else if step_hz % 1_000 == 0 {
        format!("{} kHz", step_hz / 1_000)
    } else {
        format!("{} kHz", f64::from(step_hz) / 1_000.0)
    }
}

fn tx_power_label(level: u8) -> String {
    TX_POWER_NOMINAL_MW.get(usize::from(level)).map_or_else(
        || format!("Level {level}"),
        |mw| {
            if *mw >= 1_000 {
                format!("{} W", mw / 1_000)
            } else {
                format!("{mw} mW")
            }
        },
    )
}

/// One TX timeout choice in whole minutes.
fn tx_timeout_label(choice: u8) -> String {
    match choice {
        0 => "1 minute".to_owned(),
        _ => format!("{} minutes", u32::from(choice) + 1),
    }
}

/// Chooses one of the radio's six power levels, labelled as the handset does.
fn power_editor(ui: &mut egui::Ui, power: &mut PowerLevel) {
    ComboBox::from_id_salt("power")
        .selected_text(tx_power_label(*power as u8))
        .show_ui(ui, |ui| {
            for level in 0..radio_domain::TX_POWER_LEVELS {
                if let Ok(choice) = PowerLevel::try_from(level) {
                    ui.selectable_value(power, choice, tx_power_label(level));
                }
            }
        });
}

const fn scan_resume_label(resume: ScanResume) -> &'static str {
    match resume {
        ScanResume::TimeOut => "Resume after hold",
        ScanResume::Carrier => "Resume when carrier drops",
        ScanResume::Stop => "Stop on signal",
    }
}

/// Returns the editor label for one channel row, used by tests and headers.
pub fn channel_row_label(row: usize, channel: &ChannelDraft) -> String {
    format!("{}: {} ({})", row + 1, channel.name, channel.receive_mhz)
}

/// Returns the columns of one bank row's header: number and name, channel
/// count, kind, and frequency span. A collapsed bank still shows them.
pub fn bank_row_columns(row: usize, bank: &BankDraft) -> [String; BANK_COLUMNS] {
    let channels = match bank.kind {
        BankKind::Named => bank.channels.len(),
        BankKind::Generated => usize::from(bank.channel_count),
    };
    let span = match bank.frequency_span() {
        Some((first, last)) if first == last => format!("{first} MHz"),
        Some((first, last)) => format!("{first} to {last} MHz"),
        None => String::new(),
    };
    [
        format!("{}: {}", row + 1, bank.name),
        format!("{channels} Ch"),
        bank.kind.label().to_owned(),
        span,
    ]
}

/// Spreads the header columns across the width left beside them, sharing
/// what is spare equally after each column, but never closer than
/// `MIN_COLUMN_GAP`.
// Four columns convert to `f32` exactly.
#[allow(clippy::cast_precision_loss)]
fn column_gap(available: f32, widths: &[f32; BANK_COLUMNS]) -> f32 {
    let spare = available - widths.iter().sum::<f32>();
    (spare / BANK_COLUMNS as f32).max(MIN_COLUMN_GAP)
}

/// Measures a button with the given text, with the spacing after it.
fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
            .size()
            .x
    });
    let spacing = ui.spacing();
    text + 2.0 * spacing.button_padding.x + spacing.item_spacing.x
}

/// Measures the widest text in each header column, so every bank's columns
/// start at the same tab stops.
fn column_widths(ui: &egui::Ui, rows: &[[String; BANK_COLUMNS]]) -> [f32; BANK_COLUMNS] {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut widths = [0.0_f32; BANK_COLUMNS];
    ui.fonts(|fonts| {
        for columns in rows {
            for (width, text) in widths.iter_mut().zip(columns) {
                let galley =
                    fonts.layout_no_wrap(text.clone(), font.clone(), egui::Color32::PLACEHOLDER);
                *width = width.max(galley.size().x);
            }
        }
    });
    widths
}

#[cfg(test)]
mod tests {
    use super::{
        bank_row_columns, channel_row_label, column_gap, memory_percent, scan_resume_label,
        tx_power_label, tx_timeout_label, StudioApp, Tab, MIN_COLUMN_GAP,
    };
    use super::{
        EtccKind, Generator, RadioSection, TxClass, TxRangeDraft, GENERATORS_TITLE, MAX_TX_RANGES,
    };
    use crate::{model::BankKind, projects::ProjectStore, DeviceSelector, Options};
    use radio_domain::{Modulation, ScanResume};
    use std::path::PathBuf;

    /// The calling channel is picked by the number it carries, so PMR 8 is
    /// eight, not the seventh index, and S20 is twenty on a plan from S8.
    #[test]
    fn the_calling_channel_is_picked_by_its_number() {
        use super::{calling_index_of, calling_label, calling_number, calling_numbers};
        assert_eq!(calling_numbers(1, 16), (1, 16));
        assert_eq!(calling_numbers(8, 16), (8, 23));
        assert_eq!(calling_index_of(1, 8), 7);
        assert_eq!(
            calling_label("PMR ", 1, calling_index_of(1, 8)),
            "PMR 8 CALL"
        );
        assert_eq!(calling_index_of(8, 20), 12);
        assert_eq!(calling_number(8, 12), 20);
        assert_eq!(calling_label("S", 8, 12), "S20 CALL");
    }

    /// The studio names each TX choice as the handset menu lists it.
    #[test]
    fn vfo_steps_and_tones_read_as_the_handset_shows_them() {
        use super::{vfo_step_label, vfo_tone_label};
        use radio_domain::Tone;
        let steps: Vec<String> = radio_platform::operator_shell::VFO_STEPS_HZ
            .iter()
            .map(|step| vfo_step_label(*step))
            .collect();
        assert_eq!(
            steps,
            ["6.25 kHz", "12.5 kHz", "25 kHz", "50 kHz", "100 kHz", "1 MHz"]
        );
        assert_eq!(vfo_tone_label(Tone::None), "Off");
        assert_eq!(vfo_tone_label(Tone::Ctcss(885)), "88.5");
        assert_eq!(
            vfo_tone_label(Tone::Dcs {
                code: 23,
                inverted: true
            }),
            "D023I"
        );
    }

    #[test]
    fn tx_choices_are_labelled_as_the_handset_lists_them() {
        let powers: Vec<_> = (0..radio_domain::TX_POWER_LEVELS)
            .map(tx_power_label)
            .collect();
        assert_eq!(
            powers,
            ["20 mW", "125 mW", "250 mW", "500 mW", "2 W", "5 W"]
        );
        assert_eq!(tx_timeout_label(0), "1 minute");
        assert_eq!(tx_timeout_label(4), "5 minutes");
    }

    /// A project directory of the test's own, so nothing touches the
    /// operator's projects or their last-project record.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("afik-studio-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn app_in(dir: &std::path::Path, options: &Options) -> StudioApp {
        StudioApp::with_projects(
            options,
            ProjectStore::open_in(dir.join("projects"), Some(dir.join("last"))),
        )
    }

    /// Lets a running write or read end and takes in its outcome.
    fn settle(app: &mut StudioApp) {
        while app.job.is_some() {
            app.poll_job();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn simulated(name: &str) -> StudioApp {
        app_in(
            &scratch(name),
            &Options {
                simulator: true,
                ..Options::default()
            },
        )
    }

    #[test]
    fn a_simulator_start_up_connects_and_opens_the_default_project() {
        let app = simulated("startup");
        assert!(app.session.is_some());
        assert_eq!(app.tab, Tab::Banks);
        assert!(
            app.status.contains("Connected to simulator"),
            "{}",
            app.status
        );
        assert_eq!(app.projects.current().name, "default");
        assert!(!app.is_dirty());
    }

    #[test]
    fn writing_an_invalid_project_reports_errors_without_contacting_the_radio() {
        let mut app = simulated("invalid");
        app.project.add_bank();
        app.project.add_channel(0);
        app.project.banks[0].channels[0].receive_mhz = "nope".to_owned();
        app.write_project();
        assert!(!app.errors.is_empty());
        assert!(app.last_write.is_none());
        assert_eq!(
            app.session.as_ref().unwrap().simulator().unwrap().writes(),
            0
        );
    }

    #[test]
    fn channels_and_a_plan_write_and_read_back_through_the_simulator() {
        let mut app = simulated("write");
        app.project.add_bank();
        app.project.add_channel(0);
        app.project.add_generated_bank();
        app.project.banks[1].name = "PMR446".to_owned();
        app.project.banks[1].base_mhz = "446.006250".to_owned();
        app.project.banks[1].designator = "PMR ".to_owned();
        app.write_project();
        // The write runs on a worker, which holds the session meanwhile, so
        // nothing else may use or replace the connection until it ends.
        assert!(app.job.is_some() && app.session.is_none());
        app.read_project();
        assert!(app.status.contains("Wait until"), "{}", app.status);
        settle(&mut app);
        assert!(app.errors.is_empty(), "{:?}", app.errors);
        assert!(app.last_write.as_ref().unwrap().changed_blocks > 0);
        let index = app
            .session
            .as_ref()
            .unwrap()
            .simulator()
            .unwrap()
            .index()
            .unwrap();
        assert_eq!(index.len(), 17);

        let written = app.project.clone();
        app.project = crate::model::ProjectModel::new();
        app.read_project();
        settle(&mut app);
        assert_eq!(app.project, written);
        assert_eq!(app.project.banks[1].kind, BankKind::Generated);

        // Another radio has not had this write, so its report is not shown.
        app.connect_simulator();
        assert!(app.last_write.is_none());
        app.write_project();
        app.disconnect();
        assert!(app.status.contains("Wait until"), "{}", app.status);
        settle(&mut app);
        assert!(app.last_write.is_some());
        app.disconnect();
        assert!(app.last_write.is_none());
        assert!(app.session.is_none());
    }

    #[test]
    fn projects_are_managed_on_their_own_tab() {
        use eframe::egui::{CentralPanel, Context, Pos2, RawInput, Rect, Vec2};

        let mut app = app_in(&scratch("projects-tab"), &Options::default());
        app.save_project();
        app.new_project = "Club".to_owned();
        app.create_project();
        let context = Context::default();
        let draw = |app: &mut StudioApp, tab: fn(&mut StudioApp, &mut eframe::egui::Ui)| {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
                ..RawInput::default()
            };
            context.run(input, |context| {
                CentralPanel::default().show(context, |ui| tab(app, ui));
            })
        };
        let banks = draw(&mut app, StudioApp::banks_tab);
        assert!(
            text_centres(&banks, "New project").is_empty(),
            "not on Banks"
        );
        assert!(
            text_centres(&banks, "Revert").is_empty(),
            "moved to the tab bar"
        );
        // The tab bar carries the project at its right, on every tab.
        let bar = draw(&mut app, StudioApp::tab_bar);
        let flash = text_centres(&bar, "Flash")[0];
        let name = text_centres(&bar, "Club")[0];
        let revert = text_centres(&bar, "Revert")[0];
        assert!((name.y - flash.y).abs() < 1.0, "in line with the tabs");
        assert!(
            name.x > flash.x && revert.x > name.x,
            "{flash:?} {name:?} {revert:?}"
        );
        assert!(revert.x > 1_000.0, "at the right edge: {revert:?}");
        assert_eq!(text_centres(&bar, "Save").len(), 1);
        let projects = draw(&mut app, StudioApp::projects_tab);
        assert_eq!(text_centres(&projects, "New project").len(), 1);
        assert_eq!(text_centres(&projects, "default").len(), 1, "listed");
        assert_eq!(text_centres(&projects, "Delete project").len(), 1);
    }

    #[test]
    fn a_project_is_exported_as_saved_and_imported_under_a_free_name() {
        let dir = scratch("import-export");
        let mut app = app_in(&dir, &Options::default());
        app.project.add_bank();
        app.project.add_channel(0);
        assert!(
            app.project_export().is_none(),
            "unsaved changes are refused"
        );
        app.save_project();
        let (name, bytes) = app.project_export().unwrap();
        assert_eq!(name, "default.afik");

        // The browser stages the file it took in; importing it twice gives
        // the second a numbered name.
        let staged = dir.join("imports").join("default.afik");
        for expected in ["default 2", "default 3"] {
            std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
            std::fs::write(&staged, &bytes).unwrap();
            app.import_project(&staged);
            assert_eq!(app.projects.current().name, expected);
            assert!(app.status.starts_with(&format!("Imported {expected}.")));
            assert_eq!(app.project.channel_count(), 1);
            assert!(!staged.exists(), "the staged copy is removed");
        }

        std::fs::write(&staged, b"not a project").unwrap();
        app.import_project(&staged);
        assert!(
            app.status.starts_with("Not an AFIK project"),
            "{}",
            app.status
        );
        assert_eq!(app.projects.entries().len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn projects_are_renamed_and_deleted() {
        let dir = scratch("rename-delete");
        let projects = dir.join("projects");
        let mut app = app_in(&dir, &Options::default());
        app.project.add_bank();
        app.save_project();
        app.new_project = "Club".to_owned();
        app.create_project();

        app.new_project = "default".to_owned();
        app.rename_project();
        assert!(app.status.contains("already exists"), "{}", app.status);
        app.new_project = "Club 2m".to_owned();
        app.rename_project();
        assert_eq!(app.status, "Renamed Club to Club 2m.");
        assert!(app.new_project.is_empty());
        assert!(projects.join("Club 2m.afik").is_file());
        assert!(!projects.join("Club.afik").exists());

        // Delete asks first; the project goes only on Yes.
        let context = eframe::egui::Context::default();
        let _ = context.run(eframe::egui::RawInput::default(), |context| {
            eframe::egui::CentralPanel::default().show(context, |ui| app.projects_tab(ui));
        });
        assert!(!app.confirm_delete);
        app.confirm_delete = true;
        app.delete_project();
        assert!(!app.confirm_delete);
        assert!(!projects.join("Club 2m.afik").exists());
        assert_eq!(app.projects.current().name, "default");
        assert_eq!(app.project.banks.len(), 1, "the next project is loaded");
        assert!(
            app.status.starts_with("Deleted Club 2m. Opened default"),
            "{}",
            app.status
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn projects_are_saved_by_name_and_reopened_at_start_up() {
        let dir = scratch("projects");
        let options = Options::default();
        let mut app = app_in(&dir, &options);
        app.project.add_bank();
        assert!(app.is_dirty());

        // A changed project is not dropped by opening or creating another.
        app.new_project = "Club".to_owned();
        app.create_project();
        assert_eq!(app.projects.current().name, "default");
        app.save_project();
        assert!(!app.is_dirty());
        assert!(dir.join("projects").join("default.afik").is_file());

        app.new_project = "Club".to_owned();
        app.create_project();
        assert_eq!(app.projects.current().name, "Club");
        assert!(app.project.banks.is_empty());
        assert!(dir.join("projects").join("Club.afik").is_file());
        app.project.add_generated_bank();
        app.open_project(1);
        assert_eq!(
            app.projects.current().name,
            "Club",
            "unsaved changes keep it open"
        );
        app.load_project();
        app.open_project(1);
        assert_eq!(app.projects.current().name, "default");
        assert_eq!(app.project.banks.len(), 1);

        // The next start opens the project opened last.
        let app = app_in(&dir, &options);
        assert_eq!(app.projects.current().name, "default");
        assert_eq!(app.project.banks.len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn opening_a_project_shares_the_banks_locations_have_in_common() {
        use super::{BankSource, Origin, ProjectModel};
        use crate::repeaters::{repeater_bank, Coverage, Repeater};

        let dir = scratch("share-on-open");
        let mut app = app_in(&dir, &Options::default());
        let repeater = |callsign: &str, output_hz: u32| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz,
            input_hz: output_hz - 600_000,
            ctcss_tenths_hz: Some(825),
            wide: false,
            latitude: 51.5,
            longitude: -0.1,
            coverage: Coverage::Standard,
        };
        let [a, b, c, d] = [
            repeater("GB3AA", 145_600_000),
            repeater("GB3BB", 145_625_000),
            repeater("GB3CC", 145_650_000),
            repeater("GB3DD", 145_675_000),
        ];
        // Home hears three, Work those and one more: three of four common.
        let mut project = ProjectModel::new();
        for (locator, name, heard) in [
            ("IO91WM", "HOME", vec![&a, &b, &c]),
            ("IO91WL", "WORK", vec![&c, &b, &a, &d]),
        ] {
            let near: Vec<_> = heard.into_iter().map(|r| (r, 1.0)).collect();
            let mut bank = repeater_bank(locator, &near);
            bank.origin = Some(Origin::new(BankSource::Repeaters, locator));
            let group = project.ensure_group(name, Some(locator)).unwrap();
            bank.groups.push(group);
            project.banks.push(bank);
        }
        let path = app.projects.current().path.clone();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, project.to_image().unwrap()).unwrap();

        app.load_project();
        let names: Vec<_> = app.project.banks.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["RPT HOME+WORK"]);
        assert_eq!(app.project.banks[0].channels.len(), 4);
        assert!(app.status.contains("Shared 1 bank"), "{}", app.status);
        assert!(app.is_dirty(), "saving keeps it");
        app.save_project();
        assert!(!app.is_dirty());
        assert_eq!(app.status, "Saved default.");

        // Saving shares too, whatever made the banks.
        app.project = project;
        app.save_project();
        assert_eq!(app.project.banks.len(), 1);
        assert!(app.status.contains("Shared 1 bank"), "{}", app.status);
        assert!(!app.is_dirty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unopenable_start_up_device_is_reported_without_a_session() {
        let app = app_in(
            &scratch("unopenable"),
            &Options {
                device: Some(DeviceSelector::Explicit(PathBuf::from(
                    "/definitely/missing",
                ))),
                ..Options::default()
            },
        );
        assert!(app.session.is_none());
        assert!(app.status.contains("serial setup failed"), "{}", app.status);
        assert_eq!(app.chooser.manual_path, "/definitely/missing");
        assert_eq!(app.chooser.baud, crate::device::DEFAULT_BAUD);
    }

    #[test]
    fn bank_headers_give_name_count_kind_and_span_columns() {
        let mut project = crate::model::ProjectModel::new();
        project.add_bank();
        project.add_channel(0);
        project.banks[0].channels[0].receive_mhz = "145.5".to_owned();
        assert_eq!(
            bank_row_columns(0, &project.banks[0]),
            ["1: Bank 1", "1 Ch", "Channels", "145.500000 MHz"]
        );
        project.add_generated_bank();
        project.banks[1].base_mhz = "446.00625".to_owned();
        assert_eq!(
            bank_row_columns(1, &project.banks[1]),
            [
                "2: Bank 2",
                "16 Ch",
                "Generated plan",
                "446.006250 to 446.193750 MHz"
            ]
        );
        project.banks[1].base_mhz = "nope".to_owned();
        assert_eq!(bank_row_columns(1, &project.banks[1])[3], "");
    }

    /// Draws one headless frame of the Banks tab with the given input events.
    fn frame(
        context: &eframe::egui::Context,
        app: &mut StudioApp,
        events: Vec<eframe::egui::Event>,
    ) -> eframe::egui::FullOutput {
        use eframe::egui::{CentralPanel, Pos2, RawInput, Rect, Vec2};
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 4000.0))),
            events,
            ..RawInput::default()
        };
        context.run(input, |context| {
            CentralPanel::default().show(context, |ui| match app.tab {
                Tab::Map => app.map_tab(ui),
                _ => app.banks_tab(ui),
            });
        })
    }

    /// Draws one headless frame of the Radio tab.
    fn radio_frame(
        context: &eframe::egui::Context,
        app: &mut StudioApp,
    ) -> eframe::egui::FullOutput {
        use eframe::egui::{CentralPanel, Pos2, RawInput, Rect, Vec2};
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 4000.0))),
            ..RawInput::default()
        };
        context.run(input, |context| {
            CentralPanel::default().show(context, |ui| app.radio_tab(ui));
        })
    }

    #[test]
    fn default_sets_are_offered_on_the_banks_tab_only() {
        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("default-set"), &Options::default());
        let banks = frame(&context, &mut app, Vec::new());
        assert_eq!(text_left_edges(&banks, "Default set").len(), 1);
        let radio = radio_frame(&context, &mut app);
        assert!(text_left_edges(&radio, "Default set").is_empty());
    }

    #[test]
    fn the_map_tab_shows_one_generator_at_a_time() {
        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("generators"), &Options::default());
        let banks = frame(&context, &mut app, Vec::new());
        app.tab = Tab::Map;
        assert!(
            text_left_edges(&banks, GENERATORS_TITLE).is_empty(),
            "moved off the Banks tab"
        );
        let rows = [
            (Generator::Repeaters, "Repeaters within"),
            (Generator::Gateways, "UK gateways within"),
            (Generator::Beacons, "UK beacons within"),
            (Generator::Stations, "FM stations (UK, US), strongest"),
            (Generator::Airports, "Airports within"),
            (Generator::Airfields, "Airfields within"),
            (Generator::Marine, "Marine within"),
        ];
        for generator in Generator::ALL {
            app.generator = generator;
            let output = frame(&context, &mut app, Vec::new());
            // The panel, its shared locator and every tab are always there.
            assert_eq!(text_left_edges(&output, GENERATORS_TITLE).len(), 1);
            assert_eq!(text_left_edges(&output, "Your locator").len(), 1);
            for other in Generator::ALL {
                let tabs = output
                    .shapes
                    .iter()
                    .filter(|clipped| {
                        matches!(&clipped.shape, eframe::egui::Shape::Text(text)
                            if text.galley.text() == other.label())
                    })
                    .count();
                // Once as a tab; the map's Show box leads with its icon.
                assert_eq!(tabs, 1, "{generator:?} offers {other:?}");
            }
            for (owner, row) in rows {
                let shown = text_left_edges(&output, row).len();
                assert_eq!(
                    shown,
                    usize::from(owner == generator),
                    "{generator:?} {row}"
                );
            }
        }
    }

    #[test]
    fn clicking_a_generator_tab_chooses_it() {
        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("generator-click"), &Options::default());
        app.tab = Tab::Map;
        let output = frame(&context, &mut app, Vec::new());
        let at = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                eframe::egui::Shape::Text(text) if text.galley.text() == "Marine" => {
                    Some(text.pos + eframe::egui::vec2(3.0, 3.0))
                }
                _ => None,
            })
            .expect("the Marine tab is drawn");
        assert_eq!(app.generator, Generator::Repeaters);
        click(&context, &mut app, at);
        assert_eq!(app.generator, Generator::Marine);
        let output = frame(&context, &mut app, Vec::new());
        assert_eq!(text_left_edges(&output, "Marine within").len(), 1);
    }

    #[test]
    fn the_radio_tab_shows_one_group_of_settings_at_a_time() {
        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("radio-sections"), &Options::default());
        // Every section is offered, and each setting appears in exactly one.
        let expected = [
            (RadioSection::Receive, "Squelch level"),
            (RadioSection::Transmit, "TX timeout"),
            (RadioSection::Transmit, "TX ranges"),
            (RadioSection::Scan, "Scan dwell ms"),
            (RadioSection::General, "Backlight seconds"),
        ];
        for section in RadioSection::ALL {
            app.radio_section = section;
            let output = radio_frame(&context, &mut app);
            for other in RadioSection::ALL {
                let tabs = output
                    .shapes
                    .iter()
                    .filter(|clipped| {
                        matches!(&clipped.shape, eframe::egui::Shape::Text(text)
                            if text.galley.text() == other.label())
                    })
                    .count();
                assert_eq!(tabs, 1, "{section:?} offers {other:?}");
            }
            for (owner, label) in expected {
                let shown = text_left_edges(&output, label).len();
                assert_eq!(shown, usize::from(owner == section), "{section:?} {label}");
            }
        }
    }

    /// Returns where each drawn text starting with `prefix` begins.
    fn text_left_edges(output: &eframe::egui::FullOutput, prefix: &str) -> Vec<f32> {
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                eframe::egui::Shape::Text(text) if text.galley.text().starts_with(prefix) => {
                    Some(text.pos.x)
                }
                _ => None,
            })
            .collect()
    }

    /// Returns the centre of each drawn text equal to `text`, in drawing order.
    fn text_centres(output: &eframe::egui::FullOutput, text: &str) -> Vec<eframe::egui::Pos2> {
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                eframe::egui::Shape::Text(shape) if shape.galley.text() == text => {
                    Some(shape.visual_bounding_rect().center())
                }
                _ => None,
            })
            .collect()
    }

    /// Opens a channel row, which starts collapsed, by clicking its header.
    fn open_channel(context: &eframe::egui::Context, app: &mut StudioApp, bank: usize, row: usize) {
        let label = channel_row_label(row, &app.project.banks[bank].channels[row]);
        let output = frame(context, app, Vec::new());
        let at = output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                eframe::egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + eframe::egui::vec2(2.0, 2.0))
                }
                _ => None,
            })
            .expect("the channel header is drawn");
        click(context, app, at);
    }

    /// Clicks the primary button at `at` over two frames of the Banks tab.
    fn click(context: &eframe::egui::Context, app: &mut StudioApp, at: eframe::egui::Pos2) {
        use eframe::egui::{Event, Modifiers, PointerButton};
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(context, app, vec![Event::PointerMoved(at), button(true)]);
        frame(context, app, vec![button(false)]);
        frame(context, app, Vec::new());
    }

    #[test]
    fn each_group_has_a_tab_listing_its_banks_and_editing_its_members() {
        use super::{bank_rows, BankView, COMMON_GROUP};

        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("group-tabs"), &Options::default());
        for name in ["RPT JO02EH", "SEA CORE", "MINE", "LOOSE"] {
            app.add_bank(BankKind::Named);
            app.project.banks.last_mut().unwrap().name = name.to_owned();
        }
        assert!(app.project.file_bank("RPT JO02EH", "HOME", "JO02EH", false));
        assert!(app.project.file_bank("SEA CORE", "HOME", "JO02EH", false));
        assert!(app.project.file_in("SEA CORE", "MARINE"));
        assert!(app.project.file_in("MINE", COMMON_GROUP));
        let project = &app.project;
        assert_eq!(bank_rows(project, BankView::All), [0, 1, 2, 3]);
        assert_eq!(bank_rows(project, BankView::Group(0)), [0, 1]);
        assert_eq!(bank_rows(project, BankView::Group(1)), [1]);
        assert_eq!(bank_rows(project, BankView::Loose), [3]);

        // Every group has a tab; the open one lists its banks alone.
        frame(&context, &mut app, Vec::new());
        let output = frame(&context, &mut app, Vec::new());
        for tab in [
            "All banks (4)",
            "HOME (2)",
            "MARINE (1)",
            "COMMON (1)",
            "Not in a group (1)",
        ] {
            assert_eq!(text_centres(&output, tab).len(), 1, "{tab}");
        }
        assert_eq!(text_centres(&output, "Delete").len(), 4);
        app.banks_tab.view = BankView::Group(0);
        frame(&context, &mut app, Vec::new());
        let output = frame(&context, &mut app, Vec::new());
        assert_eq!(text_centres(&output, "Delete").len(), 2);
        assert_eq!(text_centres(&output, "Remove group").len(), 1);

        // A bank added in a group's tab joins the group.
        app.add_bank(BankKind::Named);
        let added = app.project.banks.len() - 1;
        app.project.set_member(added, 0, true);
        assert_eq!(bank_rows(&app.project, BankView::Group(0)), [0, 1, added]);

        // A tab whose group goes falls back to every bank.
        app.banks_tab.view = BankView::Group(2);
        app.project.remove_group(2);
        frame(&context, &mut app, Vec::new());
        assert_eq!(app.banks_tab.view, BankView::All);
    }

    #[test]
    fn a_bank_header_has_no_up_or_down_buttons() {
        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("reorder"), &Options::default());
        for name in ["A", "B", "C"] {
            app.add_bank(BankKind::Named);
            app.project.banks.last_mut().unwrap().name = name.to_owned();
        }
        frame(&context, &mut app, Vec::new());
        let output = frame(&context, &mut app, Vec::new());
        assert_eq!(text_centres(&output, "Delete").len(), 3);
        assert!(text_centres(&output, "Down").is_empty());
        assert!(text_centres(&output, "Up").is_empty());
    }

    #[test]
    fn bank_header_columns_line_up_and_delete_sits_at_the_right() {
        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("columns"), &Options::default());
        app.add_bank(BankKind::Named);
        app.project.banks[0].name = "A".to_owned();
        app.project.add_channel(0);
        app.add_bank(BankKind::Named);
        app.project.banks[1].name = "A much longer bank name".to_owned();
        app.add_bank(BankKind::Generated);
        frame(&context, &mut app, Vec::new());
        let output = frame(&context, &mut app, Vec::new());

        let counts = [
            text_left_edges(&output, "1 Ch"),
            text_left_edges(&output, "0 Ch"),
            text_left_edges(&output, "16 Ch"),
        ];
        assert!(counts.iter().all(|edges| edges.len() == 1), "{counts:?}");
        assert!((counts[0][0] - counts[1][0]).abs() < 0.5, "{counts:?}");
        assert!((counts[0][0] - counts[2][0]).abs() < 0.5, "{counts:?}");
        let longest = text_left_edges(&output, "2: A much longer");
        // Spare width is shared between the columns, not left at the end.
        assert!(counts[0][0] > longest[0] + 250.0, "{counts:?} {longest:?}");

        // Delete sits at the right edge of the 1100-point window on every row;
        // matched whole, so Delete project above is not counted.
        let deletes = text_centres(&output, "Delete");
        assert_eq!(deletes.len(), 3, "{deletes:?}");
        assert!(deletes.iter().all(|at| at.x > 1_000.0), "{deletes:?}");
    }

    #[test]
    fn a_shared_setting_is_edited_on_the_bank_and_shown_as_the_banks_on_a_channel() {
        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("shared"), &Options::default());
        app.add_bank(BankKind::Named);
        app.project.add_channel(0);
        app.project.banks[0].shared = radio_channel_plan::SharedSettings::none()
            .with(radio_channel_plan::SharedSettings::POWER, true);
        frame(&context, &mut app, Vec::new());
        let output = frame(&context, &mut app, Vec::new());
        // Channel rows start collapsed.
        assert!(text_left_edges(&output, "from bank").is_empty());
        open_channel(&context, &mut app, 0, 0);
        let output = frame(&context, &mut app, Vec::new());
        // Power is shared; the other six settings stay per channel.
        assert_eq!(text_left_edges(&output, "from bank").len(), 1);
        assert_eq!(text_left_edges(&output, "per channel").len(), 6);
    }

    /// Tabs through the widgets until typed text lands in the probed field.
    fn focus_field(
        context: &eframe::egui::Context,
        app: &mut StudioApp,
        probe: impl Fn(&StudioApp) -> String,
    ) {
        use eframe::egui::{Event, Key, Modifiers};
        let before = probe(app);
        for _ in 0..60 {
            let tab_key = Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            };
            frame(context, app, vec![tab_key]);
            frame(context, app, vec![Event::Text("Z".to_owned())]);
            if probe(app) != before {
                return;
            }
        }
        panic!("the probed field never took keyboard focus");
    }

    #[test]
    fn typing_into_a_row_keeps_focus_while_its_header_label_changes() {
        use eframe::egui::{Context, Event};
        // Row headers show the name and frequency being edited. Typing must
        // keep going to the same field, not stop after one character.
        let context = Context::default();
        let mut app = app_in(&scratch("focus"), &Options::default());
        app.add_bank(BankKind::Named);
        app.project.add_channel(0);
        frame(&context, &mut app, Vec::new());
        open_channel(&context, &mut app, 0, 0);
        focus_field(&context, &mut app, |app| {
            app.project.banks[0].channels[0].receive_mhz.clone()
        });
        for text in ["Y", "X"] {
            frame(&context, &mut app, vec![Event::Text(text.into())]);
        }
        assert!(
            app.project.banks[0].channels[0]
                .receive_mhz
                .ends_with("ZYX"),
            "{}",
            app.project.banks[0].channels[0].receive_mhz
        );

        let context = Context::default();
        let mut app = app_in(&scratch("focus-plan"), &Options::default());
        app.add_bank(BankKind::Generated);
        frame(&context, &mut app, Vec::new());
        focus_field(&context, &mut app, |app| app.project.banks[0].name.clone());
        for text in ["Y", "X"] {
            frame(&context, &mut app, vec![Event::Text(text.into())]);
        }
        assert!(
            app.project.banks[0].name.ends_with("ZYX"),
            "{}",
            app.project.banks[0].name
        );
    }

    /// Captures log records so a test can see what its own thread logged.
    mod capture {
        use log::{Level, LevelFilter, Log, Metadata, Record};
        use std::{
            sync::{Mutex, Once},
            thread::{self, ThreadId},
        };

        static RECORDS: Mutex<Vec<(ThreadId, Level, String)>> = Mutex::new(Vec::new());
        static CAPTURE: Capture = Capture;

        struct Capture;

        impl Log for Capture {
            fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
                true
            }

            fn log(&self, record: &Record<'_>) {
                RECORDS.lock().unwrap().push((
                    thread::current().id(),
                    record.level(),
                    record.args().to_string(),
                ));
            }

            fn flush(&self) {}
        }

        /// Installs the capture logger once for the whole test process.
        pub fn install() {
            static ONCE: Once = Once::new();
            ONCE.call_once(|| {
                log::set_logger(&CAPTURE).unwrap();
                log::set_max_level(LevelFilter::Trace);
            });
        }

        /// Takes the records this thread logged.
        pub fn take() -> Vec<(Level, String)> {
            let me = thread::current().id();
            let mut records = RECORDS.lock().unwrap();
            let (mine, others) = records.drain(..).partition(|(id, _, _)| *id == me);
            *records = others;
            mine.into_iter()
                .map(|(_, level, text)| (level, text))
                .collect()
        }
    }

    #[test]
    fn each_new_status_line_is_logged_once_at_debug() {
        use log::Level;
        capture::install();
        let mut app = simulated("status-log");
        let _earlier = capture::take();
        "Added 3 gateways.".clone_into(&mut app.status);
        app.log_status();
        app.log_status();
        let logged = capture::take();
        assert_eq!(
            logged,
            [(Level::Debug, "status: Added 3 gateways.".to_owned())]
        );
    }

    #[test]
    fn failures_are_logged_with_what_was_being_done() {
        use log::Level;
        capture::install();
        let _earlier = capture::take();

        let mut app = app_in(
            &scratch("logged"),
            &Options {
                device: Some(DeviceSelector::Explicit(PathBuf::from(
                    "/definitely/missing",
                ))),
                ..Options::default()
            },
        );
        let logged = capture::take();
        assert!(
            logged.iter().any(|(level, text)| *level == Level::Error
                && text.contains("connecting /definitely/missing at 38400 baud")
                && text.contains("serial setup failed")),
            "{logged:?}"
        );

        // A refused write is a warning naming every field which failed.
        app.project.add_bank();
        app.project.add_channel(0);
        app.project.banks[0].channels[0].receive_mhz = "nope".to_owned();
        app.write_project();
        let logged = capture::take();
        assert!(logged
            .iter()
            .any(|(level, text)| *level == Level::Warn && text.contains("writing refused")));
        assert!(logged
            .iter()
            .any(|(level, text)| *level == Level::Warn && text.contains("receive")));
    }

    #[test]
    fn banks_keep_their_open_state_when_rows_move_or_go() {
        use super::{reorder_open, BankAction};
        let mut open = vec![true, false, false, true];
        reorder_open(&mut open, 1, BankAction::MoveTo(0));
        assert_eq!(open, [false, true, false, true]);
        reorder_open(&mut open, 2, BankAction::MoveTo(4));
        assert_eq!(open, [false, true, true, false]);
        reorder_open(&mut open, 0, BankAction::Remove);
        assert_eq!(open, [true, true, false, false]);
        reorder_open(&mut open, 0, BankAction::AddChannel);
        assert_eq!(open, [true, true, false, false]);
        reorder_open(&mut open, 0, BankAction::MoveTo(4));
        assert_eq!(open, [true, false, false, true]);
        reorder_open(&mut open, 3, BankAction::MoveTo(1));
        assert_eq!(open, [true, true, false, false]);
    }

    #[test]
    fn a_dragged_bank_lands_before_the_first_bank_whose_middle_is_below() {
        use super::drop_slot;
        let spans = [(0.0, 20.0), (20.0, 100.0), (100.0, 120.0)];
        assert_eq!(drop_slot(&spans, -5.0), 0);
        assert_eq!(drop_slot(&spans, 9.0), 0);
        assert_eq!(drop_slot(&spans, 11.0), 1);
        // An open bank's middle is far below its header.
        assert_eq!(drop_slot(&spans, 59.0), 1);
        assert_eq!(drop_slot(&spans, 61.0), 2);
        assert_eq!(drop_slot(&spans, 500.0), 3);
        assert_eq!(drop_slot(&[], 10.0), 0);
    }

    #[test]
    fn a_bank_dragged_by_its_handle_moves_where_it_is_dropped() {
        use eframe::egui::{vec2, Event, Modifiers, PointerButton, Pos2};
        let context = eframe::egui::Context::default();
        let mut app = app_in(&scratch("drag"), &Options::default());
        for name in ["A", "B", "C"] {
            app.add_bank(BankKind::Named);
            app.project.banks.last_mut().unwrap().name = name.to_owned();
        }
        let names = |app: &StudioApp| -> Vec<String> {
            app.project.banks.iter().map(|b| b.name.clone()).collect()
        };
        let drag = |app: &mut StudioApp, from: Pos2, to: Pos2| {
            let button = |pos, pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            frame(
                &context,
                app,
                vec![Event::PointerMoved(from), button(from, true)],
            );
            frame(
                &context,
                app,
                vec![Event::PointerMoved(from + vec2(0.0, 10.0))],
            );
            frame(&context, app, vec![Event::PointerMoved(to)]);
            frame(&context, app, vec![Event::PointerMoved(to)]);
            frame(&context, app, vec![button(to, false)]);
            frame(&context, app, Vec::new());
        };
        frame(&context, &mut app, Vec::new());
        let output = frame(&context, &mut app, Vec::new());
        let handles = text_centres(&output, super::DRAG_HANDLE);
        assert_eq!(handles.len(), 3, "one handle per header");

        // A below the last bank, which is open for editing, goes last.
        drag(&mut app, handles[0], handles[2] + vec2(0.0, 400.0));
        assert_eq!(names(&app), ["B", "C", "A"]);
        // C above the first bank goes first.
        drag(&mut app, handles[1], handles[0] - vec2(0.0, 8.0));
        assert_eq!(names(&app), ["C", "B", "A"]);
        // Dropped where it already is, nothing moves.
        drag(&mut app, handles[1], handles[1] + vec2(0.0, 2.0));
        assert_eq!(names(&app), ["C", "B", "A"]);
    }

    #[test]
    fn banks_open_collapsed_and_a_new_bank_opens_for_editing() {
        use eframe::egui::{Context, Event};
        // A loaded bank is collapsed: typing reaches none of its fields.
        let context = Context::default();
        let mut app = app_in(&scratch("collapsed"), &Options::default());
        app.project.add_bank();
        let name = app.project.banks[0].name.clone();
        frame(&context, &mut app, Vec::new());
        for _ in 0..40 {
            let tab_key = Event::Key {
                key: eframe::egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: eframe::egui::Modifiers::NONE,
            };
            frame(&context, &mut app, vec![tab_key]);
            frame(&context, &mut app, vec![Event::Text("Z".to_owned())]);
        }
        assert_eq!(app.project.banks[0].name, name);

        // A bank the operator adds is opened, so its name can be typed.
        app.add_bank(BankKind::Named);
        frame(&context, &mut app, Vec::new());
        focus_field(&context, &mut app, |app| app.project.banks[1].name.clone());
        assert_eq!(app.project.banks[0].name, name);
    }

    #[test]
    fn header_columns_share_the_spare_width_but_keep_a_least_gap() {
        let widths = [200.0, 50.0, 100.0, 250.0];
        assert!((column_gap(1_000.0, &widths) - 100.0).abs() < f32::EPSILON);
        assert!((column_gap(500.0, &widths) - MIN_COLUMN_GAP).abs() < f32::EPSILON);
    }

    #[test]
    fn the_status_bar_reads_memory_in_tenths() {
        assert_eq!(memory_percent(0), "0.0%");
        assert_eq!(memory_percent(125), "12.5%");
        assert_eq!(memory_percent(1_000), "100.0%");
        assert_eq!(memory_percent(1_124), "112.4%");
    }

    #[test]
    fn labels_cover_every_enumerated_value() {
        assert_eq!(scan_resume_label(ScanResume::Stop), "Stop on signal");
        let mut project = crate::model::ProjectModel::new();
        project.add_bank();
        project.add_channel(0);
        assert_eq!(
            channel_row_label(0, &project.banks[0].channels[0]),
            "1: CH1 (145.500000)"
        );
    }

    fn preset_index(name: &str) -> usize {
        crate::presets::PRESETS
            .iter()
            .position(|p| p.name() == name)
            .expect("preset")
    }

    #[test]
    fn sync_takes_the_lists_never_synced_or_synced_over_a_day_ago() {
        let mut app = simulated("datasets-due");
        let wanted = |app: &StudioApp, wanted| -> Vec<bool> {
            app.datasets(wanted).iter().map(|(_, want)| *want).collect()
        };
        assert_eq!(wanted(&app, super::never), [true; 6]);
        assert_eq!(wanted(&app, super::due), [true; 6]);

        let now = std::time::SystemTime::now();
        app.world = Some(crate::world::WorldList {
            repeaters: Vec::new(),
            synced: now,
        });
        app.marine = Some(crate::marine::MarineList {
            places: Vec::new(),
            synced: now - std::time::Duration::from_secs(super::SYNC_AFTER_SECS + 60),
        });
        let never = [true, false, true, true, false, true];
        let due = [true, false, true, true, true, true];
        assert_eq!(wanted(&app, super::never), never);
        assert_eq!(wanted(&app, super::due), due);
        assert!(!super::due(Some(now + std::time::Duration::from_secs(60))));

        // A station cache missing a source, from before the FCC, is due.
        app.stations = Some(crate::stations::StationList {
            stations: Vec::new(),
            synced: now,
            complete: false,
        });
        assert!(wanted(&app, super::due)[2]);
        app.stations.as_mut().unwrap().complete = true;
        assert!(!wanted(&app, super::due)[2]);
    }

    #[test]
    fn the_satellite_set_is_built_in_until_a_sync_replaces_it() {
        use crate::presets::SATELLITE_PRESET;
        use crate::satellites::{SatelliteChannel, SatelliteList};

        let mut app = simulated("satellites");
        assert!(app.satellites.is_none(), "tests never read the real cache");
        assert_eq!(app.satellite_source(), "Built-in list; not synced.");
        app.preset = preset_index(SATELLITE_PRESET);
        app.add_preset();
        let built_in = app.project.banks.last().unwrap();
        assert_eq!(built_in.name, "SATELLITES");
        assert!(built_in.channels.iter().any(|c| c.name == "ISS RPT"));

        let mut app = simulated("satellites-synced");
        app.satellites = Some(SatelliteList {
            channels: vec![SatelliteChannel {
                name: "SO-50".to_owned(),
                receive_hz: 436_795_000,
                transmit_hz: 145_850_000,
                tx_ctcss_tenths_hz: Some(670),
            }],
            synced: std::time::SystemTime::now(),
        });
        assert!(app
            .satellite_source()
            .starts_with("1 channels from SatNOGS"));
        app.preset = preset_index(SATELLITE_PRESET);
        app.add_preset();
        let synced = app.project.banks.last().unwrap();
        assert_eq!(synced.name, "SATELLITES");
        assert_eq!(synced.channels.len(), 1);
        assert_eq!(synced.channels[0].name, "SO-50");
        assert_eq!(synced.channels[0].receive_mhz, "436.795000");
        assert_eq!(synced.channels[0].transmit_mhz, "145.850000");
        assert_eq!(synced.channels[0].tx_tone.value, "67.0");
        assert!(app.project.validate().is_ok());
    }

    #[test]
    fn marine_channels_are_the_core_until_synced_then_the_nearest_places() {
        use crate::marine::{channel, MarineList, MarinePlace, CORE_CHANNELS};

        let mut app = simulated("marine");
        assert!(app.marine.is_none(), "tests never read the real cache");
        app.repeater_area.locator = "JO01HP".to_owned();
        // Burnham is about 13 km from the square's centre.
        app.marine_area.radius_km = 20;
        let source = app.marine_source();
        assert!(source.starts_with("8 core and nearest 0 of 0"), "{source}");
        let banks = app.project.banks.len();
        app.add_marine();
        // Unsynced, only the shared core: no empty bank for the location.
        assert_eq!(app.project.banks.len(), banks + 1);
        let bank = app.project.banks.last().unwrap();
        assert_eq!(bank.name, "SEA CORE");
        assert_eq!(bank.channels.len(), CORE_CHANNELS);
        let home = app.project.location("JO01HP").unwrap();
        let marine = app.project.group_named(super::MARINE_GROUP).unwrap();
        assert!(bank.groups.contains(&home) && bank.groups.contains(&marine));

        app.marine = Some(MarineList {
            places: vec![MarinePlace {
                name: "BURNHAM".to_owned(),
                latitude: 51.627,
                longitude: 0.806,
                channels: vec![channel("80").unwrap(), channel("16").unwrap()],
            }],
            synced: std::time::SystemTime::now(),
        });
        app.add_marine();
        assert_eq!(app.project.banks.len(), banks + 2);
        let bank = app.project.banks.last().unwrap();
        assert_eq!(bank.name, "SEA JO01HP");
        let names: Vec<_> = bank.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["80 BURNHAM"], "16 is in the shared core");
        assert!(app.project.validate().is_ok());

        // A location with nothing in range needs no core.
        "JO02PP".clone_into(&mut app.repeater_area.locator);
        assert!(app
            .marine_source()
            .starts_with("No local channels within 20 km, so no core"));
        app.add_marine();
        assert_eq!(
            app.status,
            "No marine channels within 20 km of JO02PP, so SEA CORE is not filed there."
        );
        assert!(app.project.location("JO02PP").is_none(), "nothing filed");
        assert!(
            app.project.banks.iter().all(|b| b.name != "SEA JO02PP"),
            "nothing local near Norwich"
        );
        assert!(
            app.project.banks.iter().any(|b| b.name == "SEA JO01HP"),
            "another location's bank stays"
        );

        // Once no location has places in range, the core goes too.
        app.marine_area.radius_km = 1;
        "JO01HP".clone_into(&mut app.repeater_area.locator);
        app.add_marine();
        assert!(app
            .project
            .banks
            .iter()
            .all(|b| !b.name.starts_with("SEA ")));

        app.repeater_area.locator = "nonsense".to_owned();
        app.add_marine();
        assert!(app.status.starts_with("Cannot add marine channels"));
    }

    #[test]
    fn repeaters_from_both_lists_share_one_bank() {
        use crate::{
            repeaters::{Repeater, RepeaterList},
            world::WorldList,
        };

        let repeater = |callsign: &str, latitude: f64| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz: 145_700_000,
            input_hz: 145_100_000,
            ctcss_tenths_hz: Some(885),
            wide: true,
            latitude,
            longitude: 2.35,
            coverage: crate::repeaters::Coverage::Standard,
        };
        let mut app = simulated("world");
        assert!(app.world.is_none(), "tests never read the real cache");
        assert_eq!(app.repeater_source(), "Not synced.");
        let banks = app.project.banks.len();
        app.add_repeaters();
        assert!(
            app.status.starts_with("Cannot add repeaters"),
            "{}",
            app.status
        );

        // hearham's alone is enough outside the UK.
        app.world = Some(WorldList {
            repeaters: vec![repeater("F1FAR", 49.0), repeater("F1NEAR", 48.86)],
            synced: std::time::SystemTime::now(),
        });
        "JN18EU".clone_into(&mut app.repeater_area.locator);
        let source = app.repeater_source();
        assert!(
            source.starts_with("2 heard within 50 km, bank "),
            "{source}"
        );
        assert!(
            source.ends_with("in the UK not synced; 2 from hearham elsewhere, synced just now."),
            "{source}"
        );
        app.add_repeaters();
        let names = |app: &StudioApp| -> Vec<String> {
            let bank = app.project.banks.last().unwrap();
            assert_eq!(bank.name, "RPT JN18EU");
            bank.channels.iter().map(|c| c.name.clone()).collect()
        };
        assert_eq!(names(&app), ["F1NEAR", "F1FAR"]);
        assert!(app.project.validate().is_ok());

        // The ETCC's join them in the one bank, most hearable first.
        app.repeaters = Some(RepeaterList {
            repeaters: vec![repeater("GB3MID", 48.93)],
            gateways: Vec::new(),
            beacons: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        app.add_repeaters();
        assert_eq!(names(&app), ["F1NEAR", "GB3MID", "F1FAR"]);
        app.repeater_area.radius_km = 10;
        app.add_repeaters();
        assert_eq!(app.project.banks.len(), banks + 1, "replaced, not added");
        assert_eq!(names(&app), ["F1NEAR", "GB3MID"]);
    }

    #[test]
    fn the_filters_and_map_view_are_kept_as_they_change() {
        let dir = scratch("filters");
        let mut app = simulated("filters");
        app.remembered.map_view_file = Some(dir.join("map-view"));
        app.airport_options_file = Some(dir.join("airport-options"));
        app.remember_filters();
        assert!(
            !dir.join("map-view").exists(),
            "nothing written while nothing changes"
        );

        app.map_shown = [false; Generator::ALL.len()];
        app.map_shown[Generator::Airports as usize] = true;
        app.map_all_locations = true;
        app.airport_options.per_airport = true;
        app.remember_filters();
        let view = std::fs::read_to_string(dir.join("map-view")).unwrap();
        assert_eq!(view, "airports all-locations\n");
        assert!(crate::airports::AirportOptions::load(&dir.join("airport-options")).per_airport);

        let mut next = simulated("filters-next");
        next.load_map_view(&dir.join("map-view"));
        assert_eq!(next.map_shown, app.map_shown);
        assert!(next.map_all_locations);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_older_projects_generated_banks_are_given_their_origin_once() {
        use super::infer_origin;
        use crate::model::Origin;
        use radio_channel_plan::BankSource;

        let mut app = simulated("origins");
        let mut airport = crate::airports::am_bank(
            "EGSC",
            "",
            vec![crate::airports::am_channel("TWR", 125_900_000)],
        );
        "EGSC CAMBRIDGE C".clone_into(&mut airport.name);
        let mut own = airport.clone();
        "MY AIRBAND".clone_into(&mut own.name);
        let mut rpt = airport.clone();
        "RPT JO02EH".clone_into(&mut rpt.name);
        let mut core = airport.clone();
        "SEA CORE".clone_into(&mut core.name);
        app.project.banks = vec![airport, own, rpt, core];
        let project = &mut app.project;
        assert!(project.file_in("MY AIRBAND", super::AIRBAND_GROUP));
        assert!(project.file_bank("EGSC CAMBRIDGE C", "HOME", "JO02EH", true));
        assert_eq!(
            infer_origin(&app.project, &app.project.banks[1]),
            None,
            "an airband bank at no location is the operator's"
        );

        assert_eq!(app.record_origins(), 3);
        let origins: Vec<_> = app.project.banks.iter().map(|b| b.origin.clone()).collect();
        assert_eq!(
            origins,
            [
                Some(Origin::new(BankSource::Airports, "JO02EH")),
                None,
                Some(Origin::new(BankSource::Repeaters, "JO02EH")),
                Some(Origin::new(BankSource::Marine, "")),
            ]
        );
        // Once a project keeps origins, names are not read again.
        "RPT IO91WM".clone_into(&mut app.project.banks[1].name);
        assert_eq!(app.record_origins(), 0);
        assert_eq!(app.project.banks[1].origin, None);
    }

    #[test]
    fn a_location_group_takes_its_saved_description() {
        let mut app = simulated("location-names");
        "JO02EH".clone_into(&mut app.repeater_area.locator);
        app.add_marine();
        let home = app.project.location("JO02EH").unwrap();
        assert_eq!(app.project.groups[home].name, "JO02EH", "nothing saved yet");

        // Saved as Home without a rename of the group, as an older studio
        // or another project left it: the next add names it.
        app.project.locations.put("JO02EH", "Home").unwrap();
        app.project.locations.put("JO02BE", "Work").unwrap();
        app.add_marine();
        assert_eq!(app.project.groups[home].name, "HOME");

        // A name the operator chose is kept.
        "COTTAGE".clone_into(&mut app.project.groups[home].name);
        assert_eq!(app.name_locations(), 0);
        assert_eq!(app.project.groups[home].name, "COTTAGE");
    }

    #[test]
    fn airfields_start_with_the_sporting_core_unsynced() {
        use crate::airfields::CORE_CHANNELS;
        use crate::model::Origin;
        use radio_channel_plan::BankSource;

        let mut app = simulated("airfields");
        assert!(app.airports.is_none(), "tests never read the real cache");
        let banks = app.project.banks.len();
        app.add_airfields();
        assert!(
            app.status.starts_with("Cannot add airfields"),
            "{}",
            app.status
        );
        assert_eq!(app.project.banks.len(), banks);

        "IO91WM".clone_into(&mut app.repeater_area.locator);
        let source = app.airfield_source();
        assert!(
            source.starts_with("9 sporting and nearest 0 of 0 local frequencies"),
            "{source}"
        );
        assert!(source.ends_with("Not synced: the sporting frequencies only."));
        app.add_airfields();
        let bank = app.project.banks.last().unwrap();
        assert_eq!(bank.name, "FLD CORE");
        assert_eq!(bank.channels.len(), CORE_CHANNELS);
        assert_eq!(bank.channels[0].name, "SAFETYCOM");
        assert!(app.project.validate().is_ok());
        app.add_airfields();
        assert_eq!(app.project.banks.len(), banks + 1, "replaced, not added");

        // A location's bank an older studio made with the core in it loses
        // the core; left with nothing it goes, the core filed in its place.
        let mut old = crate::airfields::core_bank();
        "FLD JO02PP".clone_into(&mut old.name);
        old.origin = Some(Origin::new(BankSource::Airfields, "JO02PP"));
        let mut kept = crate::airfields::core_bank();
        "FLD JO01PX".clone_into(&mut kept.name);
        kept.origin = Some(Origin::new(BankSource::Airfields, "JO01PX"));
        kept.channels
            .push(crate::airports::am_channel("GB-0177", 123_200_000));
        app.project.banks.extend([old, kept]);
        assert!(app
            .project
            .file_bank("FLD JO02PP", "NORWICH", "JO02PP", true));
        assert!(app
            .project
            .file_bank("FLD JO01PX", "FELIXSTOWE", "JO01PX", true));
        app.add_airfields();
        assert!(app.project.banks.iter().all(|b| b.name != "FLD JO02PP"));
        let felixstowe = app.project.banks.iter().find(|b| b.name == "FLD JO01PX");
        let names: Vec<_> = felixstowe
            .unwrap()
            .channels
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["GB-0177"]);
        let norwich = app.project.location("JO02PP").unwrap();
        let here = app.project.location("IO91WM").unwrap();
        let felixstowe = app.project.location("JO01PX").unwrap();
        let core = app
            .project
            .banks
            .iter()
            .find(|b| b.name == "FLD CORE")
            .unwrap();
        assert!(core.groups.contains(&norwich) && core.groups.contains(&here));
        assert!(!core.groups.contains(&felixstowe), "until added there");
        let fld = app
            .project
            .banks
            .iter()
            .find(|b| b.name == "FLD JO01PX")
            .unwrap();
        assert!(!fld.groups.contains(&here), "stays filed where it was");

        // Synced with no small field in range, a location needs no sporting
        // set: the core leaves it, and goes once no location keeps it.
        app.airports = Some(crate::airports::AirportList {
            frequencies: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        assert!(app
            .airfield_source()
            .starts_with("No local frequencies within"));
        app.add_airfields();
        assert!(app.status.ends_with("so FLD CORE is not filed there."));
        let core = app.project.banks.iter().find(|b| b.name == "FLD CORE");
        assert!(!core.unwrap().groups.contains(&here), "Norwich keeps it");
        "JO02PP".clone_into(&mut app.repeater_area.locator);
        app.add_airfields();
        assert!(app.project.banks.iter().all(|b| b.name != "FLD CORE"));
    }

    #[test]
    fn choosing_a_receiver_sets_every_radius_and_keeps_them() {
        use crate::{airports::AirportArea, receiver::Receiver};

        let dir = scratch("receiver");
        let mut app = simulated("receiver");
        app.receiver_file = Some(dir.join("receiver"));
        app.beacon_area_file = Some(dir.join("beacon-area"));
        app.marine_area_file = Some(dir.join("marine-area"));
        assert_eq!(app.receiver, Receiver::Handheld);
        assert_eq!(app.marine_area.radius_km, 10);

        app.choose_receiver(Receiver::Base);
        assert_eq!(app.repeater_area.radius_km, 100);
        assert_eq!(app.gateway_area.radius_km, 50);
        assert_eq!(app.beacon_area.radius_km, 200);
        assert_eq!(app.airport_area.radius_km, 300);
        assert_eq!(app.airfield_area.radius_km, 100);
        assert_eq!(app.marine_area.radius_km, 10);
        assert!(
            app.status.starts_with("Radii set for a base station:"),
            "{}",
            app.status
        );
        assert_eq!(Receiver::load(&dir.join("receiver")), Receiver::Base);
        assert_eq!(AirportArea::load(&dir.join("beacon-area")).radius_km, 200);
        assert_eq!(AirportArea::load(&dir.join("marine-area")).radius_km, 10);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_map_tooltip_keeps_each_line_whole_as_the_pointer_moves() {
        use eframe::egui::{Event, Pos2, Shape};
        let context = eframe::egui::Context::default();
        context.style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let mut app = app_in(&scratch("map-tooltip"), &Options::default());
        app.tab = Tab::Map;
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        // Moving along the map changes the text a little each time; egui
        // keeps the tooltip's last size, which then squeezed it narrower.
        let mut rows = Vec::new();
        for x in [1_050.0, 500.0, 800.0, 1_050.0] {
            for _ in 0..3 {
                let at = Pos2::new(x, 600.0);
                let output = frame(&context, &mut app, vec![Event::PointerMoved(at)]);
                rows.extend(
                    output
                        .shapes
                        .iter()
                        .filter_map(|clipped| match &clipped.shape {
                            Shape::Text(text) if text.galley.text().contains("click to choose") => {
                                Some(text.galley.rows.len())
                            }
                            _ => None,
                        }),
                );
            }
        }
        assert!(!rows.is_empty(), "the tooltip is shown");
        assert!(rows.iter().all(|rows| *rows == 1), "{rows:?}");
    }

    #[test]
    fn every_location_shares_one_bank_of_the_large_airports_approach() {
        use crate::airports::{AirportFrequency, AirportList, AirportSize, Service, MAJOR_BANK};

        let at = |name: &str, size, frequency_hz: u32, km: f64| AirportFrequency {
            name: name.to_owned(),
            ident: name.split(' ').next().unwrap().to_owned(),
            airport: "Field".to_owned(),
            frequency_hz,
            latitude: 51.520_833 + km / 111.2,
            longitude: -0.125,
            size,
            kind: "APP".to_owned(),
            service: Service::Approach,
        };
        let mut app = simulated("major-airports");
        app.airports = Some(AirportList {
            frequencies: vec![
                at("EGLL APP", AirportSize::Large, 119_725_000, 60.0),
                at("EGKK APP", AirportSize::Large, 126_825_000, -90.0),
                at("EGWU APP", AirportSize::Medium, 126_450_000, 5.0),
            ],
            synced: std::time::SystemTime::now(),
        });
        app.airport_area.radius_km = 150;
        for locator in ["IO91WM", "IO92WM"] {
            locator.clone_into(&mut app.repeater_area.locator);
            app.add_airports();
        }
        let bank = |name: &str| app.project.banks.iter().find(|b| b.name == name).unwrap();
        let names = |name: &str| -> Vec<String> {
            bank(name).channels.iter().map(|c| c.name.clone()).collect()
        };
        // Gatwick is 90 km south of IO91WM but past the 150 km radius from
        // IO92WM; the shared bank keeps both locations' airports, once each.
        assert_eq!(names(MAJOR_BANK), ["EGLL APP", "EGKK APP"]);
        assert_eq!(names("AIR IO91WM"), ["EGWU APP"]);
        assert!(app.project.banks.iter().all(|b| b.name != "AIR IO92WM"));
        let filed = &bank(MAJOR_BANK).groups;
        for locator in ["IO91WM", "IO92WM"] {
            let location = app.project.location(locator).unwrap();
            assert!(filed.contains(&location), "{locator}");
        }
        let airband = app.project.group_named(super::AIRBAND_GROUP).unwrap();
        assert!(filed.contains(&airband));
        assert_eq!(
            super::infer_origin(&app.project, bank(MAJOR_BANK)),
            Some(super::Origin::new(super::BankSource::Airports, ""))
        );
    }

    #[test]
    fn a_repeaters_mark_and_rings_follow_its_coverage() {
        use super::{repeater_rings, repeater_sites};
        use crate::{
            map::SiteSize,
            receiver::Receiver,
            repeaters::{Coverage, Repeater, RepeaterReach},
        };

        let at = |coverage| Repeater {
            callsign: "GB3PI".to_owned(),
            town: String::new(),
            output_hz: 145_750_000,
            input_hz: 145_150_000,
            ctcss_tenths_hz: None,
            wide: false,
            latitude: 52.0,
            longitude: 0.0,
            coverage,
        };
        let repeaters = [Coverage::WideArea, Coverage::Standard, Coverage::Local].map(at);
        let near: Vec<_> = repeaters.iter().map(|r| (r, 1.0)).collect();
        let sizes: Vec<_> = repeater_sites(&near).iter().map(|s| s.size).collect();
        assert_eq!(sizes, [SiteSize::Large, SiteSize::Medium, SiteSize::Small]);
        assert_eq!(
            repeater_rings(&RepeaterReach::default(), Receiver::Mobile),
            [
                (45.0, "Standard RPT 45 km".to_owned()),
                (22.0, "Local RPT 22 km".to_owned()),
            ]
        );
    }

    #[test]
    fn an_airports_mark_is_sized_by_the_airport() {
        use super::airport_site;
        use crate::{
            airports::{AirportFrequency, AirportSize, Service},
            map::SiteSize,
        };

        let at = |size| AirportFrequency {
            name: "EGSS APP".to_owned(),
            ident: "EGSS".to_owned(),
            airport: "Stansted".to_owned(),
            frequency_hz: 120_625_000,
            latitude: 51.885,
            longitude: 0.235,
            size,
            kind: "APP".to_owned(),
            service: Service::Approach,
        };
        let sizes = [AirportSize::Small, AirportSize::Medium, AirportSize::Large]
            .map(|size| airport_site(&at(size), false).size);
        assert_eq!(sizes, [SiteSize::Small, SiteSize::Medium, SiteSize::Large]);
        assert!(airport_site(&at(AirportSize::Large), true).highlight);
        let radii = sizes.map(|size| size.points().0);
        assert!(radii[0] < radii[1] && radii[1] < radii[2], "{radii:?}");
    }

    #[test]
    fn the_approach_hover_gives_each_airport_size_its_range() {
        use super::heard_to;
        use crate::{airports::Service, receiver::Receiver};

        let options = crate::airports::AirportOptions::default();
        let text = heard_to(
            Service::Approach,
            "Approach",
            &options,
            Receiver::Handheld,
            120,
        );
        assert!(text.contains("airport radius, 120 km"), "{text}");
        assert!(text.contains("about 50 km"), "{text}");
        assert!(text.contains("small field's 30 km"), "{text}");
        let text = heard_to(Service::Tower, "Tower", &options, Receiver::Base, 120);
        let rings = super::approach_rings(&options, Receiver::Base);
        assert_eq!(
            rings,
            [
                (100.0, "Medium APP 100 km".to_owned()),
                (60.0, "Small APP 60 km".to_owned()),
            ]
        );
        assert!(
            text.ends_with("heard to about 60 km on a base station"),
            "{text}"
        );
    }

    #[test]
    fn the_memory_bar_colours_each_role_and_fits_one_line() {
        use super::{role_colour, usage_bar};
        use crate::model::{BankRole, StorageSummary};
        use eframe::egui::{CentralPanel, Pos2, RawInput, Rect, Shape, Vec2};

        let summary = StorageSummary {
            banks: 40,
            stored_channels: 900,
            expanded_channels: 120,
            plans: 6,
            image_bytes: Some(20_000),
        };
        let context = eframe::egui::Context::default();
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 400.0))),
            ..RawInput::default()
        };
        let output = context.run(input, |context| {
            CentralPanel::default().show(context, |ui| {
                usage_bar(ui, &summary, Some([9_000, 4_000, 6_000]));
            });
        });
        let shapes: Vec<_> = output.shapes.iter().map(|clipped| &clipped.shape).collect();
        for role in BankRole::ALL {
            let fill = role_colour(role);
            assert!(
                shapes
                    .iter()
                    .any(|shape| matches!(shape, Shape::Rect(rect) if rect.fill == fill)),
                "{role:?} is drawn"
            );
        }
        let tops: Vec<f32> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Text(text) => Some(text.pos.y),
                _ => None,
            })
            .collect();
        assert!(tops.len() >= 5, "{tops:?}");
        let (low, high) = tops.iter().fold((f32::MAX, f32::MIN), |(low, high), y| {
            (low.min(*y), high.max(*y))
        });
        assert!(high - low < 10.0, "one line at 1100 points: {tops:?}");
    }

    #[test]
    fn add_all_shown_files_each_bank_under_its_location() {
        use super::{band_counts, counted};
        use crate::repeaters::{AmateurBand, Repeater, RepeaterList};

        let repeater = |callsign: &str, hz: u32| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz: hz,
            input_hz: hz,
            ctcss_tenths_hz: None,
            wide: false,
            latitude: 51.52,
            longitude: -0.12,
            coverage: crate::repeaters::Coverage::Standard,
        };
        let mut app = simulated("add-all");
        app.repeaters = Some(RepeaterList {
            repeaters: vec![
                repeater("GB3TWO", 145_625_000),
                repeater("GB3UHF", 433_100_000),
                repeater("GB3SIX", 51_510_000),
            ],
            gateways: Vec::new(),
            beacons: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        app.project.locations.put("IO91WM", "Home").unwrap();

        // Each band says how many are in range, ticked or not.
        let list = &app.repeaters.as_ref().unwrap().repeaters;
        let counts = band_counts(list, "IO91WM", 50).unwrap();
        assert_eq!(counts[AmateurBand::SixMetres.index()], 1);
        assert_eq!(counts[AmateurBand::TwoMetres.index()], 1);
        assert_eq!(counts[AmateurBand::FourMetres.index()], 0);
        assert_eq!(counted("6 m", Some(1)), "6 m (1)");
        assert_eq!(counted("6 m", None), "6 m");
        assert!(app.station_kind_counts().is_none(), "unsynced");
        assert!(app.airport_service_counts().is_none(), "unsynced");

        app.add_all_shown();
        assert!(
            app.status
                .starts_with("Added or updated Repeaters, Airfields, Marine. Skipped "),
            "{}",
            app.status
        );
        assert!(
            app.status.contains("UK gateways (none in range)"),
            "{}",
            app.status
        );
        let home = app.project.location("IO91WM").unwrap();
        assert_eq!(app.project.groups[home].name, "HOME");
        let rpt = app
            .project
            .banks
            .iter()
            .find(|b| b.name == "RPT IO91WM")
            .unwrap();
        let names: Vec<_> = rpt.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["GB3TWO", "GB3UHF"], "2 m and 70 cm by default");
        assert!(rpt.groups.contains(&home));
        // The marine bank is filed under MARINE too.
        let marine = app.project.group_named(super::MARINE_GROUP).unwrap();
        let sea = app
            .project
            .banks
            .iter()
            .find(|b| b.name == "SEA CORE")
            .unwrap();
        assert!(sea.groups.contains(&home) && sea.groups.contains(&marine));

        // Again: the same banks are updated, not added twice.
        let banks = app.project.banks.len();
        app.add_all_shown();
        assert_eq!(app.project.banks.len(), banks);
        assert!(
            app.status.starts_with("Added or updated Repeaters"),
            "{}",
            app.status
        );

        // A new description renames the location's group.
        "Allotment".clone_into(&mut app.locator_description);
        app.save_locator();
        assert_eq!(app.project.groups[home].name, "ALLOTMENT");
        assert!(
            app.status.ends_with("Renamed its group HOME to ALLOTMENT."),
            "{}",
            app.status
        );
    }

    #[test]
    fn all_locations_shows_and_adds_for_every_saved_locator() {
        use crate::repeaters::{Repeater, RepeaterList};

        let repeater = |callsign: &str, latitude: f64, longitude: f64| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz: 145_625_000,
            input_hz: 145_025_000,
            ctcss_tenths_hz: None,
            wide: false,
            latitude,
            longitude,
            coverage: crate::repeaters::Coverage::Standard,
        };
        let mut app = simulated("all-locations");
        app.repeaters = Some(RepeaterList {
            // Near London and near Manchester.
            repeaters: vec![
                repeater("GB3LDN", 51.52, -0.12),
                repeater("GB3MAN", 53.48, -2.24),
            ],
            gateways: Vec::new(),
            beacons: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        app.project.locations.put("IO91WM", "Home").unwrap();
        app.project.locations.put("IO83WL", "Away").unwrap();
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        app.map_shown = [false; Generator::ALL.len()];
        app.map_shown[0] = true;
        let shown = |app: &mut StudioApp| {
            let layers = app.map_layers();
            let layer = &layers[0];
            let mut names: Vec<_> = layer.sites.iter().map(|s| s.name.clone()).collect();
            names.sort();
            (names, layer.centres.len())
        };

        assert_eq!(shown(&mut app), (vec!["GB3LDN".to_owned()], 1));
        app.map_all_locations = true;
        assert_eq!(
            shown(&mut app),
            (vec!["GB3LDN".to_owned(), "GB3MAN".to_owned()], 2),
            "each saved locator's repeaters and radius"
        );
        // A saved locator with no repeaters in range gets no radius.
        app.project.locations.put("IO70LI", "Cornwall").unwrap();
        assert_eq!(
            shown(&mut app),
            (vec!["GB3LDN".to_owned(), "GB3MAN".to_owned()], 2),
            "no radius where nothing is taken"
        );
        app.project.locations.remove("IO70LI");

        app.add_all_shown();
        assert_eq!(app.status, "Added or updated 2 generators at 2 locations.");
        for (bank, location) in [("RPT IO91WM", "IO91WM"), ("RPT IO83WL", "IO83WL")] {
            let bank = app.project.banks.iter().find(|b| b.name == bank).unwrap();
            let group = app.project.location(location).unwrap();
            assert!(bank.groups.contains(&group), "{location}");
        }
        assert_eq!(app.repeater_area.locator, "IO91WM", "the locator stays");

        // Every location's banks are rewritten: one for a location no longer
        // saved goes, and so do a generator's no longer shown; the
        // operator's own banks and an own-airport bank's group stay.
        let mut stale = app.project.banks[0].clone();
        "RPT JO02EH".clone_into(&mut stale.name);
        stale.origin = Some(crate::model::Origin::new(
            radio_channel_plan::BankSource::Repeaters,
            "JO02EH",
        ));
        let mut own = app.project.banks[0].clone();
        "MY RPT".clone_into(&mut own.name);
        own.origin = None;
        app.project.banks.extend([stale, own]);
        app.add_all_shown();
        assert!(app
            .status
            .starts_with("Dropped 3 location banks and rewrote 2 generators at 2 locations"));
        let names: Vec<_> = app.project.banks.iter().map(|b| b.name.as_str()).collect();
        assert!(!names.contains(&"RPT JO02EH"), "{names:?}");
        assert!(names.contains(&"MY RPT") && names.contains(&"RPT IO83WL"));
        app.map_shown[0] = false;
        app.add_all_shown();
        assert!(app
            .project
            .banks
            .iter()
            .all(|b| !b.name.starts_with("RPT ")));
        assert_eq!(app.status, "Dropped 2 location banks and added nothing.");
        app.map_shown[0] = true;

        // Choosing one locator shows it alone again.
        app.choose_locator("IO83WL");
        assert!(!app.map_all_locations);
        assert_eq!(shown(&mut app), (vec!["GB3MAN".to_owned()], 1));
    }

    #[test]
    fn add_all_shown_adds_past_the_radios_memory() {
        use crate::repeaters::{Repeater, RepeaterList};

        // Full-length names and frequencies too far apart to store short.
        let mut app = simulated("add-all-over");
        app.repeaters = Some(RepeaterList {
            repeaters: (0..500_u32)
                .map(|n| Repeater {
                    callsign: format!("GB3R{n:08}"),
                    town: String::new(),
                    output_hz: 29_000_000 + n * 1_250_000,
                    input_hz: 29_100_000 + n * 1_250_000,
                    ctcss_tenths_hz: None,
                    wide: false,
                    latitude: 51.52,
                    longitude: -0.12,
                    coverage: crate::repeaters::Coverage::Standard,
                })
                .collect(),
            gateways: Vec::new(),
            beacons: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        app.repeater_bands = crate::repeaters::BandFilter::ALL;
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        app.repeater_area.max_channels = crate::repeaters::MAX_CHANNELS_LIMIT;
        // A copy already in the project leaves no room for the bank.
        let mut copy = app.repeater_preview().unwrap().project.banks.pop().unwrap();
        "COPY".clone_into(&mut copy.name);
        app.project.banks.push(copy);
        app.map_shown = [false; Generator::ALL.len()];
        app.map_shown[0] = true;

        app.add_all_shown();
        assert!(
            app.project.banks.iter().any(|b| b.name == "RPT IO91WM"),
            "added anyway"
        );
        let used = app.project.storage_summary().used_per_mille().unwrap();
        assert!(used > 1_000, "{used}");
        assert_eq!(
            app.status,
            format!(
                "Added or updated Repeaters. The project is now {} of the radio's \
                 memory; remove banks before Write.",
                memory_percent(used)
            )
        );
        // A generator's own Add still refuses.
        app.repeater_area.max_channels -= 1;
        app.add_repeaters();
        assert!(app.status.contains("would not fit"), "{}", app.status);
    }

    #[test]
    fn every_map_icon_is_in_the_fonts() {
        use eframe::egui::{Context, FontId, RawInput};
        let context = Context::default();
        let _ = context.run(RawInput::default(), |_| {});
        for generator in Generator::ALL {
            for glyph in generator.icon().chars() {
                let found =
                    context.fonts(|fonts| fonts.has_glyph(&FontId::proportional(13.0), glyph));
                assert!(found, "{generator:?} {glyph}");
            }
        }
    }

    #[test]
    fn the_map_singles_out_the_airports_given_their_own_bank() {
        use crate::airports::{AirportFrequency, AirportList, Service};

        let tower = |ident: &str, km: f64, frequency_hz: u32| AirportFrequency {
            name: format!("{ident} TWR"),
            ident: ident.to_owned(),
            airport: format!("{ident} Field"),
            frequency_hz,
            latitude: 51.520_833 + km / 111.2,
            longitude: -0.125,
            size: crate::airports::AirportSize::Medium,
            kind: "TWR".to_owned(),
            service: Service::Tower,
        };
        let mut app = simulated("map-airports");
        app.airports = Some(AirportList {
            frequencies: vec![
                tower("EGNR", 5.0, 118_500_000),
                tower("EGFA", 10.0, 119_250_000),
            ],
            synced: std::time::SystemTime::now(),
        });
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        let highlighted = |app: &mut StudioApp| {
            let layers = app.map_layers();
            let airports = layers.iter().find(|l| l.label == "Airports").unwrap();
            let mut names: Vec<_> = airports
                .sites
                .iter()
                .map(|site| (site.name.clone(), site.highlight))
                .collect();
            names.sort();
            names
        };

        app.airport_options.per_airport = false;
        assert_eq!(
            highlighted(&mut app),
            [
                ("EGFA TWR".to_owned(), false),
                ("EGNR TWR".to_owned(), false)
            ]
        );
        app.airport_options.per_airport = true;
        app.airport_options.airports = 1;
        assert_eq!(
            highlighted(&mut app),
            [
                ("EGFA TWR".to_owned(), false),
                ("EGNR TWR".to_owned(), true)
            ],
            "only the nearest airport has a bank of its own"
        );
    }

    #[test]
    fn the_map_shows_each_generators_sites_and_picks_a_locator() {
        use crate::repeaters::{Repeater, RepeaterList};
        use eframe::egui::{CentralPanel, Context, Pos2, RawInput, Rect, Vec2};

        let mut app = simulated("map");
        app.repeaters = Some(RepeaterList {
            repeaters: vec![Repeater {
                callsign: "GB3NEAR".to_owned(),
                town: String::new(),
                output_hz: 145_625_000,
                input_hz: 145_025_000,
                ctcss_tenths_hz: None,
                wide: false,
                latitude: 51.52,
                longitude: -0.12,
                coverage: crate::repeaters::Coverage::Standard,
            }],
            gateways: Vec::new(),
            beacons: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        let layers = app.map_layers();
        let repeaters = layers.iter().find(|l| l.label == "Repeaters").unwrap();
        assert_eq!(repeaters.sites.len(), 1);
        assert_eq!(repeaters.sites[0].name, "GB3NEAR");
        assert_eq!(
            repeaters.radius_km,
            Some(f64::from(app.repeater_area.radius_km))
        );

        // A filter change shows at once, with no wait.
        app.repeater_bands.bands[crate::repeaters::AmateurBand::TwoMetres.index()] = false;
        let layers = app.map_layers();
        let repeaters = layers.iter().find(|l| l.label == "Repeaters").unwrap();
        assert!(
            repeaters.sites.is_empty(),
            "the 2 m repeater is filtered out"
        );
        app.repeater_bands = crate::repeaters::BandFilter::ALL;
        app.repeater_area.radius_km = 10;
        let layers = app.map_layers();
        let repeaters = layers.iter().find(|l| l.label == "Repeaters").unwrap();
        assert_eq!(repeaters.radius_km, Some(10.0));
        assert_eq!(repeaters.sites.len(), 1);
        app.map_shown = [false; Generator::ALL.len()];
        assert!(
            app.map_layers().is_empty(),
            "hidden generators are left out"
        );

        // Drawn in a frame: the map centres on the locator, then a pick moves
        // the locator but not the map.
        app.tab = Tab::Map;
        let context = Context::default();
        let input = || RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 800.0))),
            ..RawInput::default()
        };
        let _ = context.run(input(), |context| {
            CentralPanel::default().show(context, |ui| app.map_tab(ui));
        });
        assert_eq!(app.map_locator, "IO91WM");
        let centre = app.map_view.centre;
        app.project.locations.put("JN18EU", "Paris").unwrap();
        app.pick_on_map((48.86, 2.35));
        assert_eq!(app.repeater_area.locator, "JN18EU");
        assert_eq!(app.locator_description, "Paris");
        assert_eq!(app.status, "Chose JN18EU on the map.");
        let _ = context.run(input(), |context| {
            CentralPanel::default().show(context, |ui| app.map_tab(ui));
        });
        assert_eq!(app.map_view.centre, centre, "the map stays put");
    }

    #[test]
    fn locations_are_kept_in_the_project_and_chosen_again() {
        use crate::locators::{SavedLocator, SavedLocators};

        let dir = scratch("saved-locators");
        let mut app = app_in(&dir, &Options::default());
        assert!(
            app.project.locations.list.is_empty(),
            "tests never read the real file"
        );

        "nonsense".clone_into(&mut app.repeater_area.locator);
        app.save_locator();
        assert!(app.status.starts_with("Cannot save"), "{}", app.status);
        "io91wm".clone_into(&mut app.repeater_area.locator);
        "Café".clone_into(&mut app.locator_description);
        app.save_locator();
        assert!(
            app.status.starts_with("Cannot save"),
            "the file holds ASCII"
        );
        "Home".clone_into(&mut app.locator_description);
        app.save_locator();
        assert_eq!(
            app.status,
            "Saved Home (IO91WM) in the project; save the project to keep it."
        );
        assert!(app.is_dirty(), "a saved location is a project change");
        "JN18EU".clone_into(&mut app.repeater_area.locator);
        "Paris".clone_into(&mut app.locator_description);
        app.save_locator();

        app.choose_locator("IO91WM");
        assert_eq!(app.repeater_area.locator, "IO91WM");
        assert_eq!(app.locator_description, "Home");
        app.forget_locator();
        assert_eq!(app.status, "Forgot IO91WM; save the project to keep that.");
        app.forget_locator();
        assert_eq!(app.status, "IO91WM is not saved.");
        assert_eq!(app.repeater_area.locator, "IO91WM", "the field keeps it");

        // The project file keeps them, so another studio opening it has them.
        app.save_project();
        let reopened = app_in(&dir, &Options::default());
        let labels: Vec<_> = reopened
            .project
            .locations
            .list
            .iter()
            .map(SavedLocator::label)
            .collect();
        assert_eq!(labels, ["Paris (JN18EU)"]);

        // A project with none takes the older studios' file, unsaved.
        let mut older = app_in(&scratch("legacy-locators"), &Options::default());
        older.legacy_locators.put("IO83WL", "Away").unwrap();
        older.load_project();
        assert_eq!(older.project.locations, older.legacy_locators);
        assert!(older.is_dirty());
        assert!(
            older.status.contains("Took 1 saved location"),
            "{}",
            older.status
        );
        // One that keeps its own is left alone.
        let mut kept = app_in(&dir, &Options::default());
        kept.legacy_locators = SavedLocators::default();
        kept.legacy_locators.put("IO83WL", "Away").unwrap();
        kept.load_project();
        assert_eq!(kept.project.locations.list.len(), 1);
        assert!(!kept.is_dirty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn repeaters_are_taken_from_the_ticked_bands() {
        use crate::{
            repeaters::{AmateurBand, BandFilter, Repeater, RepeaterList},
            world::WorldList,
        };

        let repeater = |callsign: &str, hz: u32| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz: hz,
            input_hz: hz - 600_000,
            ctcss_tenths_hz: None,
            wide: false,
            latitude: 51.52,
            longitude: -0.12,
            coverage: crate::repeaters::Coverage::Standard,
        };
        let names = |app: &StudioApp| -> Vec<String> {
            let bank = app.project.banks.iter().find(|b| b.name == "RPT IO91WM");
            bank.unwrap()
                .channels
                .iter()
                .map(|c| c.name.clone())
                .collect()
        };
        let mut app = simulated("repeater-bands");
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        app.repeaters = Some(RepeaterList {
            repeaters: vec![
                repeater("GB3TWO", 145_625_000),
                repeater("GB3UHF", 433_100_000),
            ],
            gateways: Vec::new(),
            beacons: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        app.world = Some(WorldList {
            repeaters: vec![repeater("F1TWO", 145_650_000)],
            synced: std::time::SystemTime::now(),
        });
        assert_eq!(app.repeater_bands, BandFilter::default(), "2 m and 70 cm");
        app.add_repeaters();
        assert_eq!(names(&app), ["F1TWO", "GB3TWO", "GB3UHF"]);

        app.repeater_bands.bands[AmateurBand::TwoMetres.index()] = false;
        app.add_repeaters();
        assert_eq!(names(&app), ["GB3UHF"], "both lists take the bands");
    }

    #[test]
    fn gateways_and_beacons_come_from_the_repeater_list() {
        use crate::repeaters::{AmateurBand, BandFilter, Repeater, RepeaterList};

        let system = |callsign: &str, hz: u32, latitude: f64| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz: hz,
            input_hz: hz,
            ctcss_tenths_hz: None,
            wide: false,
            latitude,
            longitude: -0.12,
            coverage: crate::repeaters::Coverage::Standard,
        };
        let mut app = simulated("etcc-kinds");
        assert_eq!(app.etcc_source(EtccKind::Beacons), "Not synced.");
        let banks = app.project.banks.len();
        app.add_etcc(EtccKind::Beacons);
        assert_eq!(app.project.banks.len(), banks, "nothing without a sync");
        assert!(
            app.status.starts_with("Cannot add beacons"),
            "{}",
            app.status
        );

        app.repeaters = Some(RepeaterList {
            repeaters: Vec::new(),
            gateways: vec![
                system("MB7NEAR", 145_337_500, 51.52),
                system("MB7SIX", 51_810_000, 51.53),
            ],
            beacons: vec![
                system("GB3VHF", 144_430_000, 51.3),
                // About 50 km north: inside the beacons' wider default.
                system("GB3FAR", 432_490_000, 51.95),
            ],
            synced: std::time::SystemTime::now(),
        });
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        assert_eq!(app.beacon_area.radius_km, 60);
        assert_eq!(app.gateway_area.radius_km, 20);
        assert_eq!(app.airport_area.radius_km, 150);
        assert_eq!(app.airfield_area.radius_km, 50);
        assert_eq!(app.marine_area.radius_km, 10);
        let source = app.etcc_source(EtccKind::Beacons);
        assert!(source.starts_with("2 within 60 km, bank "), "{source}");
        assert!(source.ends_with("2 beacons from the ETCC, synced just now."));
        app.add_etcc(EtccKind::Beacons);
        let bank = app.project.banks.last().unwrap();
        assert_eq!(bank.name, "BCN IO91WM");
        let names: Vec<_> = bank.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["GB3VHF", "GB3FAR"]);
        assert!(bank
            .channels
            .iter()
            .all(|c| c.modulation == Modulation::Usb));
        assert_eq!(app.beacon_bands, BandFilter::default(), "2 m and 70 cm");
        app.beacon_bands.bands[AmateurBand::TwoMetres.index()] = false;
        app.add_etcc(EtccKind::Beacons);
        assert_eq!(app.project.banks.len(), banks + 1, "replaced, not added");
        let names: Vec<_> = app
            .project
            .banks
            .last()
            .unwrap()
            .channels
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["GB3FAR"]);

        app.gateway_area.radius_km = 10;
        app.add_etcc(EtccKind::Gateways);
        assert_eq!(app.project.banks.len(), banks + 2);
        let bank = app.project.banks.last().unwrap();
        assert_eq!(bank.name, "GW IO91WM");
        assert_eq!(bank.channels.len(), 1, "6 m is off by default");
        assert_eq!(bank.channels[0].transmit_mhz, "145.337500");
        assert!(app.project.validate().is_ok());

        app.gateway_bands.bands[AmateurBand::SixMetres.index()] = true;
        app.add_etcc(EtccKind::Gateways);
        assert_eq!(app.project.banks.len(), banks + 2, "replaced, not added");
        let names: Vec<_> = app
            .project
            .banks
            .last()
            .unwrap()
            .channels
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, ["MB7NEAR", "MB7SIX"]);
        app.gateway_bands.bands = [false; AmateurBand::ALL.len()];
        app.add_etcc(EtccKind::Gateways);
        assert_eq!(app.status, "No gateways within 10 km of IO91WM.");

        app.beacon_area.radius_km = 10;
        app.add_etcc(EtccKind::Beacons);
        assert_eq!(app.status, "No beacons within 10 km of IO91WM.");
        assert_eq!(app.project.banks.len(), banks + 2);
    }

    #[test]
    fn repeaters_heard_at_a_locator_are_added_most_hearable_first_once_synced() {
        use crate::repeaters::{Coverage, Repeater, RepeaterArea, RepeaterList};

        let repeater = |callsign: &str, latitude: f64, longitude: f64| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz: 145_625_000,
            input_hz: 145_025_000,
            ctcss_tenths_hz: Some(825),
            wide: false,
            latitude,
            longitude,
            coverage: crate::repeaters::Coverage::Standard,
        };
        let mut app = simulated("repeaters");
        assert!(app.repeaters.is_none(), "tests never read the real cache");
        assert_eq!(app.repeater_source(), "Not synced.");
        let banks = app.project.banks.len();
        app.add_repeaters();
        assert_eq!(app.project.banks.len(), banks, "nothing without a sync");

        let sized = |callsign: &str, latitude: f64, coverage| Repeater {
            coverage,
            ..repeater(callsign, latitude, -0.12)
        };
        // A wide-area repeater 42 km off is heard across the 50 km radius, a
        // local one only within 15 km.
        app.repeaters = Some(RepeaterList {
            repeaters: vec![
                sized("GB3FAR", 51.9, Coverage::WideArea),
                repeater("GB3NEAR", 51.52, -0.12),
                sized("GB3TOWN", 51.62, Coverage::Local),
                sized("GB3QUIET", 51.75, Coverage::Local),
                repeater("GB3OUT", 53.0, -0.1),
            ],
            gateways: Vec::new(),
            beacons: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        app.repeater_area = RepeaterArea {
            locator: "io91wm".to_owned(),
            radius_km: 50,
            max_channels: 40,
        };
        let source = app.repeater_source();
        assert!(
            source.starts_with("3 heard within 50 km, bank "),
            "{source}"
        );
        assert!(source.contains("% after adding."), "{source}");

        // The limit takes the most hearable only.
        app.repeater_area.max_channels = 1;
        let source = app.repeater_source();
        assert!(
            source.starts_with("3 heard within 50 km, most hearable 1 taken, bank "),
            "{source}"
        );
        app.add_repeaters();
        assert_eq!(app.project.banks.last().unwrap().channels.len(), 1);
        app.repeater_area.max_channels = 40;
        app.add_repeaters();
        let bank = app.project.banks.last().unwrap();
        assert_eq!(bank.name, "RPT IO91WM");
        let names: Vec<_> = bank.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["GB3NEAR", "GB3TOWN", "GB3FAR"]);
        assert!(app.project.validate().is_ok());
        // A better aerial hears the quiet one too.
        app.receiver = super::Receiver::Base;
        assert!(app.repeater_source().starts_with("4 heard"));
        app.receiver = super::Receiver::Handheld;
        // Filed under a location for the locator (`ADR-116`).
        let home = app.project.location("IO91WM").expect("a location");
        assert_eq!(bank.groups, [home]);

        // Adding again replaces the bank rather than adding a second.
        app.repeater_area.radius_km = 10;
        app.add_repeaters();
        assert_eq!(app.project.banks.len(), banks + 1);
        assert_eq!(app.project.banks.last().unwrap().channels.len(), 1);
        assert_eq!(
            app.project.banks.last().unwrap().groups,
            [home],
            "a regenerated bank stays filed"
        );

        app.repeater_area.locator = "nonsense".to_owned();
        app.add_repeaters();
        assert_eq!(app.project.banks.len(), banks + 1);
    }

    #[test]
    fn nearby_locations_hearing_the_same_share_one_bank() {
        use crate::repeaters::{Coverage, Repeater, RepeaterList};

        let repeater = |callsign: &str, latitude: f64, coverage| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz: 145_625_000,
            input_hz: 145_025_000,
            ctcss_tenths_hz: Some(825),
            wide: false,
            latitude,
            longitude: -0.125,
            coverage,
        };
        let mut app = simulated("share-banks");
        app.project.locations.put("IO91WM", "Home").unwrap();
        app.project.locations.put("IO91WL", "Work").unwrap();
        let synced = |repeaters| {
            Some(RepeaterList {
                repeaters,
                gateways: Vec::new(),
                beacons: Vec::new(),
                synced: std::time::SystemTime::now(),
            })
        };
        let near = repeater("GB3NEAR", 51.53, Coverage::Standard);
        // Wide-area, so heard from both, but most hearable second from Home
        // and first from Work.
        let mut far = repeater("GB3FAR", 51.42, Coverage::WideArea);
        far.output_hz = 145_750_000;
        // Local, 11 km from IO91WL's centre and 16 km from IO91WM's.
        let south = repeater("GB3SOUTH", 51.38, Coverage::Local);
        let add_at = |app: &mut StudioApp, locator: &str| {
            locator.clone_into(&mut app.repeater_area.locator);
            app.add_repeaters();
        };
        let rpt = |app: &StudioApp| -> Vec<(String, Vec<String>)> {
            app.project
                .banks
                .iter()
                .filter(|b| b.name.starts_with("RPT"))
                .map(|b| {
                    let groups = b
                        .groups
                        .iter()
                        .map(|g| app.project.groups[*g].name.clone())
                        .collect();
                    (b.name.clone(), groups)
                })
                .collect()
        };
        let shared = || {
            vec![(
                "RPT HOME+WORK".to_owned(),
                vec!["HOME".to_owned(), "WORK".to_owned()],
            )]
        };

        app.repeaters = synced(vec![near.clone(), far.clone()]);
        let pair = [near.clone(), far.clone()];
        let order = |locator: &str| -> Vec<String> {
            let reach = crate::repeaters::RepeaterReach::default();
            crate::repeaters::hearable(&pair, locator, 50, &reach, super::Receiver::Handheld)
                .unwrap()
                .iter()
                .map(|(r, _)| r.callsign.clone())
                .collect()
        };
        assert_eq!(order("IO91WM"), ["GB3NEAR", "GB3FAR"]);
        assert_eq!(order("IO91WL"), ["GB3FAR", "GB3NEAR"]);
        add_at(&mut app, "IO91WM");
        add_at(&mut app, "IO91WL");
        assert_eq!(rpt(&app), shared());
        let bank = app.project.banks.iter().find(|b| b.name == "RPT HOME+WORK");
        assert_eq!(bank.unwrap().origin.as_ref().unwrap().locator, "");

        // Work now hears one Home does not: each has its own bank again.
        app.repeaters = synced(vec![near.clone(), far.clone(), south.clone()]);
        add_at(&mut app, "IO91WL");
        assert_eq!(
            rpt(&app),
            [
                ("RPT IO91WM".to_owned(), vec!["HOME".to_owned()]),
                ("RPT IO91WL".to_owned(), vec!["WORK".to_owned()]),
            ]
        );
        add_at(&mut app, "IO91WM");
        assert_eq!(rpt(&app).len(), 2, "still different");

        // The same again, they share again.
        app.repeaters = synced(vec![near.clone(), far.clone()]);
        add_at(&mut app, "IO91WL");
        assert_eq!(rpt(&app), shared());
        // Adding at one of them again keeps the one bank.
        add_at(&mut app, "IO91WM");
        assert_eq!(rpt(&app), shared());

        // Four in common and one Work alone hears: mostly the same, so they
        // share one bank of all five.
        let mut more = vec![near.clone(), far.clone(), south.clone()];
        for (n, latitude) in [(1, 51.50), (2, 51.51)] {
            let mut also = repeater(&format!("GB3ALSO{n}"), latitude, Coverage::Standard);
            also.output_hz = 145_575_000 + n * 12_500;
            more.push(also);
        }
        app.repeaters = synced(more);
        add_at(&mut app, "IO91WM");
        add_at(&mut app, "IO91WL");
        assert_eq!(rpt(&app), shared());
        let bank = app.project.banks.iter().find(|b| b.name == "RPT HOME+WORK");
        assert_eq!(bank.unwrap().channels.len(), 5);
    }

    #[test]
    fn banks_share_when_three_quarters_of_their_channels_are_common() {
        use super::overlap_enough;
        use crate::model::{BankDraft, ChannelDraft};

        let bank = |names: &[&str]| BankDraft {
            channels: names
                .iter()
                .map(|name| ChannelDraft {
                    name: (*name).to_owned(),
                    ..ChannelDraft::default()
                })
                .collect(),
            ..BankDraft::default()
        };
        // Order does not matter.
        assert!(overlap_enough(&bank(&["A", "B"]), &bank(&["B", "A"])));
        // Three of four is enough, two of three is not.
        assert!(overlap_enough(
            &bank(&["A", "B", "C"]),
            &bank(&["A", "B", "C", "D"])
        ));
        assert!(!overlap_enough(&bank(&["A", "B"]), &bank(&["A", "B", "C"])));
        assert!(!overlap_enough(
            &bank(&["A", "B", "C"]),
            &bank(&["A", "B", "D"])
        ));
    }

    #[test]
    fn more_repeaters_than_a_radio_holds_say_so_in_their_cost() {
        use crate::repeaters::{Repeater, RepeaterArea, RepeaterList};

        let repeater = |callsign: &str, latitude: f64, longitude: f64| Repeater {
            callsign: callsign.to_owned(),
            town: String::new(),
            output_hz: 145_625_000,
            input_hz: 145_025_000,
            ctcss_tenths_hz: Some(825),
            wide: false,
            latitude,
            longitude,
            coverage: crate::repeaters::Coverage::Standard,
        };
        let mut app = simulated("repeaters-cost");
        app.repeaters = Some(RepeaterList {
            repeaters: vec![repeater("GB3NEAR", 51.52, -0.12)],
            gateways: Vec::new(),
            beacons: Vec::new(),
            synced: std::time::SystemTime::now(),
        });
        app.repeater_area = RepeaterArea {
            locator: "IO91WM".to_owned(),
            radius_km: 50,
            max_channels: 40,
        };
        app.add_repeaters();

        // More repeaters than a radio holds: the cost says so.
        // Full-length names and frequencies too far apart to store short.
        app.repeaters.as_mut().unwrap().repeaters = (0..500_u32)
            .map(|n| Repeater {
                output_hz: 29_000_000 + n * 1_250_000,
                input_hz: 29_100_000 + n * 1_250_000,
                ..repeater(&format!("GB3R{n:08}"), 51.52, -0.12)
            })
            .collect();
        // They spread past 2 m and 70 cm, so every band is taken.
        app.repeater_bands = crate::repeaters::BandFilter::ALL;
        app.repeater_area.locator = "IO91WM".to_owned();
        app.repeater_area.max_channels = crate::repeaters::MAX_CHANNELS_LIMIT;
        let source = app.repeater_source();
        assert!(
            // The IO91WM location the first add made costs its 0.5%.
            source.contains(", bank 95.0%, project 95.9% after"),
            "{source}"
        );
        // With a second copy already in the project, the two do not fit.
        let mut copy = app.repeater_preview().unwrap().project.banks.pop().unwrap();
        "COPY".clone_into(&mut copy.name);
        app.project.banks.push(copy);
        let source = app.repeater_source();
        let after = app.repeater_preview().unwrap().project.storage_summary();
        let over = after.used_per_mille().unwrap();
        assert!(over > 1_900, "both copies counted past 100%: {over}");
        let said = format!(
            ", project {} after adding, too big for the radio.",
            memory_percent(over)
        );
        assert!(source.contains(&said), "{source}");
        let before = app.project.banks.len();
        app.add_repeaters();
        assert_eq!(app.project.banks.len(), before, "refused, not added");
        assert!(app.status.contains("would not fit"), "{}", app.status);
    }

    #[test]
    fn a_locator_edit_is_kept_once_it_reads_as_one() {
        use crate::repeaters::RepeaterArea;

        let mut app = simulated("locator");
        let file = scratch("locator-state").join("repeater-area");
        app.repeater_area_file = Some(file.clone());
        app.repeater_area.locator = "JO0".to_owned();
        app.remember_locator();
        assert!(!file.exists(), "half a locator is not kept");
        app.repeater_area.locator = "JO02eh".to_owned();
        app.remember_locator();
        assert_eq!(RepeaterArea::load(&file).locator, "JO02eh");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn every_locations_stations_share_one_bank_each_frequency_once() {
        use crate::stations::{Station, StationKind, StationList};

        let station = |name: &str, mhz: u32, latitude: f64| Station {
            name: name.to_owned(),
            station: name.to_owned(),
            site: String::new(),
            frequency_hz: mhz * 100_000,
            erp_watts: 125_000.0,
            height_m: None,
            latitude,
            longitude: -0.12,
            kind: StationKind::Local,
        };
        let fm = |app: &StudioApp| -> Vec<String> {
            let shared: Vec<_> = app
                .project
                .banks
                .iter()
                .filter(|b| b.name == "FM STATIONS")
                .collect();
            assert_eq!(shared.len(), 1);
            shared[0].channels.iter().map(|c| c.name.clone()).collect()
        };
        // SOUTH is strongest at IO91WM, NORTH at IO94WA, both on 97.7 MHz.
        let mut app = simulated("stations-shared");
        app.stations = Some(StationList {
            stations: vec![station("SOUTH", 977, 51.9), station("NORTH", 977, 54.0)],
            synced: std::time::SystemTime::now(),
            complete: true,
        });
        "IO91WM".clone_into(&mut app.repeater_area.locator);
        app.station_area.kinds = [true; StationKind::ALL.len()];
        app.station_area.max_channels = 1;
        app.add_stations();
        assert_eq!(fm(&app), ["SOUTH"]);

        // A one-location bank an older studio made is folded in, and the
        // shared bank is filed where it was.
        let mut old = app.project.banks.last().unwrap().clone();
        "FM IO94WA".clone_into(&mut old.name);
        old.origin = Some(crate::model::Origin::new(
            radio_channel_plan::BankSource::Stations,
            "IO94WA",
        ));
        old.groups.clear();
        app.project.banks.push(old);
        assert!(app.project.file_bank("FM IO94WA", "NORTH", "IO94WA", false));
        app.add_stations();
        assert!(app.project.banks.iter().all(|b| b.name != "FM IO94WA"));
        let north = app.project.location("IO94WA").unwrap();
        let home = app.project.location("IO91WM").unwrap();
        let bank = app.project.banks.iter().find(|b| b.name == "FM STATIONS");
        assert!(bank.is_some_and(|b| b.groups.contains(&north) && b.groups.contains(&home)));
        assert_eq!(fm(&app), ["SOUTH"], "this locator's station keeps 97.7");

        // On their own frequencies, each location's strongest is held.
        app.stations.as_mut().unwrap().stations[1].frequency_hz = 100_900_000;
        app.add_stations();
        assert_eq!(fm(&app), ["SOUTH", "NORTH"]);
    }

    #[test]
    fn stations_near_the_locator_are_added_strongest_first_once_synced() {
        use crate::stations::{Station, StationArea, StationKind, StationList};
        use radio_domain::Modulation;

        let station = |name: &str, mhz: u32, kw: f64, latitude: f64| Station {
            name: name.to_owned(),
            station: name.to_owned(),
            site: String::new(),
            frequency_hz: mhz * 100_000,
            erp_watts: kw * 1_000.0,
            height_m: None,
            latitude,
            longitude: -0.12,
            kind: StationKind::Local,
        };
        let mut app = simulated("stations");
        assert!(app.stations.is_none(), "tests never read the real cache");
        assert_eq!(app.station_source(), "Not synced.");
        let banks = app.project.banks.len();
        app.add_stations();
        assert_eq!(app.project.banks.len(), banks, "nothing without a sync");

        app.stations = Some(StationList {
            stations: vec![
                station("CLASSIC", 1_009, 125.0, 51.9),
                station("LOCAL", 1_062, 0.05, 51.52),
                station("FAR AWAY", 943, 125.0, 54.0),
            ],
            synced: std::time::SystemTime::now(),
            complete: true,
        });
        app.repeater_area.locator = "io91wm".to_owned();
        assert_eq!(
            app.station_area.kinds,
            StationArea::NO_BBC,
            "every kind but the BBC by default"
        );
        app.station_area = StationArea {
            max_channels: 30,
            kinds: [true; StationKind::ALL.len()],
        };
        let source = app.station_source();
        assert!(
            source.starts_with("Strongest 3 of 3 stations, furthest transmitter "),
            "{source}"
        );

        // The limit takes the strongest: 125 kW at 42 km beats 50 W nearby.
        app.station_area.max_channels = 1;
        app.add_stations();
        let bank = app.project.banks.last().unwrap();
        assert_eq!(bank.name, "FM STATIONS");
        let names: Vec<_> = bank.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["CLASSIC"]);

        // Again with room for all: the same bank, now in frequency order, and
        // with no distance cut-off the far transmitter is taken too.
        app.station_area.max_channels = 30;
        app.add_stations();
        assert_eq!(app.project.banks.len(), banks + 1);
        let bank = app.project.banks.last().unwrap();
        let names: Vec<_> = bank.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["FAR AWAY", "CLASSIC", "LOCAL"]);
        assert!(bank
            .channels
            .iter()
            .all(|c| c.modulation == Modulation::Broadcast));
        assert!(app.project.validate().is_ok());
        assert!(
            app.status.starts_with("Added the 3 strongest FM stations"),
            "{}",
            app.status
        );

        // Without the BBC, the weaker station sharing its frequency plays.
        let list = app.stations.as_mut().unwrap();
        list.stations.push(Station {
            kind: StationKind::Bbc,
            ..station("BBC R4", 1_062, 125.0, 51.9)
        });
        list.stations.push(Station {
            kind: StationKind::Community,
            ..station("TOWN FM", 1_070, 0.05, 51.52)
        });
        app.add_stations();
        let last_names = |app: &StudioApp| -> Vec<String> {
            let bank = app.project.banks.last().unwrap();
            bank.channels.iter().map(|c| c.name.clone()).collect()
        };
        assert_eq!(
            last_names(&app),
            ["FAR AWAY", "CLASSIC", "BBC R4", "TOWN FM"]
        );
        app.station_area.kinds = StationArea::NO_BBC;
        app.add_stations();
        assert_eq!(app.project.banks.len(), banks + 1, "replaced, not added");
        assert_eq!(
            last_names(&app),
            ["FAR AWAY", "CLASSIC", "LOCAL", "TOWN FM"]
        );
        // Only community radio.
        app.station_area.kinds = StationKind::ALL.map(|kind| kind == StationKind::Community);
        app.add_stations();
        assert_eq!(last_names(&app), ["TOWN FM"]);
    }

    #[test]
    fn adding_a_preset_keeps_the_existing_banks() {
        let mut app = simulated("add-preset");
        app.preset = preset_index("UK and EU simplex");
        app.add_preset();
        let before: Vec<_> = app.project.banks.iter().map(|b| b.name.clone()).collect();

        app.preset = preset_index("Marine VHF");
        app.add_preset();
        let after: Vec<_> = app.project.banks.iter().map(|b| b.name.clone()).collect();
        assert_eq!(after[..before.len()], before[..]);
        assert_eq!(after[before.len()..], ["MARINE DUPLEX", "MARINE SIMPLEX"]);
        assert!(app.project.validate().is_ok());

        // Adding the same set again duplicates nothing.
        app.add_preset();
        assert_eq!(app.project.banks.len(), after.len());

        // A default set is in COMMON, so every group on the radio has it;
        // one added before COMMON was is filed there when added again.
        let common = app.project.group_named(super::COMMON_GROUP).unwrap();
        assert!(app.project.banks.iter().all(|b| b.groups == [common]));
        assert!(app
            .status
            .ends_with("is already in the project, in COMMON."));
        for bank in &mut app.project.banks {
            bank.groups.clear();
        }
        app.add_preset();
        let marine = app.project.banks.iter().filter(|b| b.groups == [common]);
        assert_eq!(marine.count(), 2);
    }

    #[test]
    fn adding_a_preset_past_the_bank_limit_adds_nothing() {
        let mut app = simulated("add-preset-full");
        while app.project.banks.len() < usize::from(radio_channel_plan::MAX_BANKS) - 1 {
            app.project.add_bank();
            let last = app.project.banks.len() - 1;
            app.project.banks[last].name = format!("B{last}");
        }
        let full = app.project.banks.len();
        app.preset = preset_index("Marine VHF");
        app.add_preset();
        assert_eq!(app.project.banks.len(), full);
        assert!(app.status.contains("needs 2 more banks"), "{}", app.status);
    }

    fn tx_preset_index(name: &str) -> usize {
        crate::presets::TX_RANGE_PRESETS
            .iter()
            .position(|p| p.name() == name)
            .expect("TX preset")
    }

    #[test]
    fn tx_range_presets_add_after_the_existing_ranges_without_duplicates() {
        let mut app = simulated("add-tx-preset");
        app.tx_preset = tx_preset_index("UK");
        app.add_tx_preset();
        assert_eq!(app.project.tx_ranges.len(), 3);
        // EU shares all three with UK, so nothing is added.
        app.tx_preset = tx_preset_index("EU (CEPT)");
        app.add_tx_preset();
        assert_eq!(app.project.tx_ranges.len(), 3);
        assert!(app.status.contains("already"), "{}", app.status);

        app.tx_preset = tx_preset_index("Marine VHF");
        app.add_tx_preset();
        assert_eq!(app.project.tx_ranges.len(), 4);
        assert_eq!(app.project.tx_ranges[3].class, TxClass::Marine);
        let compiled = app.project.validate().expect("validates");
        assert_eq!(compiled.tx_ranges().len(), 4);
    }

    #[test]
    fn a_tx_preset_past_the_range_limit_adds_nothing() {
        let mut app = simulated("add-tx-preset-full");
        for index in 0..MAX_TX_RANGES - 1 {
            app.project.tx_ranges.push(TxRangeDraft {
                first_mhz: format!("{}.000000", 430 + index),
                last_mhz: format!("{}.500000", 430 + index),
                ..TxRangeDraft::default()
            });
        }
        app.tx_preset = tx_preset_index("UK");
        app.add_tx_preset();
        assert_eq!(app.project.tx_ranges.len(), MAX_TX_RANGES - 1);
        assert!(
            app.status.contains("needs 3 more TX ranges"),
            "{}",
            app.status
        );
    }
}
