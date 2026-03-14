use super::AnimatedScroll;

/// Catches stalling or overshoot when easing downward. Sets current=0,
/// target=100 and ticks until convergence. Asserts monotonic increase and
/// convergence within a bounded number of ticks.
#[test]
fn tick_converges_downward() {
    let mut s = AnimatedScroll::zero();
    s.scroll_by(100, 200);
    assert_eq!(s.target(), 100);

    for _ in 0..20 {
        let prev = s.position();
        s.tick();
        assert!(s.position() >= prev, "current must not decrease going down");
        assert!(
            s.position() <= s.target(),
            "current must not overshoot target"
        );
        if !s.is_animating() {
            break;
        }
    }
    assert!(!s.is_animating(), "must converge within 20 ticks");
}

/// Catches asymmetric bug in upward easing (new code path — `AnimatedCounter`
/// does not animate decreases). Asserts monotonic decrease and convergence.
#[test]
fn tick_converges_upward() {
    let mut s = AnimatedScroll::zero();
    s.snap(100);
    s.scroll_by(-100, 200);
    assert_eq!(s.target(), 0);

    for _ in 0..20 {
        let prev = s.position();
        s.tick();
        assert!(s.position() <= prev, "current must not increase going up");
        assert!(
            s.position() >= s.target(),
            "current must not undershoot target"
        );
        if !s.is_animating() {
            break;
        }
    }
    assert!(!s.is_animating(), "must converge within 20 ticks");
}

/// Catches `snap` leaving `current != target`, which would cause unwanted
/// animation after auto-scroll or `ScrollToBottom`.
#[test]
fn snap_bypasses_animation() {
    let mut s = AnimatedScroll::zero();
    s.snap(42);
    assert_eq!(s.position(), 42);
    assert_eq!(s.target(), 42);
    assert!(!s.is_animating());
}

/// Catches `scroll_by` producing a target beyond `max_scroll`.
#[test]
fn scroll_by_clamps_to_max() {
    let mut s = AnimatedScroll::zero();
    s.snap(50);
    s.scroll_by(100, 60);
    assert_eq!(s.target(), 60);
}

/// Catches underflow where `scroll_by` could produce a negative target.
#[test]
fn scroll_by_clamps_to_zero() {
    let mut s = AnimatedScroll::zero();
    s.snap(5);
    s.scroll_by(-10, 100);
    assert_eq!(s.target(), 0);
}

/// Catches rendering past content bounds when `apply_max` does not clamp
/// `current`. After `apply_max(50)` both fields must be ≤ 50.
#[test]
fn apply_max_clamps_both() {
    let mut s = AnimatedScroll::zero();
    s.snap(100);
    s.scroll_by(-20, 100); // target=80, current=100
    s.apply_max(50);
    assert_eq!(s.position(), 50);
    assert_eq!(s.target(), 50);
}

/// Catches successive scroll inputs being lost during animation. Three
/// consecutive `scroll_by(-3)` without ticking must shift the target by 9.
#[test]
fn rapid_scroll_accumulates_target() {
    let mut s = AnimatedScroll::zero();
    s.snap(50);
    s.scroll_by(-3, 100);
    s.scroll_by(-3, 100);
    s.scroll_by(-3, 100);
    assert_eq!(s.target(), 41);

    // Tick to convergence and confirm final position.
    for _ in 0..30 {
        s.tick();
    }
    assert_eq!(s.position(), 41);
}

/// Catches the `u16::MAX` sentinel surviving into rendering. After `new()`
/// (both at `u16::MAX`) and `apply_max(200)`, both must be resolved to 200.
#[test]
fn sentinel_resolves_on_apply_max() {
    let mut s = AnimatedScroll::new();
    assert_eq!(s.position(), u16::MAX);
    s.apply_max(200);
    assert_eq!(s.position(), 200);
    assert_eq!(s.target(), 200);
}

/// Catches overview/tab scrolls starting at the wrong position. `zero()`
/// must produce position=0 with no animation in progress.
#[test]
fn zero_starts_at_origin() {
    let s = AnimatedScroll::zero();
    assert_eq!(s.position(), 0);
    assert!(!s.is_animating());
}

/// Catches a wrong easing divisor. With 50% easing, one tick from 0→20
/// must advance current to 10 (half the remaining gap).
#[test]
fn half_step_easing() {
    let mut s = AnimatedScroll::zero();
    s.scroll_by(20, 100);
    s.tick();
    assert_eq!(s.position(), 10);
}

/// Catches the existing `u16::MAX` sentinel bug where `handle_scroll`
/// computes `u16::MAX - 3 = 65532`, which the renderer clamps back to
/// `max_scroll` — wasting the first Up press. `scroll_by` must resolve
/// the sentinel before arithmetic so the result is `max - 3`, not 65532.
#[test]
fn scroll_by_resolves_sentinel_before_arithmetic() {
    let mut s = AnimatedScroll::new();
    // Simulate: auto-scroll set both to u16::MAX, then user presses Up.
    s.scroll_by(-3, 200);
    assert_eq!(
        s.target(),
        197,
        "target should be max - 3, not u16::MAX - 3"
    );
    assert_eq!(s.position(), 200, "current should be resolved to max");
}
