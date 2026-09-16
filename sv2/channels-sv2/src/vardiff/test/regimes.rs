//! What the controller does in four regimes, measured on trajectories rather than reasoned about.
//!
//! The mechanism tests elsewhere establish that each part does what it says. These establish what the
//! parts add up to, which is a different question and the one an operator cares about: a controller
//! can pass every mechanism test and still churn, sit at the wrong difficulty, or fail to notice a
//! miner leaving.
//!
//! Four regimes, chosen because they are the ones a channel is actually in. It is declining, it is
//! silent, it is steady, or it is varying faster than the estimator can follow. The first is where
//! the failures the design addresses live; the last is where a fixed memory should be expected to do
//! worst, and it is measured for that reason rather than despite it.

use super::{
    trajectory::{constant, declining, goes_silent, measure, swinging, START_HASHRATE},
    TEST_SHARES_PER_MINUTE,
};

const ONE_HOUR: u64 = 3600;

/// Over-difficulty a controller that never adjusted would settle at, after losing `drop` of the
/// hashrate. A reference point rather than a bound: a reading above it means difficulty moved the
/// wrong way, which is a different failure from failing to move.
fn never_adjusted_pct(drop: f64) -> f64 {
    (1.0 / (1.0 - drop)).ln() * 100.0
}

/// A decline is detected at every share rate, and the channel ends far from where never adjusting
/// would have left it.
///
/// This is the failure the design exists to fix, so it is stated as a distance from the do-nothing
/// outcome rather than as a distance from zero. The controller's own settled position is not zero and
/// is not meant to be: requiring more evidence to tighten than to loosen leaves it a little below the
/// truth, and that offset is deliberate. What matters here is that it is nowhere near the do-nothing
/// reading.
#[test]
fn a_decline_is_detected_at_every_share_rate() {
    /// Cells: hashrate lost per hour, total lost, and the share rates to see it at.
    const CELLS: [(f64, f64); 4] = [(5.0, 0.20), (2.0, 0.08), (10.0, 0.40), (40.0, 0.50)];
    /// The most of the do-nothing over-difficulty that may remain.
    const MAX_REMAINING: f64 = 0.5;
    const TRIALS: usize = 40;

    for (rate_pct_per_hr, drop) in CELLS {
        let reference = never_adjusted_pct(drop);
        for spm in [2.0f32, 12.0] {
            let trial = measure(
                &declining(rate_pct_per_hr, drop),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                TRIALS,
            );
            let report = format!(
                "{rate_pct_per_hr} %/hr losing {:.0}% at {spm} spm: ends {:+.2}%, worst \
                 over-difficulty {:+.2}%, {} retargets; never adjusting would end at {reference:+.2}%",
                drop * 100.0,
                trial.final_error_pct,
                trial.worst_over_pct,
                trial.retargets
            );

            assert!(
                trial.retargets > 0,
                "the decline should be acted on at all -- {report}"
            );
            assert!(
                trial.final_error_pct < reference * MAX_REMAINING,
                "and should end well clear of the do-nothing reading -- {report}"
            );
            assert!(
                trial.worst_over_pct < reference + 25.0,
                "without an excursion far past it on the way, which would mean difficulty moved the \
                 wrong way -- {report}"
            );
        }
    }
}

/// A channel that goes silent is eased to the displacement bound and stops there, at every share rate.
///
/// This is what lets the graduated no-shares divisors go. They existed because the old fixed ladder's
/// top rung fired on a zero estimate, which would otherwise have driven the belief at
/// `min_allowed_hashrate`; a threshold sized from the evidence reaches below 100% on its own once the
/// window is long enough, so the descent happens without them. What bounds where it stops is the
/// displacement floor, and that is the claim this checks -- across the whole share-rate band rather
/// than at the one rate the arithmetic was worked through.
///
/// The bound matters because a belief far below the truth serves a difficulty far too easy: a channel
/// eased to `min_allowed_hashrate` from a terahash belief returns submitting some fourteen orders of
/// magnitude over its target rate.
#[test]
fn a_silent_channel_rests_at_the_displacement_bound() {
    /// The bound the controller applies, as a ratio below the last evidenced belief.
    const EXPECTED_DISPLACEMENT: f32 = 9.0;
    /// Tolerance, because the descent lands on the floor from above by one partial step.
    const TOLERANCE: f32 = 0.15;
    const TRIALS: usize = 20;

    for spm in [2.0f32, 6.0, 10.0, 12.0, 30.0, 60.0] {
        let trial = measure(&goes_silent(ONE_HOUR), spm, ONE_HOUR, 3 * ONE_HOUR, TRIALS);
        let displacement = START_HASHRATE / trial.final_hashrate;
        let report = format!(
            "at {spm} spm a silent channel rests at {:.3e} H/s, which is {displacement:.2}x below \
             where it started, after {} retargets",
            trial.final_hashrate, trial.retargets
        );

        assert!(
            trial.retargets > 0,
            "silence should be acted on at all -- {report}"
        );
        assert!(
            (displacement / EXPECTED_DISPLACEMENT - 1.0).abs() < TOLERANCE,
            "and should rest at the {EXPECTED_DISPLACEMENT}x bound -- {report}"
        );
    }
}

/// A channel that is not changing is left largely alone, and sits at a standing offset rather than at
/// zero.
///
/// The offset is the asymmetry's, not an error: tightening requires more evidence than loosening, so
/// the controller settles a little below the truth and stays there. It is bounded here rather than
/// removed, because removing it would mean removing the asymmetry.
///
/// The retarget count is what matters more, because each one is a message and a discontinuity for the
/// miner. A controller that scored well on the decline by reacting to everything would fail here.
#[test]
fn a_stable_channel_is_left_largely_alone() {
    /// Difficulty changes an hour.
    const MAX_RETARGETS_PER_HOUR: f64 = 4.0;
    /// The standing offset, in percent. An error of this size is a share-rate error of the same size,
    /// which share accounting does not care about, because a share is credited at the difficulty it
    /// was found at.
    const MAX_MEDIAN_OFFSET_PCT: f64 = 12.0;
    const MEASURED_HOURS: u64 = 12;
    const TRIALS: usize = 9;

    for spm in [2.0f32, 6.0, 12.0, 60.0] {
        let trial = measure(
            &constant(),
            spm,
            ONE_HOUR,
            MEASURED_HOURS * ONE_HOUR,
            TRIALS,
        );
        let per_hour = trial.retargets as f64 / MEASURED_HOURS as f64;
        let report = format!(
            "at {spm} spm a stable channel saw {per_hour:.2} retargets an hour, sitting {:.2}% out \
             at the median and {:.2}% at the 95th percentile",
            trial.median_error_pct, trial.p95_error_pct
        );

        assert!(
            per_hour <= MAX_RETARGETS_PER_HOUR,
            "a stable channel should not be retargeted more than \
             {MAX_RETARGETS_PER_HOUR} times an hour -- {report}"
        );
        assert!(
            trial.median_error_pct <= MAX_MEDIAN_OFFSET_PCT,
            "and the standing offset should stay within {MAX_MEDIAN_OFFSET_PCT}% -- {report}"
        );
    }
}

/// A miner swinging on the estimator's own timescale is averaged rather than followed, and that is the
/// cost of a memory of fixed length.
///
/// A memory of `tau` averages over `tau`, so a swing on a comparable timescale puts the estimate
/// between the two levels, where it is wrong at both. This is the regime a fixed memory should be
/// expected to do worst in, and nothing else in the suite reaches it.
///
/// Two things bound the cost. It is a resonance rather than a general regression: far below the
/// memory's timescale both levels average to the same thing and the controller sits still, and far
/// above it each level is settled before it changes and is followed accurately. And the error is a
/// typical-case one -- the estimate is never far away, it is just never right either.
#[test]
fn a_miner_swinging_near_the_memory_is_averaged_rather_than_followed() {
    /// Half periods either side of the estimator's memory, in seconds.
    const NEAR_MEMORY_SECS: u64 = 900;
    const FAR_ABOVE_MEMORY_SECS: u64 = 7200;
    /// How much worse the near-memory error must be for the resonance to be real.
    const MIN_RATIO: f64 = 2.0;
    const MEASURED_HOURS: u64 = 12;
    const TRIALS: usize = 9;

    let at = |half_period_secs: u64| {
        measure(
            &swinging(half_period_secs),
            TEST_SHARES_PER_MINUTE,
            ONE_HOUR,
            MEASURED_HOURS * ONE_HOUR,
            TRIALS,
        )
    };
    let near = at(NEAR_MEMORY_SECS);
    let far = at(FAR_ABOVE_MEMORY_SECS);
    let report = format!(
        "near the memory ({NEAR_MEMORY_SECS}s half period): {:.1}% median, {:.1}% p95; far above it \
         ({FAR_ABOVE_MEMORY_SECS}s): {:.1}% median, {:.1}% p95",
        near.median_error_pct, near.p95_error_pct, far.median_error_pct, far.p95_error_pct
    );

    assert!(
        near.median_error_pct > far.median_error_pct * MIN_RATIO,
        "a swing near the memory's timescale should be read materially less accurately than one far \
         above it, or this cost is not the resonance it is described as -- {report}"
    );
    assert!(
        far.median_error_pct < 15.0,
        "and a swing far above it should be followed accurately, which is what confines the cost to \
         the band -- {report}"
    );
}
