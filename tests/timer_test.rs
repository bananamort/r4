//! Tests for the timing helper.

use r4::utils::Timer;

#[test]
fn timer_accumulates_and_labels_durations() {
    let mut timer = Timer::now();
    timer.pause();
    timer.start();
    timer.pause();
    let label = timer.duration_label();
    assert!(!label.is_empty());
    assert!(
        label.ends_with("µs") || label.ends_with("ms") || label.ends_with('s'),
        "{label}"
    );
    assert_eq!(timer, timer.clone());
}
