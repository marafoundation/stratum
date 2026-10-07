//! Whether the estimator's memory is chosen by the rates most channels run at, and what the rest pay.
//!
//! [`EWMA_TAU_SECS`] is one constant for every channel, and `sigma = sqrt(30 / (r * tau))` says it is
//! half of the only product that buys precision -- the half this crate controls. So its value is a
//! trade, and these tests establish *where on the rate band the trade is struck*.
//!
//! **Re-derived against the repaired sampler (see VARDIFF.md, Retraction), and the answer inverted.**
//! It is struck at the *top* of the band, not the bottom.
//!
//! What survives unchanged is the mechanism: a sleepier estimator is slower to believe a decline, so
//! settled error rises monotonically with `tau` at every rate. What moved is *where that rise starts*.
//! Repaired, the sparse cells settle well into **under**-difficulty at the shipped memory, so a longer
//! memory walks them toward correct rather than out of the gate. Settled error on a sustained 5 %/hr
//! decline, by rate and memory, over [`DECLINE_TRIALS`], against a 5 % gate on over-difficulty:
//!
//! | spm | 180 | 240 | **360** | 540 | 720 | 1080 | 1440 |
//! |-----|-----|-----|-----|-----|-----|------|------|
//! | 2 | −20.97 | −18.42 | **−12.49** | −6.55 | −3.58 | +1.27 | +3.88 |
//! | 4 | −16.83 | −14.39 | **−8.91** | −3.77 | −0.75 | +2.53 | +4.94 |
//! | 6 | −8.62 | −7.01 | **−4.60** | −2.83 | −1.83 | −0.14 | +1.53 |
//! | 12 | −5.37 | −4.14 | **−2.40** | −0.47 | +0.70 | +2.42 | +3.86 |
//! | 30 | −0.06 | +0.60 | **+1.90** | +3.28 | +4.13 | +6.01 | +7.04 |
//!
//! At the shipped memory only the **30**-shares-a-minute cell is on the over-difficulty side at all, and
//! it is the first to cross the gate as `tau` grows -- at 1080, not at any sparse rate. Every cell is
//! inside the gate at 720 **on this decline**. On a steeper 10 %/hr one the same cell is within half a
//! point of it at the shipped 360 (+4.57 %), and so is 2 spm at 720 (+4.86 %), both at 12,800 trials
//! (`on_a_steady_miner_the_gate_holds_by_under_half_a_point`). So the cell that fixes `tau` is the
//! densest one, and it is the calibration rate [`REFERENCE_SPM`] names.
//!
//! The "inside the gate at 720" reading is a floor on the constraint, not the whole of it, and it must not
//! be read as headroom: `wander::the_shallow_declines_memory_headroom_does_not_survive_a_steeper_one`
//! measures the corner where a sleepier memory and a steeper decline push the same way, and there 720
//! puts 30 spm outside the gate and worsens the excursion at all four rates, while the shipped memory
//! holds all four inside.
//!
//! Meanwhile steadiness improves monotonically with `tau` at every rate across the whole swept range:
//! at 2 spm a stable channel's 95th-percentile error falls 43.25 % -> 14.40 % and its churn 1.25 ->
//! 0.25 retargets an hour from 180 s to 1440 s. The cells with the most wobble to gain are the sparse
//! ones, and the cell with no headroom to give is dense.
//!
//! **Every reading in this file is of a *steady* miner, and the steadiness half does not survive wander
//! at the dense end.** `wander::the_steadiness_gain_survives_at_sparse_rates_and_not_at_dense_ones`
//! measures the same one-step-sleepier p95 gain across three disturbance levels: `1.46x / 1.33x / 1.11x`
//! at 2 spm for steady, immersion and hot air, against `1.24x / 1.12x / 0.97x` at 12 spm. A longer memory
//! averages counting noise and cannot average wander slower than itself, and the two cross near the seam
//! -- so under hot wander the dense cell gains nothing. The gate half *does* survive (`+0.60%` at 30 spm
//! at 720 s under hot wander, inside the gate), so what the two files together say is narrower than this
//! one alone: **the case for a longer memory is a sparse-rate case**, which is an argument for splitting
//! `tau` by rate rather than for raising it everywhere.
//!
//! **So: sparse channels run a memory chosen to keep dense ones decline-safe.** That is the inverse of
//! what this file recorded before the repair, and it is the same trade seen from the other end -- still
//! a statement about coupling, not a proposal.
//!
//! # The split is live, and the case against it was mine and wrong
//!
//! This section previously said the split was **closed**. It is not, and the retraction matters more than
//! the original claim because the error was in the *criterion*, not in any reading.
//!
//! The decline-safety gate is a bound on **settled over-difficulty**. Measured at the corner
//! (`10 %/hr` with hot wander), against the `5 %` gate at 400 trials:
//!
//! | arm | 1 spm | 2 | 6 | 30 | verdict |
//! |---|---|---|---|---|---|
//! | shipped `360` | `-14.08` | `-8.50` | `-2.99` | `+2.62` | passes |
//! | uniform `720` | `+0.93` | `+2.73` | `+1.06` | `+5.31` | **fails at 30 spm** |
//! | uniform `1080` | `+8.29` | `+8.89` | `+4.67` | `+8.51` | fails at five of six |
//! | **split `720/360` at 3 spm** | `+0.93` | `+2.73` | `-2.99` | `+2.62` | **passes** |
//!
//! **The split passes the gate that a uniform raise fails**, which is precisely what it was designed to do.
//! Both passes are under hot wander, which pulls the settled position in. On a steady miner the split's
//! 2 spm cell and the shipped memory's 30 spm cell pass by under half a point
//! (`on_a_steady_miner_the_gate_holds_by_under_half_a_point`).
//!
//! It also buys what motivated it, on a stable channel under hot wander:
//!
//! | | over-delivery | retargets |
//! |---|---|---|
//! | 1 spm shipped | `1.267x` | 3 |
//! | 1 spm split | **`1.172x`** | **2** |
//! | 2 spm shipped | `1.194x` | 3 |
//! | 2 spm split | **`1.136x`** | **2** |
//! | 30 spm either | `1.063x` | 4 |
//!
//! Over-delivery here is `shares_fed` against target, so it is the *chronic* harm at sparse -- a channel
//! sitting inside a wide bar is served difficulty that is too easy and over-delivers for as long as it runs.
//! That is the load the sparse regime actually imposes, and the split cuts it by `5-7.5 %` while cutting churn
//! by a third and leaving every dense cell byte-identical.
//!
//! ## What the split costs, and why that did not close it
//!
//! The transient over-difficulty **excursion** at sparse, which roughly doubles: `16.44% -> 26.60%` at 1 spm
//! at the corner, `4.55% -> 9.18%` on a stable channel. That is real and it is asserted below so it cannot
//! vanish silently.
//!
//! But **nothing bounds it.** The gate is on the settled position; there is no stated criterion on the
//! excursion at all. The previous version of this section compared arms on the excursion and read "worse at
//! every rate" as a closure, which treats an unbounded relative comparison as though it were a threshold.
//! That is the error, and it is the same shape as the one `instruments.rs` records: a criterion applied to
//! the wrong quantity.
//!
//! For scale rather than as a criterion: the split moves sparse's excursion from the *best* on the band to
//! slightly worse than what dense already lives with (`26.60%` against dense's shipped `23.54%`). It is
//! in-family, not an outlier.
//!
//! ## And a correction to the gradient below
//!
//! `a_sleepier_memory_costs_the_sparse_band_most_at_the_steep_corner` measures a real thing and its
//! *interpretation* was wrong too. At the shipped memory sparse has the **lowest** excursion of any rate
//! (`16.44%` at 1 spm against `23.54%` at 30), and at `tau = 1080` every rate converges to `26-31%`. So the
//! `2-3x` steeper penalty at sparse is mostly **sparse having more to lose**, not sparse being intrinsically
//! fragile. "Sparse is the tightest cell on the band" was the opposite of true on this axis.
//!
//! ## So the decision is open, and it is a calibration call with the trade priced
//!
//! Sparse pays `5-7.5 %` chronic over-delivery and a third more churn to keep a transient excursion that is
//! better than dense's. Whether that is the right way round needs a loss function weighing sustained load
//! against transient excursion, and no such function exists in this crate. What these tests now remove is
//! the need to *guess* the magnitudes.
//!
//! [`REFERENCE_SPM`]: crate::vardiff::classic::REFERENCE_SPM
//!
//! [`EWMA_TAU_SECS`]: crate::vardiff::classic::EWMA_TAU_SECS

use super::trajectory::{constant, declining, measure_arm, measure_arm_wander, Arm, Trial, Wander};

const ONE_HOUR: u64 = 3600;
const MEASURED_HOURS: u64 = 12;
/// The shipped memory, as an arm. Named so the comparisons read as "against what ships".
const SHIPPED: Arm = Arm::Shipped;
/// One step sleepier. Chosen because it is the sleepiest memory at which *every* swept cell is still
/// inside the gate on the shallow decline -- the whole available headroom above what ships, and so the
/// memory the steadiness gain should be priced at.
const SLEEPIER: Arm = Arm::Tau(720);

/// The memory at which the binding cell crosses the gate. One step beyond [`SLEEPIER`], and the reading
/// that identifies *which* cell binds.
const CROSSING: Arm = Arm::Tau(1080);

/// The calibration rate, as a share rate. The cell that turns out to fix the memory, which is not a
/// coincidence worth hiding: it is where the evidence term is calibrated.
const DENSE_SPM: f32 = 30.0;
/// Settled over-difficulty at which the corpus's decline-safety gate refuses a configuration. Not
/// enforced anywhere on this branch -- `the_absolute_settled_error_is_held_not_merely_the_relative_one`
/// holds a looser bound, because the shipped controller meets this one by under half a point and a
/// 40-trial reading does not resolve that -- but it is the criterion `tau` was selected by, so it is the
/// right yardstick for asking which cell does the selecting.
const DECLINE_SAFETY_GATE_PCT: f64 = 5.0;

/// Trials for the shallow-decline readings.
///
/// Their claims sit a point or two from the gate or from the next rate, against a `2 pp` standard error
/// on a 40-trial median (`instruments::the_settled_reading_is_underpowered_against_an_absolute_gate`). At
/// 40, one of twelve independent seed sets reads 30 spm at the crossing memory inside the gate (`+4.91%`).
/// At this count all twelve put it outside by at least `0.77 pp`, and 6 spm above 4 at the shipped one
/// by at least `3.15 pp`.
const DECLINE_TRIALS: usize = 400;
/// Trials for the steadiness readings, as `wander.rs`, whose 95th percentiles are noisier than a median.
const STABLE_TRIALS: usize = 120;

/// A sustained shallow decline, which is what a long memory is slow to believe.
fn declining_trial(spm: f32, arm: Arm) -> Trial {
    measure_arm(
        &declining(5.0, 0.20),
        spm,
        ONE_HOUR,
        3 * ONE_HOUR,
        DECLINE_TRIALS,
        arm,
    )
}

fn stable_trial(spm: f32, arm: Arm) -> Trial {
    measure_arm(
        &constant(),
        spm,
        ONE_HOUR,
        MEASURED_HOURS * ONE_HOUR,
        STABLE_TRIALS,
        arm,
    )
}

/// **A sleepier memory is refused by the dense cells, and only by them.**
///
/// The coupling claim, stated as the two readings that carry it, and read at [`CROSSING`] -- the memory
/// where the two ends of the band part company. There the dense cell has crossed the gate and the sparse
/// cell is still inside it, so the constraint that fixes the shipped value is not a property of the
/// sparse channels this file used to attribute it to.
///
/// Both halves are asserted, because either alone proves nothing: dense failing says only that the gate
/// binds somewhere, and sparse passing says only that it does not bind there.
///
/// This replaces `a_sleepier_memory_is_refused_by_the_sparse_cells_and_only_by_them`, whose assertion was
/// the same shape with the two rates exchanged. It held only against the broken sampler.
#[test]
fn a_sleepier_memory_is_refused_by_the_dense_cells_and_only_by_them() {
    let sparse = declining_trial(2.0, CROSSING);
    let dense = declining_trial(DENSE_SPM, CROSSING);
    eprintln!(
        "  [tau] at the crossing memory on a 5 %/hr decline: 2 spm settles {:+.2}%, \
         {DENSE_SPM:.0} spm settles {:+.2}%, against a {DECLINE_SAFETY_GATE_PCT:.0}% gate",
        sparse.final_error_pct, dense.final_error_pct
    );

    assert!(
        dense.final_error_pct > DECLINE_SAFETY_GATE_PCT,
        "a sleepier memory should put the {DENSE_SPM:.0} spm cell outside the gate -- that is what makes \
         it the cell doing the selecting. It settled {:+.2}%, inside {DECLINE_SAFETY_GATE_PCT:.0}%, so \
         either the gate no longer binds at dense or the binding cell has moved again",
        dense.final_error_pct
    );
    assert!(
        sparse.final_error_pct < DECLINE_SAFETY_GATE_PCT,
        "and should leave the 2 spm cell inside it, which is what makes the constraint a dense one rather \
         than a band-wide one; it settled {:+.2}%",
        sparse.final_error_pct
    );
    // The gap is the quantity, not either endpoint: it is how much of the gate's margin the dense cell
    // consumes that the sparse cell does not.
    let gap = dense.final_error_pct - sparse.final_error_pct;
    assert!(
        gap > 3.0,
        "the two cells should be separated by more than 3 pp at the same memory, or 'the dense cell is \
         what binds' is a statement about noise; measured {gap:+.2} pp"
    );
}

/// **And why: at the shipped memory the settled error rises with the share rate, crossing zero at the
/// dense end.**
///
/// The mechanism behind the test above, and the reading that makes the inversion legible rather than a
/// surprise. A sparse channel settles *under*-difficulty at the shipped memory and a dense one *over* it,
/// monotonically in between -- so lengthening the memory, which raises settled error at every rate, walks
/// the sparse cells toward correct and the dense cells out of the gate. Same move, opposite sign of
/// consequence, and that is the whole of why the binding end is the dense one.
///
/// Asserted as the monotone shape plus the sign change, rather than cell by cell, because the shape is
/// the claim: any single cell's sign is a reading, and the ordering is the structure.
#[test]
fn settled_error_rises_with_the_share_rate_at_the_shipped_memory() {
    let readings: Vec<(f32, f64)> = [2.0f32, 4.0, 6.0, 12.0, DENSE_SPM]
        .into_iter()
        .map(|spm| (spm, declining_trial(spm, SHIPPED).final_error_pct))
        .collect();
    eprintln!(
        "  [tau] shipped memory, 5 %/hr decline, settled by rate: {}",
        readings
            .iter()
            .map(|(spm, pct)| format!("{spm:.0} spm {pct:+.2}%"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    for pair in readings.windows(2) {
        let ((slow, lower), (fast, higher)) = (pair[0], pair[1]);
        assert!(
            higher > lower,
            "settled error should rise with the share rate at a fixed memory: {slow:.0} spm read \
             {lower:+.2}% and {fast:.0} spm read {higher:+.2}%, which is the wrong way round. The \
             inversion this file reports rests on this ordering"
        );
    }

    let (sparsest, densest) = (readings[0].1, readings[readings.len() - 1].1);
    assert!(
        sparsest < 0.0 && densest > 0.0,
        "and should change sign across the band -- under-difficulty at the sparse end, over-difficulty \
         at the dense end. Measured {sparsest:+.2}% and {densest:+.2}%; if both are the same sign then \
         a longer memory is not the two-sided move this file describes"
    );
}

/// **And that refusal costs every channel real steadiness.**
///
/// The other half of the trade, so the coupling is not reported as a free lunch forgone. A longer memory
/// averages more shares, so the belief moves less: `sigma` falls as `1/sqrt(tau)`, predicting a
/// `sqrt(2)` improvement for one step, and the 95th-percentile tracking error is where a miner would
/// notice it.
///
/// Measured at both ends of the band, because the inversion above put the constraint and its cost on
/// *opposite* ends: the dense cell is what refuses the sleepier memory, and the sparse cell pays for that
/// refusal too. Both gains are available at [`SLEEPIER`], which the test above shows every cell is still
/// inside the gate at -- so neither is hypothetical.
///
/// Which end gains more is not asserted. On a steady miner the mechanism predicts no ordering, and twelve
/// independent seed sets put the sparse gain above the dense one by `0.006` to `0.22`. Under wander it
/// predicts one, which `wander::the_steadiness_gain_survives_at_sparse_rates_and_not_at_dense_ones`
/// asserts.
#[test]
fn that_refusal_costs_both_ends_of_the_band_steadiness() {
    /// Least improvement in 95th-percentile error that counts. `sqrt(2)` is the prediction; twelve
    /// independent seed sets put the dense cell at `1.17-1.36x` and the sparse at `1.34-1.46x`, so this
    /// is about the effect existing, not its exact size. One of those twelve puts the dense cell under it.
    const MIN_IMPROVEMENT: f64 = 1.2;

    let improvement_at = |spm: f32| {
        let shipped = stable_trial(spm, SHIPPED);
        let sleepier = stable_trial(spm, SLEEPIER);
        let improvement = shipped.p95_error_pct / sleepier.p95_error_pct;
        eprintln!(
            "  [tau] {spm:.0} spm stable: shipped p95 {:.2}% against sleepier {:.2}%, a \
             {improvement:.2}x improvement ({:.2} against {:.2} retargets an hour)",
            shipped.p95_error_pct,
            sleepier.p95_error_pct,
            shipped.retargets as f64 / MEASURED_HOURS as f64,
            sleepier.retargets as f64 / MEASURED_HOURS as f64,
        );
        (improvement, shipped.p95_error_pct, sleepier.p95_error_pct)
    };

    for spm in [2.0, 12.0] {
        let (gain, was, now) = improvement_at(spm);
        assert!(
            gain > MIN_IMPROVEMENT,
            "one step sleepier should measurably steady a {spm:.0} spm channel, since sigma falls as \
             1/sqrt(tau); p95 went {was:.2}% -> {now:.2}%, a {gain:.2}x change against the \
             {MIN_IMPROVEMENT}x this asserts. Without this reading the coupling above is a constraint \
             with no cost attached, and there would be no reason to care which cell binds"
        );
    }
}

/// **The mechanism, so the two readings above are not a coincidence of one memory.**
///
/// Settled error should rise monotonically with the memory at a sparse rate: a longer memory is slower to
/// believe a decline, and a decline unbelieved leaves difficulty higher than it should be. If it did not,
/// the readings above would be two points on a curve with no shape and the choice of 720 would be doing
/// the work.
///
/// *Error*, not *over-difficulty*: this is the reading whose sign the repair moved. At a sparse rate the
/// whole shipped range sits **below** zero -- the rise runs from −20.97 % at 180 s to +2.21 % at 1200 s,
/// crossing into over-difficulty only past the shipped memory. The monotone shape is what survives; the
/// claim that the rise happens *within* over-difficulty is what did not.
#[test]
fn settled_error_rises_with_the_memory_at_a_sparse_rate() {
    let readings: Vec<(u64, f64)> = [Arm::Tau(180), SHIPPED, SLEEPIER, Arm::Tau(1200)]
        .into_iter()
        .map(|arm| {
            let tau = match arm {
                Arm::Tau(secs) => secs,
                _ => 360,
            };
            (tau, declining_trial(2.0, arm).final_error_pct)
        })
        .collect();
    eprintln!(
        "  [tau] 2 spm, 5 %/hr decline, settled by memory: {}",
        readings
            .iter()
            .map(|(tau, pct)| format!("{tau}s {pct:+.2}%"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    for pair in readings.windows(2) {
        let ((short, shallower), (long, deeper)) = (pair[0], pair[1]);
        assert!(
            deeper > shallower,
            "a longer memory should settle higher on a decline: {short}s read {shallower:+.2}% and \
             {long}s read {deeper:+.2}%, which is the wrong way round. The coupling this file reports \
             rests on this being monotone"
        );
    }
}

/// Trials for the corner readings below.
///
/// As many as [`DECLINE_TRIALS`], and not for show: the corner's per-trial spread is about `10 pp`, so a
/// 40-trial median carries a standard error near `2 pp` -- see
/// `instruments::the_settled_reading_is_underpowered_against_an_absolute_gate`. The claims here are
/// differences of a few pp between arms, which that error would swamp.
const CORNER_TRIALS: usize = 400;

/// The memory the penalty is priced at. [`CROSSING`] rather than [`SLEEPIER`], so the gradient is read over
/// the whole span a raise could plausibly cover.
const CORNER_SLEEPIER: Arm = Arm::Tau(1080);

/// A steep decline with hot wander -- the corner that fixes the memory, and the one the tests above do not
/// reach.
fn corner_trial(spm: f32, arm: Arm) -> Trial {
    measure_arm_wander(
        &declining(10.0, 0.40),
        spm,
        ONE_HOUR,
        3 * ONE_HOUR,
        CORNER_TRIALS,
        arm,
        Some(Wander::HOT_AIR),
    )
}

/// The excursion at the corner. `worst_over_pct` rather than `final_error_pct` wherever a single number has
/// to carry a claim: a gate on over-difficulty is a claim about the excursion, and the excursion is also the
/// statistic with the resolution to carry one -- `3.6 %` relative error against `117 %`.
fn corner_excursion_pct(spm: f32, arm: Arm) -> f64 {
    corner_trial(spm, arm).worst_over_pct
}

/// **A sleepier memory costs the sparse band most on the excursion -- which prices the split, not closes it.**
///
/// The reading the rest of this file lacked. Every test above measures a *shallow* decline and reports the
/// settled position; this measures the corner that actually fixes `tau` and reports the excursion.
///
/// **Read it as a price, not a verdict.** Nothing bounds the excursion, so a comparison on it cannot refuse a
/// configuration -- see the module docs, where using it as a closure is retracted. And the gradient's cause
/// is not sparse fragility: at the shipped memory sparse has the *lowest* excursion of any rate, and at
/// `tau = 1080` every rate converges to `26-31%`, so the steeper sparse penalty is mostly sparse having more
/// to lose.
///
/// Asserted as the gradient rather than as a pass/fail against the gate, because the gate's own value was
/// calibrated on the settled statistic and carrying it across to the excursion would compare two different
/// quantities to one threshold.
///
/// **Asserted on both statistics**, which is the point that makes the finding robust rather than a choice of
/// yardstick. The obvious objection is that the excursion was picked because it gives the wanted answer; it
/// does not -- the settled position gives the same ordering, and a steeper one (`3.80x` against `2.66x` at
/// 1 spm, and steeper in each of twelve independent samples). The corner is what reverses the conclusion,
/// not the statistic, and a mutation that swaps one for the other should *not* be able to break this test.
#[test]
fn a_sleepier_memory_costs_the_sparse_band_most_at_the_steep_corner() {
    /// Least ratio between the sparse and dense penalties for the gradient to be a real effect. Measured
    /// `2.66x` and `2.21x` on the excursion and `3.80x` and `2.95x` on the settled position. The 2 spm
    /// excursion ratio is the tightest, and twelve independent samples put it at `1.71-2.23x`, `2.03x` on
    /// average with a spread of `0.17`, so this sits three spreads below the mean.
    const MIN_RATIO: f64 = 1.5;

    // The memory penalty at one rate, as (excursion, settled) in percentage points.
    let penalty = |spm: f32| {
        let (shipped, sleepier) = (
            corner_trial(spm, SHIPPED),
            corner_trial(spm, CORNER_SLEEPIER),
        );
        let excursion = sleepier.worst_over_pct - shipped.worst_over_pct;
        let settled = sleepier.final_error_pct - shipped.final_error_pct;
        eprintln!(
            "  [tau] {spm} spm at the corner: excursion {:.2}% -> {:.2}% ({excursion:+.2} pp), settled \
             {:+.2}% -> {:+.2}% ({settled:+.2} pp)",
            shipped.worst_over_pct,
            sleepier.worst_over_pct,
            shipped.final_error_pct,
            sleepier.final_error_pct,
        );
        (excursion, settled)
    };

    let sparsest = penalty(1.0);
    let sparse = penalty(2.0);
    let dense = penalty(DENSE_SPM);

    for (label, sparsest, sparse, dense) in [
        ("excursion", sparsest.0, sparse.0, dense.0),
        ("settled", sparsest.1, sparse.1, dense.1),
    ] {
        for (spm, pct) in [(1.0, sparsest), (2.0, sparse), (DENSE_SPM, dense)] {
            assert!(
                pct > 0.0,
                "a sleepier memory should worsen the {label} at every rate, including the sparse ones the \
                 split was meant to serve: {spm} spm moved {pct:+.2} pp. If any rate benefits, a split has \
                 something to select and the closure in the module docs is wrong"
            );
        }
        assert!(
            sparsest > dense * MIN_RATIO && sparse > dense * MIN_RATIO,
            "and should cost the sparse band several times what it costs the dense band, on the {label}: \
             {sparsest:+.2} pp at 1 spm and {sparse:+.2} pp at 2 against {dense:+.2} pp at {DENSE_SPM:.0}. \
             This is the price a split pays, not a reason to refuse one, and it must hold on both statistics \
             or it is an artefact of the yardstick"
        );
    }
}

/// **The split passes the gate a uniform raise fails, and this test is the retraction.**
///
/// It replaces `the_split_degrades_exactly_the_cells_it_was_meant_to_help`, which asserted the split's only
/// effect was to worsen its own band. That was true of the excursion and false as a verdict: the criterion
/// this crate actually states is a bound on settled over-difficulty, and on that criterion the split passes
/// everywhere while a uniform raise fails at the dense end. A test whose name states a retracted conclusion
/// is a false record, so it is gone rather than retuned.
///
/// Four axes, because a one-axis reading is what went wrong:
///
/// 1. **the gate** -- the split passes at every rate, the uniform raise does not;
/// 2. **chronic over-delivery** -- the sparse harm, which the split reduces;
/// 3. **churn** -- retargets, which the split reduces;
/// 4. **the excursion** -- which the split worsens, asserted so the cost cannot vanish silently.
///
/// Routing is asserted first, since a knob that silently failed to route would make the arm look free.
#[test]
fn the_split_passes_the_gate_a_uniform_raise_fails() {
    /// Below the seam and below the sparsest cell that fails on the dense side.
    const SPLIT_AT_SPM: f32 = 3.0;
    /// Two rates either side of the split point, including the dense cell that refuses a uniform raise.
    const RATES: [f32; 4] = [1.0, 2.0, 6.0, DENSE_SPM];
    /// Least reduction in chronic over-delivery at sparse for the split to be buying anything. Measured
    /// `1.267x -> 1.172x` at 1 spm, so this binds at about half the effect.
    const MIN_OVER_DELIVERY_GAIN: f64 = 0.04;

    let split = Arm::TauSplit {
        split_rate: SPLIT_AT_SPM,
        below: 720,
        at_or_above: 360,
    };
    let sleepy = Arm::Tau(720);

    // Routing: sleepy below the split rate, shipped at or above it.
    for spm in [1.0f32, 2.0] {
        let (a, b) = (
            corner_excursion_pct(spm, split),
            corner_excursion_pct(spm, sleepy),
        );
        assert!(
            (a - b).abs() < 1e-9,
            "below {SPLIT_AT_SPM} spm the split must run the sleepy memory: {spm} spm read {a:.2}% \
             against {b:.2}% for a uniform 720. If these differ the knob is not routing and every reading \
             here is of something else"
        );
    }
    for spm in [6.0f32, DENSE_SPM] {
        let (a, b) = (
            corner_excursion_pct(spm, split),
            corner_excursion_pct(spm, SHIPPED),
        );
        assert!(
            (a - b).abs() < 1e-9,
            "and at or above it the shipped memory: {spm} spm read {a:.2}% against {b:.2}% shipped"
        );
    }

    // Axis 1, the one that matters: the stated gate.
    let worst_settled = |arm: Arm| {
        RATES
            .iter()
            .map(|&spm| corner_trial(spm, arm).final_error_pct)
            .fold(f64::MIN, f64::max)
    };
    let (shipped_worst, sleepy_worst, split_worst) = (
        worst_settled(SHIPPED),
        worst_settled(sleepy),
        worst_settled(split),
    );
    eprintln!(
        "  [tau] worst settled over-difficulty at the corner against a {DECLINE_SAFETY_GATE_PCT:.0}% gate: \
         shipped {shipped_worst:+.2}%, uniform 720 {sleepy_worst:+.2}%, split {split_worst:+.2}%"
    );

    assert!(
        sleepy_worst > DECLINE_SAFETY_GATE_PCT,
        "a uniform raise should fail the gate -- that is the constraint the split exists to work around. \
         It read {sleepy_worst:+.2}%, inside the gate, so either the gate has stopped binding on a uniform \
         raise or the binding cell has moved"
    );
    assert!(
        split_worst < DECLINE_SAFETY_GATE_PCT,
        "and the split should pass it: worst {split_worst:+.2}% against {DECLINE_SAFETY_GATE_PCT:.0}%. \
         This assertion is the retraction -- a previous version of this file concluded the split was closed \
         by comparing arms on the unbounded excursion instead of on this gate"
    );

    // Axes 2 and 3: what the split buys at sparse, and that it costs dense nothing.
    let stable = |spm: f32, arm: Arm| {
        let t = measure_arm_wander(
            &constant(),
            spm,
            ONE_HOUR,
            3 * ONE_HOUR,
            CORNER_TRIALS,
            arm,
            Some(Wander::HOT_AIR),
        );
        let minutes = (3 * ONE_HOUR) as f64 / 60.0;
        (t.shares_fed as f64 / (spm as f64 * minutes), t.retargets)
    };
    for spm in [1.0f32, 2.0] {
        let ((was, was_rt), (now, now_rt)) = (stable(spm, SHIPPED), stable(spm, split));
        eprintln!(
            "  [tau] {spm} spm stable: over-delivery {was:.3}x -> {now:.3}x, retargets {was_rt} -> {now_rt}"
        );
        assert!(
            was - now > MIN_OVER_DELIVERY_GAIN,
            "the split should measurably cut chronic over-delivery at {spm} spm, which is the sparse harm \
             it exists for: {was:.3}x -> {now:.3}x, a gain of {:.3} against the \
             {MIN_OVER_DELIVERY_GAIN} asserted",
            was - now
        );
        assert!(
            now_rt <= was_rt,
            "and must not cost churn while doing it: {was_rt} retargets -> {now_rt}"
        );
    }
    let ((dense_was, dense_rt), (dense_now, dense_now_rt)) =
        (stable(DENSE_SPM, SHIPPED), stable(DENSE_SPM, split));
    assert!(
        (dense_was - dense_now).abs() < 1e-9 && dense_rt == dense_now_rt,
        "and must leave the dense band untouched, since it is shipped there by construction: \
         {dense_was:.3}x/{dense_rt} against {dense_now:.3}x/{dense_now_rt}"
    );

    // Axis 4: the price, asserted so it cannot disappear without notice.
    for spm in [1.0f32, 2.0] {
        let (shipped, as_split) = (
            corner_excursion_pct(spm, SHIPPED),
            corner_excursion_pct(spm, split),
        );
        eprintln!(
            "  [tau] {spm} spm excursion, the price: {shipped:.2}% -> {as_split:.2}% ({:+.2} pp)",
            as_split - shipped
        );
        assert!(
            as_split > shipped,
            "the split's sparse side should worsen the excursion -- that is the cost being priced, and if \
             it ever stops holding the trade has changed and the module docs need re-reading: {spm} spm \
             read {as_split:.2}% against {shipped:.2}%"
        );
    }
}

/// Trials for the steady-miner margin below, thirty-two times [`CORNER_TRIALS`]: the margin is a fraction of
/// a point, and at 400 trials the readings land on either side of the gate.
const MARGIN_TRIALS: usize = 12_800;

/// **On a steady miner the shipped memory and the split both pass the gate, by under half a point.**
///
/// The corner above carries hot wander, which pulls the settled position in. On a steady miner and the same
/// 10 %/hr decline the two cells nearest the gate are the shipped memory at 30 spm and the split's sleepy
/// side at 2 spm, and they read `+4.57%` and `+4.86%` at [`MARGIN_TRIALS`]. At [`CORNER_TRIALS`] they read
/// `+4.74%` and `+4.78%`, and at the suite's usual 40, `+5.53%` and `+6.03%`. Twelve independent samples at
/// each count put the 400-trial readings at `+4.39` to `+5.03%` and `+3.66` to `+5.25%`, either side of the
/// gate, and the 12,800-trial ones at `+4.54` to `+4.71%` and `+4.78` to `+4.95%`. So the gate verdict on
/// either cell turns on sampling noise at any trial count the rest of this suite runs, and each passes
/// narrowly.
///
/// Both margins hold at these two rates and not beside them, each at 6,400 trials: the split's sleepy side
/// reads `+5.31%` at 2.5 spm and `+5.23%` at 2.9, and the shipped memory `+5.62%` at 40 spm and `+8.55%` at
/// 90. All of them are read on the decline's last tick, which is the tick it reaches its floor.
///
/// Ignored for its 25,600 trajectories, about twenty seconds in a debug build. Run it with `--ignored` to
/// reproduce the figures `VARDIFF.md` quotes for the steady-miner margin.
#[test]
#[ignore = "12,800 trials per cell; run with --ignored to reproduce the steady-miner gate margin"]
fn on_a_steady_miner_the_gate_holds_by_under_half_a_point() {
    /// Most the powered readings may sit inside the gate. Measured `0.43` and `0.14` points, and `0.29-0.46`
    /// and `0.05-0.22` across the twelve samples.
    const MAX_MARGIN_PCT: f64 = 0.5;

    let split = Arm::TauSplit {
        split_rate: 3.0,
        below: 720,
        at_or_above: 360,
    };
    let settled = |spm: f32, arm: Arm, trials: usize| {
        measure_arm(
            &declining(10.0, 0.40),
            spm,
            ONE_HOUR,
            3 * ONE_HOUR,
            trials,
            arm,
        )
        .final_error_pct
    };

    for (label, spm, arm) in [("shipped", DENSE_SPM, SHIPPED), ("split", 2.0, split)] {
        let (corner, powered) = (
            settled(spm, arm, CORNER_TRIALS),
            settled(spm, arm, MARGIN_TRIALS),
        );
        eprintln!(
            "  [tau] steady miner on a 10 %/hr decline, {label} at {spm} spm: {corner:+.2}% at \
             {CORNER_TRIALS} trials, {powered:+.2}% at {MARGIN_TRIALS}"
        );
        assert!(
            powered < DECLINE_SAFETY_GATE_PCT && DECLINE_SAFETY_GATE_PCT - powered < MAX_MARGIN_PCT,
            "the {label} arm at {spm} spm should sit inside the {DECLINE_SAFETY_GATE_PCT:.0}% gate by under \
             {MAX_MARGIN_PCT} points on a steady miner: it read {powered:+.2}% at {MARGIN_TRIALS} trials. \
             VARDIFF.md quotes this margin for both the shipped memory and the split"
        );
    }
}
