//! Operating-system appearance and accessibility preferences.
//!
//! The desktop keeps these values device-local. The observer is installed once
//! on the AppKit main thread and lives for the whole Slint event loop.

#[cfg(all(test, feature = "ui-tests"))]
thread_local! {
    static TEST_SYSTEM_LIGHT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static TEST_REDUCE_MOTION: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Returns whether the operating system currently uses a light appearance.
/// Unknown or unsupported platform appearances use the dark fallback.
///
/// Headless tests never read the host's real appearance.
pub fn system_light() -> bool {
    #[cfg(all(test, feature = "ui-tests"))]
    {
        TEST_SYSTEM_LIGHT.with(|value| value.get()).unwrap_or(false)
    }

    #[cfg(all(test, not(feature = "ui-tests")))]
    {
        false
    }

    #[cfg(all(not(test), target_os = "macos"))]
    {
        use objc2::{MainThreadMarker, rc::Retained};
        use objc2_app_kit::{
            NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication,
        };
        use objc2_foundation::NSArray;

        let Some(main_thread) = MainThreadMarker::new() else {
            return false;
        };
        let app = NSApplication::sharedApplication(main_thread);
        let appearance: Retained<NSAppearance> = app.effectiveAppearance();
        // Collapse high-contrast and vibrant variants to their base family.
        let names = [unsafe { NSAppearanceNameAqua }, unsafe {
            NSAppearanceNameDarkAqua
        }];
        let candidates = NSArray::from_slice(&names);
        appearance
            .bestMatchFromAppearancesWithNames(&candidates)
            .is_some_and(|best| &*best == unsafe { NSAppearanceNameAqua })
    }

    #[cfg(all(not(test), not(target_os = "macos")))]
    {
        false
    }
}

/// Returns whether the operating system currently requests reduced motion.
/// Headless tests never read the host's real accessibility settings.
pub fn system_reduce_motion() -> bool {
    #[cfg(all(test, feature = "ui-tests"))]
    {
        TEST_REDUCE_MOTION
            .with(|value| value.get())
            .unwrap_or(false)
    }

    #[cfg(all(test, not(feature = "ui-tests")))]
    {
        false
    }

    #[cfg(all(not(test), target_os = "macos"))]
    {
        objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }

    #[cfg(all(not(test), not(target_os = "macos")))]
    {
        false
    }
}

#[cfg(all(test, feature = "ui-tests"))]
pub fn set_system_light_for_tests(value: Option<bool>) {
    TEST_SYSTEM_LIGHT.with(|slot| slot.set(value));
}

#[cfg(all(test, feature = "ui-tests"))]
pub fn set_system_reduce_motion_for_tests(value: Option<bool>) {
    TEST_REDUCE_MOTION.with(|slot| slot.set(value));
}

#[cfg(target_os = "macos")]
use std::rc::Rc;

#[cfg(target_os = "macos")]
use objc2::{DefinedClass, MainThreadOnly};
use objc2_foundation::NSObjectProtocol;

#[cfg(target_os = "macos")]
struct AppearanceObserverIvars {
    on_change: Rc<dyn Fn()>,
}

#[cfg(target_os = "macos")]
objc2::define_class!(
    #[unsafe(super = objc2_foundation::NSObject)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = AppearanceObserverIvars]
    struct CrabHostsAppearanceObserver;

    unsafe impl NSObjectProtocol for CrabHostsAppearanceObserver {}

    impl CrabHostsAppearanceObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            _key_path: Option<&objc2_foundation::NSString>,
            _of_object: Option<&objc2::runtime::AnyObject>,
            _change: Option<&objc2_foundation::NSDictionary<objc2_foundation::NSKeyValueChangeKey, objc2::runtime::AnyObject>>,
            _context: *mut core::ffi::c_void,
        ) {
            (self.ivars().on_change)();
        }
    }
);

#[cfg(target_os = "macos")]
impl CrabHostsAppearanceObserver {
    fn new(
        main_thread: objc2::MainThreadMarker,
        on_change: Rc<dyn Fn()>,
    ) -> objc2::rc::Retained<Self> {
        use objc2::msg_send;

        let this = Self::alloc(main_thread).set_ivars(AppearanceObserverIvars { on_change });
        // SAFETY: the allocated NSObject subclass is initialized on the main thread.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(target_os = "macos")]
pub struct SystemPreferencesObserver {
    app: objc2::rc::Retained<objc2_app_kit::NSApplication>,
    appearance: objc2::rc::Retained<CrabHostsAppearanceObserver>,
    _reduce_motion_token:
        objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>,
}

#[cfg(not(target_os = "macos"))]
pub struct SystemPreferencesObserver;

#[cfg(target_os = "macos")]
impl Drop for SystemPreferencesObserver {
    fn drop(&mut self) {
        use objc2_foundation::{NSObjectNSKeyValueObserverRegistration, ns_string};

        // SAFETY: this removes the exact KVO registration made in `observe`.
        unsafe {
            self.app
                .removeObserver_forKeyPath(&self.appearance, ns_string!("effectiveAppearance"));
        }
    }
}

/// Watches macOS appearance and accessibility display options. Both callbacks
/// run on the main thread, where Slint properties may be updated safely.
#[cfg(target_os = "macos")]
pub fn observe<F>(on_change: F) -> SystemPreferencesObserver
where
    F: Fn() + 'static,
{
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{
        NSApplication, NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
    };
    use objc2_foundation::{
        NSKeyValueObservingOptions, NSNotification, NSObjectNSKeyValueObserverRegistration,
        NSOperationQueue, ns_string,
    };

    let on_change: Rc<dyn Fn()> = Rc::new(on_change);
    let main_thread =
        MainThreadMarker::new().expect("system preference observer requires main thread");
    let app = NSApplication::sharedApplication(main_thread);
    let appearance = CrabHostsAppearanceObserver::new(main_thread, Rc::clone(&on_change));
    // SAFETY: the observer implements the KVO callback with the exact signature;
    // a null context is used because the observer owns its callback in ivars.
    unsafe {
        app.addObserver_forKeyPath_options_context(
            &appearance,
            ns_string!("effectiveAppearance"),
            NSKeyValueObservingOptions::New,
            std::ptr::null_mut(),
        );
    }

    let block = RcBlock::new(move |_: NonNull<NSNotification>| on_change());
    // SAFETY: the observer is registered for a readonly NSWorkspace notification;
    // the block has the required signature and is retained by the returned token.
    let reduce_motion_token: Retained<_> = unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();
        let queue = NSOperationQueue::mainQueue();
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
            None,
            Some(&queue),
            &block,
        )
    };
    SystemPreferencesObserver {
        app,
        appearance,
        _reduce_motion_token: reduce_motion_token,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn observe<F>(_on_change: F) -> SystemPreferencesObserver
where
    F: Fn() + 'static,
{
    SystemPreferencesObserver
}
