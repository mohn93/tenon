//! Browser files: the file picker (`<input type=file>`, or the File System Access API where the
//! browser has it), writing to a file the user picked, and downloads. The File System Access
//! API is reached through `js_sys::Reflect` because the browsers that lack it must still run
//! the page.

use std::cell::RefCell;
use std::rc::Rc;

use app::platform::Filter;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AddEventListenerOptions, Blob, BlobPropertyBag, Document, Event, File, HtmlAnchorElement,
    HtmlInputElement, Url, Window,
};

use crate::names;

pub fn window() -> Window {
    web_sys::window().expect("a page has a window")
}

pub fn document() -> Document {
    window().document().expect("a page has a document")
}

/// What a failed JavaScript call says.
pub fn describe(error: &JsValue) -> String {
    if let Some(e) = error.dyn_ref::<js_sys::Error>() {
        return String::from(e.message());
    }
    error
        .as_string()
        .or_else(|| {
            Reflect::get(error, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{error:?}"))
}

/// Whether `error` is the browser saying the user dismissed a picker.
fn is_abort(error: &JsValue) -> bool {
    Reflect::get(error, &"name".into())
        .ok()
        .and_then(|n| n.as_string())
        .is_some_and(|n| n == "AbortError")
}

/// Whether the window has the function `name` (the File System Access pickers).
pub fn has_function(name: &str) -> bool {
    Reflect::get(&window(), &name.into()).is_ok_and(|f| f.is_function())
}

fn set(target: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(target, &key.into(), value);
}

/// Calls the method `name` of `target` with `args`.
fn call(target: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let method: Function = Reflect::get(target, &name.into())?.dyn_into()?;
    let args: Array = args.iter().collect();
    method.apply(target, &args)
}

async fn settle(value: Result<JsValue, JsValue>) -> Result<JsValue, JsValue> {
    JsFuture::from(Promise::resolve(&value?)).await
}

/// The bytes of `file`.
pub async fn read_file(file: &File) -> Result<Vec<u8>, String> {
    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e| describe(&e))?;
    Ok(Uint8Array::new(&buffer).to_vec())
}

/// The picker's `types`: one entry for each filter.
fn picker_types(filters: &[Filter]) -> Array {
    let types = Array::new();
    for filter in filters {
        let extensions: Array = filter
            .extensions
            .iter()
            .map(|e| JsValue::from_str(&format!(".{e}")))
            .collect();
        let accept = Object::new();
        set(&accept, "application/octet-stream", &extensions);
        let entry = Object::new();
        set(&entry, "description", &filter.name.into());
        set(&entry, "accept", &accept);
        types.push(&entry);
    }
    types
}

/// A file picked with `showOpenFilePicker`: its name, its bytes and the handle Save can write
/// back to.
pub struct PickedFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub handle: JsValue,
}

/// Asks for a file with the File System Access API. None: the user dismissed the picker.
pub async fn open_with_picker(filters: &[Filter]) -> Result<Option<PickedFile>, String> {
    let options = Object::new();
    set(&options, "types", &picker_types(filters));
    set(&options, "multiple", &JsValue::FALSE);
    let window: JsValue = window().into();
    let handles = match settle(call(&window, "showOpenFilePicker", &[options.into()])).await {
        Ok(handles) => handles,
        Err(e) if is_abort(&e) => return Ok(None),
        Err(e) => return Err(describe(&e)),
    };
    let handle = Array::from(&handles).get(0);
    let file = settle(call(&handle, "getFile", &[]))
        .await
        .map_err(|e| describe(&e))?;
    let file: File = file.dyn_into().map_err(|_| "not a file".to_string())?;
    let bytes = read_file(&file).await?;
    Ok(Some(PickedFile {
        name: file.name(),
        bytes,
        handle,
    }))
}

/// Asks where to save `name` with `showSaveFilePicker`. None: the user dismissed the picker.
pub async fn save_picker(filters: &[Filter], name: &str) -> Result<Option<JsValue>, String> {
    let options = Object::new();
    set(&options, "suggestedName", &name.into());
    set(&options, "types", &picker_types(filters));
    let window: JsValue = window().into();
    match settle(call(&window, "showSaveFilePicker", &[options.into()])).await {
        Ok(handle) => Ok(Some(handle)),
        Err(e) if is_abort(&e) => Ok(None),
        Err(e) => Err(describe(&e)),
    }
}

/// The name of a file handle.
pub fn handle_name(handle: &JsValue) -> String {
    Reflect::get(handle, &"name".into())
        .ok()
        .and_then(|n| n.as_string())
        .unwrap_or_default()
}

/// Replaces the content of the file `handle` names with `bytes`.
pub async fn write_handle(handle: &JsValue, bytes: &[u8]) -> Result<(), String> {
    let write = async {
        let writable = settle(call(handle, "createWritable", &[])).await?;
        let data = Uint8Array::from(bytes);
        settle(call(&writable, "write", &[data.into()])).await?;
        settle(call(&writable, "close", &[])).await?;
        Ok::<(), JsValue>(())
    };
    write.await.map_err(|e| describe(&e))
}

/// A file chosen with an `<input type=file>`: its name and bytes (None: nothing chosen).
pub type Chosen = Result<Option<(String, Vec<u8>)>, String>;

/// Shows the browser's file chooser for files with the extensions in `accept` and calls `done`
/// once, with the chosen file or None when the user cancels. Must run from a user gesture.
pub fn pick_with_input(accept: &str, done: impl FnOnce(Chosen) + 'static) -> Result<(), String> {
    let document = document();
    let input: HtmlInputElement = document
        .create_element("input")
        .map_err(|e| describe(&e))?
        .dyn_into()
        .map_err(|_| "not an input".to_string())?;
    input.set_type("file");
    input.set_accept(accept);
    let _ = input.style().set_property("display", "none");
    document
        .body()
        .ok_or("the page has no body")?
        .append_child(&input)
        .map_err(|e| describe(&e))?;

    let done: Rc<RefCell<Option<Box<dyn FnOnce(Chosen)>>>> =
        Rc::new(RefCell::new(Some(Box::new(done))));
    let on_change = {
        let input = input.clone();
        let done = done.clone();
        Closure::once(move |_: Event| {
            let file = input.files().and_then(|files| files.get(0));
            input.remove();
            let Some(done) = done.borrow_mut().take() else {
                return;
            };
            match file {
                None => done(Ok(None)),
                Some(file) => spawn_local(async move {
                    let bytes = read_file(&file).await;
                    done(bytes.map(|bytes| Some((file.name(), bytes))));
                }),
            }
        })
    };
    // Browsers without a `cancel` event say nothing when the chooser is dismissed: the page
    // gaining focus again with no `change` within a second means nothing was chosen.
    let on_focus = {
        let input = input.clone();
        let done = done.clone();
        Closure::once(move |_: Event| {
            let late = Closure::once_into_js(move || {
                if let Some(done) = done.borrow_mut().take() {
                    input.remove();
                    done(Ok(None));
                }
            });
            let _ = window()
                .set_timeout_with_callback_and_timeout_and_arguments_0(late.unchecked_ref(), 1000);
        })
    };
    let on_cancel = {
        let input = input.clone();
        let done = done.clone();
        Closure::once(move |_: Event| {
            input.remove();
            if let Some(done) = done.borrow_mut().take() {
                done(Ok(None));
            }
        })
    };
    let target: &web_sys::EventTarget = &input;
    target
        .add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())
        .map_err(|e| describe(&e))?;
    target
        .add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref())
        .map_err(|e| describe(&e))?;
    // Once: a later focus must not call a closure that has run.
    let once = AddEventListenerOptions::new();
    once.set_once(true);
    window()
        .add_event_listener_with_callback_and_add_event_listener_options(
            "focus",
            on_focus.as_ref().unchecked_ref(),
            &once,
        )
        .map_err(|e| describe(&e))?;
    on_focus.forget();
    // The listeners live until the input is removed and dropped with the page.
    on_change.forget();
    on_cancel.forget();
    input.click();
    Ok(())
}

/// Offers `bytes` as a download named `name`.
pub fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    let data = Uint8Array::from(bytes);
    let parts = Array::of1(&data);
    let options = BlobPropertyBag::new();
    options.set_type(names::mime_for(name));
    let blob =
        Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(|e| describe(&e))?;
    let url = Url::create_object_url_with_blob(&blob).map_err(|e| describe(&e))?;
    let document = document();
    let link: HtmlAnchorElement = document
        .create_element("a")
        .map_err(|e| describe(&e))?
        .dyn_into()
        .map_err(|_| "not a link".to_string())?;
    link.set_href(&url);
    link.set_download(name);
    let _ = link.style().set_property("display", "none");
    let body = document.body().ok_or("the page has no body")?;
    body.append_child(&link).map_err(|e| describe(&e))?;
    link.click();
    link.remove();
    // The download has started; the object URL is released shortly after.
    let release = Closure::once_into_js(move || {
        let _ = Url::revoke_object_url(&url);
    });
    let _ = window()
        .set_timeout_with_callback_and_timeout_and_arguments_0(release.unchecked_ref(), 30_000);
    Ok(())
}
