//! Native AFIK editor entry point.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use radio_programmer_gui::{app::StudioApp, logging, parse_options, theme, HELP};

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

    let log_file = logging::init();
    log::info!(
        "AFIK Studio {} starting; log file {}",
        env!("CARGO_PKG_VERSION"),
        log_file.map_or_else(|| "none".to_owned(), |path| path.display().to_string())
    );

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1100.0, 760.0]),
        ..eframe::NativeOptions::default()
    };
    match eframe::run_native(
        "AFIK Studio",
        native_options,
        Box::new(move |creation| {
            theme::apply(&creation.egui_ctx);
            Ok(Box::new(StudioApp::new(&options)))
        }),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error!("could not start the editor: {error}");
            ExitCode::FAILURE
        }
    }
}
