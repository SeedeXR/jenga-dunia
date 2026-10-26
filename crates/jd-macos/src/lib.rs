//! The documents macOS opens with Studio (ADR-0017, packaging). A `.jdproj` double-clicked in
//! Finder, dropped on the Dock icon or opened with "Open With" arrives as an "open documents" Apple
//! Event (`aevt`/`odoc`); Linux and Windows pass it on the command line instead, so [`install`]
//! does nothing there.
//!
//! winit 0.30 owns the application delegate (it panics if another replaces it) and does not
//! surface this event, so the handler is installed with `NSAppleEventManager` instead, from an
//! `NSApplicationWillFinishLaunchingNotification` observer: Apple's documented moment, after
//! AppKit's own handlers and before the launch document is dispatched.

use std::path::PathBuf;

/// Call `on_open` with the files macOS opens with the app, from now on (the first call wins).
/// Call on the main thread before the event loop starts.
#[cfg(target_os = "macos")]
pub fn install(on_open: impl Fn(Vec<PathBuf>) + Send + Sync + 'static) {
    mac::install(on_open);
}

/// Linux and Windows pass opened files on the command line: nothing to install.
#[cfg(not(target_os = "macos"))]
pub fn install(_on_open: impl Fn(Vec<PathBuf>) + Send + Sync + 'static) {}

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::NSApplicationWillFinishLaunchingNotification;
    use objc2_foundation::{
        NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter,
        NSObjectProtocol,
    };

    /// Four-character codes (CoreServices' AppleEvents.h).
    const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "JDStudioDocuments"]
        struct Documents;

        unsafe impl NSObjectProtocol for Documents {}

        impl Documents {
            #[unsafe(method(willFinishLaunching:))]
            fn will_finish_launching(&self, _note: &NSNotification) {
                let manager = NSAppleEventManager::sharedAppleEventManager();
                // SAFETY: `self` lives for the process (forgotten in `install`) and answers the
                // selector with the (event, reply) signature the manager calls.
                let () = unsafe {
                    msg_send![&*manager, setEventHandler: self,
                        andSelector: sel!(openDocuments:withReplyEvent:),
                        forEventClass: CORE_EVENT_CLASS,
                        andEventID: OPEN_DOCUMENTS]
                };
            }

            #[unsafe(method(openDocuments:withReplyEvent:))]
            fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
                // SAFETY: `paramDescriptorForKeyword:` takes an AEKeyword (a u32) and returns a
                // descriptor or nil.
                let list: Option<Retained<NSAppleEventDescriptor>> =
                    unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
                let Some(list) = list else { return };
                // A list of file URLs (descriptor indices are 1-based); one file is not a list.
                let n = list.numberOfItems();
                let items: Vec<_> = if n > 0 {
                    (1..=n).filter_map(|i| list.descriptorAtIndex(i)).collect()
                } else {
                    vec![list]
                };
                deliver(
                    items
                        .iter()
                        .filter_map(|d| d.fileURLValue()?.to_file_path())
                        .collect(),
                );
            }
        }
    );

    type Callback = Box<dyn Fn(Vec<PathBuf>) + Send + Sync>;
    static CALLBACK: OnceLock<Callback> = OnceLock::new();

    fn deliver(paths: Vec<PathBuf>) {
        tracing::info!(documents = ?paths, "macOS opened documents");
        if let Some(f) = CALLBACK.get() {
            f(paths);
        }
    }

    pub(super) fn install(on_open: impl Fn(Vec<PathBuf>) + Send + Sync + 'static) {
        if CALLBACK.set(Box::new(on_open)).is_err() {
            return; // installed already
        }
        let Some(mtm) = MainThreadMarker::new() else {
            tracing::warn!("documents: not on the main thread, Finder opens are not received");
            return;
        };
        // SAFETY: a plain NSObject subclass without instance variables.
        let documents: Retained<Documents> =
            unsafe { msg_send![super(mtm.alloc::<Documents>().set_ivars(())), init] };
        // SAFETY: the observer lives for the process (below); the selector takes one
        // NSNotification.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &documents,
                sel!(willFinishLaunching:),
                Some(NSApplicationWillFinishLaunchingNotification),
                None,
            );
        }
        // Neither the notification centre nor the event manager retains its target.
        std::mem::forget(documents);
    }
}
