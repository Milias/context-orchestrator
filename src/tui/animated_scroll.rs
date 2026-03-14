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
    /// Fraction of the remaining gap closed per tick. `2` = 50%.
    const EASE_DIVISOR: u16 = 2;

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

    /// Advance one animation step. Closes 50% of the gap (min 1 line).
    pub fn tick(&mut self) {
        if self.current == self.target {
            return;
        }
        if self.current < self.target {
            let step = ((self.target - self.current) / Self::EASE_DIVISOR).max(1);
            self.current = (self.current + step).min(self.target);
        } else {
            let step = ((self.current - self.target) / Self::EASE_DIVISOR).max(1);
            self.current = self.current.saturating_sub(step).max(self.target);
        }
    }

    /// Returns `true` while the displayed position differs from the target.
    pub fn is_animating(self) -> bool {
        self.current != self.target
    }
}

#[cfg(test)]
#[path = "animated_scroll_tests.rs"]
mod tests;
