//! Animated scroll position with ease-out deceleration.
//!
//! [`AnimatedScroll`] stores a `current` (rendered) and `target` position.
//! Each [`tick`](AnimatedScroll::tick) closes 50% of the remaining gap,
//! producing a subtle deceleration effect. Reused across all scrollable
//! panels — conversation, overview activity, and recent completions.

/// Scroll position that eases toward a target with deceleration.
///
/// Each [`tick`](Self::tick) closes 50% of the remaining gap (minimum 1 line).
/// Reused across all scrollable panels — conversation and overview activity.
#[derive(Debug, Clone, Copy)]
pub struct AnimatedScroll {
    /// Rendered scroll position (what the viewport uses).
    current: u16,
    /// Position the animation is easing toward.
    target: u16,
}

impl AnimatedScroll {
    /// Numerator of the fraction of the remaining gap closed per tick.
    /// `3` out of 4 = 75%. Arrow-key scrolls (3 lines) settle in ~2 ticks
    /// (160ms); page scrolls (~25 lines) settle in ~3 ticks (240ms).
    const EASE_NUMER: u16 = 3;
    /// Denominator of the easing fraction.
    const EASE_DENOM: u16 = 4;

    /// Create a scroll pinned to the bottom via the `u16::MAX` sentinel.
    /// The sentinel is resolved to a real offset on the first `apply_max` call.
    pub fn new() -> Self {
        Self {
            current: u16::MAX,
            target: u16::MAX,
        }
    }

    /// Create a scroll starting at the top.
    pub fn zero() -> Self {
        Self {
            current: 0,
            target: 0,
        }
    }

    /// The rendered scroll position, safe to use as viewport offset.
    pub fn position(self) -> u16 {
        self.current
    }

    /// The position the animation is moving toward.
    pub fn target(self) -> u16 {
        self.target
    }

    /// Snap both current and target to `pos` without animation.
    /// Used for auto-scroll and `ScrollToBottom`.
    pub fn snap(&mut self, pos: u16) {
        self.current = pos;
        self.target = pos;
    }

    /// Shift the target by a signed delta (positive = down, negative = up),
    /// clamping to `[0, max]`. Resolves the `u16::MAX` sentinel on both
    /// `current` and `target` before arithmetic so the first scroll action
    /// after auto-scroll produces visible movement.
    pub fn scroll_by(&mut self, delta: i32, max: u16) {
        self.current = self.current.min(max);
        self.target = self.target.min(max);
        let new = (i32::from(self.target) + delta).clamp(0, i32::from(max));
        // Cast safety: clamped to [0, i32::from(u16::MAX)] so it fits in u16.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        // Justified: `new` is clamped to [0, max] where max is u16.
        {
            self.target = new as u16;
        }
    }

    /// Clamp both current and target to `max_scroll`. Called every frame
    /// because content height (and thus `max_scroll`) changes dynamically.
    pub fn apply_max(&mut self, max: u16) {
        self.target = self.target.min(max);
        self.current = self.current.min(max);
    }

    /// Advance one animation step. Closes 75% of the gap (min 1 line).
    pub fn tick(&mut self) {
        if self.current == self.target {
            return;
        }
        if self.current < self.target {
            let remaining = self.target - self.current;
            let step = (remaining * Self::EASE_NUMER / Self::EASE_DENOM).max(1);
            self.current = (self.current + step).min(self.target);
        } else {
            let remaining = self.current - self.target;
            let step = (remaining * Self::EASE_NUMER / Self::EASE_DENOM).max(1);
            self.current = self.current.saturating_sub(step).max(self.target);
        }
    }

    /// Adjust the scroll target so `selected` is visible within a viewport
    /// of `viewport_height` rows out of `total` items.
    /// Centers the selection when possible; snaps instantly (no animation)
    /// because the user expects the selected item to be visible immediately.
    pub fn follow_selection(&mut self, selected: usize, viewport_height: u16, total: usize) {
        if total == 0 || viewport_height == 0 {
            return;
        }
        let vh = viewport_height as usize;
        if total <= vh {
            self.snap(0);
            return;
        }
        let max_offset = total.saturating_sub(vh);
        let target = selected.saturating_sub(vh / 2).min(max_offset);
        // Cast safety: max_offset bounded by item count, well within u16.
        #[allow(clippy::cast_possible_truncation)]
        // Justified: max_offset is at most total which is a usize from a Vec::len(),
        // practically never exceeds u16::MAX in a TUI tree view.
        self.snap(target as u16);
    }

    /// Returns `true` while the displayed position differs from the target.
    pub fn is_animating(self) -> bool {
        self.current != self.target
    }
}

#[cfg(test)]
#[path = "animated_scroll_tests.rs"]
mod tests;
