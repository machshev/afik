//! The Flash tab: a guided, automatic flow with manual fallbacks.
//!
//! The tab scans for serial devices and selects the first, identifies what is
//! on it without writing to a bootloader, works out the radio family, picks the
//! family's packaged image, and offers the AFIK application write. While the
//! radio is not in flashing mode it keeps checking and says what to do. Before
//! the first flash of a family it asks for an EEPROM backup, but never insists:
//! not every firmware can serve one. Every automatic choice can be overridden,
//! and the gated recovery operations stay available under Advanced.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, ComboBox, RichText, TextEdit};
use radio_flasher::{serves_eeprom_reads, RadioPresence};

use crate::{
    device::{self, DeviceCandidate},
    flash::{
        self, DigestCache, FlashJob, FlashOperation, FlashProgress, FlashRequest,
        FlashRequestError, IdentifyJob, RadioFamily,
    },
    paths,
};

/// How often an absent device list is rescanned and a radio not yet in
/// flashing mode is checked again.
pub const RECHECK: Duration = Duration::from_secs(3);
const WARNING_COLOUR: Color32 = Color32::from_rgb(0xB7, 0x6E, 0x00);
const READY_COLOUR: Color32 = Color32::from_rgb(0x2E, 0x7D, 0x32);
const ERROR_COLOUR: Color32 = Color32::from_rgb(0xC0, 0x39, 0x2B);

/// What the last identification of one device found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Finding {
    /// A radio in flashing mode: the only state a write can start from.
    Bootloader {
        /// Family the beacon belongs to.
        family: RadioFamily,
        /// Bootloader version from the beacon.
        version: String,
    },
    /// A radio running its firmware, which answered the hello.
    Firmware {
        /// Family recognised from the version, if it could be.
        family: Option<RadioFamily>,
        /// Firmware version from the hello.
        version: String,
        /// Whether this firmware can serve an EEPROM backup.
        serves_backup: bool,
    },
    /// Nothing usable answered; the text says why.
    Nothing(String),
}

impl Finding {
    /// Interprets one identification outcome.
    pub fn from_outcome(outcome: Result<RadioPresence, FlashRequestError>) -> Self {
        match outcome {
            Ok(RadioPresence::Bootloader(family)) => Self::Bootloader {
                family: RadioFamily::of_bootloader(&family),
                version: family.info().version().to_owned(),
            },
            Ok(RadioPresence::Firmware(info)) => Self::Firmware {
                family: RadioFamily::of_firmware(info.version()),
                version: info.version().to_owned(),
                serves_backup: serves_eeprom_reads(info.version()),
            },
            Err(error) => Self::Nothing(error.to_string()),
        }
    }

    /// Returns the family this finding names, if any.
    pub const fn family(&self) -> Option<RadioFamily> {
        match self {
            Self::Bootloader { family, .. } => Some(*family),
            Self::Firmware { family, .. } => *family,
            Self::Nothing(_) => None,
        }
    }
}

/// Where the EEPROM backup step stands for the current family.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackupState {
    /// The family is not known yet, so there is nothing to look for.
    NoFamily,
    /// The newest backup kept for this family.
    OnFile(PathBuf),
    /// None is kept, and the operator chose to go on without one.
    Skipped,
    /// None is kept: the operator is asked to take one.
    Missing,
}

/// Which kind of job is running, for what happens when it ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JobKind {
    Flash,
    Backup,
    Advanced,
}

/// Manual operations kept behind the Advanced section.
struct Advanced {
    operation: FlashOperation,
    request: FlashRequest,
    crc32: String,
}

/// The complete state of the Flash tab.
pub struct FlashTab {
    devices: Vec<DeviceCandidate>,
    device: PathBuf,
    /// The operator chose the device, so a rescan does not replace it.
    device_chosen: bool,
    last_scan: Option<Instant>,
    finding: Option<(PathBuf, Finding)>,
    /// The firmware version a device last answered with, kept while the radio
    /// sits in flashing mode, where only the bootloader answers.
    last_firmware: Option<(PathBuf, String)>,
    identify: Option<(IdentifyJob, PathBuf)>,
    last_identify: Option<Instant>,
    family_override: Option<RadioFamily>,
    /// Flash as the chosen family whatever bootloader version answers.
    force: bool,
    image_override: Option<PathBuf>,
    /// Where EEPROM backups are kept, one directory per family.
    eeprom_root: Option<PathBuf>,
    skipped_backup: HashSet<RadioFamily>,
    pending_backup: bool,
    job: Option<(FlashJob, JobKind, String)>,
    progress: Option<(u16, u16)>,
    status: String,
    digests: DigestCache,
    advanced: Advanced,
}

impl Default for FlashTab {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            device: PathBuf::new(),
            device_chosen: false,
            last_scan: None,
            finding: None,
            last_firmware: None,
            identify: None,
            last_identify: None,
            family_override: None,
            force: false,
            image_override: None,
            eeprom_root: paths::eeprom_dir(),
            skipped_backup: HashSet::new(),
            pending_backup: false,
            job: None,
            progress: None,
            status: String::new(),
            digests: DigestCache::default(),
            advanced: Advanced {
                operation: FlashOperation::K1Recovery,
                request: FlashRequest {
                    eeprom_output: paths::default_eeprom_backup(),
                    ..FlashRequest::default()
                },
                crc32: String::new(),
            },
        }
    }
}

fn due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= RECHECK)
}

impl FlashTab {
    /// Reports whether the tab is waiting on a worker and must keep repainting.
    pub const fn busy(&self) -> bool {
        self.job.is_some() || self.identify.is_some()
    }

    /// Advances the automatic flow: collects results, scans, and re-checks.
    ///
    /// Called every frame while the tab is shown. The port is only ever opened
    /// by one worker at a time.
    pub fn tick(&mut self, now: Instant) {
        self.poll_job();
        self.poll_identify();
        if self.busy() {
            return;
        }
        if self.pending_backup {
            self.pending_backup = false;
            self.start_backup();
            return;
        }
        if self.devices.is_empty() && due(self.last_scan, now) {
            self.scan(now);
        }
        if !self.device.as_os_str().is_empty()
            && !self.ready_to_flash()
            && self.forcing().is_none()
            && due(self.last_identify, now)
        {
            self.start_identify(now);
        }
    }

    /// Rescans serial devices, selecting the first unless the operator chose.
    fn scan(&mut self, now: Instant) {
        self.last_scan = Some(now);
        self.devices = device::discover_candidates();
        if !self.device_chosen {
            let first = self
                .devices
                .first()
                .map(|candidate| candidate.path.clone())
                .unwrap_or_default();
            self.select_device(first, false);
        }
    }

    /// Selects a device; a new device has not been identified yet.
    fn select_device(&mut self, device: PathBuf, chosen: bool) {
        self.device_chosen |= chosen;
        if device != self.device {
            log::info!("flash device {}", device.display());
            self.device = device;
            self.finding = None;
            self.last_identify = None;
        }
    }

    fn start_identify(&mut self, now: Instant) {
        self.last_identify = Some(now);
        match flash::start_identify(self.device.clone()) {
            Ok(job) => self.identify = Some((job, self.device.clone())),
            Err(error) => self.record_finding(Finding::Nothing(error.to_string())),
        }
    }

    fn poll_identify(&mut self) {
        let Some((job, device)) = self.identify.as_ref() else {
            return;
        };
        let Some(outcome) = job.poll() else {
            return;
        };
        let device = device.clone();
        self.identify = None;
        // A result for a device the operator has since left is stale.
        if device == self.device {
            self.record_finding(Finding::from_outcome(outcome));
        }
    }

    /// Stores a finding, logging it only when it changes.
    fn record_finding(&mut self, finding: Finding) {
        let changed = self
            .finding
            .as_ref()
            .is_none_or(|(device, previous)| *device != self.device || *previous != finding);
        if changed {
            match &finding {
                Finding::Nothing(reason) => {
                    log::warn!("no radio answered on {}: {reason}", self.device.display());
                }
                finding => log::info!("{} on {}", Self::describe(finding), self.device.display()),
            }
        }
        if let Finding::Bootloader {
            family: RadioFamily::K1,
            version,
        } = &finding
        {
            // The K1 recovery write confirms against the version the radio
            // reports, so it is read rather than typed.
            self.advanced.request.bootloader_version.clone_from(version);
        }
        if let Finding::Firmware { version, .. } = &finding {
            self.last_firmware = Some((self.device.clone(), version.clone()));
        }
        self.finding = Some((self.device.clone(), finding));
    }

    /// Returns the finding for the selected device, if it has one.
    pub fn finding(&self) -> Option<&Finding> {
        self.finding
            .as_ref()
            .filter(|(device, _)| *device == self.device)
            .map(|(_, finding)| finding)
    }

    /// Reports whether the selected radio is in flashing mode.
    pub fn ready_to_flash(&self) -> bool {
        matches!(self.finding(), Some(Finding::Bootloader { .. }))
    }

    /// Returns the family in use: the operator's choice, else the detected one.
    pub fn family(&self) -> Option<RadioFamily> {
        self.family_override
            .or_else(|| self.finding().and_then(Finding::family))
    }

    /// Returns the family a forced flash writes as: only one the operator
    /// chose, never a detected one.
    pub fn forcing(&self) -> Option<RadioFamily> {
        self.family_override.filter(|_| self.force)
    }

    /// Returns the image to write: the operator's, else the family's packaged one.
    pub fn image(&self) -> Option<PathBuf> {
        self.image_override.clone().or_else(|| {
            self.family()
                .map(|family| paths::packaged_image(family.image_name()))
        })
    }

    /// Returns the AFIK version the image to write carries, if it can be read.
    fn image_version(&mut self) -> Option<String> {
        let image = self.image().filter(|image| image.is_file())?;
        self.digests.version(&image)
    }

    /// Returns the firmware version the selected radio runs, or last ran
    /// before it was put into flashing mode.
    pub fn radio_version(&self) -> Option<&str> {
        self.last_firmware
            .as_ref()
            .filter(|(device, _)| *device == self.device)
            .map(|(_, version)| version.as_str())
    }

    /// Reports whether the radio already runs the image's version.
    fn up_to_date(&self, image_version: Option<&str>) -> bool {
        image_version.is_some() && image_version == self.radio_version()
    }

    /// Returns where the backup step stands for the current family.
    pub fn backup_state(&self) -> BackupState {
        let Some(family) = self.family() else {
            return BackupState::NoFamily;
        };
        // Backups taken before they were filed by family are all K5 reads.
        let newest = self.eeprom_root.as_ref().and_then(|root| {
            paths::newest_family_backup_in(root, family.slug(), family == RadioFamily::K5)
        });
        match newest {
            Some(path) => BackupState::OnFile(path),
            None if self.skipped_backup.contains(&family) => BackupState::Skipped,
            None => BackupState::Missing,
        }
    }

    /// Reports whether the write may start now.
    pub fn can_flash(&self) -> bool {
        !self.busy()
            && (self.ready_to_flash() || self.forcing().is_some())
            && self.image().is_some_and(|image| image.is_file())
    }

    /// Takes an EEPROM backup now, or as soon as the port is free.
    fn request_backup(&mut self) {
        if self.busy() {
            self.pending_backup = true;
            "The backup starts when the current check finishes.".clone_into(&mut self.status);
        } else {
            self.start_backup();
        }
    }

    fn start_backup(&mut self) {
        let Some(family) = self.family() else {
            self.fail(
                "starting the EEPROM backup",
                "choose the radio family first".into(),
            );
            return;
        };
        let output = self.eeprom_root.as_ref().map_or_else(
            || paths::default_family_eeprom_backup(family.slug()),
            |root| {
                root.join(family.slug())
                    .join(paths::eeprom_backup_name(paths::unix_now()))
            },
        );
        let request = FlashRequest {
            device: self.device.clone(),
            eeprom_output: output.clone(),
            ..FlashRequest::default()
        };
        let what = format!(
            "backing up the {} EEPROM to {}",
            family.label(),
            output.display()
        );
        self.start(FlashOperation::BackupEeprom, request, JobKind::Backup, what);
    }

    fn start_flash(&mut self) {
        let Some(image) = self.image() else {
            self.fail("starting the flash", "choose an image".into());
            return;
        };
        let transaction_id = match flash::fresh_transaction_id() {
            Ok(transaction_id) => transaction_id,
            Err(error) => {
                self.fail("generating a transaction identifier", error.to_string());
                return;
            }
        };
        let request = self.flash_request(image.clone(), transaction_id);
        let what = match request.forced_family {
            Some(family) => format!("force flashing {} as {}", image.display(), family.label()),
            None => format!("flashing {}", image.display()),
        };
        self.start(
            FlashOperation::AfikApplication,
            request,
            JobKind::Flash,
            what,
        );
    }

    fn flash_request(&self, firmware: PathBuf, transaction_id: u32) -> FlashRequest {
        FlashRequest {
            device: self.device.clone(),
            firmware,
            transaction_id,
            forced_family: self.forcing(),
            ..FlashRequest::default()
        }
    }

    fn start_advanced(&mut self) {
        let operation = self.advanced.operation;
        let mut request = self.advanced.request.clone();
        request.device.clone_from(&self.device);
        if operation.is_write() {
            match flash::fresh_transaction_id() {
                Ok(transaction_id) => request.transaction_id = transaction_id,
                Err(error) => {
                    self.fail("generating a transaction identifier", error.to_string());
                    return;
                }
            }
        }
        if operation.needs_crc32() {
            let Some(crc) = flash::parse_crc32(&self.advanced.crc32) else {
                log::warn!("refused a non-hex image CRC-32 {:?}", self.advanced.crc32);
                "Enter the image CRC-32 as up to eight hex digits.".clone_into(&mut self.status);
                return;
            };
            request.image_crc32 = crc;
        }
        let what = operation.label().to_owned();
        self.start(operation, request, JobKind::Advanced, what);
    }

    fn start(
        &mut self,
        operation: FlashOperation,
        request: FlashRequest,
        kind: JobKind,
        what: String,
    ) {
        if self.busy() {
            "Wait for the current operation to finish.".clone_into(&mut self.status);
            return;
        }
        match flash::start(operation, request) {
            Ok(job) => {
                log::info!("{what} on {}", self.device.display());
                self.status = format!("{}…", capitalise(&what));
                self.progress = None;
                self.job = Some((job, kind, what));
            }
            Err(error) => self.fail(&format!("starting {what}"), error.to_string()),
        }
    }

    fn poll_job(&mut self) {
        let Some((job, kind, what)) = self.job.as_ref() else {
            return;
        };
        let (kind, what) = (*kind, what.clone());
        let mut finished = None;
        for message in job.drain() {
            match message {
                FlashProgress::Step { done, total } => self.progress = Some((done, total)),
                FlashProgress::Finished(summary) => finished = Some(Ok(summary)),
                FlashProgress::Failed(error) => finished = Some(Err(error)),
            }
        }
        let Some(outcome) = finished else {
            return;
        };
        self.job = None;
        match outcome {
            Ok(summary) => {
                log::info!("{what} on {}: {summary}", self.device.display());
                if kind == JobKind::Flash {
                    // The radio restarts into another version.
                    self.last_firmware = None;
                }
                self.status = match kind {
                    JobKind::Flash => format!(
                        "Done: {summary}. The radio restarts into the new firmware; \
                         switch it off and on if it does not."
                    ),
                    _ => format!("Done: {summary}."),
                };
            }
            Err(error) => self.fail(&format!("{what} on {}", self.device.display()), error),
        }
        // Whatever ran, the radio may now be in another state: look again.
        self.finding = None;
        self.last_identify = None;
    }

    fn fail(&mut self, doing: &str, message: String) {
        log::error!("{doing}: {message}");
        self.status = message;
    }

    /// Describes a finding in one operator-facing sentence.
    fn describe(finding: &Finding) -> String {
        match finding {
            Finding::Bootloader { family, version } => {
                format!("{} in flashing mode (bootloader {version})", family.label())
            }
            Finding::Firmware {
                family: Some(family),
                version,
                ..
            } => format!("{} running {version}", family.label()),
            Finding::Firmware {
                family: None,
                version,
                ..
            } => format!("a radio running {version}"),
            Finding::Nothing(reason) => format!("no radio answered ({reason})"),
        }
    }
}

fn capitalise(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

/// Drawing.
impl FlashTab {
    /// Draws the whole tab.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.radio_section(ui);
            ui.separator();
            self.backup_section(ui);
            ui.separator();
            self.image_section(ui);
            ui.separator();
            self.flash_section(ui);
            ui.separator();
            egui::CollapsingHeader::new("Advanced: manual operations")
                .default_open(false)
                .show(ui, |ui| self.advanced_section(ui));
        });
    }

    fn radio_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("1. Radio");
        ui.horizontal_wrapped(|ui| {
            ui.label("Device:");
            let selected = self
                .devices
                .iter()
                .find(|candidate| candidate.path == self.device)
                .map_or_else(|| "other path".to_owned(), DeviceCandidate::label);
            let mut choice = None;
            ComboBox::from_id_salt("flash-device")
                .selected_text(selected)
                .width(260.0)
                .show_ui(ui, |ui| {
                    for candidate in &self.devices {
                        if ui
                            .selectable_label(candidate.path == self.device, candidate.label())
                            .on_hover_text(candidate.path.display().to_string())
                            .clicked()
                        {
                            choice = Some(candidate.path.clone());
                        }
                    }
                });
            if ui.button("Rescan").clicked() {
                self.scan(Instant::now());
            }
            let mut text = self.device.display().to_string();
            if ui
                .add(
                    TextEdit::singleline(&mut text)
                        .hint_text("/dev/ttyUSB0")
                        .desired_width(300.0),
                )
                .on_hover_text("Type a path to use a port detection did not offer")
                .changed()
            {
                choice = Some(PathBuf::from(text.trim()));
            }
            if let Some(device) = choice {
                self.select_device(device, true);
            }
        });

        let image_version = self.image_version();
        let (colour, text) = self.radio_line(image_version.as_deref());
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(colour, text);
            let checkable = !self.busy() && !self.device.as_os_str().is_empty();
            if ui
                .add_enabled(checkable, egui::Button::new("Check again"))
                .clicked()
            {
                self.start_identify(Instant::now());
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Radio family:");
            let detected = self.finding().and_then(Finding::family);
            let automatic = detected.map_or_else(
                || "detect automatically".to_owned(),
                |family| format!("{} (detected)", family.label()),
            );
            let selected = self
                .family_override
                .map_or(automatic.clone(), |family| family.label().to_owned());
            ComboBox::from_id_salt("flash-family")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.family_override, None, automatic);
                    for family in RadioFamily::all() {
                        ui.selectable_value(
                            &mut self.family_override,
                            Some(family),
                            family.label(),
                        );
                    }
                });
            if let Some(family) = self.family_override {
                ui.checkbox(&mut self.force, "Force").on_hover_text(format!(
                    "Flash as a {} whatever bootloader version the radio reports. \
                         It must still be in flashing mode; automatic checks pause.",
                    family.label()
                ));
            }
        });
    }

    /// Returns the colour and sentence describing the selected radio.
    fn radio_line(&self, image_version: Option<&str>) -> (Color32, String) {
        if self.device.as_os_str().is_empty() {
            return (
                WARNING_COLOUR,
                "No USB serial device found. Plug in the programming cable (scanning every \
                 few seconds), or type the port's path."
                    .to_owned(),
            );
        }
        let checking = self.identify.is_some();
        match self.finding() {
            None if checking => (
                ui_text_colour(),
                format!("Checking {}…", self.device.display()),
            ),
            None => (ui_text_colour(), "Not checked yet.".to_owned()),
            Some(finding @ Finding::Bootloader { .. }) => (
                READY_COLOUR,
                format!("{}: ready to flash.", capitalise(&Self::describe(finding))),
            ),
            Some(finding @ Finding::Firmware { .. }) if self.up_to_date(image_version) => (
                READY_COLOUR,
                format!(
                    "{}, the image's version: there is no need to flash it.",
                    capitalise(&Self::describe(finding))
                ),
            ),
            Some(finding @ Finding::Firmware { family, .. }) => {
                let choose = if family.is_none() {
                    " Choose its family below."
                } else {
                    ""
                };
                (
                    WARNING_COLOUR,
                    format!(
                        "{}.{choose} To flash it, switch it off, then hold PTT while \
                         switching it on. Watching for flashing mode…",
                        capitalise(&Self::describe(finding))
                    ),
                )
            }
            Some(Finding::Nothing(reason)) => (
                WARNING_COLOUR,
                format!(
                    "No radio answered on {} ({reason}). Check the cable and that the \
                     radio is on. Checking again every few seconds.",
                    self.device.display()
                ),
            ),
        }
    }

    fn backup_section(&mut self, ui: &mut egui::Ui) {
        let family = self.family();
        ui.heading(format!(
            "2. EEPROM backup{}",
            family.map_or_else(String::new, |family| format!(" ({})", family.label()))
        ));
        let can_back_up = matches!(
            self.finding(),
            Some(Finding::Firmware {
                serves_backup: true,
                ..
            })
        );
        match self.backup_state() {
            BackupState::NoFamily => {
                ui.label("Once the radio is identified, this checks for a kept backup.");
            }
            BackupState::OnFile(path) => {
                ui.colored_label(READY_COLOUR, format!("Backup kept: {}", path.display()));
                digest_label(ui, &mut self.digests, &path);
                if can_back_up && ui.button("Take another backup").clicked() {
                    self.request_backup();
                }
            }
            state @ (BackupState::Missing | BackupState::Skipped) => {
                let label = family.map_or("this radio", RadioFamily::label);
                if state == BackupState::Missing {
                    ui.colored_label(
                        WARNING_COLOUR,
                        format!(
                            "No EEPROM backup is kept for {label}. It holds the radio's \
                             calibration and settings, so take one before the first flash."
                        ),
                    );
                } else {
                    ui.label("Going on without a backup for this session.");
                }
                ui.label(self.backup_advice());
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(can_back_up, egui::Button::new("Back up now"))
                        .clicked()
                    {
                        self.request_backup();
                    }
                    if let (BackupState::Missing, Some(family)) = (&state, family) {
                        if ui
                            .button("Skip")
                            .on_hover_text(
                                "Flash without a backup; not every firmware can serve one",
                            )
                            .clicked()
                        {
                            log::warn!("EEPROM backup skipped for {}", family.label());
                            self.skipped_backup.insert(family);
                        }
                    }
                });
            }
        }
    }

    /// Says how to get the radio into a state which can serve a backup.
    fn backup_advice(&self) -> String {
        match self.finding() {
            Some(Finding::Firmware {
                serves_backup: true,
                ..
            }) => "The radio is running firmware which can serve the backup.".to_owned(),
            Some(Finding::Firmware { version, .. }) => format!(
                "{version} cannot serve a backup. Stock firmware and AFIK-K5 7.0.6 or later can."
            ),
            Some(Finding::Bootloader { .. }) => "The bootloader cannot read EEPROM: switch the \
                 radio off and on normally, without PTT, to take a backup."
                .to_owned(),
            _ => "A backup needs the radio switched on normally.".to_owned(),
        }
    }

    fn image_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("3. Firmware image");
        let family = self.family();
        let image = self.image();
        ui.horizontal_wrapped(|ui| {
            ui.label("Image:");
            let mut text = image
                .as_ref()
                .map(|image| image.display().to_string())
                .unwrap_or_default();
            if ui
                .add(
                    TextEdit::singleline(&mut text)
                        .hint_text("identify the radio, or type an image path")
                        .desired_width(460.0),
                )
                .changed()
            {
                let text = text.trim();
                self.image_override = (!text.is_empty()).then(|| PathBuf::from(text));
            }
            if self.image_override.is_some() {
                if family.is_some() && ui.button("Use packaged image").clicked() {
                    self.image_override = None;
                }
            } else if let Some(family) = family {
                ui.label(format!("packaged {} image", family.label()));
            }
        });
        if let Some(image) = image {
            if image.is_file() {
                digest_label(ui, &mut self.digests, &image);
                self.versions(ui, &image);
            } else {
                let hint = match (family, self.image_override.is_some()) {
                    (Some(family), false) => format!(
                        "Not built yet. Run {} or choose an image.",
                        family.build_hint()
                    ),
                    _ => "No such file.".to_owned(),
                };
                ui.colored_label(WARNING_COLOUR, hint);
            }
        }
    }

    /// Shows the image's AFIK version beside the radio's firmware version.
    fn versions(&mut self, ui: &mut egui::Ui, image: &Path) {
        let image_version = self.digests.version(image);
        ui.label(format!(
            "    Image version: {}",
            image_version
                .as_deref()
                .unwrap_or("none found (not an AFIK image?)")
        ));
        let radio = match (self.radio_version(), self.ready_to_flash()) {
            (Some(version), false) => format!("    Radio runs: {version}"),
            (Some(version), true) => format!("    Radio ran: {version} before flashing mode"),
            (None, _) => {
                "    Radio runs: not seen yet; switch it on normally to read it".to_owned()
            }
        };
        ui.label(radio);
        if self.up_to_date(image_version.as_deref()) {
            ui.colored_label(
                READY_COLOUR,
                "The radio already runs this version: there is no need to flash it.",
            );
        }
    }

    fn flash_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("4. Flash");
        let without_backup = self.backup_state() == BackupState::Missing;
        let verb = if self.forcing().is_some() {
            "Force flash"
        } else {
            "Flash"
        };
        let label = match (self.family(), self.image()) {
            (Some(family), Some(image)) => format!(
                "{verb} {} to the {}{}",
                image
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                family.label(),
                if without_backup {
                    " without an EEPROM backup"
                } else {
                    ""
                }
            ),
            _ => verb.to_owned(),
        };
        ui.label(
            "Writes the AFIK application only, never the bootloader or EEPROM. An image \
             built for the other radio is refused before any page is sent.",
        );
        if ui
            .add_enabled(
                self.can_flash(),
                egui::Button::new(RichText::new(label).strong()),
            )
            .clicked()
        {
            self.start_flash();
        }
        self.progress_and_status(ui);
    }

    fn progress_and_status(&self, ui: &mut egui::Ui) {
        if let (Some((done, total)), Some(_)) = (self.progress, self.job.as_ref()) {
            let fraction = f32::from(done) / f32::from(total.max(1));
            ui.add(egui::ProgressBar::new(fraction).text(format!("{done} of {total} pages")));
        }
        if !self.status.is_empty() {
            let colour = if self.status.starts_with("Done") {
                READY_COLOUR
            } else if self.job.is_some() {
                ui_text_colour()
            } else {
                ERROR_COLOUR
            };
            ui.colored_label(colour, RichText::new(&self.status).strong());
        }
    }

    fn advanced_section(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Recovery writes and backups to a chosen file. These keep every typed \
             confirmation the flasher library requires. They use the device above.",
        );
        ComboBox::from_label("Operation")
            .selected_text(self.advanced.operation.label())
            .show_ui(ui, |ui| {
                for operation in [
                    FlashOperation::K1Recovery,
                    FlashOperation::K5Application,
                    FlashOperation::BackupEeprom,
                ] {
                    ui.selectable_value(&mut self.advanced.operation, operation, operation.label());
                }
            });
        let request = &mut self.advanced.request;
        let digests = &mut self.digests;
        match self.advanced.operation {
            FlashOperation::BackupEeprom | FlashOperation::AfikApplication => {
                path_field(ui, "Backup output", &mut request.eeprom_output);
            }
            FlashOperation::K1Recovery => {
                path_field(ui, "Recovery image", &mut request.firmware);
                digest_label(ui, digests, &request.firmware);
                text_field(ui, "Bootloader version", &mut request.bootloader_version);
                text_field(ui, "Target confirmation", &mut request.target_confirmation);
                text_field(ui, "Image CRC-32 (hex)", &mut self.advanced.crc32);
            }
            FlashOperation::K5Application => {
                path_field(ui, "Application image", &mut request.firmware);
                digest_label(ui, digests, &request.firmware);
                path_field(ui, "Known-good recovery image", &mut request.recovery);
                digest_label(ui, digests, &request.recovery);
                path_field(ui, "Retained EEPROM backup", &mut request.eeprom_backup);
                digest_label(ui, digests, &request.eeprom_backup);
                text_field(ui, "Firmware version", &mut request.firmware_version);
                text_field(ui, "Target confirmation", &mut request.target_confirmation);
                text_field(
                    ui,
                    "Recovery rehearsed confirmation",
                    &mut request.recovery_rehearsed_confirmation,
                );
                text_field(ui, "Image CRC-32 (hex)", &mut self.advanced.crc32);
            }
        }
        let label = if self.advanced.operation.is_write() {
            "Start write"
        } else {
            "Start backup"
        };
        if ui
            .add_enabled(!self.busy(), egui::Button::new(label))
            .clicked()
        {
            self.start_advanced();
        }
    }
}

fn ui_text_colour() -> Color32 {
    Color32::GRAY
}

/// Reports the size and CRC-32 of one file, or why it cannot be read.
fn digest_label(ui: &mut egui::Ui, digests: &mut DigestCache, path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    match digests.digest(path) {
        Ok((bytes, crc)) => {
            ui.label(format!("    {bytes} bytes, CRC-32 {crc:08x}"));
        }
        Err(error) => {
            ui.colored_label(WARNING_COLOUR, format!("    {error}"));
        }
    }
}

fn path_field(ui: &mut egui::Ui, label: &str, path: &mut PathBuf) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        let mut text = path.display().to_string();
        if ui
            .add(TextEdit::singleline(&mut text).desired_width(360.0))
            .changed()
        {
            *path = PathBuf::from(text.trim());
        }
    });
}

fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        ui.add(TextEdit::singleline(value).desired_width(360.0));
    });
}

#[cfg(test)]
mod tests {
    use super::{BackupState, Finding, FlashTab, RECHECK};
    use crate::flash::RadioFamily;
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };

    fn bootloader(family: RadioFamily) -> Finding {
        Finding::Bootloader {
            family,
            version: "2.00.06".to_owned(),
        }
    }

    fn firmware(version: &str) -> Finding {
        Finding::Firmware {
            family: RadioFamily::of_firmware(version),
            version: version.to_owned(),
            serves_backup: radio_flasher::serves_eeprom_reads(version),
        }
    }

    /// A tab on a fixed device whose backups live in a scratch directory.
    fn tab(name: &str) -> (FlashTab, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("afik-flash-tab-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut tab = FlashTab {
            eeprom_root: Some(root.clone()),
            ..FlashTab::default()
        };
        tab.select_device(PathBuf::from("/dev/ttyTEST"), true);
        (tab, root)
    }

    #[test]
    fn only_a_radio_in_flashing_mode_with_an_image_can_be_flashed() {
        let (mut tab, root) = tab("ready");
        assert!(!tab.can_flash());
        tab.record_finding(firmware("AFIK-K5-7.0.5"));
        assert_eq!(tab.family(), Some(RadioFamily::K5));
        assert!(!tab.ready_to_flash());

        let image = root.join("image.raw");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&image, [0_u8; 4]).unwrap();
        tab.image_override = Some(image.clone());
        assert!(!tab.can_flash(), "running firmware cannot be flashed");
        tab.record_finding(bootloader(RadioFamily::K5));
        assert!(tab.can_flash());

        // A finding belongs to its device: another device starts unidentified.
        tab.select_device(PathBuf::from("/dev/ttyOTHER"), true);
        assert!(tab.finding().is_none());
        assert!(!tab.can_flash());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_chosen_family_can_be_forced_onto_an_unrecognised_radio() {
        let (mut tab, root) = tab("force");
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("image.raw");
        std::fs::write(&image, [0_u8; 4]).unwrap();
        tab.image_override = Some(image.clone());
        tab.record_finding(Finding::Nothing(
            "unsupported bootloader version: 9.00.01".into(),
        ));
        assert!(!tab.can_flash());

        // Force alone does nothing: only a family the operator chose is forced.
        tab.force = true;
        assert_eq!(tab.forcing(), None);
        assert!(!tab.can_flash());

        tab.family_override = Some(RadioFamily::K5);
        assert_eq!(tab.forcing(), Some(RadioFamily::K5));
        assert!(tab.can_flash());
        let request = tab.flash_request(image.clone(), 1);
        assert_eq!(request.forced_family, Some(RadioFamily::K5));

        // Automatic checks pause so they do not take the port.
        tab.tick(Instant::now());
        assert!(!tab.busy());

        tab.force = false;
        assert!(!tab.can_flash());
        assert_eq!(tab.flash_request(image, 1).forced_family, None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_radio_version_is_kept_through_flashing_mode_and_compared() {
        let (mut tab, root) = tab("versions");
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("image.raw");
        std::fs::write(&image, b"\0AFIK-K5-7.0.22\0").unwrap();
        tab.image_override = Some(image);
        assert_eq!(tab.image_version().as_deref(), Some("AFIK-K5-7.0.22"));
        assert_eq!(tab.radio_version(), None);

        tab.record_finding(firmware("AFIK-K5-7.0.21"));
        let image_version = tab.image_version();
        assert!(!tab.up_to_date(image_version.as_deref()));
        tab.record_finding(firmware("AFIK-K5-7.0.22"));
        assert!(tab.up_to_date(image_version.as_deref()));
        assert!(tab
            .radio_line(image_version.as_deref())
            .1
            .contains("no need"));

        // In flashing mode only the bootloader answers: the version stays.
        tab.record_finding(bootloader(RadioFamily::K5));
        assert_eq!(tab.radio_version(), Some("AFIK-K5-7.0.22"));
        assert!(tab.can_flash(), "the same version may still be written");
        tab.select_device(PathBuf::from("/dev/ttyOTHER"), true);
        assert_eq!(tab.radio_version(), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_image_follows_the_family_until_overridden() {
        let (mut tab, _root) = tab("image");
        assert_eq!(tab.image(), None);
        tab.record_finding(bootloader(RadioFamily::K1));
        assert!(tab.image().unwrap().ends_with("radio-firmware-k1.raw"));
        tab.family_override = Some(RadioFamily::K5);
        assert!(tab.image().unwrap().ends_with("afik-k5-v1.raw"));
        tab.image_override = Some(PathBuf::from("/tmp/mine.raw"));
        assert_eq!(tab.image(), Some(PathBuf::from("/tmp/mine.raw")));
    }

    #[test]
    fn a_missing_backup_is_asked_for_and_may_be_skipped() {
        let (mut tab, root) = tab("backup");
        assert_eq!(tab.backup_state(), BackupState::NoFamily);
        tab.record_finding(firmware("k5_2.01.26"));
        assert_eq!(tab.backup_state(), BackupState::Missing);
        tab.skipped_backup.insert(RadioFamily::K5);
        assert_eq!(tab.backup_state(), BackupState::Skipped);

        let kept = root.join("k5").join("eeprom-20261014T000000Z.bin");
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, [0_u8; 4]).unwrap();
        assert_eq!(tab.backup_state(), BackupState::OnFile(kept));

        // A K5 backup says nothing about a K1.
        tab.family_override = Some(RadioFamily::K1);
        assert_eq!(tab.backup_state(), BackupState::Missing);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn firmware_findings_say_whether_a_backup_is_possible() {
        assert!(matches!(
            firmware("AFIK-K5-7.0.6"),
            Finding::Firmware {
                family: Some(RadioFamily::K5),
                serves_backup: true,
                ..
            }
        ));
        assert!(matches!(
            firmware("AFIK-K1-7.0.6"),
            Finding::Firmware {
                family: Some(RadioFamily::K1),
                serves_backup: false,
                ..
            }
        ));
        assert!(matches!(
            firmware("AFIK-K1-7.0.25"),
            Finding::Firmware {
                family: Some(RadioFamily::K1),
                serves_backup: true,
                ..
            }
        ));
        assert!(matches!(
            firmware("F4HWN v4.3"),
            Finding::Firmware { family: None, .. }
        ));
    }

    #[test]
    fn an_unanswered_device_is_checked_again_only_after_the_interval() {
        let (mut tab, _root) = tab("recheck");
        tab.select_device(PathBuf::from("/definitely/missing"), true);
        tab.devices = vec![crate::device::DeviceCandidate::new(PathBuf::from(
            "/definitely/missing",
        ))];
        let start = Instant::now();
        tab.tick(start);
        assert!(tab.identify.is_some(), "a new device is checked at once");
        while tab.identify.is_some() {
            tab.tick(start);
            std::thread::yield_now();
        }
        assert!(matches!(tab.finding(), Some(Finding::Nothing(_))));
        tab.tick(start + Duration::from_millis(10));
        assert!(tab.identify.is_none(), "not again before the interval");
        tab.tick(start + RECHECK);
        assert!(tab.identify.is_some(), "and again once it has passed");
    }

    #[test]
    fn a_backup_asked_for_during_a_check_starts_when_the_port_is_free() {
        let (mut tab, _root) = tab("pending");
        tab.select_device(PathBuf::from("/definitely/missing"), true);
        tab.devices = vec![crate::device::DeviceCandidate::new(PathBuf::from(
            "/definitely/missing",
        ))];
        tab.family_override = Some(RadioFamily::K5);
        let now = Instant::now();
        tab.tick(now);
        assert!(tab.identify.is_some());
        tab.request_backup();
        assert!(tab.pending_backup);
        assert!(tab.job.is_none(), "never two workers on one port");
        while tab.identify.is_some() {
            tab.tick(now);
            std::thread::yield_now();
        }
        // The tick that frees the port starts the backup; another tick could
        // already collect it, since opening a missing port fails at once.
        assert!(!tab.pending_backup);
        assert!(tab.job.is_some(), "the queued backup started");
        while tab.job.is_some() {
            tab.tick(now);
            std::thread::yield_now();
        }
        // The port does not exist, so the backup fails and says why.
        assert!(tab.status.contains("serial setup failed"), "{}", tab.status);
    }

    #[test]
    fn every_state_draws() {
        let (mut tab, root) = tab("draw");
        let context = eframe::egui::Context::default();
        let draw = |tab: &mut FlashTab| {
            let _output = context.run(eframe::egui::RawInput::default(), |context| {
                eframe::egui::CentralPanel::default().show(context, |ui| tab.ui(ui));
            });
        };
        draw(&mut tab);
        for finding in [
            Finding::Nothing("timed out".to_owned()),
            firmware("F4HWN v4.3"),
            firmware("AFIK-K5-7.0.6"),
            bootloader(RadioFamily::K5),
            bootloader(RadioFamily::K1),
        ] {
            tab.record_finding(finding);
            draw(&mut tab);
            tab.skipped_backup.insert(RadioFamily::K5);
            draw(&mut tab);
        }
        tab.image_override = Some(PathBuf::from("/definitely/missing.raw"));
        tab.status = "Done: wrote 240 pages.".to_owned();
        draw(&mut tab);
        tab.select_device(PathBuf::new(), true);
        draw(&mut tab);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_malformed_crc32_is_refused_before_a_manual_write_starts() {
        let (mut tab, _root) = tab("crc");
        tab.advanced.operation = crate::flash::FlashOperation::K1Recovery;
        tab.advanced.crc32 = "not hex".to_owned();
        tab.start_advanced();
        assert!(tab.job.is_none());
        assert!(tab.status.contains("CRC-32"), "{}", tab.status);
    }
}
