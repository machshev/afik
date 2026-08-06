//! Native AFIK editor entry point.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use radio_programmer_gui_native::{app::StudioApp, parse_options, HELP};

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let options = match parse_options(&arguments) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    if options.help {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1100.0, 760.0]),
        ..eframe::NativeOptions::default()
    };
    match eframe::run_native(
        "AFIK Studio",
        native_options,
        Box::new(move |_context| Ok(Box::new(StudioApp::new(&options)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("could not start the editor: {error}");
            ExitCode::FAILURE
        }
    }
}
