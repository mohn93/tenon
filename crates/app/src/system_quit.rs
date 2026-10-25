//! An exit macOS asks for: Dock > Quit, log out, shut down. These reach AppKit's `terminate:`
//! (the app menu's Quit is the window's own item and does not), which asks the app delegate's
//! `applicationShouldTerminate:` first. winit's delegate does not answer it, so AppKit would
//! end the app at once and winit's `exiting` would be the window's only word. `install` adds
//! the method to winit's delegate class: with nothing unsaved it answers NSTerminateNow, as
//! before; otherwise NSTerminateLater, and wakes the window, which answers with `reply` once
//! it has decided (`app::system_quit`): the system's quit goes ahead, or is cancelled while the
//! unsaved prompt shows. AppKit runs its loop in the modal panel mode until then; winit's run
//! loop observers and its proxy's source are in the common modes, so the window's event
//! handler still runs.

use std::cell::{Cell, RefCell};

use objc2::MainThreadMarker;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{ffi, sel};
use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};

thread_local! {
    /// Whether the open model has unsaved changes, as last seen by the window.
    static UNSAVED: Cell<bool> = const { Cell::new(false) };
    /// Wakes the window to decide; set by `install`.
    static WAKE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    /// Whether the system asked and waits for `reply`.
    static ASKED: Cell<bool> = const { Cell::new(false) };
}

/// The class of winit 0.30's application delegate.
const DELEGATE: &std::ffi::CStr = c"WinitApplicationDelegate";

/// Answers the system's quits from now on, calling `wake` when one waits for the window (see
/// the module's comment). Main thread, after the event loop was made; does nothing when the
/// delegate is not winit's.
pub fn install(wake: impl Fn() + 'static) {
    WAKE.with_borrow_mut(|w| *w = Some(Box::new(wake)));
    let Some(class) = AnyClass::get(DELEGATE) else {
        eprintln!("warning: no application delegate to answer the system's quit");
        return;
    };
    let should_terminate: extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize =
        should_terminate;
    // SAFETY: the class is registered; the function takes the receiver, the selector and the
    // sender and returns an NSUInteger, as the type encoding says. A class that answers the
    // selector already keeps its own method (class_addMethod adds none then).
    let added = unsafe {
        ffi::class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(applicationShouldTerminate:),
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
                Imp,
            >(should_terminate),
            c"Q@:@".as_ptr(),
        )
    };
    if !added.as_bool() {
        eprintln!("warning: the application delegate answers the system's quit already");
    }
}

/// Keeps what the window last saw: whether the open model has unsaved changes.
pub fn set_unsaved(unsaved: bool) {
    UNSAVED.set(unsaved);
}

/// Answers the system's waiting quit: go ahead (`terminate`) or cancel it. Does nothing when
/// none waits.
pub fn reply(terminate: bool) {
    if !ASKED.replace(false) {
        return;
    }
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).replyToApplicationShouldTerminate(terminate);
    }
}

/// `applicationShouldTerminate:`: now when nothing is unsaved (or nothing can decide), else
/// later, once the window replies.
extern "C-unwind" fn should_terminate(_: &AnyObject, _: Sel, _: *mut AnyObject) -> usize {
    let reply = if !UNSAVED.get() {
        NSApplicationTerminateReply::TerminateNow
    } else if ASKED.get() {
        // Asked again before the window replied: the first answer stands.
        NSApplicationTerminateReply::TerminateLater
    } else if WAKE.with_borrow(|wake| wake.as_ref().map(|wake| wake()).is_some()) {
        ASKED.set(true);
        NSApplicationTerminateReply::TerminateLater
    } else {
        NSApplicationTerminateReply::TerminateNow
    };
    reply.0
}
