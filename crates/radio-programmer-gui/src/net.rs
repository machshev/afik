//! HTTPS fetches for the station, satellite and map lists.

/// Fetches `url`, allowing `timeout` and refusing a body over `max_bytes`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn get(
    url: &str,
    timeout: std::time::Duration,
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

/// Fetches `url`, relative to the page, from the page's own server or one
/// that allows its origin. A body over `maximum` bytes is refused.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn fetch(url: &str, maximum: usize) -> Result<Vec<u8>, String> {
    use js_sys::Uint8Array;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let failed = |error: wasm_bindgen::JsValue| format!("{url}: {error:?}");
    let window = web_sys::window().ok_or("no browser window")?;
    let response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(failed)?
        .unchecked_into::<web_sys::Response>();
    if !response.ok() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    let body = JsFuture::from(response.array_buffer().map_err(failed)?)
        .await
        .map_err(failed)?;
    let bytes = Uint8Array::new(&body).to_vec();
    if bytes.len() > maximum {
        return Err(format!("{url}: more than {maximum} bytes"));
    }
    Ok(bytes)
}
