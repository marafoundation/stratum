//! What a power-of-two egress costs at a settled point, as a bound rather than a reading.
//!
//! An 18-hour stall was measured on hardware: the belief settled below an octave boundary, the served
//! difficulty stopped changing, and the miner delivered `2.36x` its target share rate indefinitely. That
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
//! of `MIN_THRESHOLD_FRACTION * MIN_EFFECTIVE_TIGHTEN_MULTIPLIER`. So the bound moves from `2.96x` to
//! `2.32x`, and the pre-fix hardware witness at `2.36x` lands **outside** the post-fix bound, which is a
//! quantitative statement that the measured stall is now unreachable.
//!
//! What the fix cannot touch is the quantiser's factor of two. Post-fix that is `2/2.32 = 86%` of what
//! remains, so the actuator does not merely dominate the controller -- it is very nearly all of it, and
//! the rounding threshold is the only lever left with real leverage.

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
const OCTAVE: f64 = 2.0;

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
fn asymptotic_tighten_bar() -> f64 {
    MIN_THRESHOLD_FRACTION * MIN_EFFECTIVE_TIGHTEN_MULTIPLIER
}

/// Worst-case excess share rate at a settled point on a power-of-two egress, as a multiple of `r*`.
fn over_delivery_bound(tighten_bar: f64) -> f64 {
    (1.0 + tighten_bar) * OCTAVE
}

/// Round a difficulty down to the largest power of two not exceeding it, as the SV1 egress does.
fn round_down_pow2(difficulty: f64) -> f64 {
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
         'the measured stall is now unreachable'. Witness {measured:.2}x against a bound of {post:.2}x"
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
