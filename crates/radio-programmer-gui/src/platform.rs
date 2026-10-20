//! What differs between the desktop Studio and the browser Studio
//! (`ADR-119`): where background work runs. Everything else is shared.

use std::{future::Future, io};

/// Runs `work` off the user interface: a named worker thread blocking on its
/// future on the desktop, a task of the page in the browser.
///
/// `work` only builds the future once it is where it will run, so the future
/// itself need not be `Send`.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn<W, F>(name: &str, work: W) -> io::Result<()>
where
    W: FnOnce() -> F + Send + 'static,
    F: Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || radio_flasher::block_on(work()))
        .map(drop)
}

/// Runs `work` off the user interface: a named worker thread blocking on its
/// future on the desktop, a task of the page in the browser.
#[cfg(target_arch = "wasm32")]
pub fn spawn<W, F>(_name: &str, work: W) -> io::Result<()>
where
    W: FnOnce() -> F + Send + 'static,
    F: Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(work());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::spawn;

    #[test]
    fn spawned_work_runs_to_completion_and_reports_back() {
        let (sender, outcome) = mpsc::channel();
        spawn("afik-test", move || async move {
            let _ignored = sender.send(7);
        })
        .unwrap();
        assert_eq!(outcome.recv().unwrap(), 7);
    }
}
