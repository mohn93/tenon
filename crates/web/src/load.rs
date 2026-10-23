//! The phone viewer's browser side: reading the share out of the page's address, fetching the
//! encrypted model from the computer that served the page, and taking the key out of the
//! address bar. The decisions (what a fragment means, the size cap, the messages) are in
//! `viewer`; nothing here logs the fragment or the key.

use js_sys::{Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, ReadableStreamDefaultReader, RequestCache, RequestCredentials, RequestInit,
    Response,
};

use crate::files::window;
use crate::viewer::{Collector, Failure, check_length};

/// The page's `location.hash` ("" when it has none or the browser will not say).
pub fn hash() -> String {
    window().location().hash().unwrap_or_default()
}

/// Takes the fragment (and the query) out of the address bar and the history entry, so the key
/// is kept nowhere the browser shows or restores.
pub fn forget_fragment() {
    let window = window();
    let Ok(path) = window.location().pathname() else {
        return;
    };
    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&path));
    }
}

/// How long the server may stay silent (no response, or no next piece of the body) before the
/// request is given up as unreachable.
const IDLE_MS: i32 = 20_000;

/// Aborts `controller` after `IDLE_MS` unless `arm` is called again or the timer is dropped.
struct Timeout {
    controller: AbortController,
    callback: Closure<dyn FnMut()>,
    handle: Option<i32>,
}

impl Timeout {
    fn new(controller: AbortController) -> Self {
        let aborting = controller.clone();
        let callback = Closure::<dyn FnMut()>::new(move || aborting.abort());
        let mut timeout = Self {
            controller,
            callback,
            handle: None,
        };
        timeout.arm();
        timeout
    }

    fn arm(&mut self) {
        self.disarm();
        self.handle = window()
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                self.callback.as_ref().unchecked_ref(),
                IDLE_MS,
            )
            .ok();
    }

    fn disarm(&mut self) {
        if let Some(handle) = self.handle.take() {
            window().clear_timeout_with_handle(handle);
        }
    }
}

impl Drop for Timeout {
    fn drop(&mut self) {
        self.disarm();
        let _ = &self.controller;
    }
}

/// The model's encrypted bytes from `path` on the page's own server: nothing is cached, no
/// credentials are sent, and no more than the size cap is read. A server that cannot be
/// reached is `Unreachable`; one that answers anything but the blob, or too much, is `Damaged`.
pub async fn fetch_blob(path: &str) -> Result<Vec<u8>, Failure> {
    let init = RequestInit::new();
    init.set_cache(RequestCache::NoStore);
    init.set_credentials(RequestCredentials::Omit);
    let controller = AbortController::new().map_err(|_| Failure::Unreachable)?;
    init.set_signal(Some(&controller.signal()));
    let mut timeout = Timeout::new(controller);
    let response = JsFuture::from(window().fetch_with_str_and_init(path, &init))
        .await
        .map_err(|_| Failure::Unreachable)?;
    let response: Response = response.dyn_into().map_err(|_| Failure::Damaged)?;
    if !response.ok() {
        return Err(Failure::Damaged);
    }
    let declared = response
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|n| n.trim().parse::<u64>().ok());
    check_length(declared)?;
    let body = response.body().ok_or(Failure::Damaged)?;
    let reader: ReadableStreamDefaultReader =
        body.get_reader().dyn_into().map_err(|_| Failure::Damaged)?;
    let mut collected = Collector::default();
    loop {
        // A connection that drops half way is the server going away.
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|_| Failure::Unreachable)?;
        let done = Reflect::get(&chunk, &"done".into())
            .map_err(|_| Failure::Damaged)?
            .as_bool()
            .unwrap_or(true);
        if done {
            break;
        }
        timeout.arm();
        let value = Reflect::get(&chunk, &"value".into()).map_err(|_| Failure::Damaged)?;
        let bytes: Uint8Array = value.dyn_into().map_err(|_| Failure::Damaged)?;
        if !collected.fits(u64::from(bytes.length())) {
            let _ = reader.cancel();
            return Err(Failure::Damaged);
        }
        collected.extend(&bytes.to_vec())?;
    }
    Ok(collected.into_bytes())
}
