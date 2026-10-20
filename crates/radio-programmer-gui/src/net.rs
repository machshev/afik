//! HTTPS fetches for the station, satellite and map lists.

use std::time::Duration;

/// Fetches `url`, allowing `timeout` and refusing a body over `max_bytes`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn get(
    url: &str,
    timeout: Duration,
    max_bytes: u64,
    user_agent: &str,
) -> Result<Vec<u8>, String> {
    use std::io::Read;

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(user_agent)
        .build()
        .into();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| format!("{url}: {error}"))?;
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(max_bytes + 1)
        .read_to_end(&mut body)
        .map_err(|error| format!("{url}: {error}"))?;
    // A cut-off list could still parse, missing whatever came last.
    if body.len() as u64 > max_bytes {
        return Err(format!("{url}: over {max_bytes} bytes"));
    }
    Ok(body)
}

/// Fetches `url`: not yet in the browser, where a fetch is asynchronous and
/// each list's server must allow the page's origin.
#[cfg(target_arch = "wasm32")]
pub(crate) fn get(
    url: &str,
    _timeout: Duration,
    _max_bytes: u64,
    _user_agent: &str,
) -> Result<Vec<u8>, String> {
    Err(format!(
        "{url}: fetching is not available in the browser yet"
    ))
}
