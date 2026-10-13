//! Host contract proof for the K5 application-facing display adapter.

use radio_bk4819::{Bk4819, RegisterAddress, RegisterBus};
use radio_firmware_k5::application::{
    apply_event, apply_ui_intent, ApplyOutcome, SpeakerOutput, UiDispatcher,
};
use radio_firmware_k5::k5_display::{
    controller_column_for_visible, page_address_commands, render_operator_frame,
    visible_page_write_plan, K5BootDisplay, DISPLAY_COLUMN_OFFSET, DISPLAY_VISIBLE_COLUMNS,
};
use radio_firmware_k5::keypad::{scan, Key as PhysicalKey, MatrixIo};
use radio_firmware_k5::receive::Pmr446Channel as Pmr446ReceiveChannel;
use radio_platform::configuration::{Pmr446Channel, PMR446_CHANNELS};
use radio_platform::display::{show_boot_sequence, BootDisplay, BootStage, ReceiveDiagnostic};
use radio_platform::operator_display::{
    render_channel_list, render_info_screen, render_operating_screen, render_selector_list,
    ListRow, MemoryState, OperatingView, ResetCause, SelectorRow, SerialCounters, FRAME_BYTES,
    LIST_ROWS,
};
use radio_platform::operator_shell::{Capabilities, Context, Intent, Screen, Shell};
use radio_platform::receive_app::View;

#[derive(Default)]
struct K5AdapterContract {
    stages: [Option<BootStage>; 3],
    length: usize,
    receive: Option<ReceiveDiagnostic>,
}

impl BootDisplay for K5AdapterContract {
    type Error = ();

    fn show(&mut self, stage: BootStage) -> Result<(), Self::Error> {
        self.stages[self.length] = Some(stage);
        self.length += 1;
        Ok(())
    }

    fn show_receive(&mut self, diagnostic: ReceiveDiagnostic) -> Result<(), Self::Error> {
        self.receive = Some(diagnostic);
        Ok(())
    }
}

#[test]
fn k5_adapter_runs_the_shared_boot_behavior() {
    fn implements_shared_contract<D: BootDisplay>() {}
    implements_shared_contract::<K5BootDisplay>();

    let mut display = K5AdapterContract::default();
    show_boot_sequence(&mut display).unwrap();
    let diagnostic = ReceiveDiagnostic {
        bytes: 16,
        status: 0,
    };
    display.show_receive(diagnostic).unwrap();
    assert_eq!(display.length, 3);
    assert_eq!(display.receive, Some(diagnostic));
}

#[test]
#[allow(clippy::too_many_lines)]
fn k5_supported_shell_screens_are_deterministic_and_shared() {
    let mut shell = Shell::new();
    shell.set_capabilities(Capabilities::K5);
    shell.select_memory();
    let view = View {
        channel: 3,
        audio: true,
        monitoring: false,
        squelch_open: true,
        receiver_ok: true,
    };
    let context = Context {
        visible_channels: 16,
        active_index: 2,
        scanning: false,
    };

    let operating = render_operator_frame(&shell, view, None);
    assert_eq!(operating, render_operator_frame(&shell, view, None));
    assert!(operating.iter().any(|byte| *byte != 0));

    let channel = Pmr446Channel::new(view.channel).unwrap();
    let name = channel.display_name();
    let mut expected_operating = [0_u8; FRAME_BYTES];
    render_operating_screen(
        &mut expected_operating,
        &OperatingView {
            position: u16::from(view.channel),
            total: u16::from(PMR446_CHANNELS),
            name: &name,
            frequency_hz: channel.frequency_hz(),
            rssi_raw: 0,
            squelch_open: view.squelch_open,
            receiver_ok: view.receiver_ok,
            battery_percent: None,
            monitoring: view.monitoring,
            scanning: false,
            bank: None,
            entry: None,
            vfo_step_hz: None,
        },
    );
    assert_eq!(operating, expected_operating);
    let mut fault_view = view;
    fault_view.receiver_ok = false;
    assert_ne!(operating, render_operator_frame(&shell, fault_view, None));

    assert_eq!(
        shell.press(radio_platform::receive_app::Key::Side1, 0, context),
        radio_platform::operator_shell::Intent::Redraw
    );
    let channel_list = render_operator_frame(&shell, view, None);
    assert_ne!(operating, channel_list);
    assert_eq!(shell.screen(), Screen::ChannelList);

    let mut expected_rows = [ListRow::default(); LIST_ROWS];
    for (index, row) in expected_rows.iter_mut().enumerate() {
        let number = u8::try_from(index + 1).unwrap();
        let channel = Pmr446Channel::new(number).unwrap();
        *row = ListRow::new(
            u16::from(number),
            &channel.display_name(),
            number == view.channel,
        );
    }
    let mut expected_channel_list = [0_u8; FRAME_BYTES];
    render_channel_list(
        &mut expected_channel_list,
        &expected_rows,
        usize::from(shell.cursor()),
        u16::from(PMR446_CHANNELS),
    );
    assert_eq!(channel_list, expected_channel_list);

    assert_eq!(
        shell.press(radio_platform::receive_app::Key::Exit, 1, context),
        radio_platform::operator_shell::Intent::Redraw
    );
    assert_eq!(
        shell.press(radio_platform::receive_app::Key::Star, 2, context),
        radio_platform::operator_shell::Intent::Redraw
    );
    let source_list = render_operator_frame(&shell, view, None);
    assert_ne!(channel_list, source_list);
    let mut expected_source_list = [0_u8; FRAME_BYTES];
    render_selector_list(
        &mut expected_source_list,
        b"SOURCE",
        &[SelectorRow::text(b"ALL CHANNELS", true)],
        0,
    );
    assert_eq!(source_list, expected_source_list);

    assert_eq!(
        shell.press(radio_platform::receive_app::Key::Exit, 3, context),
        radio_platform::operator_shell::Intent::Redraw
    );
    assert_eq!(
        shell.press(radio_platform::receive_app::Key::Function, 4, context),
        radio_platform::operator_shell::Intent::Redraw
    );
    let info = render_operator_frame(&shell, view, None);
    assert_ne!(source_list, info);
    assert_ne!(operating, info);
    let mut expected_info = [0_u8; FRAME_BYTES];
    render_info_screen(
        &mut expected_info,
        K5BootDisplay::IMAGE_IDENTITY,
        0,
        u16::from(PMR446_CHANNELS),
        false,
        MemoryState::Unknown,
        SerialCounters::default(),
        0,
        ResetCause::default(),
    );
    assert_eq!(info, expected_info);
}

#[test]
fn k5_visible_page_plan_preserves_the_full_panel_without_edge_wrap() {
    let plan = visible_page_write_plan();
    assert_eq!(plan.len(), 8);

    for (page, transfer) in plan.iter().enumerate() {
        assert_eq!(usize::from(transfer.page), page);
        assert_eq!(transfer.controller_column, DISPLAY_COLUMN_OFFSET);
        assert_eq!(transfer.bytes, DISPLAY_VISIBLE_COLUMNS);
        assert_eq!(
            page_address_commands(transfer.page, transfer.controller_column),
            [0xB0 | transfer.page, 0x10, 0x04]
        );
        let last = transfer.controller_column + transfer.bytes - 1;
        assert_eq!(last, DISPLAY_COLUMN_OFFSET + DISPLAY_VISIBLE_COLUMNS - 1);
    }

    let mut controller_columns = [false; 132];
    for visible in 0..DISPLAY_VISIBLE_COLUMNS {
        let controller = controller_column_for_visible(visible).unwrap();
        assert_eq!(controller, DISPLAY_COLUMN_OFFSET + visible);
        let slot = usize::from(controller);
        assert!(!controller_columns[slot]);
        controller_columns[slot] = true;
    }
    assert!(controller_columns[..usize::from(DISPLAY_COLUMN_OFFSET)]
        .iter()
        .all(|used| !used));
    assert!(
        controller_columns[usize::from(DISPLAY_COLUMN_OFFSET + DISPLAY_VISIBLE_COLUMNS)..]
            .iter()
            .all(|used| !used)
    );
    assert_eq!(controller_column_for_visible(DISPLAY_VISIBLE_COLUMNS), None);
}

#[test]
fn k5_settings_and_squelch_frames_use_the_shared_k1_layout() {
    let mut shell = Shell::new();
    shell.set_capabilities(Capabilities::K5);
    shell.select_memory();
    let context = Context {
        visible_channels: 16,
        active_index: 0,
        scanning: false,
    };
    assert_eq!(
        shell.press(radio_platform::receive_app::Key::Menu, 0, context),
        Intent::Redraw
    );
    let settings = render_operator_frame(
        &shell,
        View {
            channel: 1,
            audio: true,
            monitoring: false,
            squelch_open: false,
            receiver_ok: true,
        },
        None,
    );
    assert_eq!(shell.screen(), Screen::Settings);
    let mut expected_settings = [0_u8; FRAME_BYTES];
    render_selector_list(
        &mut expected_settings,
        b"SETTINGS",
        &[
            SelectorRow::squelch_setting(radio_domain::SquelchLevel::CONSERVATIVE.get()),
            SelectorRow::tx_power_setting(0),
        ],
        0,
    );
    assert_eq!(settings, expected_settings);
    assert_eq!(
        shell.press(radio_platform::receive_app::Key::Menu, 1, context),
        Intent::Redraw
    );
    let squelch = render_operator_frame(
        &shell,
        View {
            channel: 1,
            audio: true,
            monitoring: false,
            squelch_open: false,
            receiver_ok: true,
        },
        None,
    );
    assert_eq!(shell.screen(), Screen::SquelchList);
    let mut expected_squelch = [0_u8; FRAME_BYTES];
    let mut rows = [SelectorRow::default(); LIST_ROWS];
    for (level, row) in rows.iter_mut().enumerate() {
        let level = u8::try_from(level).expect("bounded row");
        *row = SelectorRow::squelch_level(
            level,
            level == radio_domain::SquelchLevel::CONSERVATIVE.get(),
        );
    }
    render_selector_list(&mut expected_squelch, b"SQUELCH", &rows, 3);
    assert_eq!(squelch, expected_squelch);
}

#[test]
#[allow(clippy::too_many_lines)]
fn k5_physical_menu_settings_apply_the_selected_threshold_after_ack() {
    #[derive(Default)]
    struct Matrix {
        ptt: bool,
        held: Option<(usize, usize)>,
        side: Option<PhysicalKey>,
        row: usize,
        restored: bool,
    }

    impl MatrixIo for Matrix {
        fn select_row(&mut self, row: usize) {
            self.row = row;
        }

        fn columns(&mut self) -> u8 {
            if self.row == 4 {
                match self.side {
                    Some(PhysicalKey::Side1) => 0x0e,
                    Some(PhysicalKey::Side2) => 0x0d,
                    _ => 0x0f,
                }
            } else {
                self.held
                    .filter(|(row, _)| *row == self.row)
                    .map_or(0x0f, |(_, column)| 0x0f & !(1 << column))
            }
        }

        fn settle(&mut self) {}

        fn restore_shared_pins(&mut self) {
            self.restored = true;
        }

        fn ptt_pressed(&mut self) -> bool {
            self.ptt
        }
    }

    #[derive(Default)]
    struct Bus {
        writes: std::vec::Vec<(u8, u16)>,
    }

    impl RegisterBus for Bus {
        type Error = core::convert::Infallible;

        fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
            self.writes.push((address.get(), value));
            Ok(())
        }

        fn read(&mut self, _: RegisterAddress) -> Result<u16, Self::Error> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct Speaker {
        enabled: bool,
    }

    impl SpeakerOutput for Speaker {
        fn set_enabled(&mut self, enabled: bool) {
            self.enabled = enabled;
        }
    }

    // Menu opens settings from the operating screen, as on stock and Armel.
    let mut matrix = Matrix {
        held: Some((0, 0)),
        ..Matrix::default()
    };
    let menu_scan = scan(&mut matrix);
    assert_eq!(menu_scan, Some(PhysicalKey::Menu));
    assert!(matrix.restored);

    let context = Context {
        visible_channels: 16,
        active_index: 0,
        scanning: false,
    };
    let mut shell = Shell::new();
    shell.set_capabilities(Capabilities::K5);
    shell.select_memory();
    let mut app = radio_platform::receive_app::ReceiveApp::new();
    let mut radio = Bk4819::new(Bus::default());
    radio.initialise().unwrap();
    let mut speaker = Speaker::default();
    let mut configured = None;
    let mut metrics = None;
    let mut dispatcher = UiDispatcher::new(None);
    assert!(dispatcher
        .dispatch_scan(
            menu_scan,
            0,
            context,
            &mut shell,
            &mut app,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        )
        .is_some());
    // K5 has no hold/scan capability in this slice: an identical physical
    // sample is deliberately not a second edge or a hold action.
    assert_eq!(
        dispatcher.dispatch_scan(
            scan(&mut matrix),
            1,
            context,
            &mut shell,
            &mut app,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        ),
        None
    );
    matrix.held = None;
    dispatcher.dispatch_scan(
        scan(&mut matrix),
        2,
        context,
        &mut shell,
        &mut app,
        &mut radio,
        &mut speaker,
        &mut configured,
        &mut metrics,
    );
    matrix.held = Some((0, 0));
    dispatcher.dispatch_scan(
        scan(&mut matrix),
        3,
        context,
        &mut shell,
        &mut app,
        &mut radio,
        &mut speaker,
        &mut configured,
        &mut metrics,
    );
    matrix.held = None;
    dispatcher.dispatch_scan(
        scan(&mut matrix),
        4,
        context,
        &mut shell,
        &mut app,
        &mut radio,
        &mut speaker,
        &mut configured,
        &mut metrics,
    );
    matrix.held = Some((2, 0));
    dispatcher.dispatch_scan(
        scan(&mut matrix),
        5,
        context,
        &mut shell,
        &mut app,
        &mut radio,
        &mut speaker,
        &mut configured,
        &mut metrics,
    );
    matrix.held = None;
    dispatcher.dispatch_scan(
        scan(&mut matrix),
        6,
        context,
        &mut shell,
        &mut app,
        &mut radio,
        &mut speaker,
        &mut configured,
        &mut metrics,
    );
    matrix.held = Some((2, 0));
    dispatcher.dispatch_scan(
        scan(&mut matrix),
        7,
        context,
        &mut shell,
        &mut app,
        &mut radio,
        &mut speaker,
        &mut configured,
        &mut metrics,
    );
    matrix.held = None;
    dispatcher.dispatch_scan(
        scan(&mut matrix),
        8,
        context,
        &mut shell,
        &mut app,
        &mut radio,
        &mut speaker,
        &mut configured,
        &mut metrics,
    );
    let level = radio_domain::SquelchLevel::new(5).expect("level");
    let outcome = dispatcher.dispatch_scan(
        {
            matrix.held = Some((0, 0));
            scan(&mut matrix)
        },
        9,
        context,
        &mut shell,
        &mut app,
        &mut radio,
        &mut speaker,
        &mut configured,
        &mut metrics,
    );
    assert_eq!(outcome, Some(ApplyOutcome::Applied));
    assert_eq!(app.squelch(), level);
    assert_eq!(shell.squelch(), level);
    let threshold = radio
        .bus()
        .writes
        .iter()
        .find(|(address, _)| *address == 0x78)
        .copied();
    assert!(threshold.is_some());
    let mut conservative = Bk4819::new(Bus::default());
    conservative.initialise().unwrap();
    conservative
        .configure_receive(&Pmr446ReceiveChannel::FIRST.setup(true))
        .unwrap();
    let conservative_threshold = conservative
        .bus()
        .writes
        .iter()
        .find(|(address, _)| *address == 0x78)
        .copied();
    assert_ne!(threshold, conservative_threshold);

    let writes_before_rejection = radio.bus().writes.len();
    assert_eq!(
        apply_ui_intent(
            Intent::SetSquelch(level),
            &mut shell,
            &mut app,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        ),
        ApplyOutcome::Rejected
    );
    assert_eq!(radio.bus().writes.len(), writes_before_rejection);
    app.apply(radio_platform::receive_app::Event::ReceiverFault);
    assert_eq!(
        apply_ui_intent(
            Intent::SetSquelch(radio_domain::SquelchLevel::OPEN),
            &mut shell,
            &mut app,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        ),
        ApplyOutcome::Rejected
    );
    assert_eq!(shell.squelch(), level);
}

#[test]
fn k5_empty_or_unmatched_application_batches_are_rejected_without_target_mutation() {
    #[derive(Default)]
    struct Bus {
        writes: std::vec::Vec<(u8, u16)>,
    }

    impl RegisterBus for Bus {
        type Error = core::convert::Infallible;

        fn write(&mut self, address: RegisterAddress, value: u16) -> Result<(), Self::Error> {
            self.writes.push((address.get(), value));
            Ok(())
        }

        fn read(&mut self, _: RegisterAddress) -> Result<u16, Self::Error> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct Speaker;

    impl SpeakerOutput for Speaker {
        fn set_enabled(&mut self, _: bool) {}
    }

    let mut app = radio_platform::receive_app::ReceiveApp::new();
    let mut shell = Shell::new();
    shell.set_capabilities(Capabilities::K5);
    let mut radio = Bk4819::new(Bus::default());
    radio.initialise().unwrap();
    let mut speaker = Speaker;
    let mut configured = None;
    let mut metrics = None;
    let level = radio_domain::SquelchLevel::new(6).unwrap();
    app.apply(radio_platform::receive_app::Event::SetSquelch(level));
    let writes = radio.bus().writes.len();
    assert_eq!(
        apply_ui_intent(
            Intent::SelectNext,
            &mut shell,
            &mut app,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        ),
        ApplyOutcome::Rejected
    );
    assert_eq!(radio.bus().writes.len(), writes);

    assert_eq!(
        apply_ui_intent(
            Intent::SelectNext,
            &mut shell,
            &mut app,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        ),
        ApplyOutcome::Rejected
    );
    app.apply(radio_platform::receive_app::Event::ReceiverFault);
    assert_eq!(
        apply_ui_intent(
            Intent::ToggleMonitor,
            &mut shell,
            &mut app,
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        ),
        ApplyOutcome::Rejected
    );
    assert_eq!(
        apply_event(
            &mut app,
            radio_platform::receive_app::Event::SquelchAcknowledged {
                level,
                applied: true,
            },
            &mut radio,
            &mut speaker,
            &mut configured,
            &mut metrics,
        ),
        ApplyOutcome::Rejected
    );
}
