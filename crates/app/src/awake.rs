//! Keeps macOS from napping the app while agents are connected: App Nap slows a window in the
//! background several times over, and agents usually drive it from another app.

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

    /// A user-initiated activity held while `set(true)`.
    #[derive(Default)]
    pub struct Awake(Option<Retained<ProtocolObject<dyn NSObjectProtocol>>>);

    impl Awake {
        pub fn set(&mut self, on: bool) {
            if on && self.0.is_none() {
                let reason = NSString::from_str("Serving agent API requests");
                self.0 = Some(
                    NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                        NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                        &reason,
                    ),
                );
            } else if !on && let Some(activity) = self.0.take() {
                // SAFETY: `activity` is what `beginActivityWithOptions_reason` returned.
                unsafe { NSProcessInfo::processInfo().endActivity(&activity) };
            }
        }

        #[cfg(test)]
        pub fn is_on(&self) -> bool {
            self.0.is_some()
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    /// Nothing naps the app elsewhere.
    #[derive(Default)]
    pub struct Awake(bool);

    impl Awake {
        pub fn set(&mut self, on: bool) {
            self.0 = on;
        }

        #[cfg(test)]
        pub fn is_on(&self) -> bool {
            self.0
        }
    }
}

pub use imp::Awake;

impl Awake {
    /// Holds the activity while `work` runs, then keeps it only if `work` returns true (agents
    /// are still connected). `work` is given the guard, held.
    pub fn serve(&mut self, work: impl FnOnce(&Self) -> bool) {
        self.set(true);
        let keep = work(self);
        self.set(keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_activity_follows_the_connections() {
        let mut awake = Awake::default();
        assert!(!awake.is_on());
        awake.set(true);
        awake.set(true);
        assert!(awake.is_on());
        awake.set(false);
        assert!(!awake.is_on());
        awake.set(false);
        assert!(!awake.is_on());
    }

    #[test]
    fn serving_holds_the_activity_until_the_work_is_done() {
        let mut awake = Awake::default();
        let mut on_during = false;
        awake.serve(|a| {
            on_during = a.is_on();
            false
        });
        assert!(on_during);
        assert!(!awake.is_on());
        awake.serve(|_| true);
        assert!(awake.is_on());
        awake.serve(|a| {
            on_during = a.is_on();
            false
        });
        assert!(on_during);
        assert!(!awake.is_on());
    }
}
