//! What the measurements look like once the miner's own rate is allowed to wander.
//!
//! Every other trajectory in this suite holds the miner perfectly steady between disturbances. Real ones
//! do not: measured per unit on the truth-poller archive, an immersion miner wanders 3.00% of its rate
//! with a timescale of 301 s and an air-cooled one that ran to 94 C wanders 12.58% at 2269 s. The second
//! is the one that matters, because wander slower than the estimator's memory is not averaged away --
//! 93% of it arrives at the estimate intact, on top of the counting noise.
//!
//! So a wander-free ensemble is not conservative, and the research branch's own wander test records that
//! it is not conservative *by a varying amount*: adding wander moved which cell binds its safety gate.
//! Three of this branch's conclusions were drawn wander-free, and this re-runs them.
//!
//! **Two survive and one does not.** The estimator-memory coupling survives and strengthens: the band of
//! rates that refuse a sleepier memory widens from 2 shares a minute to 2 and 4. The window-cap
//! crossing survives unchanged. But `window_cap_form.rs`'s second reason for rejecting the rate-aware
//! cap -- that capping a sparse window costs a slow decline -- **is a wander-free artifact**: the +3.27 pp
//! it costs with a steady miner becomes −0.31 pp with a wandering one. The rejection still stands on its
//! first reason, which is unaffected.
//!
//! And one thing the wander-free readings hid outright: at the cell that binds, the shipped controller's
//! margin to the safety gate is about 1.3x rather than the 4x a steady miner suggests.

use super::trajectory::{constant, declining, measure_arm_wander, Arm, Trial, Wander};

const ONE_HOUR: u64 = 3600;
const MEASURED_HOURS: u64 = 12;
const TRIALS: usize = 40;
/// As `tau_split.rs`. The criterion the estimator's memory was selected by.
const DECLINE_SAFETY_GATE_PCT: f64 = 5.0;
/// The shipped memory, in seconds, for the reduction the estimator applies to wander.
const SHIPPED_TAU_SECS: u64 = 360;

fn declining_under(spm: f32, arm: Arm, wander: Option<Wander>) -> Trial {
    measure_arm_wander(
        &declining(5.0, 0.20),
        spm,
        ONE_HOUR,
        3 * ONE_HOUR,
        TRIALS,
        arm,
        wander,
    )
}

fn stable_under(spm: f32, arm: Arm, wander: Option<Wander>) -> Trial {
    measure_arm_wander(
        &constant(),
        spm,
        ONE_HOUR,
        MEASURED_HOURS * ONE_HOUR,
        9,
        arm,
        wander,
    )
}

/// **The model reproduces the units it claims to model, before anything is concluded from it.**
///
/// A disturbance model that did not reproduce its own measured parameters would make every reading below
/// a statement about the harness. Two things to check: that the reduction the estimator applies matches
/// the three rows measured on hardware, and that the hot unit really is the one that survives the filter.
#[test]
fn the_wander_model_matches_the_measured_units() {
    // The three measured rows, as (stationary spread, timescale, what the estimator saw).
    const ROWS: [(f64, u64, f64); 3] = [
        (0.0358, 1047, 0.0309),
        (0.0300, 301, 0.0203),
        (0.1258, 2269, 0.1168),
    ];

    for (sigma, t_c_secs, measured_seen) in ROWS {
        let seen = Wander { sigma, t_c_secs }.seen_by_estimator(SHIPPED_TAU_SECS);
        assert!(
            (seen / measured_seen - 1.0).abs() < 0.01,
            "a unit wandering {:.2}% at {t_c_secs}s should present {:.2}% to the estimator, as measured; \
             the model says {:.2}%. If this is off by about sqrt of the ratio, the reduction has reverted \
             to `T_c/(T_c+tau)`, which is what the corpus's prose says and what its own three rows \
             contradict",
            sigma * 100.0,
            measured_seen * 100.0,
            seen * 100.0
        );
    }

    // And the asymmetry that makes the hot unit the binding one: it keeps almost all of a much larger
    // spread, so it dominates the counting noise rather than adding to it.
    let hot = Wander::HOT_AIR.seen_by_estimator(SHIPPED_TAU_SECS);
    let cool = Wander::IMMERSION.seen_by_estimator(SHIPPED_TAU_SECS);
    assert!(
        hot > cool * 5.0,
        "the hot unit should present several times the immersion unit's disturbance ({:.2}% against \
         {:.2}%); if they are comparable, there is no worst case to judge the gate at",
        hot * 100.0,
        cool * 100.0
    );
}

/// **The memory coupling survives wander, and the band that refuses a sleepier memory widens.**
///
/// `tau_split.rs` establishes wander-free that one step sleepier is refused by 2 shares a minute and not
/// by 12. Under the worst measured wander that still holds, and 4 shares a minute crosses the gate too --
/// so the constraint is a sparse-band one rather than a single-cell one, and the conclusion is stronger
/// than the wander-free reading supported.
#[test]
#[ignore = "numbers measured against the broken share sampler; needs re-derivation -- see VARDIFF.md, Retraction"]
fn a_sleepier_memory_is_still_refused_by_the_sparse_band_under_wander() {
    let wander = Some(Wander::HOT_AIR);
    let sparse: Vec<(f32, f64)> = [2.0f32, 4.0]
        .into_iter()
        .map(|spm| {
            (
                spm,
                declining_under(spm, Arm::Tau(720), wander).final_error_pct,
            )
        })
        .collect();
    let dense = declining_under(12.0, Arm::Tau(720), wander).final_error_pct;
    eprintln!(
        "  [wander] one step sleepier, hot unit: {}, and 12 spm settles {dense:+.2}%",
        sparse
            .iter()
            .map(|(spm, pct)| format!("{spm} spm settles {pct:+.2}%"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    for (spm, settled) in &sparse {
        assert!(
            *settled > DECLINE_SAFETY_GATE_PCT,
            "at {spm} spm a sleepier memory should sit outside the {DECLINE_SAFETY_GATE_PCT:.0}% gate \
             under measured wander; it settled {settled:+.2}%. Wander-free, 4 spm was inside it, so this \
             widening is the finding -- if it has reversed, the band has narrowed and the coupling is \
             weaker than recorded"
        );
    }
    assert!(
        dense < DECLINE_SAFETY_GATE_PCT,
        "and 12 spm should stay inside it, which is what keeps this a sparse constraint rather than a \
         band-wide one; it settled {dense:+.2}%"
    );
}

/// **And the wobble the dense channel is paying for that constraint survives too.**
///
/// A gain that only existed with a perfectly steady miner would not be worth withholding. Measured at
/// three disturbance levels the improvement is 1.33x to 1.35x, which is the `sqrt(2)` the `sigma`
/// relation predicts and is unmoved by wander -- as it should be, since a longer memory averages the
/// counting noise and the wander alike.
#[test]
#[ignore = "numbers measured against the broken share sampler; needs re-derivation -- see VARDIFF.md, Retraction"]
fn the_dense_wobble_gain_survives_wander() {
    let mut shipped_p95 = Vec::new();
    for (label, wander) in [
        ("steady", None),
        ("immersion", Some(Wander::IMMERSION)),
        ("hot", Some(Wander::HOT_AIR)),
    ] {
        let shipped = stable_under(12.0, Arm::Shipped, wander);
        let sleepier = stable_under(12.0, Arm::Tau(720), wander);
        let improvement = shipped.p95_error_pct / sleepier.p95_error_pct;
        eprintln!(
            "  [wander] 12 spm stable, {label}: p95 {:.2}% -> {:.2}%, {improvement:.2}x",
            shipped.p95_error_pct, sleepier.p95_error_pct
        );
        assert!(
            improvement > 1.2,
            "the sleepier memory's steadiness gain at 12 spm should survive {label} wander; it read \
             {improvement:.2}x"
        );
        shipped_p95.push(shipped.p95_error_pct);
    }

    // Self-guarding: the three readings above are the same reading unless the disturbance is actually
    // reaching the controller, and a gain that "survives" a wander of zero is not evidence of anything.
    assert!(
        shipped_p95[2] > shipped_p95[0] * 1.2,
        "a wandering miner should track measurably worse than a steady one at the same memory: p95 \
         {:.2}% hot against {:.2}% steady. If these are equal the wander is not reaching the \
         controller and every 'survives wander' claim in this file is vacuous",
        shipped_p95[2],
        shipped_p95[0]
    );
}

/// **What the wander-free readings hid: the shipped margin at the binding cell is thin.**
///
/// This is the reading that changes how the other numbers should be read. Wander-free, the shipped
/// controller settles −1.18% at 2 shares a minute on a slow decline, which looks like several times the
/// margin the gate asks for. Under the worst measured wander it settles +3.74%, a margin of about 1.3x --
/// and the worst excursion on the way nearly doubles.
///
/// Bounded rather than merely printed, because the margin is the quantity a future change erodes without
/// anything else noticing.
#[test]
#[ignore = "numbers measured against the broken share sampler; needs re-derivation -- see VARDIFF.md, Retraction"]
fn the_shipped_margin_at_the_binding_cell_is_thin_under_wander() {
    let steady = declining_under(2.0, Arm::Shipped, None);
    let hot = declining_under(2.0, Arm::Shipped, Some(Wander::HOT_AIR));
    eprintln!(
        "  [wander] 2 spm, 5 %/hr decline, shipped: steady settles {:+.2}% (worst {:+.2}%), hot settles \
         {:+.2}% (worst {:+.2}%)",
        steady.final_error_pct, steady.worst_over_pct, hot.final_error_pct, hot.worst_over_pct
    );

    assert!(
        hot.final_error_pct > steady.final_error_pct + 2.0,
        "measured wander should move the settled position at the binding cell materially toward \
         over-difficulty: {:+.2}% steady against {:+.2}% hot. If it does not, wander-free measurement \
         is conservative here after all and this file's premise is wrong",
        steady.final_error_pct,
        hot.final_error_pct
    );
    assert!(
        hot.final_error_pct < DECLINE_SAFETY_GATE_PCT,
        "the shipped controller must still clear the {DECLINE_SAFETY_GATE_PCT:.0}% gate under measured \
         wander; it settled {:+.2}%",
        hot.final_error_pct
    );
    assert!(
        DECLINE_SAFETY_GATE_PCT / hot.final_error_pct < 2.0,
        "and the margin is thin -- about 1.3x, not the 4x the steady reading implies. This asserts the \
         thinness so that a change which restores comfortable margin fails here and gets explained \
         rather than absorbed; measured {:.2}x",
        DECLINE_SAFETY_GATE_PCT / hot.final_error_pct
    );
    assert!(
        hot.worst_over_pct > steady.worst_over_pct * 1.5,
        "and the transient excursion should grow with the disturbance: {:+.2}% against {:+.2}%",
        hot.worst_over_pct,
        steady.worst_over_pct
    );
}

/// **The rate-aware cap's sparse cost does not survive wander, and its other rejection reason does.**
///
/// `window_cap_form.rs` rejects the `sigma`-matched cap for two reasons. This re-runs both.
///
/// The second -- that capping a sparse window costs a slow decline -- is a wander-free artifact. The
/// +3.27 pp it costs against a steady miner is −0.31 pp against a wandering one, because the shipped arm
/// has itself degraded by more than the cap's effect. So "the seam gate is load-bearing" is supported
/// only with a steady miner, and should not be cited as a measured property of the deployment.
///
/// The first -- that the matched window is *looser* than the constant above about 8 shares a minute and
/// costs the retargets the cap exists to prevent -- is unaffected: 0.33 against 0.67 an hour at 12 shares
/// a minute, identical with and without wander. **The rejection stands on that reason alone**, and the
/// `TighterOfBoth` residue is untouched by all of it.
#[test]
#[ignore = "numbers measured against the broken share sampler; needs re-derivation -- see VARDIFF.md, Retraction"]
fn the_caps_sparse_cost_is_wander_free_but_its_crossing_cost_is_not() {
    let wander = Some(Wander::HOT_AIR);

    let steady_cost = declining_under(2.0, Arm::SigmaMatchedCap, None).final_error_pct
        - declining_under(2.0, Arm::Shipped, None).final_error_pct;
    let hot_cost = declining_under(2.0, Arm::SigmaMatchedCap, wander).final_error_pct
        - declining_under(2.0, Arm::Shipped, wander).final_error_pct;
    eprintln!("  [wander] 2 spm cap cost: {steady_cost:+.2} pp steady, {hot_cost:+.2} pp hot");
    assert!(
        steady_cost > 2.5,
        "the steady-miner cost is what window_cap_form.rs records; it read {steady_cost:+.2} pp"
    );
    assert!(
        hot_cost < 1.0,
        "and it should not survive wander -- that is this test's finding. It read {hot_cost:+.2} pp, \
         which is large enough that the cost is real under wander too, and window_cap_form.rs's second \
         rejection reason would then stand on its own after all"
    );

    // The first reason, which does survive, and the residue, which is unaffected either way.
    let shipped = stable_under(12.0, Arm::Shipped, wander).retargets;
    let matched = stable_under(12.0, Arm::SigmaMatchedCap, wander).retargets;
    let tighter = stable_under(12.0, Arm::TighterOfBothCaps, wander).retargets;
    assert!(
        matched > shipped,
        "above the crossing the matched window is looser and must still cost retargets under wander: \
         {matched} against {shipped}"
    );
    assert_eq!(
        tighter, shipped,
        "and the residue must still be an exact no-op there, wander or not: {tighter} against {shipped}"
    );
}
/// **The three re-run conclusions across the wander box, not just at the one measured unit.**
///
/// Everything above rests on a single air-cooled unit from a single archive, measured with a sensor the
/// corpus itself classes as uncharacterized. That is one point, and three conclusions stand on it. This
/// sweeps a `3x3` box around it -- spread 5% to 20%, timescale 600 s to 6000 s -- and asserts the form of
/// each conclusion that survives the whole box rather than the form that happens to hold at the point.
///
/// **One conclusion is weakened and one is made worse.**
///
/// The memory coupling survives as an *ordering*: the sparse cell is worse than the dense one by at least
/// 3.3 pp in every cell of the box. Its stronger form -- that sparse crosses the 5% gate outright -- holds
/// in six of nine; at low spread or very slow wander the sparse cells sit just inside, at +4.0 to +5.1. So
/// "sparse is what binds" is robust and "sparse fails the gate" is condition-dependent, and only the first
/// should be relied on.
///
/// The cap's sparse cost stays absent: the steady-miner `+3.27 pp` is not reproduced anywhere in the box,
/// whose worst cell is `+2.04`.
///
/// And **the shipped controller itself exceeds the gate in three of the nine cells** (+6.67, +6.26,
/// +5.35). At the measured unit it clears, which is what `the_shipped_margin_at_the_binding_cell_is_thin`
/// asserts and all that reading claims. Across the plausible box the margin is not thin but absent. That is
/// a stronger statement of the same concern and it is recorded here rather than in that test, because that
/// test is about the measured unit and this one is about the box.
#[test]
#[ignore = "numbers measured against the broken share sampler; needs re-derivation -- see VARDIFF.md, Retraction"]
fn the_conclusions_that_survive_the_whole_wander_box() {
    /// Spreads and timescales bracketing the measured unit.
    const SPREADS: [f64; 3] = [0.05, 0.1258, 0.20];
    const TIMESCALES: [u64; 3] = [600, 2269, 6000];
    /// Least margin by which the sparse cell must be worse than the dense one, in points. The box's
    /// minimum is 3.27, so this binds at 1.2x.
    const MIN_SPARSE_DENSE_GAP_PCT: f64 = 2.7;
    /// Worst sparse-cap cost anywhere in the box. Well below the steady-miner reading it replaces.
    const MAX_CAP_COST_PCT: f64 = 2.5;

    let mut worst_gap = f64::MAX;
    let mut worst_cap_cost = f64::MIN;
    let mut shipped_over_gate = 0usize;

    for sigma in SPREADS {
        for t_c_secs in TIMESCALES {
            let w = Some(Wander { sigma, t_c_secs });
            let sparse = declining_under(2.0, Arm::Tau(720), w).final_error_pct;
            let dense = declining_under(12.0, Arm::Tau(720), w).final_error_pct;
            let shipped = declining_under(2.0, Arm::Shipped, w).final_error_pct;
            let cap_cost = declining_under(2.0, Arm::SigmaMatchedCap, w).final_error_pct - shipped;

            worst_gap = worst_gap.min(sparse - dense);
            worst_cap_cost = worst_cap_cost.max(cap_cost);
            if shipped > DECLINE_SAFETY_GATE_PCT {
                shipped_over_gate += 1;
            }

            assert!(
                sparse - dense > MIN_SPARSE_DENSE_GAP_PCT,
                "at spread {sigma} and timescale {t_c_secs}s a sleepier memory should still hurt 2 spm \
                 much more than 12 spm: {sparse:+.2}% against {dense:+.2}%, a gap of {:+.2} pp. The \
                 ordering is the robust form of the coupling claim -- if it fails anywhere in this box, \
                 the claim does not survive its own uncertainty",
                sparse - dense
            );
            assert!(
                cap_cost < MAX_CAP_COST_PCT,
                "and capping the sparse window should not cost what it costs a steady miner: \
                 {cap_cost:+.2} pp at spread {sigma}, timescale {t_c_secs}s"
            );
        }
    }

    eprintln!(
        "  [wander box] worst sparse-vs-dense gap {worst_gap:+.2} pp, worst sparse-cap cost \
         {worst_cap_cost:+.2} pp, shipped over the {DECLINE_SAFETY_GATE_PCT:.0}% gate in \
         {shipped_over_gate} of 9 cells"
    );

    // Recorded, not tolerated. The shipped controller failing the gate somewhere in the plausible box is
    // the most consequential thing this sweep says, and pinning the count makes a change to it legible.
    assert_eq!(
        shipped_over_gate, 3,
        "the shipped controller exceeded the {DECLINE_SAFETY_GATE_PCT:.0}% gate in {shipped_over_gate} \
         of the 9 box cells, not the 3 recorded. Fewer means the configuration or the box improved and \
         this should be re-read; more means it got worse. Either way it is not a number to let drift"
    );
}
