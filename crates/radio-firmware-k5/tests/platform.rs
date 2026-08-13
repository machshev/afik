//! Host contract proof for the K5 application-facing display adapter.

use radio_firmware_k5::k5_display::{
    controller_column_for_visible, page_address_commands, render_operator_frame,
    visible_page_write_plan, K5BootDisplay, DISPLAY_COLUMN_OFFSET, DISPLAY_VISIBLE_COLUMNS,
};
use radio_platform::configuration::{Pmr446Channel, PMR446_CHANNELS};
use radio_platform::display::{show_boot_sequence, BootDisplay, BootStage, ReceiveDiagnostic};
use radio_platform::operator_display::{
    render_channel_list, render_info_screen, render_operating_screen, render_selector_list,
    ListRow, MemoryState, OperatingView, ResetCause, SelectorRow, SerialCounters, FRAME_BYTES,
    LIST_ROWS,
};
use radio_platform::operator_shell::{Capabilities, Context, Screen, Shell};
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
        shell.press(radio_platform::receive_app::Key::Menu, 0, context),
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
