//! Switching between Chinese and English with a tap of Shift, as common
//! Chinese input methods do. In English the input method lets every key
//! through and hides its window; the choice is shared by all applications
//! through [`crate::store::english`].

use std::time::{Duration, Instant};

/// The longest press that still counts as a tap, so holding Shift to click
/// with, or for a key the user then thought better of, switches nothing.
const TAP_LIMIT: Duration = Duration::from_millis(500);

/// Tells a tap of Shift alone from Shift used with another key. The frontend
/// reports Shift going down and up, and any other key going down in between.
/// Reporting the same event twice is harmless: the Windows text service may
/// see a key both when it is tested and when it is handled.
#[derive(Debug, Default)]
pub struct ShiftTap {
    /// When Shift went down, while nothing else has been pressed since.
    pressed: Option<Instant>,
}

impl ShiftTap {
    /// Shift went down with no other modifier held. Do not report
    /// auto-repeats, or a long press would count from the last repeat.
    pub fn press(&mut self, now: Instant) {
        self.pressed = Some(now);
    }

    /// Another key went down, or focus moved: the Shift being held is not a tap.
    pub fn interrupt(&mut self) {
        self.pressed = None;
    }

    /// Shift went up: whether that ends a tap. Reporting the same release
    /// again returns false.
    pub fn release(&mut self, now: Instant) -> bool {
        self.pressed.take().is_some_and(|t| now.saturating_duration_since(t) <= TAP_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_quick_press_and_release_is_a_tap_once() {
        let t = Instant::now();
        let mut shift = ShiftTap::default();
        shift.press(t);
        // Tested, then handled: the second report of the press changes nothing.
        shift.press(t + ms(1));
        assert!(shift.release(t + ms(120)));
        assert!(!shift.release(t + ms(121)));
    }

    #[test]
    fn shift_with_another_key_is_not_a_tap() {
        let t = Instant::now();
        let mut shift = ShiftTap::default();
        shift.press(t);
        shift.interrupt();
        assert!(!shift.release(t + ms(100)));
    }

    #[test]
    fn a_long_press_is_not_a_tap() {
        let t = Instant::now();
        let mut shift = ShiftTap::default();
        shift.press(t);
        assert!(!shift.release(t + TAP_LIMIT + ms(1)));
        shift.press(t);
        assert!(shift.release(t + TAP_LIMIT));
    }

    #[test]
    fn a_release_without_a_press_is_not_a_tap() {
        assert!(!ShiftTap::default().release(Instant::now()));
    }
}
