//! A miner that drops out and returns repeatedly, which is the disturbance that drives the silence
//! machinery against itself.
//!
//! `regimes.rs` covers a single outage: the channel goes dark and stays dark, and what matters is where
//! the ease stops. A channel that *flaps* is a different problem, because each outage eases the belief and
//! each return has to tighten it back -- and those two moves are deliberately not priced the same. So the
//! question is not where one ease stops but where the pair comes to rest after many rounds.
//!
//! **It comes to rest under-difficulty.** A flapping channel sits persistently *below* the truth, because
//! easing on an outage is cheap and tightening on a return costs `TIGHTEN_MULTIPLIER` times as much
//! evidence, so the belief ratchets down: the controller cannot tell absence from a wrong belief. The
//! displacement bound does not stop the ratchet: it is anchored on the declared hashrate, so it bounds the
//! fall across outages at a ninth of the declaration, and the ratchet comes to rest well above that.
//!
//! Median absolute tracking error over 12 h at 50% duty, 2 spm against 12 spm: `133%` / `121%` on a 30 min
//! cycle, `98%` / `53%` on 60 min, `61%` / `27%` on 120 min. So the sparse channel is 1.1x to 2.3x worse
//! depending on the cycle -- not a constant factor.
//!
//! **Re-derived against the repaired sampler, and this file came through almost untouched.** Its up-phase
//! sits at `lambda = 2` shares a tick, which is where the broken sampler's deterministic count happened to
//! equal the mean, so the ratchet, its magnitudes and the resonance all reproduce within a point or two.
//! The one number that moved was the churn *comparator*: stable-channel retargets were measured at `0.25`
//! and `0.33` an hour against a feed that gave the controller nothing to fire on, so the flapping multiple
//! was recorded as `33x`/`55x` when it is `9x`/`29x`. The defect was in the baseline, not in the arm.
//!
//! **Medians, not final readings.** A cyclic trajectory's last tick lands at a fixed phase of the cycle,
//! and that phase differs between cells: a 12 h run ends 240 s after a return on a 30 min cycle and 2940 s
//! after one on a 120 min cycle. So `final_error_pct` compares the worst moment of one cell against a
//! recovered moment of another, and reading it across cells would manufacture an effect out of phase
//! alone. Every cross-cell claim here is asserted on medians, which average over phase.
//!
//! Under difficulty-weighted accounting none of this costs payout -- a share is credited at the difficulty
//! it was found at. It costs load: four to five times the intended share bandwidth, and 9x to 29x the
//! retargets a stable channel actually makes.
//!
//! **The asymmetry is the cause, confirmed by mutation.** With `TIGHTEN_MULTIPLIER` weakened from `8.0` to
//! `3.0` -- a factor of 2.7 -- the under-difficulty shrinks in every cell, by `1.3x` to `4.7x`, and the worst
//! cell's retargets rise from 87 to 139. The under-difficulty is the price of the reluctance and the churn is
//! the price of giving it up.
//!
//! *(`3.0` because the constant assertion beside `MIN_EFFECTIVE_TIGHTEN_MULTIPLIER` refuses any multiplier
//! the discount ceiling would invert -- anything at or below `2.5` at the shipped ceiling -- so `1.0` does
//! not compile. The guard working on a real edit rather than a hypothetical one.)*

use super::trajectory::{constant, intermittent, measure_arm, Arm, Trial, START_HASHRATE};

const MEASURED_HOURS: u64 = 12;
const TRIALS: usize = 20;
/// The estimator's memory, in seconds. Mirrors `EWMA_TAU_SECS`, which the cycle sweep is indexed against.
const TAU_SECS: u64 = 360;
/// Mirrors `MAX_SILENT_DISPLACEMENT`, which is private. Used to show the bound's floor is not where a
/// flapping channel rests.
const EXPECTED_DISPLACEMENT: f64 = 9.0;

/// `period_secs` long cycles, dark for `down_secs` of each.
fn flapping(period_secs: u64, down_secs: u64, spm: f32) -> Trial {
    measure_arm(
        &intermittent(period_secs, down_secs),
        spm,
        0,
        MEASURED_HOURS * 3600,
        TRIALS,
        Arm::Shipped,
    )
}

/// Retargets an hour a *stable* channel at this rate actually makes.
///
/// Measured, not the `<= 4` an hour ceiling `regimes.rs` asserts. Quoting the ceiling as though it were
/// normal behaviour understates the churn below by an order of magnitude.
fn stable_retargets_per_hour(spm: f32) -> f64 {
    let trial = measure_arm(
        &constant(),
        spm,
        3600,
        MEASURED_HOURS * 3600,
        9,
        Arm::Shipped,
    );
    trial.retargets as f64 / MEASURED_HOURS as f64
}

/// **A flapping miner settles under-difficulty at every rate and cycle.**
///
/// The direction is the finding. A disturbance that looks like departure and is not should, naively, leave
/// the channel over-difficulty -- that is what a decline does. It does the opposite, because the two moves
/// the cycle demands are priced differently: the ease an outage triggers is the cheap direction and the
/// tighten a return demands is the expensive one, so each round loses a little ground.
///
/// Asserted as a sign rather than a magnitude, because the magnitude is cycle-dependent and the sign is
/// not. A cell that came out over-difficulty would mean the asymmetry had stopped being the dominant
/// effect and everything below would need re-reading.
///
/// *Settles* is the median trial's last tick, and it does not mean never over. Across twelve independent
/// seed sets of these 20 trials, one or two of the eight cells have some trial over-difficulty during the
/// run, always at 12 spm, by up to `+12%` on the 120 min cycle. At 12 spm up to three trials in 20 end over
/// on the 120 min cycle, and in the other cells at most one does.
/// The median trial ends under in every cell on every set.
#[test]
fn a_flapping_miner_settles_under_difficulty_at_every_cycle() {
    for (period, down) in [(3600u64, 600u64), (3600, 1800), (1800, 900), (7200, 3600)] {
        for spm in [2.0f32, 12.0] {
            let trial = flapping(period, down, spm);
            assert!(
                trial.live_ticks > 0,
                "a cycle of {period}s dark for {down}s must leave live ticks to score, or the error \
                 readings mean nothing"
            );
            eprintln!(
                "  [outage] {period}s cycle dark {down}s at {spm} spm: median {:.2}%, ends {:+.2}%, {} \
                 retargets, rests at {:.3e} H/s",
                trial.median_error_pct,
                trial.final_error_pct,
                trial.retargets,
                trial.final_hashrate,
            );
            assert!(
                trial.final_error_pct < 0.0,
                "a flapping channel should end under-difficulty, not over: {:+.2}% on a {period}s cycle \
                 dark {down}s at {spm} spm. Over-difficulty here would mean the tighten asymmetry is no \
                 longer the dominant effect on this disturbance",
                trial.final_error_pct
            );
        }
    }
}

/// **And the displacement bound is not where it rests, so the asymmetry is the thing to look at.**
///
/// Worth separating because the bound is the obvious suspect -- it is what governs a single outage. No
/// cell here comes within reach of one ninth of the opening belief, so the bound's floor is not where a
/// flapping channel rests. That is all this checks.
///
/// Within the longer outages the floor does bind. At 2 spm on the 120 min cycle it clamps 4828 of 5519
/// silent evaluations, and with the bound removed the channel retargets 250 times in 12 h rather than 59
/// and its median error rises from `61%` to `77%`. Where it rests moves by under 1% on the 30 min cycle
/// and on the 60 min one dark for 10 minutes, and by 26% on the 120 min cycle, and this test passes with
/// the bound removed: it guards the resting position, not the bound.
#[test]
fn the_displacement_bound_is_not_where_a_flapping_channel_rests() {
    let floor = START_HASHRATE as f64 / EXPECTED_DISPLACEMENT;

    for (period, down) in [(3600u64, 600u64), (1800, 900), (7200, 3600)] {
        let trial = flapping(period, down, 2.0);
        assert!(
            trial.final_hashrate as f64 > floor * 1.5,
            "a {period}s cycle dark {down}s rested at {:.3e} H/s, within reach of the {floor:.3e} \
             displacement floor. If the bound binds here, it rather than the tighten asymmetry sets \
             where a flapping channel rests",
            trial.final_hashrate
        );
    }
}

/// **What it costs: four to five times the intended share load, and tens of times the retargets a stable
/// channel makes.**
///
/// The price of the under-difficulty above, in the two units an operator pays in. Neither is a payout cost
/// -- a share is credited at the difficulty it was found at, so the miner gains nothing and the pool loses
/// nothing on that axis. Both are load: bandwidth and validation for the shares, a message and a
/// discontinuity for each retarget.
///
/// The churn comparator is the *measured* stable rate rather than `regimes.rs`'s `<= 4` an hour ceiling. A
/// stable channel actually retargets 0.83 times an hour at 2 shares a minute and 0.83 at 12, so the
/// flapping multiple is **9x and 29x** where the ceiling would imply about 2x to 6x. Comparing against an SLA
/// ceiling reads as though the churn were near-normal; it is nowhere near normal.
///
/// Re-derived against the repaired sampler: the multiples were recorded as 33x and 55x, and the error was
/// entirely in the baseline rather than the flapping arm. See `MIN_CHURN_MULTIPLE`.
#[test]
fn a_flapping_miner_costs_several_times_the_intended_load() {
    /// The worst cycle in the sweep below.
    const PERIOD_SECS: u64 = 1800;
    const DOWN_SECS: u64 = 900;
    /// Least share-load multiple that counts as the effect being real rather than sampling.
    const MIN_SHARE_MULTIPLE: f64 = 3.0;
    /// Least churn multiple over *measured* stable operation.
    ///
    /// **Re-derived.** This recorded `33x` and `55x`, and both were wrong in the comparator rather than in
    /// the flapping arm: the stable baselines they divided by were `0.25` and `0.33` retargets an hour,
    /// measured when the share feed suppressed zero-share ticks and so gave the controller nothing to fire
    /// on. Against real stable churn of `0.83` and `0.83` the multiples are **9x and 29x**. The flapping
    /// numerator (`7.2` and `23.8` an hour) was not where the error was, which is the same reason this
    /// whole file survived the repair: its up-phase sits at `lambda = 2`, where the broken count happened
    /// to equal the mean.
    const MIN_CHURN_MULTIPLE: f64 = 8.0;

    for spm in [2.0f32, 12.0] {
        let trial = flapping(PERIOD_SECS, DOWN_SECS, spm);
        // What the operator asked for: the target rate, over the fraction of the span the miner was up.
        let duty = (PERIOD_SECS - DOWN_SECS) as f64 / PERIOD_SECS as f64;
        let intended = spm as f64 * (MEASURED_HOURS * 60) as f64 * duty;
        let share_multiple = trial.shares_fed as f64 / intended;
        let per_hour = trial.retargets as f64 / MEASURED_HOURS as f64;
        let stable = stable_retargets_per_hour(spm);
        let churn_multiple = per_hour / stable;

        eprintln!(
            "  [outage] {PERIOD_SECS}s cycle at {spm} spm: {} shares against {intended:.0} intended \
             ({share_multiple:.1}x), {per_hour:.1} retargets an hour against a stable channel's measured \
             {stable:.2} ({churn_multiple:.0}x)",
            trial.shares_fed
        );

        assert!(
            share_multiple > MIN_SHARE_MULTIPLE,
            "a flapping channel at {spm} spm delivered {share_multiple:.1}x the intended share load, \
             under the {MIN_SHARE_MULTIPLE}x this records. A smaller multiple would mean the \
             under-difficulty above is not actually reaching the wire"
        );
        assert!(
            churn_multiple > MIN_CHURN_MULTIPLE,
            "and should retarget tens of times more often than a stable channel; it read \
             {churn_multiple:.0}x ({per_hour:.1} against {stable:.2} an hour)"
        );
    }
}

/// **The cost has an interior peak, and it sits at five times the estimator's memory rather than at it.**
///
/// This test exists because the claim it checks was first written as "peaks near `tau`, falls off either
/// side" on the strength of a sweep whose shortest cycle was `5*tau`. Every point was on one side of the
/// maximum, so a monotone falloff was mistaken for the far flank of a peak that had never been located.
/// Sweeping from `tau` to `40*tau` finds the shape is real and the position was wrong by a factor of five.
///
/// Median absolute error at 2 shares a minute, 50% duty: `90%` at `tau`, `119%` at `3*tau`, `133%` at
/// `5*tau`, `98%` at `10*tau`, `61%` at `20*tau`, `49%` at `40*tau`. A genuine interior maximum with
/// falloff on both sides, which is what confines the cost to a band rather than making it a general
/// degradation.
///
/// **Why `5*tau` rather than `tau` is a hypothesis, not a result.** At 50% duty a `5*tau` cycle is dark for
/// `2.5*tau`, about where the filter has forgotten the pre-outage rate (`e^-2.5` is 8%), so the ease is
/// close to complete while the up phase is still too short for the `8x`-gated tighten to undo it. That
/// predicts the peak tracks the *dark* phase rather than the period, and it is not tested here. Stated as a
/// conjecture because this corpus's own record is that a mechanism which reads cleanly is not verified
/// until the dynamics it claims are traced.
#[test]
fn the_cost_has_an_interior_peak_above_the_estimator_memory() {
    /// Cycle lengths as multiples of `tau`, spanning both flanks.
    const MULTIPLES: [u64; 6] = [1, 3, 5, 10, 20, 40];
    /// Where the maximum sits, in multiples of `tau`.
    const PEAK_MULTIPLE: u64 = 5;
    /// How far below the peak each flank must fall for this to be a band rather than a corner.
    const FLANK_FRACTION: f64 = 0.7;

    let readings: Vec<(u64, f64)> = MULTIPLES
        .iter()
        .map(|&m| {
            let period = m * TAU_SECS;
            (m, flapping(period, period / 2, 2.0).median_error_pct)
        })
        .collect();
    eprintln!(
        "  [outage] median error by cycle, 2 spm at 50% duty: {}",
        readings
            .iter()
            .map(|(m, pct)| format!("{m}tau {pct:.1}%"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let (peak_multiple, peak) = *readings
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).expect("no NaN readings"))
        .expect("the sweep is not empty");
    assert_eq!(
        peak_multiple, PEAK_MULTIPLE,
        "the worst cycle should be {PEAK_MULTIPLE} times the memory, not {peak_multiple}. The position is \
         the part of this claim that was already wrong once, so it is pinned rather than described"
    );

    // Falloff on both sides is what makes it a band. The short flank is the half the original sweep missed
    // entirely, so it is the half worth asserting.
    let shortest = readings.first().expect("non-empty").1;
    let longest = readings.last().expect("non-empty").1;
    assert!(
        shortest < peak * FLANK_FRACTION,
        "a cycle at the memory itself should cost materially less than the peak, or there is no short \
         flank and this is a corner rather than a peak: {shortest:.1}% against {peak:.1}%"
    );
    assert!(
        longest < peak * FLANK_FRACTION,
        "and so should one far above it: {longest:.1}% against {peak:.1}%"
    );
}
