//! What happens when the channel opens at the wrong difficulty, which nothing else here measures.
//!
//! Every other trajectory in this suite opens on target and disturbs the miner afterwards. A real channel
//! opens at a difficulty derived from a *declared* `nominal_hash_rate`, and that declaration is
//! miner-supplied, uncharacterized, and can be wrong in either direction by orders of magnitude. It is also
//! the only regime that exercises [`MAX_STEP_RATIO`]'s tightening arm or gives
//! `MIN_EVALUATION_INTERVAL_SECS` anything to rate limit.
//!
//! # Retraction, and it is the whole reason this file reads the way it does
//!
//! This module previously asserted that an over-declared sparse channel **locks**: a belief pinned at
//! `START/9`, `11x` over-difficulty, identical readings at 6, 12, 24 and 48 hours, and a
//! `MAX_SILENT_DISPLACEMENT 9 -> 90` mutation that appeared to confirm the cause.
//!
//! **There was no lock.** The share sampler fed that cell **zero shares** across 20 seeds over 48 hours, so
//! the controller was not latching -- it had nothing to respond to, and the "48-hour invariance" read as
//! proof of a fixed point was the signature of an empty input. The mutation "confirmed" only that a looser
//! floor let a deterministic descent land nearer the truth.
//!
//! Re-derived against the repaired sampler, the same cell **converges, in both directions, at every rate
//! tested**. The tests below assert that, and the test that asserted the lock is *gone* rather than
//! recalibrated: a name stating a retracted finding is a false record, and patching the numbers underneath
//! it would leave the falsehood in place.
//!
//! What survives unchanged is the *shape*. Convergence is geometric, and the settled position is
//! under-difficulty and widens as the share rate falls -- which is the dead zone, and is expected.
//!
//! [`MAX_STEP_RATIO`]: crate::vardiff::classic
//! [`MIN_EVALUATION_INTERVAL_SECS`]: crate::vardiff::classic

use super::trajectory::{measure_arm, mis_declared, Arm, Trial};

const HOUR_TICKS: u64 = 3600;
const TRIALS: usize = 20;

/// Cold start has no settled prefix to skip, so the whole run is measured.
fn from_declaration(factor: f64, spm: f32, hours: u64) -> Trial {
    measure_arm(
        &mis_declared(factor),
        spm,
        0,
        hours * HOUR_TICKS,
        TRIALS,
        Arm::Shipped,
    )
}

/// Ticks to reach the truth, within 10% of it or past it, or `None` if it never does.
fn ticks_to_converge(trial: &Trial) -> Option<usize> {
    trial.ticks_to_converge.filter(|t| *t != usize::MAX)
}

/// **An over-declared channel converges, at every rate, and keeps working afterwards.**
///
/// The replacement for the retracted lock test, asserting what is actually true. A channel told its miner
/// is `10x` or `100x` faster than it is serves a difficulty that hard, receives almost nothing back, and has
/// to ease its way down -- the hardest direction for a rate estimator, because the evidence needed to
/// correct the error is exactly what the error suppresses.
///
/// Two readings matter and both are asserted. It reaches the truth, which is the retraction. And it is
/// still retargeting at 48 hours, which distinguishes convergence from a slow crawl that happened to be
/// near the truth when the window closed -- the distinction the original finding got wrong in the other
/// direction.
///
/// *Reaches* counts a step across the truth as well as one into the 10% band around it. Silence cannot
/// ease the belief below a ninth of the declaration, which for a `10x` over-declaration is 10.5% above
/// the truth, so the descent waits there for an evaluation that sees a share, and the ease that
/// evaluation makes can carry the belief past the band on the far side.
#[test]
fn an_over_declared_channel_converges_at_every_rate() {
    /// Ticks allowed to reach the truth. Measured 33-83 at 2 spm and 11-21 at 12 across twelve
    /// independent seed sets, slowest at `100x` and 2 spm, so this binds at ~1.2x.
    const MAX_TICKS: usize = 100;

    for factor in [0.01f64, 0.1] {
        for spm in [2.0f32, 12.0] {
            let short = from_declaration(factor, spm, 6);
            let long = from_declaration(factor, spm, 48);
            let ticks = ticks_to_converge(&short);

            eprintln!(
                "  [cold start] {:.0}x over-declared at {spm} spm: reaches the truth after {ticks:?} ticks, ends \
                 {:+.2}% at 6 h and {:+.2}% at 48 h, with {} then {} retargets",
                1.0 / factor,
                short.final_error_pct,
                long.final_error_pct,
                short.retargets,
                long.retargets,
            );

            assert!(
                ticks.is_some_and(|t| t <= MAX_TICKS),
                "a {:.0}x over-declaration at {spm} spm should reach the truth inside \
                 {MAX_TICKS} ticks; it reported {ticks:?}. This is the assertion that replaced a claim that \
                 it never converges at all -- see the module docs",
                1.0 / factor
            );
            assert!(
                long.retargets > short.retargets,
                "and must still be working at 48 h rather than having stalled near the truth: {} retargets \
                 against {} at 6 h",
                long.retargets,
                short.retargets
            );
        }
    }
}

/// **An under-declared channel climbs geometrically, which is the surviving half of the original claim.**
///
/// The other direction, and the one that exercises the step bound's tightening arm. A channel set too
/// *easy* receives a flood, which is the one disturbance a rate estimator sees clearly: evidence arrives
/// faster than it can be acted on, so there is no information problem, only a rate limit.
///
/// Asserted on the retarget count rather than the error, because a partial step closing a fraction of a
/// shrinking gap converges geometrically -- a `100x` error is 4.6 e-folds and should cost a handful of
/// moves, not a hundred. Measured 7 to 12 across the grid, unchanged in shape from the pre-retraction
/// reading, which is why this test is recalibrated in place rather than replaced.
#[test]
fn an_under_declared_channel_climbs_geometrically_at_every_rate() {
    /// Measured 7-12 across the grid, so this binds at ~1.7x.
    const MAX_RETARGETS: usize = 20;

    for factor in [10.0f64, 100.0] {
        for spm in [2.0f32, 12.0] {
            let trial = from_declaration(factor, spm, 6);
            eprintln!(
                "  [cold start] {factor}x under-declared at {spm} spm: {} retargets, ends {:+.2}%",
                trial.retargets, trial.final_error_pct
            );
            assert!(
                trial.retargets <= MAX_RETARGETS,
                "closing a {factor}x under-declaration at {spm} spm took {} retargets, over the \
                 {MAX_RETARGETS} a geometric climb needs -- the step schedule is not compounding",
                trial.retargets
            );
            assert!(
                trial.final_error_pct < 0.0,
                "and should end on the under-difficulty side, which is where the tighten asymmetry leaves \
                 it: {:+.2}%",
                trial.final_error_pct
            );
        }
    }
}

/// **The settled offset after a cold start is the dead zone, so it widens as the share rate falls.**
///
/// Worth separating from convergence, because "converged" and "correct" are different claims and the
/// difference between them is the dead zone. Once the belief is inside the bar the controller stops, and the
/// bar is wider at a sparse rate -- so a sparse channel settles further out for a reason that is not a
/// defect.
///
/// This is also why the tests above ask the belief to reach the truth and not to stay within 10% of it,
/// which is *tighter than the controller's own tolerance* at 2 spm: a settled reading of about −17% there
/// is the threshold doing its job, and reading it as a failure to converge would be reading the dead zone
/// as an error. The retracted lock finding made
/// exactly that mistake in a harder form.
#[test]
fn the_settled_offset_after_a_cold_start_widens_as_the_rate_falls() {
    /// Least ratio between the sparse and dense settled offsets for the widening to be a real effect rather
    /// than sampling. Measured `2.69x`, and `2.5x` to `2.9x` across twelve independent seed sets.
    const MIN_RATIO: f64 = 2.0;
    /// Trials per offset. A ratio of two medians is the one shape in this suite that moves materially with
    /// trial count: at this file's 20 trials the twelve seed sets read as low as `1.92x`, under the
    /// threshold, so this test runs twenty times as many.
    const RATIO_TRIALS: usize = 400;

    let settled_offset = |spm: f32| {
        measure_arm(
            &mis_declared(0.01),
            spm,
            0,
            6 * HOUR_TICKS,
            RATIO_TRIALS,
            Arm::Shipped,
        )
        .final_error_pct
        .abs()
    };
    let sparse = settled_offset(2.0);
    let dense = settled_offset(12.0);

    eprintln!(
        "  [cold start] settled offset after a 100x over-declaration: {sparse:.2}% at 2 spm against \
         {dense:.2}% at 12 spm, a ratio of {:.1}",
        sparse / dense
    );

    assert!(
        sparse > dense * MIN_RATIO,
        "the sparse settled offset should be several times the dense one, because the bar it rests inside \
         is: {sparse:.2}% against {dense:.2}%. If they converge, the dead zone has stopped being \
         rate-dependent and the criteria in the tests above need re-reading"
    );
}
