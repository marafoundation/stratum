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
//! A second reason was recorded here and is now **withdrawn entirely**. Capping the sparse window appeared
//! to cost `+3.27 pp` of settled over-difficulty, which read as the seam gate being load-bearing. Both that
//! reading and the later "steady-miner only" narrowing were taken against a share feed that starved the
//! sparse cells; repaired, the cap changes about one sparse trajectory in 25 and moves the median by a
//! fraction of a point, because the inter-fire window seldom reaches the matched cap. The rejection rests
//! on the crossing alone, which is unaffected.
//!
//! What survives is the residue: the shipped form, never looser than the matched window. It keeps the
//! gate, so it cannot reintroduce the sparse failure, and it is a no-op above 8 shares a minute where the
//! constant is already the tighter of the two. It is confined to the band the two constants disagree
//! over, which is the only place either of them is wrong.
//!
//! [`SPARSE_SPM_SEAM`]: crate::vardiff::classic::SPARSE_SPM_SEAM
//! [`WINDOW_CAP_SECS`]: crate::vardiff::classic::WINDOW_CAP_SECS

use super::trajectory::{changed_against_shipped, constant, declining, measure_arm, Arm, Trial};
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

/// Trials for the stable-channel retarget counts.
///
/// The count is the median trial's, an integer over 12 h, and at 6 spm the two forms differ by three or four
/// retargets. At 9 trials one of twelve independent seed sets ties them. At this count all twelve separate
/// them, shipped `2.42-2.50` an hour against matched `2.17-2.25`, and at 12 spm shipped `0.83-0.92`
/// against matched `1.08-1.17`.
const STABLE_TRIALS: usize = 180;

fn stable_at(spm: f32, arm: Arm) -> Trial {
    measure_arm(
        &constant(),
        spm,
        ONE_HOUR,
        MEASURED_HOURS * ONE_HOUR,
        STABLE_TRIALS,
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

/// **Capping the sparse window changes few trajectories, because the window seldom reaches the cap.**
///
/// This recorded `+3.27 pp` of settled over-difficulty from capping a sparse window, which read as the seam
/// gate being load-bearing, and a later pass narrowed it to "steady-miner only" after wander appeared to
/// erase it. Both readings were taken against a share feed that suppressed zero-share ticks and, below one
/// arrival a tick, delivered nothing at all -- so the sparse cells were being starved rather than capped.
///
/// Against the repaired feed the controller fires often enough that the inter-fire window seldom approaches
/// the `sigma`-matched sparse cap of about 8600 s. Over [`CAP_TRIALS`] on the 5 %/hr decline the cap changes
/// 19 trajectories at 2 spm and 1 at 4; on the 40 %/hr decline, 4 and none. Twelve independent samples of
/// that many trials put the 2 spm counts at `9-19` and `1-7`, and the 4 spm counts at `0-2` and always
/// none. So the median moves by a fraction of a point (`+0.06 pp` and `+0.13 pp` at 2 spm, `-0.07` to
/// `+0.32` across the samples), and a median over a few dozen trials reads zero or not according to where
/// the changed trajectories fall.
///
/// **Where the cap does act, it acts by several points**, mostly toward over-difficulty: the changed
/// trajectories settle a median `+6.79 pp` higher at 5 %/hr, 12 of 19 of them higher, and `+10.64 pp` at
/// 40 %/hr, all four higher. The cap is **rarely reached, not harmless** -- and the rate at which it is
/// reached rises with anything that lengthens the sparse inter-fire interval.
///
/// The research branch's `+29.9%` figure for a 900 s sparse cap is not contradicted by this: 900 s is an
/// order of magnitude tighter than the matched value and would be reached routinely. What is withdrawn is
/// the claim about the *matched* cap.
#[test]
fn the_sparse_window_cap_is_reached_rarely_rather_than_never() {
    /// Enough that a cap reached on one trajectory in 25 is reached about 20 times.
    const CAP_TRIALS: usize = 480;

    let mut reached = 0;
    for (label, rate_pct_per_hr, drop) in [
        ("5 %/hr losing 20%", 5.0, 0.20),
        ("40 %/hr losing 50%", 40.0, 0.50),
    ] {
        for spm in [2.0f32, 4.0] {
            let (changed, cost) = changed_against_shipped(
                &declining(rate_pct_per_hr, drop),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                CAP_TRIALS,
                Arm::SigmaMatchedCap,
                None,
            );
            reached += changed;

            eprintln!(
                "  [cap form] {spm} spm {label}: the matched cap changes {changed} of {CAP_TRIALS} \
                 trajectories and moves the median {cost:+.2} pp"
            );
            assert!(
                changed * 10 < CAP_TRIALS,
                "the sigma-matched sparse cap should change few trajectories at {spm} spm ({label}): \
                 {changed} of {CAP_TRIALS}. More means the inter-fire window routinely reaches ~8600 s, and \
                 the question becomes whether the cap helps or strands a decline, which this test does not \
                 answer"
            );
        }
    }
    assert!(
        reached > 0,
        "and should change some: none of the trajectories reached it, which would restore the claim this \
         test replaced, that the window never gets there"
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
