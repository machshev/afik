//! The egui application drawing the validated project model.

use std::{fs, time::Instant};

use eframe::egui::{self, ComboBox, Grid, RichText, ScrollArea, TextEdit};
use radio_channel_plan::MAX_GENERATED_CHANNELS;
use radio_domain::{
    Bandwidth, Modulation, PowerLevel, ScanResume, TxClass, BACKLIGHT_ALWAYS_ON,
    MAX_BATTERY_SAVE_RATIO, MAX_SQUELCH_LEVEL,
};
use radio_programmer::CapacityReport;
use radio_storage::CHANNEL_ENCODED_LEN;

use radio_programmer_serial::SUPPORTED_BAUDS;

use crate::{
    device::{DeviceChoice, DeviceChooser},
    flash_tab::FlashTab,
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

/// What the radio-wide squelch level actually governs, said once.
const SQUELCH_HINT: &str = "Applies to every channel on the radio. The operator \
can change it from the handset settings menu, and the radio stores what they \
chose. A channel's own squelch level is kept in the project but does not \
override this.";

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
    flash: FlashTab,
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
            flash: FlashTab::default(),
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

    fn connect_simulator(&mut self) {
        match DeviceSession::connect_simulator() {
            Ok(session) => self.install_session(session),
            Err(error) => self.fail("connecting the simulator", error.to_string()),
        }
    }

    fn connect_serial(&mut self) {
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

    /// Replaces the connected device, forgetting what the previous one reported.
    fn install_session(&mut self, session: DeviceSession) {
        log::info!(
            "connected to {}: {:?}",
            session.description(),
            session.capabilities()
        );
        self.status = format!("Connected to {}.", session.description());
        self.session = Some(session);
        self.last_receipt = None;
        self.listing.clear();
        self.refresh_listing();
    }

    fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            log::info!("disconnected from {}", session.description());
            self.status = format!("Disconnected from {}.", session.description());
            self.listing.clear();
            self.last_receipt = None;
        }
    }

    fn refresh_listing(&mut self) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let device = session.description().to_owned();
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
            Err(error) => self.fail(&format!("listing objects on {device}"), error.to_string()),
        }
    }

    fn write_project(&mut self) {
        let project = match self.project.validate() {
            Ok(project) => project,
            Err(errors) => {
                self.refuse_invalid("writing", errors);
                return;
            }
        };
        self.errors.clear();
        let Some(session) = self.session.as_mut() else {
            self.refuse("Connect a device first.");
            return;
        };
        let device = session.description().to_owned();
        match session.write_project(&project) {
            Ok(receipt) => {
                log::info!(
                    "wrote and verified generation {} on {device}: {:?}",
                    receipt.generation,
                    receipt.report
                );
                self.last_receipt = Some(receipt.report);
                self.status = format!(
                    "Wrote and verified generation {} with {} channels in {} banks.",
                    receipt.generation, receipt.report.explicit_channels, receipt.report.banks
                );
                self.refresh_listing();
            }
            Err(error) => self.fail(
                &format!("writing the project to {device}"),
                error.to_string(),
            ),
        }
    }

    fn read_project(&mut self) {
        let Some(session) = self.session.as_mut() else {
            self.refuse("Connect a device first.");
            return;
        };
        let device = session.description().to_owned();
        match session.backup() {
            Ok(backup) => match ProjectModel::from_image(&backup.image) {
                Ok(project) => {
                    let channels = project.channels.len();
                    let banks = project.banks.len();
                    log::info!(
                        "read {channels} channels and {banks} banks from {device} at generation {}",
                        backup.generation
                    );
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
                Err(error) => self.fail(
                    &format!("decoding the configuration read from {device}"),
                    error.to_string(),
                ),
            },
            Err(error) => self.fail(
                &format!("reading the project from {device}"),
                error.to_string(),
            ),
        }
    }

    fn save_project(&mut self) {
        let path = self.project_path.trim().to_owned();
        if path.is_empty() {
            self.refuse("Enter a project file path.");
            return;
        }
        match self.project.to_image() {
            Ok(image) => match fs::write(&path, image) {
                Ok(()) => {
                    log::info!("saved project {path}");
                    self.errors.clear();
                    self.status = format!("Saved {path}.");
                }
                Err(error) => self.fail(
                    "saving the project",
                    format!("Could not save {path}: {error}"),
                ),
            },
            Err(errors) => self.refuse_invalid("saving", errors),
        }
    }

    fn load_project(&mut self) {
        let path = self.project_path.trim().to_owned();
        if path.is_empty() {
            self.refuse("Enter a project file path.");
            return;
        }
        match fs::read(&path) {
            Ok(bytes) if bytes.len() > MAX_PROJECT_IMAGE_BYTES => self.fail(
                "loading the project",
                format!("{path} exceeds {MAX_PROJECT_IMAGE_BYTES} bytes."),
            ),
            Ok(bytes) => match ProjectModel::from_image(&bytes) {
                Ok(project) => {
                    log::info!("loaded project {path}");
                    self.status = format!(
                        "Loaded {} channels and {} banks from {path}.",
                        project.channels.len(),
                        project.banks.len()
                    );
                    self.project = project;
                    self.errors.clear();
                }
                Err(error) => self.fail(&format!("decoding project {path}"), error.to_string()),
            },
            Err(error) => self.fail(
                "loading the project",
                format!("Could not read {path}: {error}"),
            ),
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
}

impl eframe::App for StudioApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
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
            Tab::Flash => self.flash.ui(ui),
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
                    // The header is keyed by row, not by its label: the label
                    // shows the name and frequency being edited, and a key that
                    // changed with them would drop focus after one keystroke.
                    egui::CollapsingHeader::new(channel_row_label(row, channel))
                        .id_salt(row)
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
                        .id_salt(row)
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
            ui.label("Squelch level").on_hover_text(SQUELCH_HINT);
            ui.add(egui::DragValue::new(&mut config.squelch).range(0..=MAX_SQUELCH_LEVEL))
                .on_hover_text(SQUELCH_HINT);
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
                        "{name} is a generated plan; this channel is selected beside \
                         the channels it expands, under the same bank filter"
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
            BankKind::Generated => generated_plan_editor(ui, bank),
        }
    });
    if matches!(bank.kind, BankKind::Generated) {
        generated_expansion(ui, bank);
    }
    ui.button("Remove bank").clicked()
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
    ui.label("TX class");
    tx_class_editor(ui, &mut bank.tx_class);
    ui.end_row();

    plan_naming_editor(ui, bank);

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

    ui.label("Calling channel");
    let mut has_calling = bank.calling_index.is_some();
    if ui
        .checkbox(&mut has_calling, "")
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
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(index).range(0..=bank.channel_count.saturating_sub(1)));
            ui.label(calling_label(
                bank.designator.as_str(),
                bank.first_number,
                *index,
            ));
        });
    } else {
        ui.label("none");
    }
    ui.end_row();
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
    let stored = bank.stored_bytes();
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

        // Another device has not had this write, so its receipt is not shown.
        app.connect_simulator();
        assert!(app.last_receipt.is_none());
        app.write_project();
        app.disconnect();
        assert!(app.last_receipt.is_none());
        assert!(app.listing.is_empty());
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

    /// Draws one headless frame of a tab with the given input events.
    fn frame(
        context: &eframe::egui::Context,
        app: &mut StudioApp,
        tab: Tab,
        events: Vec<eframe::egui::Event>,
    ) {
        use eframe::egui::{CentralPanel, Pos2, RawInput, Rect, Vec2};
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 4000.0))),
            events,
            ..RawInput::default()
        };
        let _output = context.run(input, |context| {
            CentralPanel::default().show(context, |ui| match tab {
                Tab::Channels => app.channels_tab(ui),
                _ => app.banks_tab(ui),
            });
        });
    }

    /// Tabs through the widgets until typed text lands in the probed field.
    fn focus_field(
        context: &eframe::egui::Context,
        app: &mut StudioApp,
        tab: Tab,
        probe: impl Fn(&StudioApp) -> String,
    ) {
        use eframe::egui::{Event, Key, Modifiers};
        let before = probe(app);
        for _ in 0..40 {
            let tab_key = Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            };
            frame(context, app, tab, vec![tab_key]);
            frame(context, app, tab, vec![Event::Text("Z".to_owned())]);
            if probe(app) != before {
                return;
            }
        }
        panic!("the probed field never took keyboard focus");
    }

    #[test]
    fn typing_into_a_row_keeps_focus_while_its_header_label_changes() {
        use eframe::egui::{Context, Event};
        // The row header shows the name and frequency being edited. Typing
        // must keep going to the same field, not stop after one character.
        let context = Context::default();
        let mut app = StudioApp::default();
        app.project.add_channel();
        frame(&context, &mut app, Tab::Channels, Vec::new());
        focus_field(&context, &mut app, Tab::Channels, |app| {
            app.project.channels[0].receive_mhz.clone()
        });
        for text in ["Y", "X"] {
            frame(
                &context,
                &mut app,
                Tab::Channels,
                vec![Event::Text(text.into())],
            );
        }
        assert!(
            app.project.channels[0].receive_mhz.ends_with("ZYX"),
            "{}",
            app.project.channels[0].receive_mhz
        );

        let context = Context::default();
        app.project.add_generated_bank();
        frame(&context, &mut app, Tab::Banks, Vec::new());
        focus_field(&context, &mut app, Tab::Banks, |app| {
            app.project.banks[0].name.clone()
        });
        for text in ["Y", "X"] {
            frame(
                &context,
                &mut app,
                Tab::Banks,
                vec![Event::Text(text.into())],
            );
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
    fn failures_are_logged_with_what_was_being_done() {
        use log::Level;
        capture::install();
        let _earlier = capture::take();

        let mut app = StudioApp::new(&Options {
            device: Some(DeviceSelector::Explicit(std::path::PathBuf::from(
                "/definitely/missing",
            ))),
            ..Options::default()
        });
        let logged = capture::take();
        assert!(
            logged.iter().any(|(level, text)| *level == Level::Error
                && text.contains("connecting /definitely/missing at 38400 baud")
                && text.contains("serial setup failed")),
            "{logged:?}"
        );

        // A refused write is a warning naming every field which failed.
        app.project.add_channel();
        app.project.channels[0].receive_mhz = "nope".to_owned();
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
