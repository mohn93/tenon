//! The browser as the app's `Platform`: files by upload and download (or the File System Access
//! API), preferences in `localStorage`, the clipboard through the page's clipboard events.
//! Dialogs are the window's own (`Features::WEB`), so there is no native question to ask.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use app::app::AppEvent;
use app::clipboard::Clipboard;
use app::platform::{
    Answer, Download, Features, OpenKind, OpenRequest, Picked, Platform, Reply, SaveRequest, Saved,
    Source, UnsavedChoice, UnsavedPrompt,
};
use js_sys::Reflect;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::Event;
use winit::event_loop::EventLoopProxy;
use winit::keyboard::ModifiersState;

use crate::files;
use crate::names::{self, Handles, Storage};

/// The file handle Save writes to (File System Access API), see `names::Handles`.
type KeptFile = Rc<RefCell<Handles<JsValue>>>;

pub struct WebPlatform {
    kept: KeptFile,
    clip: Rc<RefCell<Option<String>>>,
    /// Whether the open model has unsaved changes (`Platform::set_modified`).
    modified: Rc<Cell<bool>>,
    /// The modifier keys of the key event being delivered (`watch_modifiers`).
    held: Rc<Cell<ModifiersState>>,
    /// Reaches the window: late errors for its status line.
    proxy: EventLoopProxy<AppEvent>,
    /// The phone viewer: `Features::VIEWER` rather than `WEB`.
    viewer: bool,
}

impl WebPlatform {
    pub fn new(proxy: EventLoopProxy<AppEvent>, viewer: bool) -> Self {
        let clip = Rc::new(RefCell::new(None));
        watch_clipboard(&clip, &proxy);
        let modified = Rc::new(Cell::new(false));
        guard_unload(&modified);
        let held = Rc::new(Cell::new(ModifiersState::empty()));
        watch_modifiers(&held);
        Self {
            held,
            kept: Rc::default(),
            clip,
            modified,
            proxy,
            viewer,
        }
    }
}

fn report(proxy: &EventLoopProxy<AppEvent>, text: &str) {
    log(text);
    let _ = proxy.send_event(AppEvent::Error(text.to_string()));
}

/// Keeps `held` as the modifier keys of each key event, read before winit sees the event: winit
/// reports a modifier change after the key that came with it, so Ctrl+Z pressed as one event
/// (and Cmd, whose key-up the browser never sends) would be read without its modifier.
fn watch_modifiers(held: &Rc<Cell<ModifiersState>>) {
    let note = {
        let held = held.clone();
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            let mut state = ModifiersState::empty();
            state.set(ModifiersState::SHIFT, event.shift_key());
            state.set(ModifiersState::CONTROL, event.ctrl_key());
            state.set(ModifiersState::ALT, event.alt_key());
            state.set(ModifiersState::SUPER, event.meta_key());
            held.set(state);
        })
    };
    // In the capture phase, so before winit's listener on the canvas.
    let document = files::document();
    for name in ["keydown", "keyup"] {
        let _ = document.add_event_listener_with_callback_and_bool(
            name,
            note.as_ref().unchecked_ref(),
            true,
        );
    }
    note.forget();
}

/// Asks the browser to confirm before the tab closes or reloads while the model has unsaved
/// changes.
fn guard_unload(modified: &Rc<Cell<bool>>) {
    let modified = modified.clone();
    let on_unload = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        if modified.get() {
            event.prevent_default();
            // Older browsers want a returnValue.
            let _ = Reflect::set(&event, &"returnValue".into(), &"".into());
        }
    });
    let _ = files::window()
        .add_event_listener_with_callback("beforeunload", on_unload.as_ref().unchecked_ref());
    on_unload.forget();
}

/// `localStorage`, where the browser allows it.
struct Local;

impl Local {
    fn storage() -> Result<web_sys::Storage, String> {
        files::window()
            .local_storage()
            .map_err(|e| files::describe(&e))?
            .ok_or_else(|| "this browser keeps no local storage".to_string())
    }
}

impl Storage for Local {
    fn get(&self, key: &str) -> Option<String> {
        Self::storage().ok()?.get_item(key).ok().flatten()
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        Self::storage()?
            .set_item(key, value)
            .map_err(|e| files::describe(&e))
    }
}

impl Platform for WebPlatform {
    fn features(&self) -> Features {
        if self.viewer {
            Features::VIEWER
        } else {
            Features::WEB
        }
    }

    fn pick_open(
        &self,
        request: &OpenRequest,
        reply: Reply<Option<Picked>>,
    ) -> Answer<Option<Picked>> {
        let kept = self.kept.clone();
        let proxy = self.proxy.clone();
        let is_model = request.kind == OpenKind::Model;
        if files::has_function("showOpenFilePicker") {
            let filters = request.filters.clone();
            spawn_local(async move {
                match files::open_with_picker(&filters).await {
                    Ok(Some(file)) => {
                        // Kept once the model opened (`file_opened`), and only a model's.
                        if is_model {
                            kept.borrow_mut().picked(&file.name, Some(file.handle));
                        }
                        reply(Some(Picked {
                            name: file.name,
                            source: Source::Bytes(file.bytes),
                        }));
                    }
                    Ok(None) => reply(None),
                    Err(why) => {
                        report(&proxy, &format!("Cannot open the file: {why}"));
                        reply(None);
                    }
                }
            });
        } else {
            let accept = names::accept_attribute(&request.filters);
            let failed = Rc::new(RefCell::new(Some(reply)));
            let on_done = failed.clone();
            let on_error = proxy.clone();
            let started = files::pick_with_input(&accept, move |chosen| {
                let Some(reply) = on_done.borrow_mut().take() else {
                    return;
                };
                match chosen {
                    Ok(Some((name, bytes))) => {
                        if is_model {
                            kept.borrow_mut().picked(&name, None);
                        }
                        reply(Some(Picked {
                            name,
                            source: Source::Bytes(bytes),
                        }));
                    }
                    Ok(None) => reply(None),
                    Err(why) => {
                        report(&on_error, &format!("Cannot open the file: {why}"));
                        reply(None);
                    }
                }
            });
            if let Err(why) = started {
                report(&proxy, &format!("Cannot ask for a file: {why}"));
                if let Some(reply) = failed.borrow_mut().take() {
                    reply(None);
                }
            }
        }
        Answer::Pending
    }

    fn pick_save_path(
        &self,
        _request: &SaveRequest,
        _reply: Reply<Option<PathBuf>>,
    ) -> Answer<Option<PathBuf>> {
        // Files have no paths here (`Features::file_paths` is off).
        Answer::Ready(None)
    }

    fn pick_folder(&self, _reply: Reply<Option<PathBuf>>) -> Answer<Option<PathBuf>> {
        Answer::Ready(None)
    }

    fn save_files(
        &self,
        request: &SaveRequest,
        files: Vec<Download>,
        reply: Reply<Saved>,
    ) -> Answer<Saved> {
        let kept = self.kept.clone();
        let request = request.clone();
        spawn_local(async move { reply(save(kept, request, files).await) });
        Answer::Pending
    }

    fn read_prefs(&self) -> Option<String> {
        names::read_prefs(&Local)
    }

    fn write_prefs(&self, text: &str) -> Result<(), String> {
        names::write_prefs(&Local, text)
    }

    fn confirm_unsaved(
        &self,
        _parent: Option<&winit::window::Window>,
        _prompt: &UnsavedPrompt,
        _reply: Reply<UnsavedChoice>,
    ) -> Answer<UnsavedChoice> {
        // The window asks its own question (`Features::in_app_dialogs`).
        Answer::Ready(UnsavedChoice::Cancel)
    }

    fn clipboard(&self) -> Box<dyn Clipboard> {
        Box::new(WebClipboard {
            text: self.clip.clone(),
        })
    }

    fn event_modifiers(&self) -> Option<ModifiersState> {
        Some(self.held.get())
    }

    fn copied(&self, text: &str) {
        *self.clip.borrow_mut() = Some(text.to_string());
        write_system_clipboard(text);
    }

    fn file_opened(&self, name: &str) {
        self.kept.borrow_mut().opened(name);
    }

    fn set_modified(&self, modified: bool) {
        self.modified.set(modified);
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        files::window()
            .open_with_url_and_target(url, "_blank")
            .map(|_| ())
            .map_err(|e| files::describe(&e))
    }
}

/// Saves `files`: the first is the one the user named. A single file Save writes to the file
/// the model is bound to (`SaveRequest::write_back`); otherwise, and for Save As and Export,
/// the browser's save picker asks where when it has one; everything else is downloaded (an
/// OBJ's `.mtl` and textures are downloads beside it). Saving a model replaces the kept handle
/// (none after a download); exports leave it alone.
async fn save(kept: KeptFile, request: SaveRequest, files: Vec<Download>) -> Saved {
    let Some(main) = files.first() else {
        return Ok(None);
    };
    let name = names::download_name(&main.name, "model.skr");
    let single = files.len() == 1;
    // A model's save (`.skr`); exports have their own formats.
    let model = names::writes_back(&main.name);
    if single && !request.ask {
        let handle = kept.borrow().for_save(request.write_back);
        if let Some(handle) = handle {
            match files::write_handle(&handle, &main.bytes).await {
                Ok(()) => return Ok(Some(name)),
                // The permission was refused or the file is gone: ask where instead.
                Err(why) => log(&format!("cannot write to the file: {why}")),
            }
        }
    }
    if single && files::has_function("showSaveFilePicker") {
        match files::save_picker(&request.filters, &name).await {
            Ok(Some(handle)) => {
                let saved_as = files::handle_name(&handle);
                files::write_handle(&handle, &main.bytes).await?;
                if model {
                    kept.borrow_mut().saved(&saved_as, Some(handle));
                }
                return Ok(Some(saved_as));
            }
            Ok(None) => return Ok(None),
            Err(why) => log(&format!("cannot ask where to save, downloading: {why}")),
        }
    }
    for file in &files {
        files::download(&names::download_name(&file.name, "model.skr"), &file.bytes)?;
    }
    if model {
        kept.borrow_mut().saved(&name, None);
    }
    Ok(Some(name))
}

fn log(text: &str) {
    web_sys::console::error_1(&text.into());
}

/// The clipboard Cut, Copy and Paste use. A page cannot read the clipboard when it likes, so
/// it keeps the text last copied in the app or pasted into the page (`watch_clipboard`) and
/// hands that to Paste; Copy also writes the system clipboard.
struct WebClipboard {
    text: Rc<RefCell<Option<String>>>,
}

impl Clipboard for WebClipboard {
    fn get_text(&mut self) -> Result<Option<String>, String> {
        Ok(self.text.borrow().clone())
    }

    fn set_text(&mut self, text: String) -> Result<(), String> {
        write_system_clipboard(&text);
        *self.text.borrow_mut() = Some(text);
        Ok(())
    }
}

/// Puts `text` on the system clipboard where the browser allows it.
fn write_system_clipboard(text: &str) {
    if let Some(clipboard) = system_clipboard()
        && let Ok(promise) = call(&clipboard, "writeText", &text.into())
    {
        spawn_local(async move {
            // Refused without permission: the page's own copy still works.
            let _ = JsFuture::from(js_sys::Promise::resolve(&promise)).await;
        });
    }
}

fn system_clipboard() -> Option<JsValue> {
    let navigator: JsValue = files::window().navigator().into();
    Reflect::get(&navigator, &"clipboard".into())
        .ok()
        .filter(|c| !c.is_undefined() && !c.is_null())
}

fn call(target: &JsValue, method: &str, arg: &JsValue) -> Result<JsValue, JsValue> {
    let function: js_sys::Function = Reflect::get(target, &method.into())?.dyn_into()?;
    function.call1(target, arg)
}

/// Keeps `clip` current and pastes: every paste event (Ctrl+V or Cmd+V) stores its text and
/// sends it to the window, which pastes it (the key press itself pastes nothing on the web, so
/// text copied outside the app is never stale). Also reads the system clipboard's text when the page gains focus, where the browser allows
/// reading it.
fn watch_clipboard(clip: &Rc<RefCell<Option<String>>>, proxy: &EventLoopProxy<AppEvent>) {
    let on_paste = {
        let clip = clip.clone();
        let proxy = proxy.clone();
        Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            let data = Reflect::get(&event, &"clipboardData".into()).unwrap_or(JsValue::NULL);
            if data.is_null() || data.is_undefined() {
                return;
            }
            if let Ok(text) = call(&data, "getData", &"text/plain".into())
                && let Some(text) = text.as_string()
                && !text.is_empty()
            {
                *clip.borrow_mut() = Some(text.clone());
                // The window pastes now: the key press itself does nothing on the web.
                let _ = proxy.send_event(AppEvent::Paste(text));
            }
        })
    };
    let _ = files::document()
        .add_event_listener_with_callback("paste", on_paste.as_ref().unchecked_ref());
    on_paste.forget();

    let on_focus = {
        let clip = clip.clone();
        Closure::<dyn FnMut(Event)>::new(move |_: Event| {
            let Some(clipboard) = system_clipboard() else {
                return;
            };
            let Ok(function) = Reflect::get(&clipboard, &"readText".into())
                .and_then(|f| f.dyn_into::<js_sys::Function>())
            else {
                return;
            };
            let Ok(promise) = function.call0(&clipboard) else {
                return;
            };
            let clip = clip.clone();
            spawn_local(async move {
                // A refusal (no permission) leaves the text as it was.
                if let Ok(text) = JsFuture::from(js_sys::Promise::resolve(&promise)).await
                    && let Some(text) = text.as_string()
                    && !text.is_empty()
                {
                    *clip.borrow_mut() = Some(text);
                }
            });
        })
    };
    let _ = files::window()
        .add_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref());
    on_focus.forget();
}
