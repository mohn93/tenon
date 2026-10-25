//! Files macOS asks the app to open: a double-clicked `.skr` in Finder, Open With, a file
//! dropped on the Dock icon. They arrive as `application:openURLs:` on the app delegate, never
//! as arguments, and winit's delegate does not answer it (AppKit would say the app cannot open
//! the file). `install` adds the method to winit's delegate class, as `system_quit` adds
//! `applicationShouldTerminate:`, and hands the paths to `send`, which forwards them to the
//! event loop (`AppEvent::OpenFiles`). It must run before the event loop does: a launch to
//! open a file asks before `applicationDidFinishLaunching:`, so before the window exists.
//!
//! It also registers `NSTreatUnknownArgumentsAsOpen` as NO: otherwise AppKit takes every
//! argument not starting with `-` (the model given on the command line, a folder after
//! `--api-root`) for a file to open as well.

use std::cell::RefCell;
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{ffi, sel};
use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL, NSUserDefaults};

/// Takes the paths of the files to open.
type Sink = Box<dyn Fn(Vec<PathBuf>)>;

thread_local! {
    /// Where the paths go; set by `install`.
    static SEND: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

/// The class of winit 0.30's application delegate.
const DELEGATE: &std::ffi::CStr = c"WinitApplicationDelegate";

/// Hands the files the system asks to open to `send` from now on (see the module's comment).
/// Main thread, after the event loop was made and before it runs; does nothing when the
/// delegate is not winit's.
pub fn install(send: impl Fn(Vec<PathBuf>) + 'static) {
    SEND.with_borrow_mut(|s| *s = Some(Box::new(send)));
    let key = NSString::from_str("NSTreatUnknownArgumentsAsOpen");
    let no: Retained<AnyObject> =
        Retained::into_super(Retained::into_super(NSString::from_str("NO")));
    let defaults = NSDictionary::from_slices(&[&*key], &[&*no]);
    // SAFETY: the dictionary maps a string to an object, as `registerDefaults:` takes.
    unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&defaults) };
    let Some(class) = AnyClass::get(DELEGATE) else {
        eprintln!("warning: no application delegate to open the files Finder asks for");
        return;
    };
    let open_urls: extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut NSArray<NSURL>) =
        open_urls;
    // SAFETY: the class is registered; the function takes the receiver, the selector, the
    // application and an array of URLs and returns nothing, as its type encoding says. A class
    // that answers the selector already keeps its own method (class_addMethod adds none then).
    let added = unsafe {
        ffi::class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(application:openURLs:),
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject, *mut NSArray<NSURL>),
                Imp,
            >(open_urls),
            c"v@:@@".as_ptr(),
        )
    };
    if !added.as_bool() {
        eprintln!("warning: the application delegate opens files already");
    }
}

/// `application:openURLs:`: the file URLs' paths, to `send`; other URLs are ignored.
extern "C-unwind" fn open_urls(
    _: &AnyObject,
    _: Sel,
    _: *mut AnyObject,
    urls: *mut NSArray<NSURL>,
) {
    // SAFETY: AppKit passes an array of URLs (or nil).
    let Some(urls) = (unsafe { urls.as_ref() }) else {
        return;
    };
    let paths: Vec<PathBuf> = urls
        .iter()
        .filter(|url| url.isFileURL())
        .filter_map(|url| url.to_file_path())
        .collect();
    if paths.is_empty() {
        return;
    }
    SEND.with_borrow(|send| {
        if let Some(send) = send {
            send(paths);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::rc::Rc;

    use objc2_foundation::NSObject;

    use super::*;

    #[test]
    fn the_file_urls_reach_the_window_as_paths() {
        let got: Rc<RefCell<Vec<Vec<PathBuf>>>> = Rc::default();
        let sink = got.clone();
        SEND.with_borrow_mut(|s| *s = Some(Box::new(move |paths| sink.borrow_mut().push(paths))));
        let file = NSURL::from_file_path(Path::new("/tmp/a b/model.skr")).unwrap();
        let web = NSURL::URLWithString(&NSString::from_str("https://example.com/x.skr")).unwrap();
        let receiver = NSObject::new();
        let call = |urls: &[&NSURL]| {
            let urls = NSArray::from_slice(urls);
            open_urls(
                &receiver,
                sel!(application:openURLs:),
                std::ptr::null_mut(),
                Retained::as_ptr(&urls).cast_mut(),
            );
        };
        call(&[&file, &web]);
        call(&[&web]);
        open_urls(
            &receiver,
            sel!(application:openURLs:),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(*got.borrow(), [vec![PathBuf::from("/tmp/a b/model.skr")]]);
        SEND.with_borrow_mut(|s| *s = None);
    }
}
