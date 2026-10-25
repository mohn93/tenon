//! An exit macOS asks for: Dock > Quit, log out, shut down. These reach AppKit's `terminate:`
//! (the app menu's Quit is the window's own item and does not), which asks the app delegate's
//! `applicationShouldTerminate:` first. winit's delegate does not answer it, so AppKit would
//! end the app at once and winit's `exiting` would be the window's only word. `install` adds
//! the method to winit's delegate class: with nothing unsaved it answers NSTerminateNow, as
//! before; otherwise NSTerminateLater, and wakes the window, which answers with `reply` once
//! it has decided (`app::system_quit`): at once when the changes are in the autosave, else
//! after the unsaved prompt (go ahead after Save or Don't Save, cancel on Cancel). AppKit runs
//! its loop in the modal panel mode until then; winit's run loop observers and its proxy's
//! source are in the common modes, so the window's event handler still runs. The answer itself
//! is sent from the run loop after the handler returns (`performSelector:...inModes:`), so
//! AppKit ending the app never runs winit's handler inside the window's.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{MainThreadMarker, ffi, msg_send, sel};
use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
use objc2_foundation::{NSArray, NSString};

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
    let send_reply: extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) = send_reply;
    // SAFETY: the class is registered; each function takes the receiver, the selector and one
    // object and returns what its type encoding says (an NSUInteger, nothing). A class that
    // answers a selector already keeps its own method (class_addMethod adds none then).
    let added = unsafe {
        let class = std::ptr::from_ref(class).cast_mut();
        let asks = ffi::class_addMethod(
            class,
            sel!(applicationShouldTerminate:),
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
                Imp,
            >(should_terminate),
            c"Q@:@".as_ptr(),
        );
        let replies = ffi::class_addMethod(
            class,
            sel!(sketchupAltReplyToTerminate:),
            std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject), Imp>(
                send_reply,
            ),
            c"v@:@".as_ptr(),
        );
        asks.as_bool() && replies.as_bool()
    };
    if !added {
        eprintln!("warning: the application delegate answers the system's quit already");
    }
}

/// Keeps what the window last saw: whether the open model has unsaved changes.
pub fn set_unsaved(unsaved: bool) {
    UNSAVED.set(unsaved);
}

/// Answers the system's waiting quit, once: go ahead (`terminate`) or cancel it. The answer
/// reaches AppKit from the run loop once the window's event handler has returned. Does nothing
/// when none waits.
pub fn reply(terminate: bool) {
    if !ASKED.replace(false) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: `delegate` returns the app's delegate or nil.
    let delegate: Option<Retained<AnyObject>> = unsafe { msg_send![&*app, delegate] };
    let Some(delegate) = delegate else {
        app.replyToApplicationShouldTerminate(terminate);
        return;
    };
    // Any object for yes, nil for no.
    let answer: *const AnyObject = if terminate {
        std::ptr::from_ref::<NSApplication>(&app).cast()
    } else {
        std::ptr::null()
    };
    let modes = NSArray::from_retained_slice(&[NSString::from_str("kCFRunLoopCommonModes")]);
    // SAFETY: `install` added the selector to the delegate's class; the delay is a double and
    // the modes an array of strings, as `performSelector:withObject:afterDelay:inModes:`
    // takes. The delegate keeps the app alive, and the run loop retains its argument.
    unsafe {
        let _: () = msg_send![
            &*delegate,
            performSelector: sel!(sketchupAltReplyToTerminate:),
            withObject: answer,
            afterDelay: 0.0f64,
            inModes: &*modes
        ];
    }
}

/// `sketchupAltReplyToTerminate:`: sends the answer `reply` deferred; `answer` is nil for no.
extern "C-unwind" fn send_reply(_: &AnyObject, _: Sel, answer: *mut AnyObject) {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).replyToApplicationShouldTerminate(!answer.is_null());
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
