//! The egui application drawing the validated project model.

use std::{fs, path::Path, time::Instant};

use eframe::egui::{self, ComboBox, Grid, RichText, ScrollArea, TextEdit};
use radio_channel_plan::MAX_GENERATED_CHANNELS;
use radio_domain::{
    Bandwidth, Modulation, PowerLevel, ScanResume, TxClass, BACKLIGHT_ALWAYS_ON,
    MAX_BATTERY_SAVE_RATIO, MAX_SQUELCH_LEVEL,
};
use radio_flasher::ImageWriteReport;

use radio_programmer_serial::SUPPORTED_BAUDS;

use crate::{
    device::{DeviceChoice, DeviceChooser},
    flash_tab::FlashTab,
    model::{
        BankDraft, BankKind, ChannelDraft, ModelError, ProjectModel, StorageSummary, ToneDraft,
        ToneKind, MAX_PROJECT_IMAGE_BYTES, RADIO_IMAGE_BYTES,
    },
    presets::PRESETS,
    projects::ProjectStore,
    session::DeviceSession,
    DeviceSelector, Options,
};

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
    Banks,
    Radio,
    Device,
    Flash,
}

/// The complete native editor.
pub struct StudioApp {
    tab: Tab,
    project: ProjectModel,
    /// The project as last loaded or saved, to tell whether it has changed.
    saved: ProjectModel,
    projects: ProjectStore,
    new_project: String,
    status: String,
    errors: Vec<ModelError>,
    session: Option<DeviceSession>,
    chooser: DeviceChooser,
    last_write: Option<ImageWriteReport>,
    flash: FlashTab,
    preset: usize,
}

impl StudioApp {
    /// Builds the editor from start-up options, opening the last project.
    pub fn new(options: &Options) -> Self {
        Self::with_projects(options, ProjectStore::open())
    }

    /// Builds the editor over an explicit project store.
    pub fn with_projects(options: &Options, projects: ProjectStore) -> Self {
        let mut app = Self {
            tab: Tab::default(),
            project: ProjectModel::new(),
            saved: ProjectModel::new(),
            projects,
            new_project: String::new(),
            status: "Ready.".to_owned(),
            errors: Vec::new(),
            session: None,
            chooser: DeviceChooser::new(),
            last_write: None,
            flash: FlashTab::default(),
            preset: 0,
        };
        if let Some(baud) = options.baud {
            app.chooser.baud = baud;
        }
        if let Some(path) = &options.project {
            app.projects.adopt(path);
        }
        app.load_project();
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
        if let Some(session) = self.session.take() {
            log::info!("disconnected from {}", session.description());
            self.status = format!("Disconnected from {}.", session.description());
            self.last_write = None;
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
            self.refuse("Connect a radio first.");
            return;
        };
        let device = session.description().to_owned();
        match session.write_project(&project) {
            Ok(report) => {
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
            Err(error) => self.fail(
                &format!("writing the project to {device}"),
                error.to_string(),
            ),
        }
    }

    fn read_project(&mut self) {
        let Some(session) = self.session.as_mut() else {
            self.refuse("Connect a radio first.");
            return;
        };
        let device = session.description().to_owned();
        match session.read_project() {
            Ok(project) => {
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
            Err(error) => self.fail(
                &format!("reading the project from {device}"),
                error.to_string(),
            ),
        }
    }

    /// Whether the project differs from what was last loaded or saved.
    fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    fn save_project(&mut self) {
        let entry = self.projects.current().clone();
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
                self.status = format!("Saved {}.", entry.name);
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
        let loaded = if path.exists() {
            match read_project_file(path) {
                Ok(project) => project,
                Err(message) => {
                    self.fail(&format!("loading project {}", entry.name), message);
                    return;
                }
            }
        } else {
            ProjectModel::new()
        };
        log::info!("opened project {} from {}", entry.name, path.display());
        self.status = if path.exists() {
            format!(
                "Opened {}: {} channels in {} banks.",
                entry.name,
                loaded.channel_count(),
                loaded.banks.len()
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

    /// Replaces the project with one preset default set.
    ///
    /// This discards the edited project, so the count it replaces is reported:
    /// a preset is a starting point, not a merge.
    fn apply_preset(&mut self) {
        let Some(preset) = PRESETS.get(self.preset) else {
            return;
        };
        let replaced = self.project.banks.len();
        self.project = preset.build();
        self.errors.clear();
        let (channels, banks) = preset.size();
        self.status = format!(
            "Applied {}: {channels} channels in {banks} banks, replacing {replaced} banks. \
             Confirm every frequency against your own band plan before use.",
            preset.name()
        );
    }
}

/// Reads one project file, either format the editor reads.
fn read_project_file(path: &Path) -> Result<ProjectModel, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    if bytes.len() > MAX_PROJECT_IMAGE_BYTES {
        return Err(format!(
            "{} exceeds {MAX_PROJECT_IMAGE_BYTES} bytes.",
            path.display()
        ));
    }
    ProjectModel::from_image(&bytes).map_err(|error| error.to_string())
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
            ui.label("Project:");
            let mut chosen = self.projects.current_index();
            ComboBox::from_id_salt("project")
                .selected_text(self.projects.current().name.clone())
                .width(200.0)
                .show_ui(ui, |ui| {
                    for (index, entry) in self.projects.entries().iter().enumerate() {
                        ui.selectable_value(&mut chosen, index, &entry.name)
                            .on_hover_text(entry.path.display().to_string());
                    }
                })
                .response
                .on_hover_text(self.projects.current().path.display().to_string());
            if chosen != self.projects.current_index() {
                self.open_project(chosen);
            }
            if ui.button("Save").clicked() {
                self.save_project();
            }
            let dirty = self.is_dirty();
            if ui
                .add_enabled(dirty, egui::Button::new("Revert"))
                .on_hover_text("Discard the changes since the project was last saved")
                .clicked()
            {
                self.load_project();
            }
            if dirty {
                ui.colored_label(WARNING_COLOUR, "unsaved changes");
            }
            ui.separator();
            ui.add(
                TextEdit::singleline(&mut self.new_project)
                    .hint_text("new project name")
                    .desired_width(160.0),
            );
            if ui.button("New project").clicked() {
                self.create_project();
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
                .on_hover_text("Replaces every bank")
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

    fn banks_tab(&mut self, ui: &mut egui::Ui) {
        self.project_bar(ui);
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add channel bank").clicked() {
                self.project.add_bank();
            }
            if ui.button("Add generated plan").clicked() {
                self.project.add_generated_bank();
            }
            ui.label(format!(
                "{} banks, {} channel rows",
                self.project.banks.len(),
                self.project.channel_count()
            ));
        });
        let summary = self.project.storage_summary();
        storage_summary_label(ui, &summary);
        radio_memory_label(ui, &summary);
        ui.separator();

        let mut action = None;
        ScrollArea::vertical().show(ui, |ui| {
            for (row, bank) in self.project.banks.iter_mut().enumerate() {
                ui.push_id(row, |ui| {
                    // Headers are keyed by row, not by their label: the label
                    // shows what is being edited, and a key that changed with
                    // it would drop focus after one keystroke.
                    egui::CollapsingHeader::new(bank_row_label(row, bank))
                        .id_salt(row)
                        .default_open(true)
                        .show(ui, |ui| {
                            if let Some(requested) = bank_row_editor(ui, bank) {
                                action = Some((row, requested));
                            }
                        });
                    ui.separator();
                });
            }
        });
        match action {
            Some((row, BankAction::MoveUp)) => self.project.move_bank(row, true),
            Some((row, BankAction::MoveDown)) => self.project.move_bank(row, false),
            Some((row, BankAction::Remove)) => {
                self.project.banks.remove(row);
            }
            Some((row, BankAction::AddChannel)) => self.project.add_channel(row),
            Some((row, BankAction::Channel(channel, RowAction::Duplicate))) => {
                self.project.duplicate_channel(row, channel);
            }
            Some((row, BankAction::Channel(channel, RowAction::Remove))) => {
                self.project.banks[row].channels.remove(channel);
            }
            None => {}
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
        radio_memory_label(ui, &self.project.storage_summary());

        ui.horizontal_wrapped(|ui| {
            if ui.button("Read project from radio").clicked() {
                self.read_project();
            }
            if ui.button("Write project to radio").clicked() {
                self.write_project();
            }
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

/// What one bank asked the editor to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BankAction {
    /// Move the bank one place earlier.
    MoveUp,
    /// Move the bank one place later.
    MoveDown,
    /// Delete the bank and its channels.
    Remove,
    /// Append a channel to the bank.
    AddChannel,
    /// Act on one of the bank's channels.
    Channel(usize, RowAction),
}

/// Draws one channel row, returning the action its buttons requested.
fn channel_row_editor(ui: &mut egui::Ui, channel: &mut ChannelDraft) -> Option<RowAction> {
    Grid::new("channel").num_columns(4).show(ui, |ui| {
        ui.label("Name");
        ui.add(TextEdit::singleline(&mut channel.name).desired_width(120.0));
        ui.label("TX class");
        tx_class_editor(ui, &mut channel.tx_class);
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
        ui.label("Flags");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut channel.scan_skip, "Scan skip");
            ui.checkbox(&mut channel.busy_lockout, "Busy lockout");
            ui.checkbox(&mut channel.reverse, "Reverse");
            ui.checkbox(&mut channel.compander, "Compander");
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
            }
            BankKind::Generated => generated_plan_editor(ui, bank),
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Move up").clicked() {
            action = Some(BankAction::MoveUp);
        }
        if ui.button("Move down").clicked() {
            action = Some(BankAction::MoveDown);
        }
        if ui.button("Remove bank").clicked() {
            action = Some(BankAction::Remove);
        }
    });
    match bank.kind {
        BankKind::Generated => {
            generated_expansion(ui, bank);
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
            ui.indent("channels", |ui| {
                for (row, channel) in bank.channels.iter_mut().enumerate() {
                    ui.push_id(row, |ui| {
                        egui::CollapsingHeader::new(channel_row_label(row, channel))
                            .id_salt(row)
                            .default_open(true)
                            .show(ui, |ui| {
                                if let Some(requested) = channel_row_editor(ui, channel) {
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

/// Reports how much of a radio's configuration memory the project takes.
fn radio_memory_label(ui: &mut egui::Ui, summary: &StorageSummary) {
    match summary.image_bytes {
        Some(bytes) => {
            ui.label(format!(
                "Radio memory: {bytes} of {RADIO_IMAGE_BYTES} bytes used, {} free.",
                RADIO_IMAGE_BYTES - bytes
            ));
        }
        None => {
            ui.colored_label(
                WARNING_COLOUR,
                "Radio memory: the project does not compile into a radio image yet; \
                 Save or Write lists what to fix.",
            );
        }
    }
}

/// Reports how many channels the radio will offer.
fn storage_summary_label(ui: &mut egui::Ui, summary: &StorageSummary) {
    ui.label(format!(
        "{} selectable channels: {} stored, {} expanded from {} plans.",
        summary.selectable_channels(),
        summary.stored_channels,
        summary.expanded_channels,
        summary.plans,
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
        BankKind::Named => format!("{} channels", bank.channels.len()),
        BankKind::Generated => match bank.generated_span() {
            Some((first, last)) => format!("plan, {first} to {last} MHz"),
            None => "generated plan".to_owned(),
        },
    };
    format!("{}: {} ({detail})", row + 1, bank.name)
}

#[cfg(test)]
mod tests {
    use super::{
        bank_row_label, channel_row_label, scan_resume_label, tx_class_label, StudioApp, Tab,
    };
    use crate::{model::BankKind, projects::ProjectStore, DeviceSelector, Options};
    use radio_domain::{ScanResume, TxClass};
    use std::path::PathBuf;

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
        assert_eq!(app.project, written);
        assert_eq!(app.project.banks[1].kind, BankKind::Generated);

        // Another radio has not had this write, so its report is not shown.
        app.connect_simulator();
        assert!(app.last_write.is_none());
        app.write_project();
        app.disconnect();
        assert!(app.last_write.is_none());
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
    fn bank_row_labels_name_the_kind_and_the_generated_span() {
        let mut project = crate::model::ProjectModel::new();
        project.add_bank();
        project.add_channel(0);
        assert_eq!(
            bank_row_label(0, &project.banks[0]),
            "1: Bank 1 (1 channels)"
        );
        project.add_generated_bank();
        project.banks[1].base_mhz = "446.00625".to_owned();
        assert_eq!(
            bank_row_label(1, &project.banks[1]),
            "2: Bank 2 (plan, 446.006250 to 446.193750 MHz)"
        );
        project.banks[1].base_mhz = "nope".to_owned();
        assert!(bank_row_label(1, &project.banks[1]).contains("generated plan"));
    }

    /// Draws one headless frame of the Banks tab with the given input events.
    fn frame(
        context: &eframe::egui::Context,
        app: &mut StudioApp,
        events: Vec<eframe::egui::Event>,
    ) {
        use eframe::egui::{CentralPanel, Pos2, RawInput, Rect, Vec2};
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 4000.0))),
            events,
            ..RawInput::default()
        };
        let _output = context.run(input, |context| {
            CentralPanel::default().show(context, |ui| app.banks_tab(ui));
        });
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
        app.project.add_bank();
        app.project.add_channel(0);
        frame(&context, &mut app, Vec::new());
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
        app.project.add_generated_bank();
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
    fn labels_cover_every_enumerated_value() {
        assert_eq!(tx_class_label(TxClass::Never), "Never");
        assert_eq!(scan_resume_label(ScanResume::Stop), "Stop on signal");
        let mut project = crate::model::ProjectModel::new();
        project.add_bank();
        project.add_channel(0);
        assert_eq!(
            channel_row_label(0, &project.banks[0].channels[0]),
            "1: CH1 (145.500000)"
        );
    }
}
