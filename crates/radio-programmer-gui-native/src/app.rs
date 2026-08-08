//! The egui application drawing the validated project model.

use std::{
    fs,
    path::{Path, PathBuf},
};

use eframe::egui::{self, ComboBox, Grid, RichText, ScrollArea, TextEdit};
use radio_channel_plan::MAX_GENERATED_CHANNELS;
use radio_domain::{
    Bandwidth, Modulation, PowerLevel, ScanResume, TxClass, BACKLIGHT_ALWAYS_ON,
    MAX_BATTERY_SAVE_RATIO, MAX_SQUELCH_LEVEL,
};
use radio_programmer::CapacityReport;
use radio_storage::{CHANNEL_ENCODED_LEN, GENERATED_BANK_ENCODED_LEN};

use radio_programmer_serial::SUPPORTED_BAUDS;

use crate::{
    device::{self, DeviceCandidate, DeviceChoice, DeviceChooser},
    flash::{self, FlashJob, FlashOperation, FlashProgress, FlashRequest},
    model::{
        BankDraft, BankKind, ChannelDraft, ModelError, ProjectModel, StorageSummary, ToneDraft,
        ToneKind, MAX_PROJECT_IMAGE_BYTES,
    },
    presets::PRESETS,
    session::DeviceSession,
    DeviceSelector, Options,
};

const BANK_COUNT: u16 = 16;
/// Channels of one generated plan the editor lists before summarising the rest.
const EXPANSION_PREVIEW_ROWS: usize = 32;
/// Colour used for advisory text which is not a validation failure.
const WARNING_COLOUR: egui::Color32 = egui::Color32::from_rgb(0xB7, 0x6E, 0x00);

/// Which editor tab is visible.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Tab {
    #[default]
    Channels,
    Banks,
    Radio,
    Device,
    Flash,
}

/// The complete native editor.
pub struct StudioApp {
    tab: Tab,
    project: ProjectModel,
    project_path: String,
    status: String,
    errors: Vec<ModelError>,
    session: Option<DeviceSession>,
    chooser: DeviceChooser,
    last_receipt: Option<CapacityReport>,
    listing: Vec<(String, u16, u16)>,
    flash_operation: FlashOperation,
    flash_request: FlashRequest,
    flash_crc32: String,
    flash_job: Option<FlashJob>,
    flash_progress: Option<(u16, u16)>,
    flash_status: String,
    flash_devices: Vec<DeviceCandidate>,
    preset: usize,
}

impl Default for StudioApp {
    fn default() -> Self {
        Self {
            tab: Tab::default(),
            project: ProjectModel::new(),
            project_path: String::new(),
            status: "Ready.".to_owned(),
            errors: Vec::new(),
            session: None,
            chooser: DeviceChooser::new(),
            last_receipt: None,
            listing: Vec::new(),
            flash_operation: FlashOperation::BackupEeprom,
            flash_request: FlashRequest::default(),
            flash_crc32: String::new(),
            flash_job: None,
            flash_progress: None,
            flash_status: String::new(),
            flash_devices: Vec::new(),
            preset: 0,
        }
    }
}

impl StudioApp {
    /// Builds the editor from start-up options, connecting when requested.
    pub fn new(options: &Options) -> Self {
        let mut app = Self::default();
        if let Some(baud) = options.baud {
            app.chooser.baud = baud;
        }
        if let Some(path) = &options.project {
            app.project_path = path.display().to_string();
            app.load_project();
        }
        if options.simulator {
            app.connect_simulator();
            return app;
        }
        // Detection runs at start-up so the Device tab already knows what is
        // plugged in, but only an explicit request connects without a click.
        let detection = app.chooser.detect();
        match &options.device {
            Some(DeviceSelector::Auto) => app.connect_serial(),
            Some(DeviceSelector::Explicit(path)) => {
                app.chooser.choice = DeviceChoice::Manual;
                app.chooser.manual_path = path.display().to_string();
                app.connect_serial();
            }
            None => app.status = detection,
        }
        app
    }

    fn connect_simulator(&mut self) {
        match DeviceSession::connect_simulator() {
            Ok(session) => {
                self.status = format!("Connected to {}.", session.description());
                self.session = Some(session);
                self.refresh_listing();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn connect_serial(&mut self) {
        let (path, baud) = match self.chooser.resolve() {
            Ok(resolved) => resolved,
            Err(reason) => {
                self.status = reason;
                return;
            }
        };
        match DeviceSession::connect_serial(&path, baud) {
            Ok(session) => {
                self.status = format!("Connected to {}.", session.description());
                self.session = Some(session);
                self.refresh_listing();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            self.status = format!("Disconnected from {}.", session.description());
            self.listing.clear();
        }
    }

    fn refresh_listing(&mut self) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        match session.listing() {
            Ok(listing) => {
                self.listing = listing
                    .objects
                    .iter()
                    .map(|object| {
                        (
                            format!("{:?}", object.key.kind),
                            object.key.id,
                            object.encoded_len,
                        )
                    })
                    .collect();
                self.status = format!(
                    "Device generation {} holds {} objects.",
                    listing.generation,
                    listing.objects.len()
                );
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn write_project(&mut self) {
        let project = match self.project.validate() {
            Ok(project) => project,
            Err(errors) => {
                self.errors = errors;
                "Fix the highlighted fields before writing.".clone_into(&mut self.status);
                return;
            }
        };
        self.errors.clear();
        let Some(session) = self.session.as_mut() else {
            "Connect a device first.".clone_into(&mut self.status);
            return;
        };
        match session.write_project(&project) {
            Ok(receipt) => {
                self.last_receipt = Some(receipt.report);
                self.status = format!(
                    "Wrote and verified generation {} with {} channels in {} banks.",
                    receipt.generation, receipt.report.explicit_channels, receipt.report.banks
                );
                self.refresh_listing();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn read_project(&mut self) {
        let Some(session) = self.session.as_mut() else {
            "Connect a device first.".clone_into(&mut self.status);
            return;
        };
        match session.backup() {
            Ok(backup) => match ProjectModel::from_image(&backup.image) {
                Ok(project) => {
                    let channels = project.channels.len();
                    let banks = project.banks.len();
                    self.project = project;
                    self.errors.clear();
                    // An empty read is the ordinary state of a radio nobody has
                    // programmed yet, and it is easy to mistake for a failure:
                    // the channel and bank tabs stay empty either way.
                    self.status = if channels == 0 && banks == 0 {
                        format!(
                            "The radio holds no programmed configuration at generation {}. \
                             Anything it is showing is its own built-in set. \
                             Edit the Channels and Banks tabs and write them to it.",
                            backup.generation
                        )
                    } else {
                        self.tab = Tab::Channels;
                        format!(
                            "Loaded {channels} channels and {banks} banks from generation {} \
                             into the Channels and Banks tabs.",
                            backup.generation
                        )
                    };
                }
                Err(error) => self.status = error.to_string(),
            },
            Err(error) => self.status = error.to_string(),
        }
    }

    fn save_project(&mut self) {
        let path = self.project_path.trim().to_owned();
        if path.is_empty() {
            "Enter a project file path.".clone_into(&mut self.status);
            return;
        }
        match self.project.to_image() {
            Ok(image) => match fs::write(&path, image) {
                Ok(()) => {
                    self.errors.clear();
                    self.status = format!("Saved {path}.");
                }
                Err(error) => self.status = format!("Could not save {path}: {error}"),
            },
            Err(errors) => {
                self.errors = errors;
                "Fix the highlighted fields before saving.".clone_into(&mut self.status);
            }
        }
    }

    fn load_project(&mut self) {
        let path = self.project_path.trim().to_owned();
        if path.is_empty() {
            "Enter a project file path.".clone_into(&mut self.status);
            return;
        }
        match fs::read(&path) {
            Ok(bytes) if bytes.len() > MAX_PROJECT_IMAGE_BYTES => {
                self.status = format!("{path} exceeds {MAX_PROJECT_IMAGE_BYTES} bytes.");
            }
            Ok(bytes) => match ProjectModel::from_image(&bytes) {
                Ok(project) => {
                    self.status = format!(
                        "Loaded {} channels and {} banks from {path}.",
                        project.channels.len(),
                        project.banks.len()
                    );
                    self.project = project;
                    self.errors.clear();
                }
                Err(error) => self.status = error.to_string(),
            },
            Err(error) => self.status = format!("Could not read {path}: {error}"),
        }
    }

    fn poll_flash(&mut self) {
        let Some(job) = self.flash_job.as_ref() else {
            return;
        };
        let mut finished = false;
        for message in job.drain() {
            match message {
                FlashProgress::Step { done, total } => self.flash_progress = Some((done, total)),
                FlashProgress::Finished(summary) => {
                    self.flash_status = summary;
                    finished = true;
                }
                FlashProgress::Failed(error) => {
                    self.flash_status = error;
                    finished = true;
                }
            }
        }
        if finished {
            self.flash_job = None;
        }
    }

    /// Replaces the project with one preset default set.
    ///
    /// This discards the edited project, so the count it replaces is reported:
    /// a preset is a starting point, not a merge.
    fn apply_preset(&mut self) {
        let Some(preset) = PRESETS.get(self.preset) else {
            return;
        };
        let replaced = self.project.channels.len() + self.project.banks.len();
        self.project = preset.build();
        self.errors.clear();
        let (channels, banks) = preset.size();
        self.status = format!(
            "Applied {}: {channels} channels in {banks} banks, replacing {replaced} rows. \
             Confirm every frequency against your own band plan before use.",
            preset.name()
        );
    }

    /// Classifies the radio on the selected port without writing to it.
    fn identify_radio(&mut self) {
        match flash::identify(&self.flash_request.device) {
            Ok(identity) => {
                // The version becomes the confirmation the write compares
                // against the radio, so it is read rather than remembered.
                self.flash_request
                    .bootloader_version
                    .clone_from(&identity.version);
                self.flash_status = format!(
                    "{} bootloader {} on {}.",
                    identity.family,
                    identity.version,
                    self.flash_request.device.display()
                );
            }
            Err(error) => self.flash_status = error.to_string(),
        }
    }

    fn start_flash(&mut self) {
        // A fresh identifier per run is generated rather than typed: the
        // bootloader ties every acknowledgement to it and reuse would make one
        // run's acknowledgements indistinguishable from another's.
        match flash::fresh_transaction_id() {
            Ok(transaction_id) => self.flash_request.transaction_id = transaction_id,
            Err(error) => {
                self.flash_status = error.to_string();
                return;
            }
        }
        self.flash_request.image_crc32 =
            u32::from_str_radix(self.flash_crc32.trim().trim_start_matches("0x"), 16).unwrap_or(0);
        match flash::start(self.flash_operation, self.flash_request.clone()) {
            Ok(job) => {
                self.flash_status = format!(
                    "Running: {} under transaction {:08x}",
                    job.operation().label(),
                    self.flash_request.transaction_id
                );
                self.flash_progress = None;
                self.flash_job = Some(job);
            }
            Err(error) => self.flash_status = error.to_string(),
        }
    }
}

impl eframe::App for StudioApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_flash();
        if self.flash_job.is_some() {
            context.request_repaint();
        }

        egui::TopBottomPanel::top("tabs").show(context, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Channels, "Channels");
                ui.selectable_value(&mut self.tab, Tab::Banks, "Banks");
                ui.selectable_value(&mut self.tab, Tab::Radio, "Radio");
                ui.selectable_value(&mut self.tab, Tab::Device, "Device");
                ui.selectable_value(&mut self.tab, Tab::Flash, "Flash");
            });
        });

        egui::TopBottomPanel::bottom("status").show(context, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&self.status).strong());
            });
            for error in &self.errors {
                ui.colored_label(egui::Color32::from_rgb(0xC0, 0x39, 0x2B), error.to_string());
            }
        });

        egui::CentralPanel::default().show(context, |ui| match self.tab {
            Tab::Channels => self.channels_tab(ui),
            Tab::Banks => self.banks_tab(ui),
            Tab::Radio => self.radio_tab(ui),
            Tab::Device => self.device_tab(ui),
            Tab::Flash => self.flash_tab(ui),
        });
    }
}

impl StudioApp {
    fn project_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Project file:");
            ui.add(TextEdit::singleline(&mut self.project_path).desired_width(320.0));
            if ui.button("Load").clicked() {
                self.load_project();
            }
            if ui.button("Save").clicked() {
                self.save_project();
            }
        });
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
            // Applying replaces the project, so it is never automatic.
            if ui
                .button("Apply")
                .on_hover_text("Replaces every channel and bank row")
                .clicked()
            {
                self.apply_preset();
            }
            if let Some(preset) = PRESETS.get(self.preset) {
                ui.label(preset.detail());
            }
        });
        ui.separator();
    }

    /// Returns the configuration bytes the connected radio declares, if any.
    ///
    /// Offline there is no radio to ask, and a project can be written to any of
    /// them, so no capacity is claimed rather than one being invented.
    fn configuration_capacity(&self) -> u32 {
        self.session
            .as_ref()
            .map_or(0, |session| session.capabilities().configuration_bytes)
    }

    fn channels_tab(&mut self, ui: &mut egui::Ui) {
        self.project_bar(ui);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add channel").clicked() {
                self.project.add_channel();
            }
            ui.label(format!("{} channel rows", self.project.channels.len()));
        });
        let summary = self.project.storage_summary();
        storage_summary_label(ui, &summary);
        configuration_space_label(ui, self.configuration_capacity(), &summary);
        ui.separator();

        // Membership only means something for a named bank, so each checkbox
        // says which bank it joins, whether that bank exists yet, and whether
        // the identifier already belongs to a plan the radio expands.
        let bank_slots = self.project.bank_slots();
        let mut action = None;
        ScrollArea::both().show(ui, |ui| {
            for (row, channel) in self.project.channels.iter_mut().enumerate() {
                ui.push_id(row, |ui| {
                    egui::CollapsingHeader::new(channel_row_label(row, channel))
                        .default_open(true)
                        .show(ui, |ui| {
                            if let Some(requested) = channel_row_editor(ui, channel, &bank_slots) {
                                action = Some((requested, row));
                            }
                        });
                    ui.separator();
                });
            }
        });
        match action {
            Some((RowAction::Duplicate, row)) => self.project.duplicate_channel(row),
            Some((RowAction::Remove, row)) => {
                self.project.channels.remove(row);
            }
            None => {}
        }
    }

    fn banks_tab(&mut self, ui: &mut egui::Ui) {
        self.project_bar(ui);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add named bank").clicked() {
                self.project.add_bank();
            }
            if ui.button("Add generated plan").clicked() {
                self.project.add_generated_bank();
            }
            ui.label(format!("{} banks", self.project.banks.len()));
        });
        let summary = self.project.storage_summary();
        storage_summary_label(ui, &summary);
        configuration_space_label(ui, self.configuration_capacity(), &summary);
        // A compact plan is only storable on a target which advertises the
        // encoding, so say so before the operator writes one.
        if self
            .project
            .banks
            .iter()
            .any(|bank| matches!(bank.kind, BankKind::Generated))
        {
            if let Some(session) = self.session.as_ref() {
                if !session.supports_generated_plans() {
                    ui.colored_label(
                        WARNING_COLOUR,
                        format!(
                            "{} does not advertise compact plan encodings; \
                             generated plans can be saved but not written to it.",
                            session.description()
                        ),
                    );
                }
            }
        }
        ui.separator();

        let mut remove = None;
        ScrollArea::vertical().show(ui, |ui| {
            for (row, bank) in self.project.banks.iter_mut().enumerate() {
                ui.push_id(row, |ui| {
                    egui::CollapsingHeader::new(bank_row_label(row, bank))
                        .default_open(true)
                        .show(ui, |ui| {
                            if bank_row_editor(ui, bank) {
                                remove = Some(row);
                            }
                        });
                });
            }
        });
        if let Some(row) = remove {
            self.project.banks.remove(row);
        }
    }

    fn radio_tab(&mut self, ui: &mut egui::Ui) {
        self.project_bar(ui);
        let config = &mut self.project.config;
        Grid::new("radio").num_columns(2).show(ui, |ui| {
            ui.label("Squelch level");
            ui.add(egui::DragValue::new(&mut config.squelch).range(0..=MAX_SQUELCH_LEVEL));
            ui.end_row();

            ui.label("Backlight seconds");
            ui.add(
                egui::DragValue::new(&mut config.backlight_seconds).range(0..=BACKLIGHT_ALWAYS_ON),
            );
            ui.end_row();

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

            ui.label("Battery save ratio");
            ui.add(
                egui::DragValue::new(&mut config.battery_save_ratio)
                    .range(0..=MAX_BATTERY_SAVE_RATIO),
            );
            ui.end_row();

            ui.label("Options");
            ui.vertical(|ui| {
                ui.checkbox(&mut config.dual_watch, "Dual watch");
                ui.checkbox(&mut config.key_beep, "Key beep");
                ui.checkbox(&mut config.busy_lockout_default, "Busy lockout by default");
                ui.checkbox(&mut config.am_fix, "AM gain compensation");
                ui.checkbox(&mut config.tone_tail_elimination, "Tone tail elimination");
            });
            ui.end_row();
        });
    }

    fn device_tab(&mut self, ui: &mut egui::Ui) {
        self.device_picker(ui);
        ui.separator();

        if let Some(session) = self.session.as_ref() {
            let capabilities = session.capabilities();
            ui.label(format!(
                "Connected to {}: protocol {}, storage {}, {} objects, {} object bytes.",
                session.description(),
                capabilities.protocol_version,
                capabilities.storage_version,
                capabilities.max_objects,
                capabilities.max_object_size
            ));
            ui.label(if session.supports_generated_plans() {
                "The target expands compact generated plans."
            } else {
                "The target stores explicit channels only, not compact plans."
            });
        } else {
            ui.label("No device is connected.");
        }

        ui.horizontal_wrapped(|ui| {
            if ui.button("Refresh listing").clicked() {
                self.refresh_listing();
            }
            if ui.button("Read project from device").clicked() {
                self.read_project();
            }
            if ui.button("Write project to device").clicked() {
                self.write_project();
            }
        });
        ui.separator();

        if let Some(report) = self.last_receipt {
            ui.label(format!(
                "Last verified write: {} objects, {} bytes, {} channels, {} banks.",
                report.object_count, report.storage_bytes, report.explicit_channels, report.banks
            ));
        }

        ScrollArea::vertical().show(ui, |ui| {
            Grid::new("listing").num_columns(3).show(ui, |ui| {
                ui.label(RichText::new("Kind").strong());
                ui.label(RichText::new("Id").strong());
                ui.label(RichText::new("Bytes").strong());
                ui.end_row();
                for (kind, id, bytes) in &self.listing {
                    ui.label(kind);
                    ui.label(id.to_string());
                    ui.label(bytes.to_string());
                    ui.end_row();
                }
            });
        });
    }

    fn flash_tab(&mut self, ui: &mut egui::Ui) {
        ui.label(
            "Firmware and EEPROM operations use the recovery-gated flasher library. \
             Every confirmation phrase is required exactly as documented.",
        );
        ui.separator();

        ComboBox::from_label("Operation")
            .selected_text(self.flash_operation.label())
            .show_ui(ui, |ui| {
                for operation in FlashOperation::all() {
                    ui.selectable_value(&mut self.flash_operation, operation, operation.label());
                }
            });

        self.flash_device_picker(ui);
        ui.separator();
        self.flash_operation_fields(ui);
        ui.separator();
        self.flash_controls(ui);
    }

    /// Draws detection, selection, and the connection controls.
    fn device_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Detect devices").clicked() {
                self.status = self.chooser.detect();
            }
            if ui.button("Connect").clicked() {
                self.connect_serial();
            }
            ui.add_enabled_ui(self.session.is_some(), |ui| {
                if ui.button("Disconnect").clicked() {
                    self.disconnect();
                }
            });
            ui.separator();
            if ui.button("Connect simulator").clicked() {
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

    fn flash_device_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Serial device:");
            let mut device = self.flash_request.device.display().to_string();
            if ui
                .add(TextEdit::singleline(&mut device).desired_width(280.0))
                .changed()
            {
                self.flash_request.device = PathBuf::from(device.trim());
            }
            if ui.button("Detect devices").clicked() {
                self.flash_devices = device::discover_candidates();
                self.flash_status = match self.flash_devices.as_slice() {
                    [] => "No USB serial device was detected.".to_owned(),
                    [only] => format!("Detected {}.", only.label()),
                    candidates => format!("Detected {} devices; choose one.", candidates.len()),
                };
            }
            // Reading the bootloader version off the radio is read-only and
            // fills in the confirmation the write checks against it.
            if ui.button("Identify radio").clicked() {
                self.identify_radio();
            }
        });
        // A flashing target is never selected automatically, however few
        // candidates there are: the operator clicks the exact unit.
        for candidate in self.flash_devices.clone() {
            if ui.button(candidate.label()).clicked() {
                self.flash_request.device = candidate.path;
            }
        }
    }

    fn flash_operation_fields(&mut self, ui: &mut egui::Ui) {
        match self.flash_operation {
            FlashOperation::BackupEeprom => path_field(
                ui,
                "EEPROM backup output",
                &mut self.flash_request.eeprom_output,
            ),
            FlashOperation::K1Recovery => {
                path_field(ui, "Recovery image", &mut self.flash_request.firmware);
                digest_label(ui, &self.flash_request.firmware);
                self.k1_confirmations(ui, false);
            }
            FlashOperation::K1Application => {
                path_field(ui, "Application image", &mut self.flash_request.firmware);
                digest_label(ui, &self.flash_request.firmware);
                path_field(
                    ui,
                    "Known-good recovery image",
                    &mut self.flash_request.recovery,
                );
                digest_label(ui, &self.flash_request.recovery);
                path_field(
                    ui,
                    "Retained EEPROM backup",
                    &mut self.flash_request.eeprom_backup,
                );
                digest_label(ui, &self.flash_request.eeprom_backup);
                self.k1_confirmations(ui, true);
            }
            FlashOperation::K5Application => {
                path_field(ui, "Application image", &mut self.flash_request.firmware);
                path_field(
                    ui,
                    "Known-good recovery image",
                    &mut self.flash_request.recovery,
                );
                path_field(
                    ui,
                    "Retained EEPROM backup",
                    &mut self.flash_request.eeprom_backup,
                );
                digest_label(ui, &self.flash_request.eeprom_backup);
                text_field(
                    ui,
                    "Firmware version",
                    &mut self.flash_request.firmware_version,
                );
                text_field(
                    ui,
                    "Target confirmation",
                    &mut self.flash_request.target_confirmation,
                );
                text_field(
                    ui,
                    "Recovery rehearsed confirmation",
                    &mut self.flash_request.recovery_rehearsed_confirmation,
                );
                text_field(ui, "Image CRC-32 (hex)", &mut self.flash_crc32);
            }
        }
    }

    fn flash_controls(&mut self, ui: &mut egui::Ui) {
        let running = self.flash_job.is_some();
        ui.add_enabled_ui(!running, |ui| {
            let label = if self.flash_operation.is_write() {
                "Start write"
            } else {
                "Start read"
            };
            if ui.button(label).clicked() {
                self.start_flash();
            }
        });
        if let Some((done, total)) = self.flash_progress {
            ui.label(format!("{done} of {total} pages acknowledged."));
        }
        if !self.flash_status.is_empty() {
            ui.label(RichText::new(&self.flash_status).strong());
        }
    }

    fn k1_confirmations(&mut self, ui: &mut egui::Ui, rehearsal: bool) {
        text_field(
            ui,
            "Bootloader version",
            &mut self.flash_request.bootloader_version,
        );
        text_field(
            ui,
            "Target confirmation",
            &mut self.flash_request.target_confirmation,
        );
        if rehearsal {
            text_field(
                ui,
                "Recovery rehearsed confirmation",
                &mut self.flash_request.recovery_rehearsed_confirmation,
            );
        }
        text_field(ui, "Image CRC-32 (hex)", &mut self.flash_crc32);
    }
}

/// Reports the size and CRC-32 of one selected file, or why it cannot be read.
///
/// Firmware cannot be read back off a radio, so the digests of the retained
/// recovery image and EEPROM backup are the only evidence the operator has that
/// the files selected are the pair kept for this exact unit.
fn digest_label(ui: &mut egui::Ui, path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    match flash::artefact_digest(path) {
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

fn tone_editor(ui: &mut egui::Ui, id: &str, tone: &mut ToneDraft) {
    ui.horizontal(|ui| {
        ComboBox::from_id_salt(id)
            .selected_text(tone.kind.label())
            .width(110.0)
            .show_ui(ui, |ui| {
                for kind in ToneKind::all() {
                    ui.selectable_value(&mut tone.kind, kind, kind.label());
                }
            });
        ui.add_enabled(
            !matches!(tone.kind, ToneKind::None),
            TextEdit::singleline(&mut tone.value).desired_width(70.0),
        );
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

/// Draws one channel row, returning the action its buttons requested.
fn channel_row_editor(
    ui: &mut egui::Ui,
    channel: &mut ChannelDraft,
    bank_slots: &[Option<(BankKind, String)>],
) -> Option<RowAction> {
    Grid::new("channel").num_columns(4).show(ui, |ui| {
        ui.label("Id");
        ui.add(egui::DragValue::new(&mut channel.id));
        ui.label("Name");
        ui.add(TextEdit::singleline(&mut channel.name).desired_width(120.0));
        ui.end_row();

        ui.label("Receive MHz");
        ui.add(TextEdit::singleline(&mut channel.receive_mhz).desired_width(120.0));
        ui.label("Transmit MHz");
        ui.add(TextEdit::singleline(&mut channel.transmit_mhz).desired_width(120.0));
        ui.end_row();

        ui.label("RX tone");
        tone_editor(ui, "rx", &mut channel.rx_tone);
        ui.label("TX tone");
        tone_editor(ui, "tx", &mut channel.tx_tone);
        ui.end_row();

        ui.label("Modulation");
        modulation_editor(ui, &mut channel.modulation);
        ui.label("Bandwidth");
        bandwidth_editor(ui, &mut channel.bandwidth);
        ui.end_row();

        ui.label("Power");
        power_editor(ui, &mut channel.power);
        ui.label("Step Hz");
        ui.add(egui::DragValue::new(&mut channel.step_hz).speed(125.0));
        ui.end_row();

        ui.label("Squelch");
        ui.add(egui::DragValue::new(&mut channel.squelch).range(0..=MAX_SQUELCH_LEVEL));
        ui.label("TX class");
        tx_class_editor(ui, &mut channel.tx_class);
        ui.end_row();

        ui.label("Flags");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut channel.scan_skip, "Scan skip");
            ui.checkbox(&mut channel.busy_lockout, "Busy lockout");
            ui.checkbox(&mut channel.reverse, "Reverse");
            ui.checkbox(&mut channel.compander, "Compander");
        });
        ui.label("Banks");
        ui.horizontal_wrapped(|ui| {
            for bank in 0..BANK_COUNT {
                let index = usize::from(bank);
                let response = ui.checkbox(&mut channel.banks[index], format!("{bank}"));
                match bank_slots.get(index).and_then(Option::as_ref) {
                    Some((BankKind::Named, name)) => response.on_hover_text(name.clone()),
                    Some((BankKind::Generated, name)) => response.on_hover_text(format!(
                        "{name} is a generated plan; the radio expands its own channels \
                         into this bank and a stored channel cannot join them"
                    )),
                    None => response.on_hover_text("no named bank defines this identifier"),
                };
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

/// Draws one bank row, reporting whether its removal was requested.
fn bank_row_editor(ui: &mut egui::Ui, bank: &mut BankDraft) -> bool {
    Grid::new("bank").num_columns(4).show(ui, |ui| {
        ui.label("Id");
        ui.add(egui::DragValue::new(&mut bank.id).range(0..=BANK_COUNT - 1));
        ui.label("Name");
        ui.add(TextEdit::singleline(&mut bank.name).desired_width(160.0));
        ui.end_row();

        ui.label("Kind");
        bank_kind_editor(ui, &mut bank.kind);
        ui.label(bank.kind.detail());
        ui.end_row();

        match bank.kind {
            BankKind::Named => {
                ui.label("Scanning");
                ui.checkbox(&mut bank.scan_enabled, "Scan enabled");
                ui.end_row();
            }
            BankKind::Generated => {
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
                ui.add(
                    egui::DragValue::new(&mut bank.channel_count).range(1..=MAX_GENERATED_CHANNELS),
                );
                ui.label("TX class");
                tx_class_editor(ui, &mut bank.tx_class);
                ui.end_row();

                ui.label("Span");
                match bank.generated_span() {
                    Some((first, last)) => ui.label(format!(
                        "{first} to {last} MHz, {} channels",
                        bank.channel_count
                    )),
                    None => {
                        ui.colored_label(WARNING_COLOUR, "the plan is incomplete or out of range")
                    }
                };
                ui.end_row();

                // Every expanded channel shares these settings: they are stored
                // once with the plan, not once per channel.
                ui.label("RX tone");
                tone_editor(ui, "plan-rx", &mut bank.rx_tone);
                ui.label("TX tone");
                tone_editor(ui, "plan-tx", &mut bank.tx_tone);
                ui.end_row();

                ui.label("Modulation");
                modulation_editor(ui, &mut bank.modulation);
                ui.label("Bandwidth");
                bandwidth_editor(ui, &mut bank.bandwidth);
                ui.end_row();

                ui.label("Power");
                power_editor(ui, &mut bank.power);
                ui.label("Step Hz");
                ui.add(egui::DragValue::new(&mut bank.step_hz).speed(125.0));
                ui.end_row();

                ui.label("Squelch");
                ui.add(egui::DragValue::new(&mut bank.squelch).range(0..=MAX_SQUELCH_LEVEL));
                ui.label("Flags");
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut bank.scan_skip, "Scan skip");
                    ui.checkbox(&mut bank.busy_lockout, "Busy lockout");
                    ui.checkbox(&mut bank.compander, "Compander");
                });
                ui.end_row();
            }
        }
    });
    if matches!(bank.kind, BankKind::Generated) {
        generated_expansion(ui, bank);
    }
    ui.button("Remove bank").clicked()
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
    let stored = GENERATED_BANK_ENCODED_LEN;
    let explicit = usize::from(bank.channel_count) * CHANNEL_ENCODED_LEN;
    egui::CollapsingHeader::new(format!(
        "Expands to {} channels on the radio: {stored} stored bytes instead of {explicit}",
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

/// Reports how much of a connected radio's configuration memory is left.
///
/// The capacity is the radio's own declared bound, not a host guess, so this
/// says nothing at all when no radio is connected or when one declares none.
fn configuration_space_label(ui: &mut egui::Ui, capacity: u32, summary: &StorageSummary) {
    if capacity == 0 {
        return;
    }
    let used = u32::try_from(summary.image_bytes()).unwrap_or(u32::MAX);
    let remaining = capacity.saturating_sub(used);
    let text =
        format!("Radio configuration memory: {used} of {capacity} bytes used, {remaining} free.");
    if used > capacity {
        ui.colored_label(
            WARNING_COLOUR,
            format!(
                "Radio configuration memory: {used} bytes needed, {capacity} available. \
                 This project is {} bytes too large to write.",
                used - capacity
            ),
        );
    } else {
        ui.label(text);
    }
}

/// Reports what the project costs a radio and what its plans saved.
fn storage_summary_label(ui: &mut egui::Ui, summary: &StorageSummary) {
    let saving = if summary.expanded_channels > 0 {
        format!(
            " Plans saved {} bytes against storing those channels.",
            summary.bytes_saved()
        )
    } else {
        String::new()
    };
    ui.label(format!(
        "{} selectable channels: {} stored, {} expanded from plans. \
         {} objects, {} stored bytes.{saving}",
        summary.selectable_channels(),
        summary.stored_channels,
        summary.expanded_channels,
        summary.objects,
        summary.bytes,
    ));
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
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(modulation, Modulation::Fm, "FM");
            ui.selectable_value(modulation, Modulation::Am, "AM");
            ui.selectable_value(modulation, Modulation::Usb, "USB");
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

fn power_editor(ui: &mut egui::Ui, power: &mut PowerLevel) {
    ComboBox::from_id_salt("power")
        .selected_text(match power {
            PowerLevel::Low => "Low",
            PowerLevel::Medium => "Medium",
            PowerLevel::High => "High",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(power, PowerLevel::Low, "Low");
            ui.selectable_value(power, PowerLevel::Medium, "Medium");
            ui.selectable_value(power, PowerLevel::High, "High");
        });
}

fn tx_class_editor(ui: &mut egui::Ui, class: &mut TxClass) {
    ComboBox::from_id_salt("tx-class")
        .selected_text(tx_class_label(*class))
        .show_ui(ui, |ui| {
            for candidate in [
                TxClass::Never,
                TxClass::LicenceFreePlan,
                TxClass::Amateur,
                TxClass::Marine,
                TxClass::Aeronautical,
                TxClass::Business,
                TxClass::Experimental,
            ] {
                ui.selectable_value(class, candidate, tx_class_label(candidate));
            }
        });
}

const fn tx_class_label(class: TxClass) -> &'static str {
    match class {
        TxClass::Never => "Never",
        TxClass::LicenceFreePlan => "Licence free",
        TxClass::Amateur => "Amateur",
        TxClass::Marine => "Marine",
        TxClass::Aeronautical => "Aeronautical",
        TxClass::Business => "Business",
        TxClass::Experimental => "Experimental",
    }
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

/// Returns the editor label for one bank row, used by tests and headers.
pub fn bank_row_label(row: usize, bank: &BankDraft) -> String {
    let detail = match bank.kind {
        BankKind::Named => "named channels".to_owned(),
        BankKind::Generated => match bank.generated_span() {
            Some((first, last)) => format!("{first} to {last} MHz"),
            None => "generated plan".to_owned(),
        },
    };
    format!("{}: {} [{}] ({detail})", row + 1, bank.name, bank.id)
}

#[cfg(test)]
mod tests {
    use super::{
        bank_row_label, channel_row_label, scan_resume_label, tx_class_label, StudioApp, Tab,
    };
    use crate::{model::BankKind, DeviceSelector, Options};
    use radio_domain::{ScanResume, TxClass};

    #[test]
    fn a_simulator_start_up_connects_and_lists_objects() {
        let options = Options {
            simulator: true,
            ..Options::default()
        };
        let app = StudioApp::new(&options);
        assert!(app.session.is_some());
        assert_eq!(app.tab, Tab::Channels);
        assert!(app.status.contains("0 objects"));
    }

    #[test]
    fn writing_an_invalid_project_reports_errors_without_contacting_the_device() {
        let mut app = StudioApp::new(&Options {
            simulator: true,
            ..Options::default()
        });
        app.project.add_channel();
        app.project.channels[0].receive_mhz = "nope".to_owned();
        app.write_project();
        assert!(!app.errors.is_empty());
        assert!(app.last_receipt.is_none());
    }

    #[test]
    fn a_valid_project_writes_and_reads_back_through_the_simulator() {
        let mut app = StudioApp::new(&Options {
            simulator: true,
            ..Options::default()
        });
        app.project.add_bank();
        app.project.add_channel();
        app.project.channels[0].banks[0] = true;
        app.write_project();
        assert!(app.errors.is_empty(), "{:?}", app.errors);
        assert_eq!(app.last_receipt.unwrap().explicit_channels, 1);

        app.project = crate::model::ProjectModel::new();
        app.read_project();
        assert_eq!(app.project.channels.len(), 1);
        assert_eq!(app.project.banks.len(), 1);
    }

    #[test]
    fn a_generated_plan_writes_and_reads_back_through_the_simulator() {
        let mut app = StudioApp::new(&Options {
            simulator: true,
            ..Options::default()
        });
        assert!(app.session.as_ref().unwrap().supports_generated_plans());
        app.project.add_generated_bank();
        app.project.banks[0].name = "PMR446".to_owned();
        app.project.banks[0].base_mhz = "446.00625".to_owned();
        app.write_project();
        assert!(app.errors.is_empty(), "{:?}", app.errors);
        let report = app.last_receipt.unwrap();
        assert_eq!(report.generated_channels, 16);
        assert_eq!(report.explicit_channels, 0);

        app.project = crate::model::ProjectModel::new();
        app.read_project();
        assert_eq!(app.project.banks.len(), 1);
        assert_eq!(app.project.banks[0].kind, BankKind::Generated);
        assert_eq!(app.project.banks[0].channel_count, 16);
    }

    #[test]
    fn an_unopenable_start_up_device_is_reported_without_a_session() {
        let app = StudioApp::new(&Options {
            device: Some(DeviceSelector::Explicit(std::path::PathBuf::from(
                "/definitely/missing",
            ))),
            ..Options::default()
        });
        assert!(app.session.is_none());
        assert!(app.status.contains("serial setup failed"), "{}", app.status);
        assert_eq!(app.chooser.manual_path, "/definitely/missing");
        assert_eq!(app.chooser.baud, crate::device::DEFAULT_BAUD);
    }

    #[test]
    fn bank_row_labels_name_the_kind_and_the_generated_span() {
        let mut project = crate::model::ProjectModel::new();
        project.add_bank();
        assert_eq!(
            bank_row_label(0, &project.banks[0]),
            "1: Bank 0 [0] (named channels)"
        );
        project.add_generated_bank();
        project.banks[1].base_mhz = "446.00625".to_owned();
        assert_eq!(
            bank_row_label(1, &project.banks[1]),
            "2: Bank 1 [1] (446.006250 to 446.193750 MHz)"
        );
        project.banks[1].base_mhz = "nope".to_owned();
        assert!(bank_row_label(1, &project.banks[1]).contains("generated plan"));
    }

    #[test]
    fn labels_cover_every_enumerated_value() {
        assert_eq!(tx_class_label(TxClass::Never), "Never");
        assert_eq!(scan_resume_label(ScanResume::Stop), "Stop on signal");
        let mut project = crate::model::ProjectModel::new();
        project.add_channel();
        assert_eq!(
            channel_row_label(0, &project.channels[0]),
            "1: CH1 (145.500000)"
        );
    }
}
