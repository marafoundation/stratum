/// Classic implementation test suite
use crate::vardiff::test::{
    simulate_shares_and_wait, TEST_INITIAL_HASHRATE, TEST_MIN_ALLOWED_HASHRATE,
    TEST_SHARES_PER_MINUTE,
};
use crate::{target::hash_rate_to_target, vardiff::VardiffError, VardiffState};

use super::{
    test_backwards_clock_step_reanchors_window, test_increment_and_reset_shares,
    test_try_vardiff_low_hashrate_decrease_target,
    test_try_vardiff_stable_hashrate_minimal_change_or_no_change,
    test_try_vardiff_with_less_spm_than_expected, test_try_vardiff_with_shares_30_to_60s,
    test_try_vardiff_with_shares_less_than_30, test_try_vardiff_with_shares_more_than_60s, Vardiff,
};
use crate::vardiff::classic::{
    DIRECTION_DISCOUNT_PER_OBSERVATION, EWMA_TAU_SECS, MAX_DIRECTION_DISCOUNT,
    MAX_REPORTED_RATIO_STD, MIN_THRESHOLD_FRACTION, STEP_FRACTION_BASE, STEP_FRACTION_GROWTH,
    STEP_FRACTION_MAX, UNCERTAINTY_FLOOR_WEIGHT,
};

fn new_test_vardiff_state() -> Result<VardiffState, VardiffError> {
    VardiffState::new_with_min(TEST_MIN_ALLOWED_HASHRATE)
}

#[test]
fn test_initialization_and_getters() {
    let vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");

    assert_eq!(vardiff.min_allowed_hashrate(), TEST_MIN_ALLOWED_HASHRATE);
    assert_eq!(vardiff.shares_since_last_update(), 0);
}

#[test]
fn test_increment_and_reset_shares_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_increment_and_reset_shares(&mut vardiff)
}

#[test]
fn test_backwards_clock_step_reanchors_window_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_backwards_clock_step_reanchors_window(&mut vardiff);
}

#[test]
fn test_try_vardiff_stable_hashrate_minimal_change_or_no_change_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_stable_hashrate_minimal_change_or_no_change(&mut vardiff);
}

#[test]
pub fn test_try_vardiff_low_hashrate_decrease_target_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_low_hashrate_decrease_target(&mut vardiff);
}

#[test]
pub fn test_try_vardiff_with_shares_less_than_30_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_with_shares_less_than_30(&mut vardiff);
}

#[test]
pub fn test_try_vardiff_with_shares_30_to_60s_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_with_shares_30_to_60s(&mut vardiff);
}

#[test]
pub fn test_try_vardiff_with_shares_more_than_60s_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_with_shares_more_than_60s(&mut vardiff);
}

#[test]
fn test_try_vardiff_with_less_spm_than_expected_classic() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    test_try_vardiff_with_less_spm_than_expected(&mut vardiff);
}

#[test]
fn test_try_vardiff_hashrate_clamps_to_minimum() {
    let hashrate = TEST_MIN_ALLOWED_HASHRATE * 1.5;
    let target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
        .unwrap()
        .into();

    let mut vardiff = VardiffState::new_with_min(TEST_MIN_ALLOWED_HASHRATE)
        .expect("Failed to create VardiffState");

    // Two changes from the original form of this test, both consequences of earlier patches. The
    // window is 300 seconds rather than 16, because the threshold is sized from the evidence and a
    // 16-second window does not justify acting. And it takes several evaluations rather than one,
    // because a retarget now closes part of the gap rather than all of it.
    let mut belief = hashrate;
    let mut acted = 0;
    for _ in 0..40 {
        simulate_shares_and_wait(&mut vardiff, 0, 300);
        if let Some(updated) = vardiff
            .try_vardiff(belief, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed")
        {
            assert!(
                updated >= TEST_MIN_ALLOWED_HASHRATE,
                "the floor must never be crossed: got {updated}"
            );
            belief = updated;
            acted += 1;
        }
    }

    assert!(acted > 0, "a silent channel should have been acted on");
    assert_eq!(
        belief, TEST_MIN_ALLOWED_HASHRATE,
        "sustained silence should settle exactly at the floor"
    );
    assert_eq!(vardiff.shares_since_last_update(), 0);
}

// Every unusable prior hashrate must fall back to the configured floor. `hashrate` is
// the divisor for the delta percentage and the base every special-case cap scales, so
// without the fallback: `0.0` pins to the floor forever (`0.0 * 10.0` is absorbing),
// a value below the floor pins to it for the round, `NaN` makes every `should_update`
// arm compare false, and `inf` makes the percentage `inf / inf == NaN` with the same
// effect. A miner picks this value via `nominal_hash_rate` at channel open.
#[test]
fn test_try_vardiff_unusable_prior_hashrate_falls_back_to_floor() {
    for bad in [0.0_f32, 0.01, -5.0, f32::NAN, f32::INFINITY] {
        let mut vardiff = VardiffState::new_with_min(TEST_MIN_ALLOWED_HASHRATE)
            .expect("Failed to create VardiffState");

        let target = hash_rate_to_target(1000.0_f64, TEST_SHARES_PER_MINUTE.into()).unwrap();

        // 1000 shares in 16s => 3750 shares/min, far above TEST_SHARES_PER_MINUTE.
        simulate_shares_and_wait(&mut vardiff, 1000, 16);

        match vardiff.try_vardiff(bad, &target, TEST_SHARES_PER_MINUTE) {
            Ok(Some(v)) => assert!(
                v.is_finite() && v > TEST_MIN_ALLOWED_HASHRATE * 1.5,
                "prior hashrate {bad} pinned difficulty to the floor: {v}"
            ),
            other => panic!("prior hashrate {bad} silently disabled vardiff: {other:?}"),
        }
    }
}

// `min_allowed_hashrate` is a `pub` field, so the validation in `new_with_min` can be
// bypassed by direct assignment; `try_vardiff` must sanitize the floor itself before
// relying on it as the fallback baseline and the output clamp.
#[test]
fn test_try_vardiff_sanitizes_unusable_floor() {
    for bad_floor in [0.0_f32, -5.0, f32::NAN, f32::INFINITY] {
        let mut vardiff = VardiffState::new_with_min(TEST_MIN_ALLOWED_HASHRATE)
            .expect("Failed to create VardiffState");
        vardiff.min_allowed_hashrate = bad_floor;

        let target = hash_rate_to_target(1000.0_f64, TEST_SHARES_PER_MINUTE.into()).unwrap();

        // 1000 shares in 16s => 3750 shares/min, far above TEST_SHARES_PER_MINUTE.
        simulate_shares_and_wait(&mut vardiff, 1000, 16);

        // prior hashrate 0.0 forces the fallback onto the (unusable) floor
        match vardiff.try_vardiff(0.0, &target, TEST_SHARES_PER_MINUTE) {
            Ok(Some(v)) => assert!(
                v.is_finite() && v > 0.0,
                "floor {bad_floor} produced unusable hashrate {v}"
            ),
            other => panic!("floor {bad_floor} silently disabled vardiff: {other:?}"),
        }
    }
}

// The floor itself is the fallback baseline used by `try_vardiff`, so a non-positive or
// non-finite `min_allowed_hashrate` would reintroduce the division by zero.
#[test]
fn test_new_with_min_rejects_unusable_floor() {
    for bad in [0.0_f32, -5.0, f32::NAN, f32::INFINITY] {
        let vardiff = VardiffState::new_with_min(bad).expect("Failed to create VardiffState");
        let min = vardiff.min_allowed_hashrate();
        assert!(
            min.is_finite() && min > 0.0,
            "min_allowed_hashrate {min} from input {bad} is unusable as a baseline"
        );
    }
}

// A fully-silent channel must not be eased all the way to the floor. Unbounded, each silent
// evaluation eases again with nothing limiting how many times it runs, so displacement compounds
// without limit. A returning miner is then served a difficulty that much too easy and floods shares
// until vardiff climbs back.
#[test]
fn test_silent_channel_ease_displacement_is_bounded() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    let mut hashrate = TEST_INITIAL_HASHRATE;
    let target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    // Forty consecutive silent evaluations. Unbounded, the ~4 that carry
    // TEST_INITIAL_HASHRATE down to TEST_MIN_ALLOWED_HASHRATE are spent in the first few.
    for _ in 0..40 {
        simulate_shares_and_wait(&mut vardiff, 0, 61);
        if let Ok(Some(updated)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
            hashrate = updated;
        }
    }

    // Displacement must settle at the `9×` that MAX_SILENT_DISPLACEMENT sets. Written as a literal
    // rather than derived from that constant deliberately: a derived expectation moves with the
    // constant and would assert nothing, and this way the test also compiles and fails against a
    // tree without the bound at all.
    let expected = TEST_INITIAL_HASHRATE / 9.0;
    assert!(
        (hashrate / expected - 1.0).abs() < 1e-3,
        "silent channel settled at {hashrate} H/s, not the {expected} H/s that a 9× bound \
         allows (floor is {}) — a returning miner would be served a difficulty that much too easy",
        vardiff.min_allowed_hashrate()
    );
}

// The bound is on *consecutive* silence, so any share activity must restore the full budget.
// Otherwise a long-lived channel that goes briefly quiet many times would eventually exhaust it
// and stop easing for a genuine decline.
#[test]
fn test_share_activity_rearms_the_silence_bound() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    let mut hashrate = TEST_INITIAL_HASHRATE;
    let mut target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    // 1. Sustained silence settles at the bound: 9x below the belief that last had evidence.
    for _ in 0..40 {
        simulate_shares_and_wait(&mut vardiff, 0, 61);
        if let Ok(Some(updated)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
            hashrate = updated;
            target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
        }
    }
    let first_bound = TEST_INITIAL_HASHRATE / 9.0;
    assert!(
        (hashrate / first_bound - 1.0).abs() < 1e-3,
        "silence should settle at {first_bound} H/s, got {hashrate}"
    );

    // 2. Evidence arrives at the lowered belief, re-anchoring the bound to it.
    simulate_shares_and_wait(&mut vardiff, TEST_SHARES_PER_MINUTE as u32, 61);
    if let Ok(Some(updated)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
        hashrate = updated;
        target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
            .expect("valid target");
    }
    let rearmed_from = hashrate;

    // 3. Silence may now descend past the previous bound, because the bound is relative to the
    //    last evidenced belief rather than an absolute floor. Without the re-anchor the channel
    //    would be stuck at `first_bound` forever, unable to follow a miner that really had slowed.
    for _ in 0..40 {
        simulate_shares_and_wait(&mut vardiff, 0, 61);
        if let Ok(Some(updated)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
            hashrate = updated;
            target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
        }
    }
    assert!(
        hashrate < first_bound,
        "activity did not re-arm the bound: still at {hashrate} H/s, no lower than the earlier \
         bound of {first_bound} H/s despite evidence at {rearmed_from} H/s"
    );
    assert!(
        hashrate >= rearmed_from / 9.0 * (1.0 - 1e-3),
        "descent went past the re-armed bound: {hashrate} H/s is below {} H/s",
        rearmed_from / 9.0
    );
}

/// The estimator must forget on the clock, by `e^(−Δt/tau)`, which is the property the cumulative
/// mean lacked and the reason it entrenched.
///
/// Asserted at the mechanism rather than end-to-end because the shared harness cannot express a
/// growing evaluation window: `simulate_shares_and_wait` re-anchors the window on every call, so a
/// sequence of non-retargeting evaluations — the situation in which the old estimator entrenches —
/// is not reachable through it. An end-to-end entrenchment test needs a harness able to advance the
/// evaluation clock independently of the retarget clock.
///
/// The intervals below are chosen to stay under the decision boundary, so no retarget intervenes.
/// They are shorter than they once were because the boundary is now sized from the evidence: at
/// this share rate a 180-second gap clears it, where the old fixed ladder would not have fired.
/// That matters: on a retarget `rescale_ewma` re-expresses the rate against the new difficulty, so
/// the stored rate returns to roughly its pre-decay value — correct, but it hides the decay from
/// an observer reading the rate afterwards.
#[test]
fn test_estimator_forgets_on_the_clock() {
    let hashrate = TEST_INITIAL_HASHRATE;
    let target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    let seed = |vardiff: &mut VardiffState| {
        simulate_shares_and_wait(vardiff, TEST_SHARES_PER_MINUTE as u32, 60);
        let fired = vardiff
            .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        assert!(fired.is_none(), "on-target seeding should not retarget");
    };

    // Seeding takes the first observation whole rather than blending it against a zero prior.
    let mut single = new_test_vardiff_state().expect("Failed to create VardiffState");
    seed(&mut single);
    let seeded = single.ewma_rate();
    assert!(
        (seeded - TEST_SHARES_PER_MINUTE as f64).abs() < 1e-2,
        "seeding should take the first observation whole, got {seeded}"
    );

    // A gap of `GAP` seconds of silence discards `1 − e^(−GAP/tau)` of the stored rate. The gap is
    // an absolute duration rather than a fraction of `tau`, deliberately: a fraction would grow with
    // `tau` until it crossed the decision boundary and the test would start measuring the boundary
    // instead of the decay. The *expectation* is what carries `tau`.
    const GAP: u64 = 90;
    let decay = -(GAP as f64) / EWMA_TAU_SECS as f64;
    // Self-check: a `tau` large enough to make the gap's decay unmeasurable would leave the
    // assertions below true of any filter, so require the effect to be worth asserting.
    assert!(
        decay.exp() < 0.99,
        "a {GAP}s gap decays only {:.4}x at tau={EWMA_TAU_SECS}s, too little to test a decay law",
        decay.exp()
    );
    simulate_shares_and_wait(&mut single, 0, GAP);
    let fired = single
        .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
        .expect("try_vardiff failed");
    assert!(
        fired.is_none(),
        "a {GAP}s gap should stay under the boundary"
    );
    let expected = seeded * decay.exp();
    let decayed = single.ewma_rate();
    assert!(
        (decayed / expected - 1.0).abs() < 1e-3,
        "after {GAP}s at tau={EWMA_TAU_SECS}s the rate should be {expected}, got {decayed}"
    );

    // Decay tracks elapsed time, not the number of evaluations: two eighth-tau gaps must land
    // where one quarter-tau gap did. A per-evaluation constant would forget twice as much here, which
    // is how an irregular cadence silently changes a filter's memory.
    let mut split = new_test_vardiff_state().expect("Failed to create VardiffState");
    seed(&mut split);
    for _ in 0..2 {
        simulate_shares_and_wait(&mut split, 0, GAP / 2);
        let fired = split
            .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        assert!(
            fired.is_none(),
            "a {}s gap should stay under the boundary",
            GAP / 2
        );
    }
    assert!(
        (split.ewma_rate() / decayed - 1.0).abs() < 1e-3,
        "two {}s gaps ({}) should decay the same as one {GAP}s gap ({decayed})",
        GAP / 2,
        split.ewma_rate()
    );
}

/// A decline is detected in the same few evaluations whatever the channel's age.
///
/// The same scenario across the series: an hour of on-target delivery over sixty non-retargeting
/// evaluations, then the miner drops to a tenth of its rate. The fixed ladder over a cumulative
/// window took **thirteen** evaluations to react. The clock-decayed average took **two**, because
/// evidence ages whether or not the controller fires. Sizing the threshold from the evidence takes
/// it to **one**: after an hour the window holds enough observation that a tenfold shortfall clears
/// the bar immediately, where the ladder still demanded a fixed 15%.
///
/// Both figures are asserted exactly rather than as bounds. Reaction time is the quantity these
/// patches trade against each other, so a change to it should appear as a change to this line.
///
/// Needs a controllable clock: `simulate_shares_and_wait` re-anchors the window on every call, so
/// it can express "the window is an hour long" but not "an hour arrived as sixty separate
/// evaluations" — and the second is what grows the window.
///
/// [`MockClock`]: crate::vardiff::clock::MockClock
#[test]
fn test_decline_detected_regardless_of_window_age() {
    use crate::vardiff::clock::MockClock;
    use std::sync::Arc;

    let clock = Arc::new(MockClock::new(1_000_000));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("Failed to create VardiffState");
    let hashrate = TEST_INITIAL_HASHRATE;
    let target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    // An hour of delivery exactly on target. Nothing retargets, so the window reaches an hour.
    for evaluation in 1..=60 {
        for _ in 0..TEST_SHARES_PER_MINUTE as u32 {
            vardiff.increment_shares_since_last_update();
        }
        clock.advance(60);
        let outcome = vardiff
            .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        assert!(
            outcome.is_none(),
            "on-target delivery should not retarget (evaluation {evaluation})"
        );
    }

    // The miner collapses to a tenth of its rate and stays there.
    let mut evaluations_to_react = 0;
    loop {
        evaluations_to_react += 1;
        vardiff.increment_shares_since_last_update();
        clock.advance(60);
        let outcome = vardiff
            .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        if outcome.is_some() || evaluations_to_react > 60 {
            break;
        }
    }

    assert!(
        evaluations_to_react <= 2,
        "an hour-old window must not delay detection; took {evaluations_to_react} evaluations \
         (the fixed ladder over a cumulative window took 13)"
    );
    assert_eq!(
        evaluations_to_react, 1,
        "reaction time is 1 evaluation; a change here is a change in reaction time"
    );
}

/// Tightening requires more evidence than loosening, and this is what that costs.
///
/// Compares the *same* deviation in each direction. That needs care: the deviation is measured as
/// `|estimate/belief − 1|`, which floors at −100% but has no ceiling, so equal *rate ratios* are
/// not equal deviations — a miner at six times its target rate reads 500%, one at a sixth reads
/// 83%. The rates below are chosen to produce the same 83% either way, leaving the multiplier as
/// the only difference.
///
/// The delay is the deliberate price of the asymmetry, so it is measured rather than left implicit.
/// Note it is not simply the multiplier: the threshold has an additive floor that does not scale,
/// so an 8x evidence requirement stretches the window by rather more than 8x.
#[test]
fn test_tightening_needs_more_evidence_than_loosening() {
    use crate::vardiff::clock::MockClock;
    use std::sync::Arc;

    // Shortest window, in whole seconds, at which the controller acts on a channel delivering
    // `realized_spm` against a target of TEST_SHARES_PER_MINUTE.
    let first_action_at = |realized_spm: f64| -> u64 {
        for window in 1..=4000u64 {
            let clock = Arc::new(MockClock::new(1_000_000));
            let mut vardiff =
                VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
                    .expect("Failed to create VardiffState");
            let hashrate = TEST_INITIAL_HASHRATE;
            let target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
            let shares = (realized_spm * window as f64 / 60.0).round() as u32;
            for _ in 0..shares {
                vardiff.increment_shares_since_last_update();
            }
            clock.advance(window);
            if vardiff
                .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
                .expect("try_vardiff failed")
                .is_some()
            {
                return window;
            }
        }
        u64::MAX
    };

    let deviation = 0.8333;
    let tighten_window = first_action_at(TEST_SHARES_PER_MINUTE as f64 * (1.0 + deviation));
    let loosen_window = first_action_at(TEST_SHARES_PER_MINUTE as f64 * (1.0 - deviation));

    assert!(
        tighten_window > loosen_window,
        "tightening should need the longer observation: tighten at {tighten_window}s, \
         loosen at {loosen_window}s"
    );
    assert_eq!(
        (tighten_window, loosen_window),
        (1393, 72),
        "the asymmetry delays action on an 83% over-delivery from 72s to 1393s; a change here is \
         a change in that cost"
    );
    // WAS (945, 68) BEFORE THE C6b FED FLOOR. This pin did its job: the cost moved and said so.
    //
    // Feeding `0.5 × ratio_std` into the dense floor costs **1.47× on the tighten direction**
    // (945s → 1393s, i.e. 15.8 min → 23.2 min) and **1.06× on loosen** (68s → 72s). Both are the
    // floor moving, since at these windows the evidence term has largely decayed and the floor is
    // what remains — and the tighten side pays more because `TIGHTEN_MULTIPLIER` scales the floor
    // along with everything else.
    //
    // **This qualifies a published claim.** `C6_QUANTIFIED_2026_09_08.md` argued feeding "costs no
    // reaction time at high rate, which is the property neither fixA (uniform 3×) nor fixB (14× on
    // tighten) has." The RANKING survives and is the reason to prefer this fix — 1.47× against
    // fixA's ~3× and fixB's 14× — but "costs no reaction time" is too strong at the shipping
    // operating point. Say "cheapest of the three", not "free".
    //
    // Corroborated across instruments, which is why it is believed: simulation put the settled
    // belief 2–3 pp further under difficulty, and `test_asymmetry_leaves_a_dead_zone_of_uncorrected_
    // over_delivery` independently measures the dead zone widening 42% → 46%. Three readings of one
    // mechanism — a wider floor — agreeing in sign and rough size.
}

/// The extra burden of proof attaches to the move being made, not to the observed share rate.
///
/// Those two agree only while the caller's `target` and `hashrate` describe the same difficulty,
/// and nothing in `try_vardiff`'s signature enforces that. Here they deliberately disagree: the
/// target implies a quarter of the stated belief, and the channel delivers twice its target rate.
/// Reading the rate would call that a tightening and demand eight times the evidence; the move is
/// in fact a *loosening*, halving the belief, and must be judged on the ordinary threshold.
///
/// Without this distinction the asymmetry inverts — it would make loosening harder, which is the
/// opposite of the property it exists to provide.
#[test]
fn test_asymmetry_follows_the_move_not_the_share_rate() {
    use crate::vardiff::clock::MockClock;
    use std::sync::Arc;

    let clock = Arc::new(MockClock::new(1_000_000));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("Failed to create VardiffState");

    let hashrate = TEST_INITIAL_HASHRATE;
    // Target consistent with a quarter of `hashrate`, so a rate above target still implies a
    // belief below it.
    let target = hash_rate_to_target((hashrate / 4.0).into(), TEST_SHARES_PER_MINUTE.into())
        .expect("valid target");

    // Twice the target share rate, over TEN minutes: 200 shares where 100 are expected.
    //
    // This was two minutes and 40 shares, and it passed on a 1.70 pp margin — a 50.00%
    // deviation against a 48.30% threshold. The C6b fed floor adds `0.5 × ratio_std`, worth
    // 1.77 pp at this operating point (20 spm realized, σ = 3.53%), which took the threshold
    // to 50.07% and stopped this firing. The property under test did not change; the test was
    // simply reading it off a knife edge.
    //
    // Lengthening the window shrinks the evidence term (`EVIDENCE_AT_ONE_MINUTE / minutes`)
    // without touching the observed rate, so the same scenario is judged with room to spare:
    // 50.00% against 15.41%. **And the guard binds harder than it did.** If the burden of
    // proof were wrongly keyed on the observed rate exceeding target, the ×8 multiplier would
    // put the bar at 123.3% and this could not fire — a 73 pp discrimination, where the old
    // form had 1.7 pp between passing and failing for the wrong reason.
    let window = 600u64;
    for _ in 0..200 {
        vardiff.increment_shares_since_last_update();
    }
    clock.advance(window);

    let outcome = vardiff
        .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
        .expect("try_vardiff failed");

    let new_hashrate = outcome.expect(
        "a loosening move should be judged on the ordinary threshold; if the extra burden of \
         proof were keyed on the observed rate exceeding target, this would not have fired",
    );
    assert!(
        new_hashrate < hashrate,
        "the move is a loosening: {hashrate} -> {new_hashrate}"
    );
}

/// The same-direction run accumulates, resets on reversal, and clears with the window.
///
/// The run length is what the threshold discount is computed from, so these three behaviours are
/// the mechanism. The reset on reversal is what keeps noise from earning a discount: a deviation
/// that changes sign starts over, while a persistent one accumulates.
///
/// Clearing on `reset_counter` matters for a second reason. The discount applies to tightening as
/// well as loosening, so a stale run would carry its accumulated relaxation into a fresh cycle and
/// erode the extra burden of proof that tightening is supposed to carry.
#[test]
fn test_same_direction_run_accumulates_resets_and_clears() {
    use crate::vardiff::clock::MockClock;
    use std::sync::Arc;

    let clock = Arc::new(MockClock::new(1_000_000));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("Failed to create VardiffState");
    let hashrate = TEST_INITIAL_HASHRATE;
    let target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    assert_eq!(
        vardiff.direction_run(),
        (0, 0),
        "a fresh channel has no run"
    );

    // Three evaluations delivering under target: three loosening observations in a row.
    for expected_run in 1..=3u32 {
        vardiff.increment_shares_since_last_update();
        clock.advance(60);
        let _ = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE);
        assert_eq!(
            vardiff.direction_run(),
            (-1, expected_run),
            "a loosening run should reach {expected_run}"
        );
    }

    // Over-target delivery eventually reverses the direction, and the moment it does the count
    // must restart at 1 rather than continue the loosening run. Driven to the crossover rather
    // than assuming one evaluation reaches it: a single observation carries weight
    // `1 − e^(−dt/EWMA_TAU_SECS)` in the estimate, so how many evaluations it takes to pull the
    // estimate above the belief is a function of `tau`, while the property under test is not.
    let mut at_reversal = None;
    for _ in 0..40 {
        for _ in 0..(TEST_SHARES_PER_MINUTE as u32 * 6) {
            vardiff.increment_shares_since_last_update();
        }
        clock.advance(60);
        let _ = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE);
        if vardiff.direction_run().0 == 1 {
            at_reversal = Some(vardiff.direction_run());
            break;
        }
    }
    assert_eq!(
        at_reversal,
        Some((1, 1)),
        "a reversal should restart the run rather than continue it"
    );

    // Re-anchoring the window clears the run outright.
    vardiff.reset_counter().expect("reset_counter failed");
    assert_eq!(
        vardiff.direction_run(),
        (0, 0),
        "reset_counter must clear the run, or its discount outlives the evidence"
    );
}

/// The asymmetry leaves a permanent band of over-delivery that is never corrected.
///
/// As a window lengthens the evidence term vanishes and only the floor remains, so the loosening
/// bar approaches `MIN_THRESHOLD_FRACTION` while the tightening bar approaches that floor times
/// `TIGHTEN_MULTIPLIER`. Measured at a ten-hour window: the controller acts on a 6% shortfall but
/// not on a 41% excess.
///
/// This band is where the controller's settled offset lives — recorded at −6.12% in simulation and
/// −6.96% on hardware, both inside it. That offset is deliberate rather than an error: a difficulty
/// settling slightly easy rather than exactly on target keeps the miner's own share-rate excursions inside
/// a band it tolerates, where a difficulty settled exactly on target would let the upper excursions
/// cross the point at which a rate-switching miner disconnects. Pinned here because the band is a
/// product of three constants and changing any of them moves the settled behaviour.
#[test]
fn test_asymmetry_leaves_a_dead_zone_of_uncorrected_over_delivery() {
    use crate::vardiff::clock::MockClock;
    use std::sync::Arc;

    let hashrate = TEST_INITIAL_HASHRATE;
    let window = 36_000u64; // ten hours: long enough that the evidence term is negligible

    // Smallest whole-percent deviation the controller acts on, in the given direction.
    let smallest_acted_on = |tighten: bool| -> u32 {
        for pct in 1..=200u32 {
            let factor = if tighten {
                1.0 + pct as f64 / 100.0
            } else {
                1.0 - pct as f64 / 100.0
            };
            let clock = Arc::new(MockClock::new(1_000_000));
            let mut vardiff =
                VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
                    .expect("Failed to create VardiffState");
            let target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
            let shares =
                (TEST_SHARES_PER_MINUTE as f64 * factor * window as f64 / 60.0).round() as u32;
            for _ in 0..shares {
                vardiff.increment_shares_since_last_update();
            }
            clock.advance(window);
            if vardiff
                .try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
                .expect("try_vardiff failed")
                .is_some()
            {
                return pct;
            }
        }
        panic!("no deviation up to 200% was acted on");
    };

    let loosen = smallest_acted_on(false);
    let tighten = smallest_acted_on(true);

    assert!(
        tighten > loosen * 4,
        "the dead zone should be several times the loosening bar: loosen {loosen}%, tighten {tighten}%"
    );
    assert_eq!(
        (loosen, tighten),
        (6, 46),
        "the dead zone is a product of MIN_THRESHOLD_FRACTION and TIGHTEN_MULTIPLIER; a change \
         here moves the settled offset with it"
    );
    // WAS (6, 42) BEFORE THE C6b FED FLOOR — the dead zone widened by 4 percentage points, from
    // 42% to 46% of uncorrected over-delivery, while the loosening bar held at 6%.
    //
    // This is the cost of the fix in the units that matter most, and this test's own message
    // predicted it: the dead zone is a product of the floor and the tighten multiplier, and the fix
    // raises the floor. `MIN_THRESHOLD_FRACTION`'s doc says the same thing in the other direction —
    // "widening this constant or narrowing the multiplier moves the dead zone and therefore moves
    // the offset."
    //
    // So the settled offset moves with it, and the offset is load-bearing rather than an error: it
    // parks the excursion band below the level at which a switch-miner leaves. The recorded values
    // are −6.12% in simulation and −6.96% on hardware; simulation on the composed pipeline puts the
    // fed version 2–3 pp deeper. **Whether ~3 pp of extra under-difficulty is worth buying
    // `floor/σ ≥ 1` is a policy call, not a derivation**, and it is the question phase 2 should be
    // read against — not the fire rate, which is the easy half.
    //
    // Asymmetric on purpose: 6% → 6% means the fix does NOT dull the loosening path, the direction
    // that strands a slowing miner. It buys quiet in the direction that can compound and leaves the
    // safety-relevant direction alone.
}

// The discount must change a *decision*, not just a counter. A persistent shortfall evaluated every
// 20 seconds clears the loosening bar sooner once the run has earned a few discount steps, so the
// first retarget arrives earlier than it would on a tree with the run counter but no discount.
// Asserted as an exact index because that index is the whole behavioural claim: without the
// discount, `DIRECTION_DISCOUNT_PER_OBSERVATION` could be set to zero and every other test here
// would still pass.
#[test]
fn test_the_discount_brings_a_persistent_deviation_forward() {
    use crate::vardiff::clock::MockClock;
    use std::sync::Arc;

    let clock = Arc::new(MockClock::new(0));
    let mut vardiff = VardiffState::new_with_clock(1.0, clock.clone()).expect("state");
    let hashrate = 1_000_000.0_f32;
    let target =
        hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    // A steady 40% shortfall, evaluated every 20 seconds: too small to fire on one short window,
    // large enough to fire once persistence has relaxed the bar.
    let mut first_fire = None;
    for evaluation in 1..=20 {
        let delivered = ((TEST_SHARES_PER_MINUTE as f64) * (20.0 / 60.0) * 0.6).round() as u32;
        for _ in 0..delivered {
            vardiff.increment_shares_since_last_update();
        }
        clock.advance(20);
        if let Ok(Some(_)) = vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE) {
            first_fire = Some(evaluation);
            break;
        }
    }

    assert_eq!(
        first_fire,
        Some(6),
        "a persistent 40% shortfall should first retarget at evaluation 6; without the run discount \
         the same deviation waits until 8"
    );
}

/// Consecutive retargets in one direction close a larger share of the gap each time.
///
/// Observable without instrumentation on a silent channel: the estimate is zero, so the fraction of
/// the gap a step closes is just `(before − after) / before`. It should walk up from
/// `STEP_FRACTION_BASE` by `STEP_FRACTION_GROWTH` per consecutive move, and a reversal should return it to the base.
///
/// This is the mechanism's whole content. Its cost is measured separately and is substantial —
/// convergence on a genuine threefold rise takes roughly five times as many evaluations as a full
/// retarget — which is why the acceleration exists at all: without it every step would close only
/// `STEP_FRACTION_BASE`.
#[test]
fn test_consecutive_retargets_close_a_growing_share_of_the_gap() {
    let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
    let mut belief = TEST_INITIAL_HASHRATE;
    let target =
        hash_rate_to_target(belief.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    let mut fractions = Vec::new();
    for _ in 0..5 {
        simulate_shares_and_wait(&mut vardiff, 0, 300);
        let before = belief;
        let Some(after) = vardiff
            .try_vardiff(belief, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed")
        else {
            break;
        };
        // The estimate on a silent channel is zero, so the gap is `before` itself.
        fractions.push((before - after) / before);
        belief = after;
    }

    assert!(
        fractions.len() >= 3,
        "expected at least three retargets to compare, got {}",
        fractions.len()
    );
    assert!(
        (fractions[0] - 0.2).abs() < 1e-4,
        "a first move should close STEP_FRACTION_BASE of the gap, closed {}",
        fractions[0]
    );
    for pair in fractions.windows(2) {
        assert!(
            pair[1] > pair[0],
            "each consecutive move should close more than the last: {pair:?}"
        );
        assert!(
            (pair[1] - pair[0] - 0.05).abs() < 1e-4,
            "the step should grow by STEP_FRACTION_GROWTH: {pair:?}"
        );
    }
}

/// Both accumulating mechanisms can actually reach their stated ceilings.
///
/// Each clamps its run length at a point derived by dividing two float constants and truncating,
/// so whether the ceiling is reachable depends on which side of an integer boundary the division
/// lands. It currently lands correctly for both — the step fraction saturates at a run of 9 and the
/// threshold discount at 11 — but that is a property of the constants' float representation rather
/// than of the arithmetic, and changing any of them could move it by one without any other test
/// noticing. Guarded here so that would fail loudly instead.
#[test]
fn test_accumulating_ceilings_are_reachable() {
    let step_run = 1u32 + ((STEP_FRACTION_MAX - STEP_FRACTION_BASE) / STEP_FRACTION_GROWTH) as u32;
    let step_at_run = STEP_FRACTION_BASE + STEP_FRACTION_GROWTH * (step_run - 1) as f32;
    assert!(
        step_at_run >= STEP_FRACTION_MAX,
        "a run of {step_run} reaches only {step_at_run}, so STEP_FRACTION_MAX of \
         {STEP_FRACTION_MAX} is unreachable — the derived clamp is short by one"
    );

    let discount_run = 1u32 + (MAX_DIRECTION_DISCOUNT / DIRECTION_DISCOUNT_PER_OBSERVATION) as u32;
    let discount_at_run = DIRECTION_DISCOUNT_PER_OBSERVATION * (discount_run - 1) as f64;
    assert!(
        discount_at_run >= MAX_DIRECTION_DISCOUNT,
        "a run of {discount_run} reaches only {discount_at_run}, so MAX_DIRECTION_DISCOUNT of \
         {MAX_DIRECTION_DISCOUNT} is unreachable — the derived clamp is short by one"
    );
}

// The retarget run must count moves that reach the wire, not evaluations that enter the fire path.
// A belief already at `min_allowed_hashrate` clears the threshold on every silent evaluation — the
// deviation is 100% — computes an ease, has all of it absorbed by the clamp, and returns `Ok(None)`
// with nothing sent. Those must not advance the run, because the run sizes the next step and an
// evaluation that moved nothing has not shown its steps are too small.
#[test]
fn test_absorbed_evaluations_do_not_advance_the_retarget_run() {
    use crate::vardiff::clock::MockClock;
    use std::sync::Arc;

    let clock = Arc::new(MockClock::new(0));
    let mut vardiff = VardiffState::new_with_clock(1_000.0, clock.clone()).expect("state");
    let belief = 1_000.0_f32; // exactly the floor, so every ease is absorbed whole
    let target =
        hash_rate_to_target(belief.into(), TEST_SHARES_PER_MINUTE.into()).expect("valid target");

    for evaluation in 1..=6 {
        clock.advance(61);
        let outcome = vardiff
            .try_vardiff(belief, &target, TEST_SHARES_PER_MINUTE)
            .expect("try_vardiff failed");
        assert!(
            outcome.is_none(),
            "evaluation {evaluation} should send nothing: the belief is already at the floor"
        );
        assert_eq!(
            vardiff.fire_run(),
            (0, 0),
            "evaluation {evaluation} sent nothing but advanced the retarget run, so the next real \
             move would close a larger share of the gap than a first move should"
        );
    }
}

// No single evaluation may move the belief more than [`MAX_STEP_RATIO`], in either direction.
// Sustained rather than one window, because the downward bound only binds once a same-direction run
// has grown the step fraction: a first move travels 20% of the gap, which cannot halve the belief.
#[test]
fn test_no_evaluation_moves_the_belief_past_the_step_bound() {
    for (label, delivered) in [
        (
            "a hundredfold burst",
            (TEST_SHARES_PER_MINUTE * 100.0) as u32,
        ),
        ("a collapse to near nothing", 1),
    ] {
        let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
        let mut hashrate = TEST_INITIAL_HASHRATE;
        let mut widest = 1.0_f32;

        for _ in 0..40 {
            let target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
            simulate_shares_and_wait(&mut vardiff, delivered, 61);
            if let Ok(Some(updated)) =
                vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            {
                let ratio = updated / hashrate;
                widest = widest.max(if ratio >= 1.0 { ratio } else { 1.0 / ratio });
                hashrate = updated;
            }
        }

        // Literal rather than derived from `MAX_STEP_RATIO`, matching the silence bound's test: a
        // derived expectation moves with the constant and would assert nothing, and this way the
        // test also fails against the buckets this replaces, whose widest observed step was 6.4x.
        assert!(
            widest <= 2.0 + 1e-3,
            "{label}: an evaluation moved the belief {widest:.2}x, outside the 2x bound"
        );
    }
}

// The step bound is the one mechanism in this controller that could quietly slow the response to a
// genuine decline, which is what the decline-safety gate measures. It must not reach that far: a
// halved hashrate puts the estimate at half the belief, and a partial move toward it is nowhere
// near a halving of the difficulty. Asserted rather than reasoned, because the margin depends on
// three constants that later work may move.
#[test]
fn test_the_step_bound_does_not_slow_the_response_to_a_decline() {
    for (label, surviving_fraction) in [("half", 0.5_f32), ("a tenth", 0.1), ("a hundredth", 0.01)]
    {
        let mut vardiff = new_test_vardiff_state().expect("Failed to create VardiffState");
        let mut hashrate = TEST_INITIAL_HASHRATE;

        // Settle first: an on-target half hour, so the decline starts from a converged belief.
        for _ in 0..30 {
            let target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
            simulate_shares_and_wait(&mut vardiff, TEST_SHARES_PER_MINUTE as u32, 61);
            if let Ok(Some(updated)) =
                vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            {
                hashrate = updated;
            }
        }

        // The gate's observation window: two hours at the reduced rate.
        let delivered = (TEST_SHARES_PER_MINUTE * surviving_fraction).round() as u32;
        let mut widest = 1.0_f32;
        for _ in 0..120 {
            let target = hash_rate_to_target(hashrate.into(), TEST_SHARES_PER_MINUTE.into())
                .expect("valid target");
            simulate_shares_and_wait(&mut vardiff, delivered, 61);
            if let Ok(Some(updated)) =
                vardiff.try_vardiff(hashrate, &target, TEST_SHARES_PER_MINUTE)
            {
                let ratio = updated / hashrate;
                widest = widest.max(if ratio >= 1.0 { ratio } else { 1.0 / ratio });
                hashrate = updated;
            }
        }

        assert!(
            widest < 2.0,
            "a decline to {label} of the rate moved the belief {widest:.2}x in one evaluation, \
             reaching the 2x step bound — the bound is now shaping decline response, which the \
             decline-safety gate measures"
        );
    }
}

/// The fingerprint must carry the constants, or a run cannot be attributed from its logs.
///
/// The failure this guards is a literal in the format string drifting from the constant it
/// describes -- which would make the line actively misleading rather than merely absent. Only the
/// constants already visible to this module are checked; that is enough to catch the drift, since
/// they are formatted through the same path as the rest.
#[test]
fn test_parameter_fingerprint_carries_the_constants() {
    let f = VardiffState::parameter_fingerprint();
    for expected in [
        format!("tau={EWMA_TAU_SECS}s"),
        format!("discount_per_obs={DIRECTION_DISCOUNT_PER_OBSERVATION}"),
        format!("max_discount={MAX_DIRECTION_DISCOUNT}"),
        format!(
            "step_fraction={STEP_FRACTION_BASE}..{STEP_FRACTION_MAX} by {STEP_FRACTION_GROWTH}"
        ),
    ] {
        assert!(
            f.contains(&expected),
            "fingerprint is missing or has drifted from `{expected}`: {f}"
        );
    }
}

// ── C6b: the fed dense floor ────────────────────────────────────────────────────
//
// The shipping operating point these assert against: `shares_per_minute = 6.0`
// configured, realized ~8.26 spm after the pow2 floor at the SV1 egress. Naming the
// operating point, because every figure that moved in this arc was one quoted
// without one.
const SHIPPING_REALIZED_SPM: f64 = 8.26;

/// The check that depends on no gate, no simulation and no rig: the producer must
/// reproduce the closed form the C6b algebra was derived from, `sigma ~ sqrt(30/(r*tau))`.
/// If this drifts, the published floor/sigma table stops describing this code.
#[test]
fn ratio_std_reproduces_the_closed_form() {
    let dt = 60u64;
    let sigma = VardiffState::ratio_std_for(dt, SHIPPING_REALIZED_SPM);

    // Exact: 1/sqrt(lambda * n_eff).
    let alpha = (-(dt as f64) / (EWMA_TAU_SECS as f64)).exp();
    let n_eff = (1.0 + alpha) / (1.0 - alpha);
    let lambda = SHIPPING_REALIZED_SPM * (dt as f64 / 60.0);
    let exact = 1.0 / (lambda * n_eff).sqrt();
    assert!(
        (sigma - exact).abs() < 1e-12,
        "ratio_std {sigma} must be the exact EWMA form {exact}"
    );

    // Asymptotic: the dt << tau collapse the algebra used. tau=1200 agrees to 0.01%,
    // tau=360 to 0.12%; assert the looser bound so this test is tau-agnostic.
    let asymptotic = (30.0 / (SHIPPING_REALIZED_SPM * EWMA_TAU_SECS as f64)).sqrt();
    let disagreement = (sigma / asymptotic - 1.0).abs();
    assert!(
        disagreement < 0.0013,
        "exact {sigma} vs closed form {asymptotic} disagree by {:.4}%, over the 0.12% claimed",
        disagreement * 100.0
    );
}

/// `n_eff` is `(1+alpha)/(1-alpha)`, NOT `tau/dt`. Using `tau/dt` would overstate
/// sigma by ~sqrt(2) and over-widen the floor, so pin the distinction.
#[test]
fn effective_sample_count_is_not_tau_over_dt() {
    let dt = 60u64;
    let alpha = (-(dt as f64) / (EWMA_TAU_SECS as f64)).exp();
    let n_eff = (1.0 + alpha) / (1.0 - alpha);
    let naive = EWMA_TAU_SECS as f64 / dt as f64;
    assert!(
        (n_eff / naive - 2.0).abs() < 0.01,
        "n_eff {n_eff} should be ~2x the naive tau/dt {naive} — if this stops holding, \
         re-derive rather than adjusting the bound"
    );
}

/// The margin this fix is bought for: `floor/sigma` must clear 1.0 — the boundary above
/// which noise-driven firing is algebraically impossible rather than merely rarer.
/// Unfed it is 0.50 at tau=360 and 0.91 at tau=1200; fed it gains exactly the weight.
#[test]
fn fed_floor_clears_the_no_hunting_boundary() {
    let sigma = VardiffState::ratio_std_for(60, SHIPPING_REALIZED_SPM);
    let unfed_margin = MIN_THRESHOLD_FRACTION / sigma;
    let fed_margin = (MIN_THRESHOLD_FRACTION + UNCERTAINTY_FLOOR_WEIGHT * sigma) / sigma;

    // Feeding adds exactly the weight to the margin, at every rate. That rate-independence
    // is the property a larger MIN_THRESHOLD_FRACTION would not have.
    assert!(
        (fed_margin - unfed_margin - UNCERTAINTY_FLOOR_WEIGHT).abs() < 1e-9,
        "fed margin {fed_margin} should be unfed {unfed_margin} + {UNCERTAINTY_FLOOR_WEIGHT}"
    );
    assert!(
        fed_margin >= 1.0,
        "fed margin {fed_margin} must reach the floor>=sigma no-hunting boundary at the \
         shipping rate; unfed was {unfed_margin}"
    );
}

/// A silent or unseeded channel must behave EXACTLY as the unfed build does, so an arm
/// that loses its miner does not quietly become a different controller.
#[test]
fn no_estimate_means_the_static_floor_exactly() {
    assert_eq!(VardiffState::ratio_std_for(60, 0.0), 0.0);
    assert_eq!(VardiffState::ratio_std_for(60, -1.0), 0.0);
    assert_eq!(VardiffState::ratio_std_for(0, 8.26), 0.0);
    assert_eq!(VardiffState::ratio_std_for(60, f64::NAN), 0.0);
    let s = new_test_vardiff_state().unwrap();
    assert_eq!(s.ratio_std, 0.0, "a fresh state reports no spread");
    assert_eq!(
        s.effective_floor(),
        MIN_THRESHOLD_FRACTION,
        "with no spread the floor must be the static one, bit for bit"
    );
}

/// The rail, not a tuning knob: sigma scales as 1/sqrt(rate), so an uncapped value would
/// push the loosening bar past the ~100pp that total silence produces and strand a dead
/// channel at its last difficulty for good.
#[test]
fn ratio_std_is_capped_so_loosening_stays_reachable() {
    let sigma = VardiffState::ratio_std_for(60, 1.0e-9);
    assert_eq!(sigma, MAX_REPORTED_RATIO_STD);
    let fed_floor_pp = (MIN_THRESHOLD_FRACTION + UNCERTAINTY_FLOOR_WEIGHT * sigma) * 100.0;
    assert!(
        fed_floor_pp < 100.0,
        "the fed loosening bar reached {fed_floor_pp}pp; silence yields ~100pp, so the \
         loosen fire must stay reachable below it"
    );
    // And it must not bind anywhere in the measured band, or it would be a parameter of
    // the result rather than a rail under it.
    for &r in &[4.0, 6.0, SHIPPING_REALIZED_SPM, 12.0, 20.0, 30.0, 60.0] {
        assert!(
            VardiffState::ratio_std_for(60, r) < MAX_REPORTED_RATIO_STD,
            "the cap binds at {r} spm — it must not touch the band"
        );
    }
}

/// The interval matters and pairing the wrong one is silent. `ratio_std` must be keyed on
/// the ESTIMATOR's interval; feeding it a long retarget window instead would report a
/// confidence the estimator never had, and would do so most on the quiet converged
/// channels this fix exists for.
#[test]
fn a_longer_interval_reports_more_confidence_so_the_wrong_one_understates_the_floor() {
    let short = VardiffState::ratio_std_for(60, SHIPPING_REALIZED_SPM);
    let long = VardiffState::ratio_std_for(1800, SHIPPING_REALIZED_SPM);
    assert!(
        long < short,
        "a 30-minute interval ({long}) must read as more confident than a 1-minute one \
         ({short}) — so using the retarget window in place of the evaluation gap would \
         silently shrink the floor exactly where C6b needs it widened"
    );
}

/// The fingerprint must carry the new constant, or a fed build is indistinguishable from
/// an unfed one in the logs — and the A/B's identity check keys on this line.
#[test]
fn fingerprint_carries_the_uncertainty_weight() {
    let f = VardiffState::parameter_fingerprint();
    assert!(
        f.contains(&format!(
            "uncertainty_floor_weight={UNCERTAINTY_FLOOR_WEIGHT}"
        )),
        "fingerprint must name the C6b weight so a fed arm is attributable: {f}"
    );
}

// ── The runtime-override seam ───────────────────────────────────────────────────
//
// Three properties, and each corresponds to a way this seam can fail while every log
// line still looks plausible.

/// A default build must be byte-for-byte the controller it was before the seam existed.
///
/// The accessors exist in both builds; only their bodies differ. So the property that makes the
/// feature safe to add to a shipping lineage is that *without* the feature every accessor returns
/// its constant — which is checkable, and is checked here rather than argued from the `cfg`.
#[test]
fn params_default_to_the_shipped_constants() {
    use crate::vardiff::classic::params;
    use crate::vardiff::classic::{MAX_SILENT_DISPLACEMENT, MAX_STEP_RATIO, TIGHTEN_MULTIPLIER};

    // Under the feature these hold only while no `VARDIFF_*` variable is set, which is the
    // condition a shipping deployment is in anyway. The assertion is the same either way.
    assert_eq!(params::tau_secs(), EWMA_TAU_SECS);
    assert_eq!(params::min_threshold_fraction(), MIN_THRESHOLD_FRACTION);
    assert_eq!(params::uncertainty_floor_weight(), UNCERTAINTY_FLOOR_WEIGHT);
    assert_eq!(params::tighten_multiplier(), TIGHTEN_MULTIPLIER);
    assert_eq!(params::max_direction_discount(), MAX_DIRECTION_DISCOUNT);
    assert_eq!(params::max_step_ratio(), MAX_STEP_RATIO);
    assert_eq!(params::max_silent_displacement(), MAX_SILENT_DISPLACEMENT);
}

/// The run clamp must follow the *effective* discount ceiling, not the default one.
///
/// This is the one derived quantity in the set: it was a `const` computed from
/// `MAX_DIRECTION_DISCOUNT`, and leaving it a `const` would have clamped the same-direction run at
/// the default ceiling while `threshold` discounted against an overridden one. A swept discount
/// would then have stopped taking effect above `0.6` with nothing in the logs to show it — the
/// clamp is invisible, only its consequence is. Asserted as the relationship rather than as `11`,
/// so it holds at any ceiling.
#[test]
fn direction_run_clamp_follows_the_effective_ceiling() {
    use crate::vardiff::classic::params;

    let expected =
        1 + (params::max_direction_discount() / DIRECTION_DISCOUNT_PER_OBSERVATION) as u32;
    assert_eq!(params::direction_run_at_max_discount(), expected);

    // And at the shipped ceiling it is the value the discount arithmetic saturates at: the run
    // reaching this length is exactly when `discount == max_direction_discount`.
    let at_clamp =
        DIRECTION_DISCOUNT_PER_OBSERVATION * (params::direction_run_at_max_discount() - 1) as f64;
    assert!(
        at_clamp >= params::max_direction_discount(),
        "the clamp must be reached no earlier than the discount ceiling, else the last \
         observations of a run buy nothing: clamp reaches {at_clamp}, ceiling {}",
        params::max_direction_discount()
    );
}

/// An unparsable override keeps the default; a parsable one is taken; whitespace is tolerated.
///
/// The failure mode being guarded is silent: `VARDIFF_EWMA_TAU_SECS=360s` (with the unit) does not
/// parse as `u64`, and if that returned `0` or panicked-then-defaulted without saying so, the arm
/// would run at 1200 while the deploy record said 360. Reachable in every build because
/// `parse_or_default` is not itself feature-gated — a guard behind the same `cfg` as the code it
/// guards is a guard the default test run never executes.
#[test]
fn an_unparsable_override_keeps_the_default() {
    use crate::vardiff::classic::params::parse_or_default;

    assert_eq!(
        parse_or_default("VARDIFF_EWMA_TAU_SECS", "360", 1200u64),
        360
    );
    assert_eq!(
        parse_or_default("VARDIFF_EWMA_TAU_SECS", "  360\n", 1200u64),
        360
    );
    assert_eq!(
        parse_or_default("VARDIFF_EWMA_TAU_SECS", "360s", 1200u64),
        1200,
        "a value carrying its unit must not be read as a number"
    );
    assert_eq!(parse_or_default("VARDIFF_EWMA_TAU_SECS", "", 1200u64), 1200);
    assert_eq!(
        parse_or_default("VARDIFF_UNCERTAINTY_FLOOR_WEIGHT", "0", 0.5f64),
        0.0,
        "zero is a legitimate override — it is the unfed arm — and must not be read as absent"
    );
}

/// The parameter line's schedule: emit at start, once an hour after, and again if the clock moves
/// backwards.
///
/// The failure this guards against is the one already observed on the rig — the fingerprint gone
/// from `docker logs` while the arm was still running, leaving the identity check with nothing to
/// read. The schedule is asserted rather than the emission because the emit is rate-limited through
/// a process-wide atomic that no test can put into a known state.
#[test]
fn the_parameter_line_is_due_at_start_hourly_and_after_a_backwards_clock_step() {
    use crate::vardiff::classic::FINGERPRINT_REEMIT_SECS;

    assert!(
        VardiffState::fingerprint_due(0, 1_000_000),
        "never emitted must be due, whatever the clock says"
    );
    assert!(
        !VardiffState::fingerprint_due(1_000_000, 1_000_000 + FINGERPRINT_REEMIT_SECS - 1),
        "one second short of the interval must not re-emit, or the rate limit is not one"
    );
    assert!(VardiffState::fingerprint_due(
        1_000_000,
        1_000_000 + FINGERPRINT_REEMIT_SECS
    ));
    assert!(
        VardiffState::fingerprint_due(1_000_000, 999_000),
        "a backwards clock step must re-emit; otherwise `last` sits in the future and the line \
         is suppressed for the length of the step"
    );
}
