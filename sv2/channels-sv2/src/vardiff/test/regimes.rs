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
///
/// # Why the bound is two-sided, and why 0.25
///
/// It was one-sided (`final_error_pct < reference * 0.5`) and that let six of eight readings pass
/// *trivially*, because a controller that over-corrects settles **negative** and any negative number is
/// below a positive bound. Over-correction is a real cost — it multiplies the miner's submission load
/// without changing its payout under difficulty-weighted accounting — so the bound is now on the
/// magnitude.
///
/// `0.25` is measured, not chosen. Across these cells the largest `|settled| / reference` is `0.160`
/// (40 %/hr at 12 spm), then `0.122` and `0.110`, so `0.25` binds every cell at a margin of 1.6x to
/// 2.3x. The previous `0.5` was never approached: its tightest margin was 4.5x and six cells did not
/// test it at all.
///
/// A single scalar works here **only because the shallowest cell moved out**. See
/// `a_disturbance_inside_the_dead_zone_is_a_noise_test_not_a_decline_test` for why, and for what that
/// cell actually measures.
#[test]
fn a_decline_is_detected_at_every_share_rate() {
    /// Cells: hashrate lost per hour, total lost, and the share rates to see it at.
    ///
    /// Each disturbance here is larger than the controller's threshold at the settle point, so the
    /// decline is visible to it as signal. `2 %/hr` losing `8%` is deliberately absent — at `8.34%`
    /// it is smaller than the bar and cannot be acted on by signal at all.
    const CELLS: [(f64, f64); 3] = [(5.0, 0.20), (10.0, 0.40), (40.0, 0.50)];
    /// The most of the do-nothing over-difficulty that may remain, in **magnitude**.
    const MAX_REMAINING: f64 = 0.25;
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
                trial.final_error_pct.abs() < reference * MAX_REMAINING,
                "and should end well clear of the do-nothing reading, in either direction -- \
                 {report}"
            );
            assert!(
                trial.worst_over_pct < reference,
                "without ever being worse than not acting at all, which is the only excursion that \
                 would mean difficulty moved the wrong way -- {report}"
            );
        }
    }
}

/// **A disturbance smaller than the threshold is a noise test, not a decline test.**
///
/// `2 %/hr` losing `8%` used to sit in the decline grid. It cannot belong there: the do-nothing
/// over-difficulty is `8.34%`, and the controller's threshold at the settle point is `18.72%` at
/// 2 spm and `11.32%` at 12 spm. **The whole disturbance is inside the dead zone**, so a threshold
/// controller cannot act on it by signal, and the retargets it does make are Poisson fluctuations
/// crossing the bar rather than the decline being detected.
///
/// In the decline grid all three of its assertions were vacuous in different ways: `retargets > 0`
/// was satisfied by noise, the relative bound by a negative residual, and the excursion bound was
/// 30 percentage points away.
///
/// What it *is* worth measuring is that being unable to see a disturbance does not make the
/// controller wander. So the assertion is on drift: whatever the noise-driven fires do, the settled
/// position must stay inside the dead zone that produced them rather than walking out of it.
#[test]
fn a_disturbance_inside_the_dead_zone_is_a_noise_test_not_a_decline_test() {
    const RATE_PCT_PER_HR: f64 = 2.0;
    const DROP: f64 = 0.08;
    const TRIALS: usize = 40;
    /// The threshold at the settle point is 18.72% at 2 spm and 11.32% at 12 spm. Drift must stay
    /// inside the tighter of the two with headroom: measured worst is -6.61%, so 10% binds at 1.5x.
    const MAX_DRIFT_PCT: f64 = 10.0;

    let reference = never_adjusted_pct(DROP);
    for spm in [2.0f32, 12.0] {
        let trial = measure(
            &declining(RATE_PCT_PER_HR, DROP),
            spm,
            ONE_HOUR,
            3 * ONE_HOUR,
            TRIALS,
        );
        let report = format!(
            "{RATE_PCT_PER_HR} %/hr losing {:.0}% at {spm} spm: ends {:+.2}%, worst \
             over-difficulty {:+.2}%, {} retargets; the whole disturbance is {reference:+.2}%, \
             which is inside the threshold",
            DROP * 100.0,
            trial.final_error_pct,
            trial.worst_over_pct,
            trial.retargets
        );
        assert!(
            trial.final_error_pct.abs() < MAX_DRIFT_PCT,
            "a disturbance the controller cannot see must not make it wander: drift exceeded \
             {MAX_DRIFT_PCT:.1}% -- {report}"
        );
        assert!(
            trial.worst_over_pct < reference + MAX_DRIFT_PCT,
            "and it must not overshoot past the disturbance it cannot see -- {report}"
        );
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

/// **What the absolute settled criterion reads on the shipped controller.**
///
/// The criteria above are *relative* — a final error below half of what never
/// adjusting would have given. That scales with the disturbance, and on the
/// deepest cell here it permits **34.7%** settled over-difficulty, where the
/// criterion this controller's window and sensitivity were *selected* by is an
/// absolute **5%** (`DECLINE_SAFETY_GATE_PCT`, which lives on the simulation
/// branch and has no assertion here). So the selection criterion and the
/// regression criterion diverged, by up to 6.9x on these cells, and nothing on
/// this branch would notice the shipped configuration drifting between them.
///
/// This measures the absolute reading and holds it, so that drift is caught. It
/// is deliberately not phrased as the 5% gate: the number asserted is what the
/// controller actually achieves, so the test fails on a *regression* rather than
/// encoding a target it may or may not clear.
#[test]
fn the_absolute_settled_error_is_held_not_merely_the_relative_one() {
    /// Cells shared with `a_decline_is_detected_at_every_share_rate`.
    const CELLS: [(f64, f64); 4] = [(5.0, 0.20), (2.0, 0.08), (10.0, 0.40), (40.0, 0.50)];
    const TRIALS: usize = 40;
    /// The worst absolute settled over-difficulty this controller is allowed to
    /// show across the grid. Set from measurement with headroom, not from the
    /// selection gate — see the doc comment.
    /// The worst absolute settled over-difficulty this controller may show across
    /// the grid. Set from measurement with headroom — **measured +5.64% on the
    /// sparse deep-decline cell, which already exceeds the 5% selection gate by
    /// 13%.** Asserting 5.0 here would encode a target this controller does not
    /// meet; asserting the measured value holds the line against regression while
    /// the doc comment records the gap. See `C10_EVALUATION_2026_09_21.md` §13.
    const MAX_ABSOLUTE_SETTLED_PCT: f64 = 7.0;
    /// And the *under*-difficulty side, which no criterion on either branch bounds.
    /// Measured worst −11.10%; held so that over-correction is a regression too.
    const MIN_ABSOLUTE_SETTLED_PCT: f64 = -15.0;
    /// Mirrors `MAX_REMAINING` in `a_decline_is_detected_at_every_share_rate`,
    /// which is local to that test. Used only to report what the relative
    /// criterion would have allowed, never to assert.
    const RELATIVE_PERMITTED: f64 = 0.5;

    let mut worst = (f64::MIN, String::new());
    for (rate_pct_per_hr, drop) in CELLS {
        for spm in [2.0f32, 12.0] {
            let trial = measure(
                &declining(rate_pct_per_hr, drop),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                TRIALS,
            );
            let reference = never_adjusted_pct(drop);
            let label = format!(
                "{rate_pct_per_hr} %/hr losing {:.0}% at {spm} spm: settled {:+.2}%, \
                 relative criterion permitted {:+.2}%",
                drop * 100.0,
                trial.final_error_pct,
                reference * RELATIVE_PERMITTED
            );
            if trial.final_error_pct > worst.0 {
                worst = (trial.final_error_pct, label.clone());
            }
            assert!(
                trial.final_error_pct >= MIN_ABSOLUTE_SETTLED_PCT,
                "settled *under*-difficulty regressed past \
                 {MIN_ABSOLUTE_SETTLED_PCT:.1}% -- {label}. Over-correcting downward costs the \
                 miner nothing in payout under difficulty-weighted accounting but multiplies its \
                 submission load, and no criterion on either branch bounds it."
            );
            assert!(
                trial.final_error_pct <= MAX_ABSOLUTE_SETTLED_PCT,
                "absolute settled over-difficulty regressed past \
                 {MAX_ABSOLUTE_SETTLED_PCT:.1}% -- {label}. The relative criterion above would \
                 still pass here, which is the point of this test: it is the only thing on this \
                 branch that would catch the shipped configuration drifting away from the \
                 criterion it was selected by."
            );
        }
    }
    assert!(
        worst.0 > f64::MIN,
        "no cell produced a reading -- the harness measured nothing"
    );
    eprintln!(
        "  [absolute settled] worst across the grid: {:+.2}%  ({})",
        worst.0, worst.1
    );
}

/// **The settled position is inside the threshold that produced it**, at every cell and share rate.
///
/// This is the property every settled-error criterion in this suite depends on and none of them
/// stated. A threshold controller does not act while the deviation is below its threshold, so
/// wherever it stops is inside that number — which means **the threshold, not any written bound, is
/// what limits settled error.** A criterion asking for a settled error tighter than the threshold is
/// asking the controller to be more precise than its own trigger, and no parameter choice satisfies
/// that.
///
/// It is not a tautology. The controller compares its *estimate* against its *belief*, while the
/// reading here is belief against *truth*. They coincide only while the estimator is tracking, so
/// this asserts that it is — a real property, and the one that makes threshold-relative reasoning
/// about settled error valid at all.
///
/// The threshold is read from [`VardiffState::last_threshold_pct`] rather than recomputed. Three
/// records' worth of analysis had been recomputing the formula alongside the implementation, which is
/// exactly the arrangement in which a copy drifts from what it checks.
#[test]
fn the_settled_position_is_inside_the_threshold_that_produced_it() {
    const CELLS: [(f64, f64); 4] = [(5.0, 0.20), (2.0, 0.08), (10.0, 0.40), (40.0, 0.50)];
    const TRIALS: usize = 40;

    let mut tightest = f64::MAX;
    for (rate_pct_per_hr, drop) in CELLS {
        for spm in [2.0f32, 12.0] {
            let trial = measure(
                &declining(rate_pct_per_hr, drop),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                TRIALS,
            );
            let report = format!(
                "{rate_pct_per_hr} %/hr losing {:.0}% at {spm} spm: settled {:+.2}%, threshold in \
                 force {:.2}%",
                drop * 100.0,
                trial.final_error_pct,
                trial.final_threshold_pct
            );
            assert!(
                trial.final_threshold_pct.is_finite(),
                "no evaluation recorded a threshold -- the accessor is not being reached -- {report}"
            );
            assert!(
                trial.final_error_pct.abs() < trial.final_threshold_pct,
                "the settled position escaped the threshold that produced it, which would mean the \
                 estimator is not tracking and threshold-relative reasoning about settled error is \
                 invalid -- {report}"
            );
            tightest = tightest.min(trial.final_threshold_pct / trial.final_error_pct.abs());
        }
    }
    // Containment is not vacuous: the tightest cell sits within a small multiple of its threshold.
    // Measured ratios run 6.0x to 84.5x, so 10x binds on the tightest cell at 1.67x. The guard is
    // oriented to fail when containment stops being *exercised* — if every cell moved far inside its
    // threshold this test would assert nothing, and that is the regression worth catching here.
    assert!(
        tightest < 10.0,
        "every cell settled more than 10x inside its threshold, so this test is not exercising \
         containment and its assertion is vacuous (tightest ratio {tightest:.1}x)"
    );
}
