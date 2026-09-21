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
    EVIDENCE_AT_ONE_MINUTE, EWMA_TAU_SECS, MAX_DIRECTION_DISCOUNT,
    MIN_EFFECTIVE_TIGHTEN_MULTIPLIER, MIN_THRESHOLD_FRACTION, REFERENCE_SPM, SPARSE_SPM_SEAM,
    STEP_FRACTION_BASE, STEP_FRACTION_ENGAGING_THE_EASING_BOUND, STEP_FRACTION_GROWTH,
    STEP_FRACTION_MAX, TIGHTEN_MULTIPLIER, WINDOW_CAP_SECS,
};

/// The estimator's own relative spread at a target share rate, as a fraction.
///
/// `sigma = sqrt(30 / (r * tau))`, from the EWMA's variance against its effective window of `2*tau`.
/// Computed from [`EWMA_TAU_SECS`] rather than tabulated, so it follows the shipped memory.
///
/// It is the scale every threshold has to be read against: a bar below `sigma` is a bar the
/// controller's own wobble clears, so the deviation that crosses it is as likely to be noise as signal.
fn estimator_spread(spm: f64) -> f64 {
    (30.0 / (spm * EWMA_TAU_SECS as f64)).sqrt()
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
/// nothing making them agree -- so they do not: the cap is `sigma`-matched at **8.05** shares a minute
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

    // And the converse: the cap the seam's own rate would ask for.
    let seam_matched_cap =
        60.0 * EVIDENCE_AT_ONE_MINUTE * (SPARSE_SPM_SEAM as f64 / REFERENCE_SPM).sqrt()
            / (estimator_spread(SPARSE_SPM_SEAM as f64) - MIN_THRESHOLD_FRACTION);

    eprintln!(
        "  [relations] the cap is sigma-matched at {cap_matched_spm:.2} spm, the seam sits at \
         {SPARSE_SPM_SEAM:.2}; matched at the seam the cap would be {seam_matched_cap:.0}s, not \
         {WINDOW_CAP_SECS}s"
    );

    assert!(
        (cap_matched_spm - 8.05).abs() < 0.05,
        "the window cap is sigma-matched at {cap_matched_spm:.2} spm, not the recorded 8.05. If the \
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
