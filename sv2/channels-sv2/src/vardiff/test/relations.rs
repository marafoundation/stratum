//! Constants that are not independent, and what they are worth when they disagree.
//!
//! The file next door carries nineteen constants, two of them derived from others so they cannot drift.
//! The rest are written down side by side as though each were a free choice. Several are not: they are
//! the same quantity expressed twice, or one quantity and a bound on it, and a reader tuning either
//! member of a pair has no way to tell from the pair itself.
//!
//! These tests state each relation in the one place it can be checked, from the constants themselves
//! rather than from copies. Three of the four pass because the relation holds. The fourth passes on a
//! *recorded disagreement*, which is the finding: two constants that must describe one rate describe
//! two rates 34% apart.

use crate::vardiff::classic::{
    VardiffState, EVIDENCE_AT_ONE_MINUTE, EWMA_TAU_SECS, MAX_DIRECTION_DISCOUNT,
    MIN_EFFECTIVE_TIGHTEN_MULTIPLIER, MIN_THRESHOLD_FRACTION, REFERENCE_SPM, SPARSE_SPM_SEAM,
    STEP_FRACTION_BASE, STEP_FRACTION_ENGAGING_THE_EASING_BOUND, STEP_FRACTION_GROWTH,
    STEP_FRACTION_MAX, TIGHTEN_MULTIPLIER, WINDOW_CAP_SECS,
};

/// The estimator's own relative spread, read from the controller rather than recomputed here.
///
/// A local copy would be a second implementation of the quantity every threshold in this file is
/// compared against, free to drift from the one the controller uses. Wrapped only to take the `f64`
/// these tests work in.
fn estimator_spread(spm: f64) -> f64 {
    VardiffState::estimator_spread(spm as f32)
}

/// **The closed form for `sigma` is an approximation, and this is where it holds.**
///
/// `1/sqrt(lambda * n_eff)` is exact; `sqrt(30 / (r * tau))` is its limit as the evaluation interval
/// shrinks against `tau`, because `n_eff -> 2*tau/dt` there. The corpus quotes the closed form
/// throughout and the pinned instrument computes the exact one, so the two must be shown to agree at the
/// cadence the quoting is done at -- and to diverge away from it, or "approximation" is an unsupported
/// word.
///
/// It also settles a disagreement. The corpus carries a second convention, `1/sqrt(r*tau/60)`, which is
/// this one without the factor of two from the effective window being `2*tau`; it reads 16.7% at 6 shares
/// a minute against the 11.8% the instrument computes. The instrument is what the measurements were made
/// with, so the factor of two is real and the other convention is the one to stop using.
#[test]
fn the_closed_form_for_sigma_holds_at_the_nominal_cadence() {
    let closed = |spm: f64| (30.0 / (spm * EWMA_TAU_SECS as f64)).sqrt();

    for spm in [2.0f64, 6.0, 12.0, 30.0] {
        let exact = VardiffState::estimator_spread_at(spm as f32, 60);
        let error = (exact / closed(spm) - 1.0).abs();
        // 0.115% at the nominal cadence, independent of the rate: both forms scale as 1/sqrt(r), so the
        // discrepancy is a function of dt/tau alone.
        assert!(
            error < 0.002,
            "at {spm} spm the exact spread is {exact:.6} against the closed form's {:.6}, off by              {:.3}% -- more than the 0.2% that makes quoting the closed form safe at this cadence",
            closed(spm),
            error * 100.0
        );
        // The convention that drops the factor of two is not a rounding difference.
        let without_the_two = 1.0 / (spm * EWMA_TAU_SECS as f64 / 60.0).sqrt();
        assert!(
            (without_the_two / exact - 1.0).abs() > 0.4,
            "the no-factor-of-two convention should read about sqrt(2) high; at {spm} spm it read              {without_the_two:.4} against {exact:.4}, which is close enough that the two conventions              cannot be told apart and this test cannot settle which is right"
        );
    }

    // And it stops holding once the interval is a real fraction of tau, which is why the exact form is
    // what the code carries.
    let long = VardiffState::estimator_spread_at(6.0, 900);
    assert!(
        (long / closed(6.0) - 1.0).abs() > 0.1,
        "at a 900 s interval the closed form should be visibly wrong ({long:.4} against {:.4}); if it          is not, the approximation is better than described and the exact form is unnecessary machinery",
        closed(6.0)
    );
}

/// The dense bar once the window has grown to the cap, as a fraction.
fn capped_dense_bar(spm: f64) -> f64 {
    EVIDENCE_AT_ONE_MINUTE * (spm / REFERENCE_SPM).sqrt() * 60.0 / WINDOW_CAP_SECS as f64
        + MIN_THRESHOLD_FRACTION
}

/// **The tighten asymmetry and the persistence discount are one setting, not two.**
///
/// The discount scales the whole threshold and applies to both directions, so a sustained
/// same-direction run erodes the extra evidence tightening requires. That makes the enforced multiplier
/// a range whose floor is [`MIN_EFFECTIVE_TIGHTEN_MULTIPLIER`], and the floor is the number that
/// governs: an asymmetry present only at a fresh run is not one.
///
/// A compile-time assertion next door already refuses an outright inversion. What this adds is the
/// margin, so that moving the ceiling toward the inversion point is visible rather than silent.
#[test]
fn the_discount_ceiling_and_the_tighten_multiplier_are_one_setting() {
    // Discount ceiling at which the asymmetry inverts: the run makes tightening *cheaper* than a
    // fresh loosening. Derived from the multiplier, which is the whole point -- it moves if it moves.
    let inversion_ceiling = 1.0 - 1.0 / TIGHTEN_MULTIPLIER;
    // How much of the way to inversion the shipped ceiling sits. Recorded so a change is legible.
    let consumed = MAX_DIRECTION_DISCOUNT / inversion_ceiling;

    assert!(
        (MIN_EFFECTIVE_TIGHTEN_MULTIPLIER - 3.2).abs() < 1e-9,
        "a sustained tightening run bottoms out at {MIN_EFFECTIVE_TIGHTEN_MULTIPLIER:.3}x the fresh \
         loosening bar, not the 3.2x recorded; TIGHTEN_MULTIPLIER or MAX_DIRECTION_DISCOUNT moved and \
         the other was not re-read"
    );
    assert!(
        consumed < 0.8,
        "the discount ceiling {MAX_DIRECTION_DISCOUNT} has consumed {:.0}% of the distance to the \
         inversion ceiling {inversion_ceiling:.3}, at which a sustained run would make tightening \
         cheaper than loosening. The compile-time assertion only catches crossing it; this catches \
         approaching it",
        consumed * 100.0
    );
}

/// **The easing arm of the step bound is gated by the step fraction, and only just reachable.**
///
/// A retarget moves `f` of the gap, so downward it cannot reach `h/ratio` unless `f > 1/ratio` however
/// far the estimate has collapsed. The bound and the step schedule are therefore not independent, and
/// this pins where the gate opens: a run of 8 against a clamp of 9.
///
/// Worth stating because the arm reads as symmetric with the tightening one and is not. A reader
/// reasoning about what bounds a collapsing belief needs to know that for the first seven retargets of
/// a run, the answer is not this bound.
#[test]
fn the_easing_arm_of_the_step_bound_opens_only_at_a_long_run() {
    // Matches `partial_retarget`'s schedule. Not the clamp: the question is which run first clears the
    // gate, and the clamp is what limits how far the run may go.
    let step_fraction_at = |run: u32| {
        (STEP_FRACTION_BASE + STEP_FRACTION_GROWTH * (run.saturating_sub(1)) as f32)
            .min(STEP_FRACTION_MAX)
    };
    let first_open = (1..=64)
        .find(|&run| step_fraction_at(run) > STEP_FRACTION_ENGAGING_THE_EASING_BOUND)
        .expect("the compile-time assertion guarantees some run clears the gate");

    assert_eq!(
        first_open, 8,
        "the easing arm of MAX_STEP_RATIO first becomes reachable at retarget run {first_open}, not \
         the 8 recorded; the step schedule or the bound moved"
    );
    assert!(
        step_fraction_at(first_open - 1) <= STEP_FRACTION_ENGAGING_THE_EASING_BOUND,
        "and must be unreachable at run {}, or the gate is not where this test says it is",
        first_open - 1
    );
}

/// **The dense bar has one sensitivity, written as two constants.**
///
/// It carries `EVIDENCE_AT_ONE_MINUTE * sqrt(r / REFERENCE_SPM)`, in which the two appear only through
/// `EVIDENCE_AT_ONE_MINUTE / sqrt(REFERENCE_SPM)`. So they cannot be tuned separately, and the trap is
/// specific: `REFERENCE_SPM` reads like a documentation constant -- the rate the requirement is quoted
/// at -- and moving it silently rescales how sensitive the controller is at every rate.
#[test]
fn the_dense_evidence_term_has_one_degree_of_freedom() {
    let sensitivity = |evidence: f64, reference: f64| evidence / reference.sqrt();
    let shipped = sensitivity(EVIDENCE_AT_ONE_MINUTE, REFERENCE_SPM);

    // Doubling the quoted requirement and quadrupling the rate it is quoted at is the same controller.
    let rescaled = sensitivity(EVIDENCE_AT_ONE_MINUTE * 2.0, REFERENCE_SPM * 4.0);
    assert!(
        (shipped - rescaled).abs() < 1e-12,
        "the two constants should enter only as evidence/sqrt(reference); {shipped} against \
         {rescaled} means the dense term has been rewritten and they are now separable"
    );
    assert!(
        (shipped - 0.273_861_278_752_583_1).abs() < 1e-9,
        "the shipped dense sensitivity is {shipped}, not the recorded 0.2739. That is the one number \
         behind both constants, so it moving is a calibration change however innocent either edit looked"
    );
}

/// **The seam and the window cap must describe one rate, and they describe two.**
///
/// Both answer the same question: below which share rate is the dense branch's bar untrustworthy?
/// [`SPARSE_SPM_SEAM`] answers it by switching to a count-based bar. [`WINDOW_CAP_SECS`] answers it by
/// holding the bar up so it cannot decay under the estimator's own spread. One rate, two constants, and
/// nothing making them agree -- so they do not: the cap is `sigma`-matched at **8.03** shares a minute
/// while the seam sits at **6.00**, and in the band between them the shipped controller runs the dense
/// branch with a bar that its own wobble clears for part of every cap cycle.
///
/// This test does not assert they agree, because they do not and closing the gap is a calibration
/// decision either way -- moving the seam up changes which channels get which controller, and shortening
/// the cap costs reaction time on every dense channel. It asserts the *size* of the disagreement, so it
/// is recorded, cannot drift unnoticed, and fails when someone closes it.
#[test]
fn the_seam_and_the_window_cap_disagree_about_the_same_rate() {
    // Rate at which the capped dense bar sits exactly at the estimator's spread. Solving
    // `a*u^2 + floor*u - b = 0` for `u = sqrt(r)`, with `a` the cap's coefficient and `b = sqrt(30/tau)`.
    let a = 60.0 * EVIDENCE_AT_ONE_MINUTE / (WINDOW_CAP_SECS as f64 * REFERENCE_SPM.sqrt());
    let b = (30.0 / EWMA_TAU_SECS as f64).sqrt();
    let u = (-MIN_THRESHOLD_FRACTION
        + (MIN_THRESHOLD_FRACTION * MIN_THRESHOLD_FRACTION + 4.0 * a * b).sqrt())
        / (2.0 * a);
    let cap_matched_spm = u * u;

    // And the converse: the cap the seam's own rate would ask for. Read from the controller, which
    // solves this per branch, rather than recomputed -- `window_cap_form.rs` measures what adopting it
    // would cost, and both files must be talking about the same number.
    let seam_matched_cap = VardiffState::sigma_matched_window_secs(SPARSE_SPM_SEAM)
        .expect("sigma exceeds the floor at the seam, so a matched window exists");

    eprintln!(
        "  [relations] the cap is sigma-matched at {cap_matched_spm:.2} spm, the seam sits at \
         {SPARSE_SPM_SEAM:.2}; matched at the seam the cap would be {seam_matched_cap}s, not \
         {WINDOW_CAP_SECS}s"
    );
    assert_eq!(
        seam_matched_cap, 594,
        "the seam's own matched window is {seam_matched_cap}s, not the 594 recorded here and in \
         window_cap_form.rs"
    );

    assert!(
        (cap_matched_spm - 8.03).abs() < 0.05,
        "the window cap is sigma-matched at {cap_matched_spm:.2} spm, not the recorded 8.03. If the \
         cap, the floor, the evidence term or tau moved, this is the constant that has to be re-read \
         with them"
    );
    assert!(
        cap_matched_spm > SPARSE_SPM_SEAM as f64,
        "the cap should be matched ABOVE the seam, which is what makes the band between them the one \
         where the dense bar is below the estimator's spread. Matched below the seam would be a \
         different defect and this test's message would be wrong for it"
    );
    assert!(
        capped_dense_bar(SPARSE_SPM_SEAM as f64) < estimator_spread(SPARSE_SPM_SEAM as f64),
        "at the seam the capped dense bar {:.4} should sit below the estimator's spread {:.4}, which \
         is the consequence the disagreement has and the reason it is worth recording. If this has \
         reversed, the gap has been closed and this test should go",
        capped_dense_bar(SPARSE_SPM_SEAM as f64),
        estimator_spread(SPARSE_SPM_SEAM as f64),
    );
}
