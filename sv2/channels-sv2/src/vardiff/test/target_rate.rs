//! The target share rate is operator configuration, and it reaches the controller unvalidated.
//!
//! `shares_per_minute` is a divisor in the estimator's fallback path, and that path is not exotic: it
//! is taken whenever `hash_rate_from_target` refuses the realized rate, which it does for a rate of
//! zero — so it is the ordinary path for a window with no shares. A target rate of zero therefore
//! reaches `0.0 / 0.0` on any silent channel.
//!
//! What makes that worth a test rather than a comment is that NaN does not stop at any of the three
//! guards after it. `NaN < threshold` is false, so the "too small to act on" return is skipped;
//! `f32::clamp` propagates NaN rather than clamping it; `NaN < min_hashrate` and `NaN == hashrate` are
//! both false. Every comparison a guard makes against NaN answers in the direction that lets it
//! through, so the caller is handed NaN as the hashrate to set a difficulty from.

use super::{TEST_INITIAL_HASHRATE, TEST_MIN_ALLOWED_HASHRATE, TEST_SHARES_PER_MINUTE};
use crate::{
    target::hash_rate_to_target,
    vardiff::{classic::VardiffState, clock::MockClock, Vardiff},
};
use std::sync::Arc;

/// Clock origin, clear of zero.
const CLOCK_ORIGIN: u64 = 1_000_000;
/// Long enough to clear the minimum-interval guard.
const TICK_SECS: u64 = 60;

/// Runs one evaluation at `spm` on a channel that has delivered no shares, and returns what the
/// controller decided.
///
/// No shares is the condition that reaches the estimator's fallback, so this is the shortest path to
/// the case the guard exists for. The `target` is built from a valid rate so that the target itself is
/// never the thing under test.
fn one_silent_evaluation(spm: f32) -> Option<f32> {
    let clock = Arc::new(MockClock::new(CLOCK_ORIGIN));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("a positive floor and a mock clock construct");
    let target = hash_rate_to_target(TEST_INITIAL_HASHRATE.into(), TEST_SHARES_PER_MINUTE.into())
        .expect("a valid target");

    clock.advance(TICK_SECS);
    vardiff
        .try_vardiff(TEST_INITIAL_HASHRATE, &target, spm)
        .expect("a target rate is not an error, it is a decision not to act")
}

/// A target rate of zero is declined, rather than answered with a NaN difficulty.
///
/// This is the regression test for the fallback's divisor. Without the entry guard the sequence is
/// `hash_rate_from_target(target, 0.0) -> Err(DivisionByZero)`, then
/// `hashrate * 0.0 / 0.0 = NaN`, and the NaN is returned as a new hashrate.
#[test]
fn a_zero_target_rate_is_declined_rather_than_answered_with_nan() {
    let decision = one_silent_evaluation(0.0);
    assert!(
        decision.is_none(),
        "a target rate of zero has no difficulty that satisfies it, so there is nothing to \
         converge to and the only safe answer is to decline -- got {decision:?}"
    );
}

/// No target rate, however hostile, produces a belief that is not a usable hashrate.
///
/// Stated as a property over the whole set rather than one case per value, because the failure being
/// guarded against is not specific to zero: every one of these is either a divisor that cannot be
/// divided by or a value that poisons the arithmetic it enters, and the caller's contract is the same
/// for all of them. A difficulty is computed from whatever comes back, so "finite and positive" is
/// the whole of what the caller needs and the whole of what is asserted.
#[test]
fn a_hostile_target_rate_never_yields_an_unusable_belief() {
    for spm in [
        0.0f32,
        -1.0,
        -TEST_SHARES_PER_MINUTE,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ] {
        let decision = one_silent_evaluation(spm);
        if let Some(h) = decision {
            assert!(
                h.is_finite() && h > 0.0,
                "at shares_per_minute={spm} the controller returned {h}, which cannot be turned \
                 into a difficulty"
            );
        }
    }
}

/// The guard declines an unusable rate without declining a usable one.
///
/// The companion every input guard needs: a check that refuses everything passes the test above
/// vacuously, and would be indistinguishable from one that works until someone read the code. A
/// silent channel at a valid rate has a real decision to make -- its estimate is zero against a
/// non-zero belief -- so acting on it is evidence the guard is narrow.
#[test]
fn a_valid_target_rate_is_still_acted_on() {
    // 10 shares a minute sits in the band where a zero-share evaluation clears its own threshold:
    // above `SPARSE_SPM_SEAM`, so the dense branch applies, and below the rate at which the dense
    // bar rises past the 100% a zero estimate deviates by.
    let decision = one_silent_evaluation(TEST_SHARES_PER_MINUTE);
    let eased = decision.expect(
        "a silent channel at a valid rate should be eased, or this test is not exercising the \
         path the guard sits in front of",
    );
    assert!(
        eased.is_finite() && eased > 0.0 && eased < TEST_INITIAL_HASHRATE,
        "the ease should be a usable hashrate below the previous belief -- got {eased}"
    );
}
