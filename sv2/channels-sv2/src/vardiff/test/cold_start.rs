//! What happens when the channel opens at the wrong difficulty, which nothing else here measured.
//!
//! Every other trajectory in this suite opens on target and disturbs the miner afterwards. A real channel
//! opens at a difficulty derived from a *declared* `nominal_hash_rate`, and that declaration can be wrong
//! in either direction by orders of magnitude -- it is miner-supplied and uncharacterized. This is also
//! the only regime that exercises [`MAX_STEP_RATIO`]'s tightening arm and the only one
//! `MIN_EVALUATION_INTERVAL_SECS` exists to rate limit, so leaving it unmeasured left two guards
//! unexercised.
//!
//! **It found a lock.** A channel that over-declares by `100x` at 2 shares a minute settles at `11x`
//! over-difficulty and stays there -- identical readings at 6, 12, 24 and 48 hours, with the same six
//! retargets in all four, so it is stopped rather than slow. The same over-declaration at 12 shares a
//! minute recovers in 77 ticks. That is the death-spiral shape the design is meant to bound, arriving
//! through the guard that exists to bound it.
//!
//! [`MAX_STEP_RATIO`]: crate::vardiff::classic
//! [`MIN_EVALUATION_INTERVAL_SECS`]: crate::vardiff::classic

use super::trajectory::{measure_arm, mis_declared, Arm, Trial, START_HASHRATE};

const HOUR_TICKS: u64 = 3600;
const TRIALS: usize = 20;
/// The bound the silence floor applies, as a ratio below the last evidenced belief. Mirrors
/// `MAX_SILENT_DISPLACEMENT`, which is private.
const EXPECTED_DISPLACEMENT: f64 = 9.0;

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

/// **A channel that over-declares by 100x at a sparse rate locks at the displacement bound.**
///
/// The failure, stated as the three readings that establish it is a lock and not slowness.
///
/// The mechanism is legible from where it stops. `MAX_SILENT_DISPLACEMENT` floors an ease at
/// `evidenced_hashrate / 9`, and `evidenced_hashrate` only re-anchors on an evaluation that saw a share.
/// A channel set 100x too hard delivers `spm/100` -- one share per fifty minutes at 2 spm -- so almost
/// every evaluation sees none, the anchor stays at the *opening* belief, and the floor stays at one ninth
/// of a number that was never evidence for anything. The belief comes to rest exactly there: measured
/// `1.111e11` against `START_HASHRATE / 9 = 1.111e11`.
///
/// So the guard that bounds how far a *departed* miner's difficulty may be eased also bounds how far an
/// *over-declared* one's may be corrected, because at a sparse rate the controller cannot tell them
/// apart. The 9x bound is a bounded-regret policy against a returning miner's share flood; this is the
/// other side of that bet, and it is unbounded in duration.
#[test]
fn an_over_declared_sparse_channel_locks_at_the_displacement_bound() {
    let short = from_declaration(0.01, 2.0, 6);
    let long = from_declaration(0.01, 2.0, 48);

    eprintln!(
        "  [cold start] 100x over-declared at 2 spm: 6 h ends {:+.2}% ({:.3e} H/s, {} retargets), 48 h \
         ends {:+.2}% ({:.3e} H/s, {} retargets); START/9 is {:.3e}",
        short.final_error_pct,
        short.final_hashrate,
        short.retargets,
        long.final_error_pct,
        long.final_hashrate,
        long.retargets,
        START_HASHRATE as f64 / EXPECTED_DISPLACEMENT,
    );

    // It is a lock: eight times the observation buys nothing at all.
    assert_eq!(
        short.retargets, long.retargets,
        "a slow recovery would keep retargeting over 48 h; {} against {} in 6 h means it has stopped",
        long.retargets, short.retargets
    );
    assert!(
        (long.final_error_pct - short.final_error_pct).abs() < 0.01,
        "and would keep moving: 6 h ends {:+.2}%, 48 h ends {:+.2}%",
        short.final_error_pct,
        long.final_error_pct
    );
    // And it is the silence floor specifically, because that is exactly where it stops.
    let floor = START_HASHRATE as f64 / EXPECTED_DISPLACEMENT;
    assert!(
        (long.final_hashrate as f64 / floor - 1.0).abs() < 0.01,
        "it should come to rest on the silence floor computed from the opening belief, {floor:.4e}; it \
         rested at {:.4e}. If this has moved, the lock has a different cause than the one recorded \
         above and the mechanism needs re-tracing rather than re-asserting",
        long.final_hashrate
    );
    assert!(
        long.final_error_pct > 200.0,
        "and the resting place should be far outside anything the controller would tolerate on a \
         settled channel: {:+.2}%",
        long.final_error_pct
    );
}

/// **The same over-declaration at a dense rate recovers, which is what identifies the cause.**
///
/// The control for the test above. Nothing differs but the configured rate, and therefore how often an
/// evaluation sees no shares: at the locked belief a sparse channel delivers 0.18 shares a minute and a
/// dense one 1.08, so re-anchoring opportunities are some two and a half times more frequent. The dense
/// channel escapes; the sparse one does not.
///
/// Without this reading the lock could be blamed on the 100x magnitude rather than on the rate, and the
/// fix would be aimed at the wrong guard.
#[test]
fn the_same_over_declaration_recovers_at_a_dense_rate() {
    let dense = from_declaration(0.01, 12.0, 24);
    let ticks = dense.ticks_to_converge.filter(|t| *t != usize::MAX);
    eprintln!(
        "  [cold start] 100x over-declared at 12 spm: within 10% after {ticks:?} ticks, {} retargets, \
         ends {:+.2}%",
        dense.retargets, dense.final_error_pct
    );

    assert!(
        ticks.is_some_and(|t| t < 200),
        "a dense channel should recover from the same over-declaration; it reached 10% after {ticks:?} \
         ticks. If this has stopped recovering, the lock is not rate-dependent and the sparse test's \
         mechanism is wrong"
    );
    assert!(
        dense.final_error_pct.abs() < 20.0,
        "and should end near the truth rather than parked above it: {:+.2}%",
        dense.final_error_pct
    );
}

/// **Under-declaring is the safe direction, and converges at every rate.**
///
/// The other half of cold start, and the half that exercises the tightening arm of the step bound. A
/// channel set too *easy* delivers a flood, which is the one disturbance the controller sees clearly --
/// evidence arrives faster than it can be acted on, so there is no information problem, only a rate
/// limit.
///
/// Convergence is asserted on the retarget count rather than on the error, because the settled offset is
/// the dead zone's and is larger than the 10% this harness calls converged: the thresholds in force at a
/// settle point run 42-92%, so a `-13%` ending is the controller at rest and not a failure to arrive.
/// What matters is that the climb is geometric -- a 100x error should cost a handful of moves, not a
/// hundred.
#[test]
fn an_under_declared_channel_climbs_geometrically_at_every_rate() {
    /// A 100x error is 4.6 e-folds; a partial step closing a fraction of the remaining gap converges
    /// geometrically, so this should be single digits at any rate.
    const MAX_RETARGETS: usize = 15;

    for spm in [2.0f32, 12.0] {
        for factor in [10.0f64, 100.0] {
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
                trial.final_error_pct < 0.0 && trial.final_error_pct > -25.0,
                "and should end just inside the dead zone on the safe side, not parked outside it: \
                 {:+.2}%",
                trial.final_error_pct
            );
        }
    }
}
