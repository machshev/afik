//! AFIK Studio entry point: a desktop window, or a page in the browser
//! (`ADR-119`).

#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    use radio_programmer_gui::{app::StudioApp, logging, parse_options, theme, HELP};

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

    let log_file = logging::init(options.verbose);
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

/// Starts Studio on the page's `afik_studio` canvas, logging to the console.
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast;
    use radio_programmer_gui::{app::StudioApp, theme, Options};

    // Another logger is only installed by a second start, which never runs.
    let _installed = eframe::WebLogger::init(log::LevelFilter::Info);
    log::info!(
        "AFIK Studio {} starting in the browser",
        env!("CARGO_PKG_VERSION")
    );
    wasm_bindgen_futures::spawn_local(async {
        let Some(canvas) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("afik_studio"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            log::error!("the page has no afik_studio canvas");
            return;
        };
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|creation| {
                    theme::apply(&creation.egui_ctx);
                    Ok(Box::new(StudioApp::new(&Options::default())))
                }),
            )
            .await;
        if let Err(error) = started {
            log::error!("could not start the editor: {error:?}");
        }
    });
}
