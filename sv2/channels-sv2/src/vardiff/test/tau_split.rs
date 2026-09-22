//! Whether the estimator's memory is chosen by the rates most channels run at, and what the rest pay.
//!
//! [`EWMA_TAU_SECS`] is one constant for every channel, and `sigma = sqrt(30 / (r * tau))` says it is
//! half of the only product that buys precision -- the half this crate controls. So its value is a
//! trade, and these tests establish *where on the rate band the trade is struck*.
//!
//! The answer is that it is struck at the bottom. A sleepier estimator settles further into
//! over-difficulty on a sustained decline, because it is slower to believe the decline is real, and the
//! first cell to cross the safety gate as `tau` grows is the sparsest. One step sleepier is refused by
//! the 2-shares-a-minute cell while a 12-shares-a-minute channel would be comfortably inside the gate
//! *and* measurably steadier. **Dense channels run a memory chosen to keep sparse ones safe.**
//!
//! That is a statement about coupling, not a proposal. Splitting `tau` at the seam is two lines; whether
//! to is a calibration decision, and it is not free -- the `tau` that minimises over-difficulty is not
//! the `tau` that minimises wobble, and the two disagree in different directions at different rates.
//! What these tests remove is the need to argue about which cells are binding.
//!
//! [`EWMA_TAU_SECS`]: crate::vardiff::classic::EWMA_TAU_SECS

use super::trajectory::{constant, declining, measure_arm, Arm, Trial};

const ONE_HOUR: u64 = 3600;
const MEASURED_HOURS: u64 = 12;
/// The shipped memory, as an arm. Named so the comparisons read as "against what ships".
const SHIPPED: Arm = Arm::Shipped;
/// One step sleepier. Chosen because it is the first step at which the split matters: the sparse cell
/// crosses the gate here while the dense one does not.
const SLEEPIER: Arm = Arm::Tau(720);
/// Settled over-difficulty at which the corpus's decline-safety gate refuses a configuration. Not
/// enforced anywhere on this branch -- `the_absolute_settled_error_is_held_not_merely_the_relative_one`
/// records that the shipped controller does not meet it -- but it is the criterion `tau` was selected by,
/// so it is the right yardstick for asking which cell does the selecting.
const DECLINE_SAFETY_GATE_PCT: f64 = 5.0;

/// A sustained shallow decline, which is what a long memory is slow to believe.
fn declining_trial(spm: f32, arm: Arm) -> Trial {
    measure_arm(&declining(5.0, 0.20), spm, ONE_HOUR, 3 * ONE_HOUR, 40, arm)
}

fn stable_trial(spm: f32, arm: Arm) -> Trial {
    measure_arm(
        &constant(),
        spm,
        ONE_HOUR,
        MEASURED_HOURS * ONE_HOUR,
        9,
        arm,
    )
}

/// **A sleepier memory is refused by the sparse cells, and only by them.**
///
/// The decoupling claim, stated as the two readings that matter. At one step sleepier a 2-shares-a-minute
/// channel settles outside the gate and a 12-shares-a-minute channel settles well inside it, so the
/// constraint that fixes the shipped value is not a property of the channels most affected by it.
///
/// Both halves are asserted, because either alone proves nothing: sparse failing says only that the gate
/// binds somewhere, and dense passing says only that it does not bind there.
#[test]
#[ignore = "numbers measured against the broken share sampler; needs re-derivation -- see VARDIFF.md, Retraction"]
fn a_sleepier_memory_is_refused_by_the_sparse_cells_and_only_by_them() {
    let sparse = declining_trial(2.0, SLEEPIER);
    let dense = declining_trial(12.0, SLEEPIER);
    eprintln!(
        "  [tau] one step sleepier on a 5 %/hr decline: 2 spm settles {:+.2}%, 12 spm settles {:+.2}%, \
         against a {DECLINE_SAFETY_GATE_PCT:.0}% gate",
        sparse.final_error_pct, dense.final_error_pct
    );

    assert!(
        sparse.final_error_pct > DECLINE_SAFETY_GATE_PCT,
        "a sleepier memory should put the 2 spm cell outside the gate -- that is what makes it the cell \
         doing the selecting. It settled {:+.2}%, inside {DECLINE_SAFETY_GATE_PCT:.0}%, so either the \
         gate no longer binds at sparse or the sparse branch has changed and the coupling this file \
         describes is gone",
        sparse.final_error_pct
    );
    assert!(
        dense.final_error_pct < DECLINE_SAFETY_GATE_PCT,
        "and should leave the 12 spm cell inside it, which is what makes the constraint a sparse one \
         rather than a band-wide one; it settled {:+.2}%",
        dense.final_error_pct
    );
    // The gap is the quantity, not either endpoint: it is how much of the gate's margin the sparse cell
    // consumes that the dense cell does not.
    let gap = sparse.final_error_pct - dense.final_error_pct;
    assert!(
        gap > 3.0,
        "the two cells should be separated by more than 3 pp at the same memory, or 'the sparse cell is \
         what binds' is a statement about noise; measured {gap:+.2} pp"
    );
}

/// **And that refusal costs the dense channel real steadiness.**
///
/// The other half of the trade, so the coupling is not reported as a free lunch forgone. A longer memory
/// averages more shares, so the belief moves less: `sigma` falls as `1/sqrt(tau)`, predicting a
/// `sqrt(2)` improvement for one step, and the 95th-percentile tracking error is where a miner would
/// notice it.
///
/// Measured at 12 shares a minute, which is the cell the test above shows would be safe at this memory.
/// So this is not a hypothetical gain: it is available to that channel today and withheld by a
/// constraint measured elsewhere on the band.
#[test]
fn the_dense_channel_pays_for_that_refusal_in_wobble() {
    /// Least improvement in 95th-percentile error that counts. `sqrt(2)` is the prediction; this is set
    /// below the measured 1.34x so the assertion is about the effect existing, not its exact size.
    const MIN_IMPROVEMENT: f64 = 1.2;

    let shipped = stable_trial(12.0, SHIPPED);
    let sleepier = stable_trial(12.0, SLEEPIER);
    let improvement = shipped.p95_error_pct / sleepier.p95_error_pct;
    eprintln!(
        "  [tau] 12 spm stable: shipped p95 {:.2}% against sleepier {:.2}%, a {improvement:.2}x \
         improvement ({:.2} against {:.2} retargets an hour)",
        shipped.p95_error_pct,
        sleepier.p95_error_pct,
        shipped.retargets as f64 / MEASURED_HOURS as f64,
        sleepier.retargets as f64 / MEASURED_HOURS as f64,
    );

    assert!(
        improvement > MIN_IMPROVEMENT,
        "one step sleepier should measurably steady a dense channel, since sigma falls as 1/sqrt(tau); \
         p95 went {:.2}% -> {:.2}%, a {improvement:.2}x change against the {MIN_IMPROVEMENT}x this \
         asserts. Without this reading the coupling above is a constraint with no cost attached, and \
         there would be no reason to care which cell binds",
        shipped.p95_error_pct,
        sleepier.p95_error_pct
    );
}

/// **The mechanism, so the two readings above are not a coincidence of one memory.**
///
/// Settled over-difficulty should rise monotonically with the memory at a sparse rate: a longer memory is
/// slower to believe a decline, and a decline unbelieved is over-difficulty. If it did not, the readings
/// above would be two points on a curve with no shape and the choice of 720 would be doing the work.
#[test]
fn settled_over_difficulty_rises_with_the_memory_at_a_sparse_rate() {
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
            "a longer memory should settle further into over-difficulty on a decline: {short}s read \
             {shallower:+.2}% and {long}s read {deeper:+.2}%, which is the wrong way round. The \
             coupling this file reports rests on this being monotone"
        );
    }
}
