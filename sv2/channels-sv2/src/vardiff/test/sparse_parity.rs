//! Where this controller and the validated champion part company, measured rather than argued.
//!
//! The configuration the simulation framework selected switches boundary at the seam: at or above it a
//! sequential test carrying the tighten multiplier and the persistence discount, below it a bare
//! symmetric Poisson interval carrying neither. This controller sizes the base by branch and then scales
//! it once, after the dispatch, so both apply at every rate. The two therefore agree above the seam and
//! disagree below it -- in the regime with the least evidence per decision, and the regime whose cells
//! select the estimator's memory for the whole band.
//!
//! That matters beyond tidiness, because the mechanism analysis recorded for the sparse regime was done
//! on the champion. Conclusions of the form "the boundary contributes no directional resistance below the
//! seam" are true of that artifact and false of this one, so they cannot be carried over to reason about
//! what ships.
//!
//! These tests do not decide which arm is right; that is a calibration question. They pin the two facts
//! needed before deciding: that the divergence is confined to the sparse branch, and which way it points.

use super::trajectory::{constant, declining, measure_arm, Arm};

const ONE_HOUR: u64 = 3600;

/// Above the seam the two arms are the same controller, and this is the control that says so.
///
/// It fails if the sparse flag ever leaks into the dense path, which would make every reading in the
/// companion test below a comparison of two things that differ in more than one place. Equality is exact
/// rather than approximate: same seeds, same clock, and -- above the seam -- the same code, so any
/// difference at all is a defect rather than noise.
#[test]
fn the_two_arms_are_the_same_controller_above_the_seam() {
    const TRIALS: usize = 9;

    for spm in [6.0f32, 12.0, 30.0] {
        for (name, delivered) in [
            ("constant", &constant() as &dyn Fn(u64) -> f64),
            ("declining 10%/hr", &declining(10.0, 0.40)),
        ] {
            let at = |arm| measure_arm(&delivered, spm, ONE_HOUR, 3 * ONE_HOUR, TRIALS, arm);
            let shipped = at(Arm::Shipped);
            let champion = at(Arm::ChampionSparse);

            assert_eq!(
                shipped.final_error_pct, champion.final_error_pct,
                "at {spm} spm ({name}) the arms must be identical, since the champion's switch is \
                 below the seam and cannot reach this cell"
            );
            assert_eq!(
                shipped.retargets, champion.retargets,
                "and must retarget identically at {spm} spm ({name})"
            );
            assert_eq!(
                shipped.final_threshold_pct, champion.final_threshold_pct,
                "and must use the same threshold at {spm} spm ({name})"
            );
        }
    }
}

/// Below the seam they are different controllers, and the shipped one is the more eager easer.
///
/// The direction follows from which mechanisms reach the sparse branch here and not there. The discount
/// scales the *whole* threshold, so on the ease side this controller's bar is up to `0.6` lower than the
/// champion's and it eases on less evidence; the multiplier scales only the tighten side, so its tighten
/// bar is up to `8x` higher and it tightens on much more. Both push the settled position the same way,
/// down, and under-difficulty is the safe side for payout but multiplies the miner's submission load.
///
/// Stated as a signed gap rather than as two absolute numbers, because the absolute settled position is
/// already pinned by `the_absolute_settled_error_is_held_not_merely_the_relative_one`. What this adds is
/// that the gap is real, has a sign, and is confined to where the branches differ.
#[test]
fn the_shipped_sparse_arm_settles_below_the_champion_and_eases_more() {
    const TRIALS: usize = 40;
    /// The smallest settled gap that counts as the arms being materially different controllers. Below
    /// this the divergence would be cosmetic and could be left alone.
    ///
    /// From measurement rather than choice: the gaps are `+7.08`, `+15.63`, `+12.12` and `+9.84` pp, so
    /// `4.0` binds the narrowest cell at `1.8x`. Verified to bind by mutation -- `8.0` fails on the
    /// `2 spm constant` cell.
    const MIN_GAP_PCT: f64 = 4.0;

    let mut worst_gap = f64::MAX;
    for spm in [2.0f32, 4.0] {
        for (name, delivered) in [
            ("constant", &constant() as &dyn Fn(u64) -> f64),
            ("declining 5%/hr losing 20%", &declining(5.0, 0.20)),
        ] {
            let at = |arm| measure_arm(&delivered, spm, ONE_HOUR, 3 * ONE_HOUR, TRIALS, arm);
            let shipped = at(Arm::Shipped);
            let champion = at(Arm::ChampionSparse);
            let gap = champion.final_error_pct - shipped.final_error_pct;

            eprintln!(
                "  [sparse parity] {spm} spm {name}: shipped settles {:+.2}% with {} retargets \
                 (threshold {:.1}%), champion settles {:+.2}% with {} retargets (threshold {:.1}%); \
                 gap {gap:+.2} pp",
                shipped.final_error_pct,
                shipped.retargets,
                shipped.final_threshold_pct,
                champion.final_error_pct,
                champion.retargets,
                champion.final_threshold_pct,
            );
            worst_gap = worst_gap.min(gap);
        }
    }

    assert!(
        worst_gap > MIN_GAP_PCT,
        "the shipped sparse arm should settle at least {MIN_GAP_PCT:.1} pp below the champion's on \
         every sparse cell, since both of the mechanisms it adds there push the same way; the \
         narrowest gap measured was {worst_gap:+.2} pp. A gap that has closed means the branches have \
         converged and this test and its flag can go; a gap that has changed sign means one of the two \
         mechanisms stopped reaching the sparse branch"
    );
}
