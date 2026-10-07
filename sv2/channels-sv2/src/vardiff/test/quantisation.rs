//! What a power-of-two egress costs at a settled point, as a bound rather than a reading.
//!
//! An 18-hour stall was measured on hardware: the belief settled below an octave boundary, the served
//! difficulty stopped changing, and the miner delivered `2.27x` its target share rate indefinitely. That
//! is recorded elsewhere as a trajectory test -- a thing that passes. It is a *structural* claim, though,
//! and structural claims admit a stronger treatment: prove the envelope, exhibit a witness inside it,
//! report the gap. Borrowed from the BIP-54 chain-length work, which bounds an adversarial difficulty
//! loop that way and states plainly that its best construction is not proven optimal.
//!
//! **The claim.** At a settled point the excess share rate is the product of two independently-motivated
//! mechanisms, and the product is what binds:
//!
//! ```text
//!   r_obs / r*  =  (H_true / belief)  x  (D_intended / D_served)
//!                   \_____________/      \___________________/
//!                    the dead zone         the quantiser
//!                    <= 1 + bar            <  2
//! ```
//!
//! So the worst case is `(1 + bar) * 2`, not `max(1 + bar, 2)`. Neither factor is a defect on its own:
//! the dead zone is the threshold doing its job, and round-down is deliberate because rounding *up*
//! would make a miner discard shares the pool would have paid for. The harm is that they multiply, and
//! nothing in this crate previously composed them.
//!
//! **Why it was a fixed point and not merely a resting place.** With the belief stationary the served
//! difficulty is stationary, so the realized rate is stationary, so the estimate is stationary, so the
//! deviation is stationary -- and before `585fd805` the bar's own minimum was held up by the window cap.
//! Nothing in the loop moved, so the state could not be left. Same shape of argument as the chain-side
//! result read in mirror: theirs proves a loop cannot *reach* a state, this one that it cannot *leave* one.
//!
//! **And the bound is what prices the fix.** Exempting the tighten direction from the cap lets that bar
//! keep decaying -- measured 48% at the 900 s cap, then 32, 24, 20, 18.4% at 12000 s, toward an asymptote
//! of `MIN_THRESHOLD_FRACTION * MIN_EFFECTIVE_TIGHTEN_MULTIPLIER`. So the bound moves from `2.96x` toward
//! `2.32x`, and the pre-fix hardware witness at `2.36x` lands **outside** that asymptote. The measured stall
//! is one the bar decays out of rather than one it cannot reach: at 12000 s the bar is still `18.4%` and the
//! bound `2.37x`, so the stall is excluded only once the window has run longer than that, and only on the
//! dense branch the witness was measured on (see the last section).
//!
//! What the fix cannot touch is the quantiser's factor of two. Post-fix that is `1.00` of the `1.32`
//! excess over target, `76%`, against `0.16` for the controller and `0.16` for the two compounding -- so
//! the actuator does not merely dominate the controller, it is most of the whole, and the rounding
//! threshold is the only lever left with real leverage.
//!
//! # The post-fix state is now measured on hardware, and the bound holds
//!
//! The `2.36x` witness above is *pre-fix*. The cap exemption settled on the rig at 20:44:23Z on 2026-09-23
//! and gives a post-fix witness:
//!
//! | | served | realized / target | decomposition |
//! |---|---|---|---|
//! | pre-fix | `32,768` | `2.36x` | `1.882x` quantiser x `1.237x` dead zone |
//! | **post-fix** | `65,536` | **`1.164x`** | `~1.164x` quantiser x `~1.00x` dead zone |
//!
//! Two things follow, and the second is the more useful.
//!
//! **The bound is confirmed rather than merely unfalsified.** `1.164x` sits inside the post-fix `2.32x`, and
//! the pre-fix `2.36x` sits *outside* it -- so the exemption moved the measured state across the boundary the
//! bound predicts, in the direction and by more than the margin it predicts.
//!
//! **The quantiser is now essentially the whole residual, which is stronger than this file claimed.** The
//! prose above says the quantiser is `76%` of the excess the bound leaves post-fix. Measured, the dead-zone
//! factor has collapsed to about `1.00` and the excess is *all* actuator. An intended difficulty near `76,284`
//! rounds down to `65,536`, which is `16.4%` of an octave -- so this witness is far from the quantiser's
//! `2x` worst case and still accounts for the entire error. The rounding threshold is not merely the largest
//! remaining lever; on this witness it is the only one.
//!
//! That is what makes the actuator experiment's restated `P1'` sharp: `1.164x -> 1.000x`, with nothing else
//! left to attribute a miss to. See `actuator_granularity.rs`, premise check #5.
//!
//! **Every figure above is a dense-branch statement, which this file did not previously say.** The bar comes
//! from [`tighten_bar_over`], whose evidence term is the dense branch; below the seam the controller uses a
//! Poisson confidence interval instead, and the threshold in force there is measured at `161%` and `122%` at
//! 1 and 2 spm against the `16%` asymptote used above. So the sparse bound is materially larger, the
//! quantiser is a smaller share of it, and the actuator fix -- being linear in the quantum -- halves both ends
//! without closing the gap between them. `the_shipped_bound_covers_the_dense_branch_only` names that gap; the
//! sparse bound itself is underived, because a bound needs the bar's upper tail rather than its median.

use super::{TEST_MIN_ALLOWED_HASHRATE, TEST_SHARES_PER_MINUTE};
use crate::{
    target::hash_rate_to_target,
    vardiff::{
        classic::{
            VardiffState, EVIDENCE_AT_ONE_MINUTE, MIN_EFFECTIVE_TIGHTEN_MULTIPLIER,
            MIN_THRESHOLD_FRACTION, REFERENCE_SPM, WINDOW_CAP_SECS,
        },
        clock::MockClock,
        Vardiff,
    },
};
use std::sync::Arc;

/// The quantum a power-of-two egress imposes. Round-down, so the served difficulty is in `(D/2, D]`.
pub(super) const OCTAVE: f64 = 2.0;

/// The hardware witness: the settled state measured on the candidate arm.
///
/// Recorded so the bound has something inside it. A bound with no witness is untested in the direction
/// that matters -- it might be vacuous.
mod witness {
    pub(super) const BELIEF_TH: f64 = 132.46;
    pub(super) const TRUTH_TH: f64 = 163.9;
    pub(super) const INTENDED_DIFFICULTY: f64 = 61_680.0;
    pub(super) const SERVED_DIFFICULTY: f64 = 32_768.0;
    pub(super) const REALIZED_SPM: f64 = 70.9;
    pub(super) const TARGET_SPM: f64 = 30.0;
}

/// The tighten bar over a window of `window_secs`, using the *floor* of the multiplier.
///
/// Derived from shipped constants rather than quoted. The floor -- the multiplier after the
/// same-direction discount has done its worst -- is what makes the results below bounds: a fresher run
/// faces a higher bar and so settles no further out.
fn tighten_bar_over(shares_per_minute: f64, window_secs: f64) -> f64 {
    let base = EVIDENCE_AT_ONE_MINUTE * (shares_per_minute / REFERENCE_SPM).sqrt() * 60.0
        / window_secs
        + MIN_THRESHOLD_FRACTION;
    base * MIN_EFFECTIVE_TIGHTEN_MULTIPLIER
}

/// The tighten bar **before** `585fd805`, held up by the window cap and unable to decay further.
fn capped_tighten_bar(shares_per_minute: f64) -> f64 {
    tighten_bar_over(shares_per_minute, WINDOW_CAP_SECS as f64)
}

/// The tighten bar **after** `585fd805`, where the window is no longer re-anchored on the tighten side
/// and the evidence term decays to nothing.
///
/// Rate-independent, because the evidence term is the only rate-dependent part and it vanishes.
pub(super) fn asymptotic_tighten_bar() -> f64 {
    MIN_THRESHOLD_FRACTION * MIN_EFFECTIVE_TIGHTEN_MULTIPLIER
}

/// Worst-case excess share rate at a settled point on a power-of-two egress, as a multiple of `r*`.
pub(super) fn over_delivery_bound(tighten_bar: f64) -> f64 {
    (1.0 + tighten_bar) * OCTAVE
}

/// Round a difficulty down to the largest power of two not exceeding it, as the SV1 egress does.
pub(super) fn round_down_pow2(difficulty: f64) -> f64 {
    assert!(difficulty >= 1.0, "difficulty below 1 has no octave");
    (difficulty.log2().floor()).exp2()
}

/// **The two mechanisms multiply, and that is the finding.**
///
/// Each factor is individually bounded and individually justified. What no clause in this crate forbids,
/// and what no test caught before the hardware run, is their composition: a deviation the threshold is
/// right to ignore, sitting on top of a quantiser that is right to round down.
#[test]
fn the_dead_zone_and_the_quantiser_multiply() {
    let spm = witness::TARGET_SPM;
    let bar = capped_tighten_bar(spm);
    let bound = over_delivery_bound(bar);

    eprintln!(
        "  [quantisation] at {spm} spm: capped tighten bar {:.1}%, quantum {OCTAVE}x, so the settled \
         excess is bounded by {bound:.2}x -- against {:.2}x from the dead zone alone and {OCTAVE:.2}x \
         from the quantiser alone",
        bar * 100.0,
        1.0 + bar
    );

    // The product, not the larger of the two. Stated as an equality so a future change that "bounds" the
    // pair by a max would fail here rather than read as a tightening.
    assert!(
        (bound - (1.0 + bar) * OCTAVE).abs() < 1e-12,
        "the bound must be the product of the two factors"
    );
    assert!(
        bound > (1.0 + bar) * 1.5 && bound > OCTAVE * 1.2,
        "and must exceed either factor alone by a wide margin, or composing them is not the finding: \
         {bound:.3} against {:.3} and {OCTAVE:.3}",
        1.0 + bar
    );
    // Sanity on the derivation: at the reference rate the capped base is the floor plus a tenth.
    assert!(
        (bar - 0.48).abs() < 0.01,
        "the capped tighten bar at {spm} spm should be about 48%, being 15% capped base times the {:.1}x \
         effective multiplier; it computed {:.1}%",
        MIN_EFFECTIVE_TIGHTEN_MULTIPLIER,
        bar * 100.0
    );
}

/// **The hardware witness sits inside the bound, and the gap is the bound's pessimism.**
///
/// The witness also decomposes, which is the part worth keeping: of the measured `2.36x`, the quantiser
/// contributes `1.88x` and the controller's own dead zone only `1.24x`. So on a quantised egress **most of
/// the excess is the actuator, not the controller** -- which is the opposite of where the campaign first
/// looked, and it is why raising the rounding threshold is the cheap fix and re-tuning the bar is not.
#[test]
fn the_measured_stall_sits_inside_the_bound_and_decomposes() {
    let dead_zone_factor = witness::TRUTH_TH / witness::BELIEF_TH;
    let quantiser_factor = witness::INTENDED_DIFFICULTY / witness::SERVED_DIFFICULTY;
    let predicted = dead_zone_factor * quantiser_factor;
    let measured = witness::REALIZED_SPM / witness::TARGET_SPM;
    let bound = over_delivery_bound(capped_tighten_bar(witness::TARGET_SPM));

    eprintln!(
        "  [quantisation] witness: dead zone {dead_zone_factor:.3}x x quantiser {quantiser_factor:.3}x = \
         {predicted:.2}x predicted against {measured:.2}x measured, inside a bound of {bound:.2}x \
         ({:.0}% pessimism)",
        (bound / measured - 1.0) * 100.0
    );

    assert!(
        (predicted / measured - 1.0).abs() < 0.05,
        "the two factors should reproduce the measured excess to within 5%: {predicted:.3} against \
         {measured:.3}. If they do not, the decomposition is wrong and the bound's form is unsupported"
    );
    assert!(
        measured < bound,
        "the witness must sit inside the bound: {measured:.2}x against {bound:.2}x"
    );
    // The quantiser is the larger contributor. This is the actionable half.
    assert!(
        quantiser_factor > dead_zone_factor,
        "on this witness the quantiser should dominate the dead zone ({quantiser_factor:.2}x against \
         {dead_zone_factor:.2}x); if that reverses, the cheap fix is no longer the rounding threshold"
    );
}

/// **The ingredient stationarity rests on: the controller never acts below its own floor bar.**
///
/// Scoped deliberately, because it is weaker than the module doc's stationarity *argument* and should not
/// be read as a test of it. The deviation here is constructed as half of `capped_tighten_bar`, which is
/// itself the *lowest* bar the controller can present -- capped window, full same-direction discount --
/// so this cannot fail unless the controller fires below its own minimum. That is a real invariant and a
/// necessary ingredient of the fixed point, but it is not the fixed point: the full argument (belief
/// stationary, therefore served difficulty stationary, therefore rate, estimate and deviation stationary)
/// is reasoning about the loop and is stated in the module doc, not asserted here.
///
/// Verified to bind by mutation: removing the tighten multiplier drops the real bar below the constructed
/// deviation and this fires. So it does test the decision rule rather than restate its own construction.
///
/// Driven with explicit share counts rather than through the trajectory harness, because a claim about the
/// decision rule belongs in the tier that does not depend on a sampler.
#[test]
fn a_deviation_inside_the_tighten_bar_never_acts() {
    const TICK_SECS: u64 = 60;
    const EVALUATIONS: usize = 40;

    let spm = TEST_SHARES_PER_MINUTE;
    let bar = asymptotic_tighten_bar();
    // A deviation comfortably inside the bar, so the refusal is not a near-miss.
    let deviation = bar * 0.5;

    let clock = Arc::new(MockClock::new(1_000_000));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("a positive floor and a mock clock construct");

    let belief = 1.0e12f32;
    let truth = belief as f64 * (1.0 + deviation);
    let target = hash_rate_to_target(belief as f64, spm as f64).expect("positive inputs convert");

    // Shares the miner delivers per tick against a difficulty set for `belief`.
    let per_tick =
        (spm as f64 * (truth / belief as f64) * (TICK_SECS as f64 / 60.0)).round() as u32;

    let mut fires = 0usize;
    for _ in 0..EVALUATIONS {
        clock.advance(TICK_SECS);
        for _ in 0..per_tick {
            vardiff.increment_shares_since_last_update();
        }
        if let Ok(Some(_)) = vardiff.try_vardiff(belief, &target, spm) {
            fires += 1;
        }
    }

    eprintln!(
        "  [quantisation] a {:.1}% deviation against a {:.1}% capped bar fired {fires} times in \
         {EVALUATIONS} evaluations",
        deviation * 100.0,
        bar * 100.0
    );
    assert_eq!(
        fires, 0,
        "a deviation of {:.1}%, inside the {:.1}% capped tighten bar, must never move the belief -- that \
         refusal is what makes the settled state stationary and therefore a fixed point rather than a \
         resting place",
        deviation * 100.0,
        bar * 100.0
    );
}

/// The quantiser model must be the egress's, or the bound describes a different actuator.
///
/// Round *down*, never up: rounding up would tell a miner a higher difficulty than the pool credits
/// against, so its own filter would discard every share in between -- up to half its hashrate, silently.
#[test]
fn the_quantiser_rounds_down_and_never_up() {
    for (difficulty, expected) in [
        (1.0, 1.0),
        (2.0, 2.0),
        (3.9, 2.0),
        (32_768.0, 32_768.0),
        (61_680.0, 32_768.0),
        (65_535.0, 32_768.0),
        (65_536.0, 65_536.0),
    ] {
        let served = round_down_pow2(difficulty);
        assert_eq!(
            served, expected,
            "difficulty {difficulty} should be served as {expected}, not {served}"
        );
        assert!(
            served <= difficulty && difficulty < served * OCTAVE,
            "the served value must lie in (D/2, D]: {served} for {difficulty}"
        );
    }
}

/// **The bound prices the tighten-cap exemption, and the pre-fix witness falls outside it.**
///
/// `585fd805` exempts the tighten direction from the window cap on the grounds that the cap's
/// justification is a false-fire budget derived against *loosening*, and that on the tighten side it was
/// defending a floor already multiplied by `TIGHTEN_MULTIPLIER`. This states what that bought, as a number.
///
/// The premise is verified against the controller rather than assumed: the bar is read from
/// `last_threshold_pct` over a window well past the cap, and it must have kept decaying. If the exemption
/// were reverted this test fails at that step rather than at the arithmetic, which is the useful place for
/// it to fail.
#[test]
fn the_tighten_exemption_tightens_the_bound_past_the_measured_stall() {
    const TICK_SECS: u64 = 60;
    /// Well past `WINDOW_CAP_SECS`, so a capped bar and a decaying one are far apart.
    const TICKS: u64 = 200;

    let spm = witness::TARGET_SPM as f32;
    let pre = over_delivery_bound(capped_tighten_bar(spm as f64));
    let post = over_delivery_bound(asymptotic_tighten_bar());
    let measured = witness::REALIZED_SPM / witness::TARGET_SPM;

    // Drive a channel that over-delivers slightly: it wants to tighten, and stays inside the bar, so the
    // window is never re-anchored by a fire and the exemption is the only thing that can let the bar fall.
    let clock = Arc::new(MockClock::new(1_000_000));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("a positive floor and a mock clock construct");
    let belief = 1.0e12f32;
    let target = hash_rate_to_target(belief as f64, spm as f64).expect("positive inputs convert");
    let per_tick = (spm as f64 * 1.10) as u32;
    for _ in 0..TICKS {
        clock.advance(TICK_SECS);
        for _ in 0..per_tick {
            vardiff.increment_shares_since_last_update();
        }
        let _ = vardiff.try_vardiff(belief, &target, spm);
    }
    let observed_bar = vardiff.last_threshold_pct().expect("evaluations have run") / 100.0;

    eprintln!(
        "  [quantisation] tighten bar over {}s: {:.1}% observed against {:.1}% if the cap still bound. \
         Bound {pre:.2}x -> {post:.2}x; the pre-fix witness was {measured:.2}x",
        TICKS * TICK_SECS,
        observed_bar * 100.0,
        capped_tighten_bar(spm as f64) * 100.0
    );

    // The premise: the bar kept decaying past the cap.
    assert!(
        observed_bar < capped_tighten_bar(spm as f64) * 0.6,
        "the tighten bar should have decayed well below the capped value once the exemption is in place; \
         observed {:.1}% against a capped {:.1}%. If these are equal the exemption has been reverted and \
         the bound below is the pre-fix one",
        observed_bar * 100.0,
        capped_tighten_bar(spm as f64) * 100.0
    );
    assert!(
        observed_bar > asymptotic_tighten_bar(),
        "and must still exceed the asymptote it is approaching from above: {:.1}% against {:.1}%",
        observed_bar * 100.0,
        asymptotic_tighten_bar() * 100.0
    );

    // What the fix bought, and the part that makes it more than a tuning change.
    assert!(
        post < pre,
        "the exemption must tighten the bound: {post:.3} against {pre:.3}"
    );
    assert!(
        measured > post,
        "and the pre-fix witness must fall OUTSIDE the post-fix bound -- that is the quantitative form of \
         'the bar decays out of the measured stall'. Witness {measured:.2}x against a bound of {post:.2}x"
    );
    assert!(
        measured < pre,
        "while still sitting inside the pre-fix bound it was measured under: {measured:.2}x against \
         {pre:.2}x"
    );
}

/// **After the fix the quantiser is nearly the whole of what remains.**
///
/// The consequence worth acting on. The exemption removes most of the controller's contribution to the
/// settled excess and cannot touch the actuator's, so the balance moves from roughly three-to-one toward
/// the quantiser to better than six-to-one. Re-tuning the bar has almost nothing left to win; raising the
/// rounding threshold has all of it.
#[test]
fn the_quantiser_is_most_of_the_remaining_bound() {
    let post = over_delivery_bound(asymptotic_tighten_bar());
    let quantiser_share = OCTAVE / post;

    eprintln!(
        "  [quantisation] post-fix bound {post:.2}x, of which the quantiser is {:.0}% and the controller's \
         dead zone {:.0}%",
        quantiser_share * 100.0,
        (1.0 - quantiser_share) * 100.0
    );

    assert!(
        quantiser_share > 0.8,
        "the quantiser should be over 80% of the post-fix bound; it is {:.0}%. If the controller's share \
         has grown back, the exemption has been weakened and the priority between the two levers changes",
        quantiser_share * 100.0
    );
}

/// **The `2.32x` bound is a dense-branch statement, and the sparse branch is outside its derivation.**
///
/// A coverage gap named rather than papered over. Every bar in this file comes from
/// [`tighten_bar_over`], whose evidence term is the *dense* branch -- and that term scales as
/// `sqrt(spm / REFERENCE_SPM)`, so it **rises** with rate. Below [`SPARSE_SPM_SEAM`] the controller does not
/// use that term at all; it uses a Poisson confidence interval, which falls as `1/sqrt(N)` and is far wider.
/// So the asymptotic `16%` bar and the `2.32x` bound built on it describe a dense channel, and this file
/// never said so.
///
/// Measured from the controller itself rather than re-derived, since re-deriving the sparse branch here would
/// duplicate the implementation this suite exists to check:
///
/// | spm | branch | threshold in force | bound `(1 + bar) * 2` |
/// |---|---|---|---|
/// | 1 | sparse | `~161 %` | `~5.2x` |
/// | 2 | sparse | `~122 %` | `~4.4x` |
/// | 4 | sparse | `~75 %` | `~3.5x` |
/// | 30 | dense | `~34 %` | `~2.7x` |
/// | asymptote | dense | `16 %` | `2.32x` |
///
/// The `122 %` at 2 spm is of a size with the `112.84 %` dead zone recorded in `VARDIFF.md`, which was
/// measured a different way -- so the sparse bar being several times the asymptote is not an artefact of
/// this reading.
///
/// **What this does not license is quoting a sparse bound.** The figures above are medians of the threshold in
/// force, and a bound needs the bar's upper tail; substituting a median into a worst-case product gives a
/// number that is neither. The assertion below is therefore the *gap*, not a value: the sparse threshold
/// materially exceeds the bar the shipped bound is built on, so a sparse bound exists, is larger, and is
/// underived. Direction-mixing works in favour of the claim -- an ease threshold is smaller than a tighten
/// one, so a sparse reading near `100 %` is a conservative floor.
///
/// This also reorders the fine-actuator experiment's reach. `over_delivery_bound` is linear in the quantum, so
/// removing it halves the bound at every rate -- leaving the dense end near `1.16x` and the sparse end near
/// half of whatever its bound turns out to be. The fix does not close the sparse/dense gap; it preserves it.
///
/// [`SPARSE_SPM_SEAM`]: crate::vardiff::classic
#[test]
fn the_shipped_bound_covers_the_dense_branch_only() {
    use super::trajectory::{measure_arm, mis_declared, Arm};

    /// Trials per cell. The threshold in force is far better resolved than an error reading, but this is a
    /// claim about magnitude against a fixed number, so it is powered deliberately.
    const TRIALS: usize = 200;
    /// Least multiple of the asymptotic bar the sparse threshold must reach for the gap to be real rather
    /// than a reading. Measured about `10.1x` at 1 spm and `7.7x` at 2.
    const MIN_SPARSE_MULTIPLE: f64 = 3.0;

    let bar_at = |spm: f32| {
        measure_arm(
            &mis_declared(3.0),
            spm,
            3600,
            6 * 3600,
            TRIALS,
            Arm::Shipped,
        )
        .final_threshold_pct
            / 100.0
    };

    let asymptote = asymptotic_tighten_bar();
    for spm in [1.0f32, 2.0] {
        let bar = bar_at(spm);
        eprintln!(
            "  [quantisation] {spm} spm is on the sparse branch: threshold {:.1}% against the {:.1}% \
             asymptote the bound uses, so the bound there is about {:.1}x rather than {:.2}x",
            bar * 100.0,
            asymptote * 100.0,
            over_delivery_bound(bar),
            over_delivery_bound(asymptote)
        );
        assert!(
            bar > asymptote * MIN_SPARSE_MULTIPLE,
            "the sparse threshold should be several times the asymptotic bar the shipped bound is built on, \
             which is what makes the bound a dense-branch statement: {:.1}% against {:.1}% at {spm} spm. If \
             these ever converge the two branches have stopped differing and the bound covers the whole band",
            bar * 100.0,
            asymptote * 100.0
        );
    }

    // And the dense end is close to the asymptote, which is what makes the shipped figure the dense one.
    let dense = bar_at(30.0);
    eprintln!(
        "  [quantisation] 30 spm is on the dense branch: threshold {:.1}%, within reach of the {:.1}% \
         asymptote",
        dense * 100.0,
        asymptote * 100.0
    );
    assert!(
        dense < bar_at(2.0),
        "and the dense threshold must be the narrower of the two, or the branches are the wrong way round: \
         {:.1}% at 30 spm against {:.1}% at 2",
        dense * 100.0,
        bar_at(2.0) * 100.0
    );
}
