//! The window cap, and the share rate below which it must not apply.
//!
//! The dense threshold carries an evidence term inversely proportional to the window, so on a channel
//! that submits shares without ever firing the window grows with channel age and the term decays to
//! nothing. Re-anchoring the window once it reaches the cap bounds the term from below.
//!
//! The gate is the part worth testing hardest, because getting it wrong is worse than not having the
//! cap: below the sparse seam the threshold is a statistic of the window's own count, so bounding the
//! window bounds the threshold from below at a rate where it needs to keep falling.

use super::{TEST_MIN_ALLOWED_HASHRATE, TEST_SHARES_PER_MINUTE};
use crate::{
    target::hash_rate_to_target,
    vardiff::{classic::VardiffState, clock::MockClock, Vardiff},
};
use std::sync::Arc;

const TICK_SECS: u64 = 60;
const HASHRATE: f32 = 1.0e12;
/// The cap the implementation applies. Stated here so a change to it breaks this test rather than
/// silently changing what is asserted.
const EXPECTED_CAP_SECS: u64 = 900;
/// Clock origin, clear of zero.
const CLOCK_ORIGIN: u64 = 1_000_000;

/// Runs a channel on target for `minutes` at `spm` and returns the longest window the controller ever
/// measured over.
///
/// On target means the shares delivered match what the difficulty asks for, so the deviation stays
/// small and the controller has no reason to fire -- which is exactly the state that lets the window
/// grow.
fn longest_window(spm: f32, minutes: u64) -> u64 {
    let clock = Arc::new(MockClock::new(CLOCK_ORIGIN));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("a positive floor and a mock clock construct");
    let target = hash_rate_to_target(HASHRATE.into(), spm.into()).expect("valid target");

    // Shares per tick that hold the channel on target. Fractional rates are delivered as whole shares
    // on the ticks they fall due, so the average is right even where the per-tick count is not.
    let mut owed = 0.0f64;
    let mut longest = 0u64;
    let mut now = CLOCK_ORIGIN;

    for _ in 0..minutes {
        clock.advance(TICK_SECS);
        now += TICK_SECS;
        owed += spm as f64 * (TICK_SECS as f64 / 60.0);
        while owed >= 1.0 {
            vardiff.increment_shares_since_last_update();
            owed -= 1.0;
        }
        longest = longest.max(now.saturating_sub(vardiff.last_update_timestamp()));
        let _ = vardiff.try_vardiff(HASHRATE, &target, spm);
    }
    longest
}

/// On a dense channel the window is bounded by the cap.
#[test]
fn a_dense_channel_never_measures_over_a_window_longer_than_the_cap() {
    // Well above the sparse seam, and run far longer than the cap so an unbounded window would be
    // obvious.
    let longest = longest_window(TEST_SHARES_PER_MINUTE, 240);

    assert!(
        longest <= EXPECTED_CAP_SECS + TICK_SECS,
        "the window reached {longest}s on a dense channel, over the {EXPECTED_CAP_SECS}s cap plus \
         the tick it is detected on; the cap is not being applied"
    );
    assert!(
        longest > EXPECTED_CAP_SECS / 2,
        "the window only reached {longest}s, so this channel was firing rather than holding a long \
         window, and the test is not exercising the cap"
    );
}

/// Below the sparse seam it is not, and that is deliberate.
///
/// The sparse threshold falls as the window's own share count accumulates. Capping the window there
/// would stop the threshold coming down to meet a declining channel, which is a worse failure than
/// the unnecessary retargets the cap prevents.
#[test]
fn a_sparse_channel_is_left_uncapped() {
    // Below the seam of six shares a minute.
    let longest = longest_window(2.0, 240);

    assert!(
        longest > EXPECTED_CAP_SECS + TICK_SECS,
        "the window was capped at {longest}s on a sparse channel; there the threshold is a statistic \
         of the window's own count, so capping it strands a declining channel above a bar that has \
         stopped falling"
    );
}
