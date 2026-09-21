//! Three window-cap forms, measured against each other, and why two of them are not adopted.
//!
//! `relations.rs` records that [`SPARSE_SPM_SEAM`] and [`WINDOW_CAP_SECS`] answer one question with two
//! answers: the cap is `sigma`-matched at 8.04 shares a minute, the seam sits at 6.00, and between them
//! the dense branch runs with a bar its own wobble clears for part of every cap cycle. The obvious
//! repair is to replace the constant and the gate with one rate-aware rule -- cap the window where the
//! bar reaches the estimator's spread, at every rate.
//!
//! **Measured, that repair is worse than what it replaces**, and it is rejected below rather than argued
//! against. The reason that holds under every condition tested: it is *looser* than 900 s above about 8
//! shares a minute, because the dense bar decays as `1/window` while `sigma` falls only as `1/sqrt(r)` --
//! so it costs exactly the retargets the cap exists to prevent.
//!
//! A second reason was recorded here and **has since been narrowed**. Capping the sparse window costs
//! settled over-difficulty on a slow decline, which read as the seam gate being load-bearing; measured
//! again with the miner's own rate wandering, that cost disappears (`wander.rs`). So it is a property of
//! a perfectly steady miner, it is scoped that way below, and the rejection rests on the first reason.
//!
//! What survives is the residue: the shipped form, never looser than the matched window. It keeps the
//! gate, so it cannot reintroduce the sparse failure, and it is a no-op above 8 shares a minute where the
//! constant is already the tighter of the two. It is confined to the band the two constants disagree
//! over, which is the only place either of them is wrong.
//!
//! [`SPARSE_SPM_SEAM`]: crate::vardiff::classic::SPARSE_SPM_SEAM
//! [`WINDOW_CAP_SECS`]: crate::vardiff::classic::WINDOW_CAP_SECS

use super::trajectory::{constant, declining, measure_arm, Arm, Trial};
use crate::vardiff::classic::VardiffState;

const ONE_HOUR: u64 = 3600;
const MEASURED_HOURS: u64 = 12;

/// The windows the `sigma`-matched rule asks for, against the 900 s the shipped form applies.
///
/// Every reading below is a consequence of these, and a change to the floor, the evidence term or tau
/// moves them all — so they are pinned here rather than left implicit.
#[test]
fn the_matched_window_crosses_the_shipped_constant_at_eight_shares_a_minute() {
    let at = |spm: f32| VardiffState::sigma_matched_window_secs(spm);

    // Below the seam the count-based bar decays slowly, so the matched window is hours. That is why a
    // 900 s cap stranded declining sparse channels: it is an order of magnitude too tight there.
    assert_eq!(at(2.0), Some(8600), "2 spm");
    assert_eq!(at(4.0), Some(11383), "4 spm");
    // Above it the bar decays as 1/window, so the matched window is minutes — and it crosses 900 s at
    // about 8, which is the whole reason the rule is not uniformly tighter than the constant.
    assert_eq!(at(6.0), Some(594), "6 spm, tighter than the constant");
    assert_eq!(at(8.0), Some(895), "8 spm, within a tick of it");
    assert_eq!(at(12.0), Some(1713), "12 spm, looser than it");
    // And where the floor alone already exceeds the estimator's spread there is nothing to protect.
    assert_eq!(
        at(60.0),
        None,
        "60 spm: the 5% floor is above sigma, so a cap could only cost reaction time"
    );
}

fn retargets_per_hour(trial: &Trial) -> f64 {
    trial.retargets as f64 / MEASURED_HOURS as f64
}

fn stable_at(spm: f32, arm: Arm) -> Trial {
    measure_arm(
        &constant(),
        spm,
        ONE_HOUR,
        MEASURED_HOURS * ONE_HOUR,
        9,
        arm,
    )
}

/// **The `sigma`-matched rule is not uniformly tighter, and above the crossing it costs retargets.**
///
/// The first of the two reasons it is rejected. At 6 shares a minute its window is 594 s against 900 and
/// it does better than the shipped form; at 12 its window is 1713 s and it does worse. A rule that is
/// looser than the constant it replaces, in the band where most channels run, is not a repair.
///
/// Asserted in both directions, because the finding is the *sign change* rather than either reading: a
/// version of this that only measured 6 spm would have concluded the rule is an improvement.
#[test]
fn the_matched_rule_helps_below_the_crossing_and_hurts_above_it() {
    let below = (
        stable_at(6.0, Arm::Shipped),
        stable_at(6.0, Arm::SigmaMatchedCap),
    );
    let above = (
        stable_at(12.0, Arm::Shipped),
        stable_at(12.0, Arm::SigmaMatchedCap),
    );
    for (spm, (shipped, matched)) in [(6.0, &below), (12.0, &above)] {
        eprintln!(
            "  [cap form] {spm} spm stable: shipped {:.2} retargets/hr at {:+.2}% median, matched \
             {:.2} at {:+.2}%",
            retargets_per_hour(shipped),
            shipped.median_error_pct,
            retargets_per_hour(matched),
            matched.median_error_pct
        );
    }

    assert!(
        retargets_per_hour(&below.1) < retargets_per_hour(&below.0),
        "at 6 spm the matched window is tighter than the constant, so it should retarget a stable \
         channel less often; got {:.2} against {:.2}",
        retargets_per_hour(&below.1),
        retargets_per_hour(&below.0)
    );
    assert!(
        retargets_per_hour(&above.1) > retargets_per_hour(&above.0),
        "at 12 spm the matched window is LOOSER than the constant, so it should retarget more often — \
         measured {:.2} against {:.2}. If this has reversed, the crossing has moved and the rule may \
         now be uniformly tighter, which would reopen the case for adopting it wholesale",
        retargets_per_hour(&above.1),
        retargets_per_hour(&above.0)
    );
}

/// **Against a perfectly steady miner, capping the sparse window costs a slow decline at any cap.**
///
/// The research branch measured +29.9% settled over-difficulty from capping sparse windows at 900 s.
/// Moving the cap out by an order of magnitude, to the 8600 s the estimator's spread asks for, does not
/// remove the cost -- it attenuates it to **+3.27 pp** on the slow cell. The cell it hurts is the telling
/// part: a 40 %/hr decline crosses any bar quickly and is unaffected, while the 5 %/hr decline, the one
/// that needs the bar to keep falling over a long window, pays all of it.
///
/// **Scoped to a steady miner deliberately, and that scope is the correction.** This was first recorded
/// as the seam gate being load-bearing. Re-measured with the miner's own rate wandering at the worst
/// values measured on hardware, the cost is -0.31 pp -- gone, because the shipped arm has itself degraded
/// by more than the cap's effect. See `the_caps_sparse_cost_is_wander_free_but_its_crossing_cost_is_not`.
/// The reading below is still worth keeping, because it is the one that isolates the cap's own effect from
/// the disturbance; it is just not evidence about a deployment.
#[test]
fn capping_the_sparse_window_costs_a_slow_decline_without_wander() {
    const TRIALS: usize = 40;
    /// What the slow cell pays, measured. Held as a range: a cost that has vanished means the sparse
    /// branch changed and the seam gate may no longer be needed, which is worth failing over.
    const SLOW_CELL_COST_PCT: (f64, f64) = (2.5, 4.0);

    let mut slow_cost = f64::NAN;
    for spm in [2.0f32, 4.0] {
        for (name, rate_pct_per_hr, drop) in [
            ("5 %/hr losing 20%", 5.0, 0.20),
            ("40 %/hr losing 50%", 40.0, 0.50),
        ] {
            let at = |arm| {
                measure_arm(
                    &declining(rate_pct_per_hr, drop),
                    spm,
                    ONE_HOUR,
                    3 * ONE_HOUR,
                    TRIALS,
                    arm,
                )
            };
            let shipped = at(Arm::Shipped);
            let matched = at(Arm::SigmaMatchedCap);
            let added = matched.final_error_pct - shipped.final_error_pct;

            eprintln!(
                "  [cap form] {spm} spm {name}: shipped settles {:+.2}%, matched {:+.2}% ({added:+.2} pp)",
                shipped.final_error_pct, matched.final_error_pct
            );
            assert!(
                added >= -0.01,
                "capping the sparse window should never *help* a decline; at {spm} spm ({name}) it \
                 moved settled error {added:+.2} pp, which contradicts the mechanism the seam gate is \
                 justified by and means this test is measuring something else"
            );
            if spm == 2.0 && rate_pct_per_hr == 5.0 {
                slow_cost = added;
            }
        }
    }

    let (lo, hi) = SLOW_CELL_COST_PCT;
    assert!(
        slow_cost > lo && slow_cost < hi,
        "the slow sparse cell paid {slow_cost:+.2} pp for a capped window, outside the measured \
         {lo:.1}-{hi:.1} pp. Below the range, capping sparse has stopped costing anything and the seam \
         gate may be removable; above it, the cost has grown and the 8600 s cap is further from safe \
         than recorded"
    );
}

/// **What survives: the shipped form, never looser than the matched window.**
///
/// Keeps the gate, so it cannot reintroduce the sparse cost above. Is a no-op at and above 8 shares a
/// minute, where the constant is already the tighter of the two. So it is confined to the band between
/// the seam and the crossing — the only place `relations.rs`'s disagreement has a consequence — and there
/// it buys the 6 spm improvement the rejected rule's one good reading came from.
///
/// This is the form worth proposing. It is measured here, not adopted: it is still a calibration change,
/// trading reaction time on 6-8 spm channels for fewer retargets on them.
#[test]
fn the_tighter_of_both_caps_keeps_the_gain_without_either_cost() {
    const TRIALS: usize = 40;

    // The band it is confined to. A no-op reads as exact equality, since the same seeds drive both arms
    // through the same code.
    for spm in [8.0f32, 12.0, 30.0] {
        let shipped = stable_at(spm, Arm::Shipped);
        let tighter = stable_at(spm, Arm::TighterOfBothCaps);
        assert_eq!(
            shipped.retargets, tighter.retargets,
            "at {spm} spm the constant is already the tighter of the two, so this form must be a \
             no-op; it retargeted {} against {}",
            tighter.retargets, shipped.retargets
        );
    }

    // And in the band, the gain.
    let shipped = stable_at(6.0, Arm::Shipped);
    let tighter = stable_at(6.0, Arm::TighterOfBothCaps);
    eprintln!(
        "  [cap form] 6 spm stable: shipped {:.2} retargets/hr at {:+.2}% median, tighter-of-both \
         {:.2} at {:+.2}%",
        retargets_per_hour(&shipped),
        shipped.median_error_pct,
        retargets_per_hour(&tighter),
        tighter.median_error_pct
    );
    assert!(
        retargets_per_hour(&tighter) < retargets_per_hour(&shipped),
        "at 6 spm this form should retarget a stable channel less often than the shipped cap; got \
         {:.2} against {:.2}",
        retargets_per_hour(&tighter),
        retargets_per_hour(&shipped)
    );

    // Sparse is untouched, because the gate is kept. Exact equality again.
    for spm in [2.0f32, 4.0] {
        let at = |arm| {
            measure_arm(
                &declining(5.0, 0.20),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                TRIALS,
                arm,
            )
        };
        assert_eq!(
            at(Arm::Shipped).final_error_pct,
            at(Arm::TighterOfBothCaps).final_error_pct,
            "at {spm} spm this form keeps the seam gate, so a declining sparse channel must be \
             completely unaffected"
        );
    }
}
