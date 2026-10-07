//! An audit of the measuring instruments, in the modes they are actually used in.
//!
//! # Why this module exists
//!
//! Both defects found on this branch were in the **instrument**, not the controller: a share sampler that
//! fed a constant, and a record that claimed a guard it did not have. Three review rounds went over the
//! controller's logic and neither reviewer asked whether the shares were real. So the harness got less
//! adversarial scrutiny than the thing it measures, which is the wrong way round -- every conclusion is
//! downstream of the instrument and none is downstream of the reviewer's attention.
//!
//! # The rule this module applies
//!
//! **For each instrument, name the mode it is used in, and test that mode.**
//!
//! That is not a generality; it is the specific thing that would have caught the sampler. Two self-checks
//! existed and both drove a *persistent* generator, while every trajectory constructs a fresh one per
//! tick. They validated a configuration that never runs. `the_share_feed_responds_to_the_difficulty_in_force`
//! was **still** doing that when this audit began, after the whole episode -- the lesson had been written
//! down and not fully applied. It now seeds per tick.
//!
//! Mutation testing cannot substitute. It proves an assertion would notice a change in the code, and is
//! orthogonal to whether the input exercises the code at all. Every mutation on this branch was compared
//! against the same dead feed and they all agreed with each other.
//!
//! # What the audit found
//!
//! Nothing broken. Every instrument below is correct, and that is the point worth stating plainly: both
//! real defects were in instruments that *looked* fine, so "it looks fine" is not the standard. What
//! changes here is that these move from unvalidated to pinned, in the mode that matters.
//!
//! Still uncovered, and named rather than silently omitted: the rig sampler and the hardware pipeline,
//! which are outside this crate and where the pre-registered experiment's admissibility gate applies
//! instead.

use super::trajectory::{
    constant, declining, intermittent, median_for_audit, mis_declared, run_trial, sample_poisson,
    sample_standard_normal_for_audit, sample_tick, swinging, trial_seed, Arm, Wander, XorShift64,
    SEED, START_HASHRATE, TICK_SECS,
};
use crate::target::{hash_rate_from_target, hash_rate_to_target};

/// Draws per statistical check. Enough to resolve a 4th moment.
const DRAWS: u64 = 40_000;

/// **The normal draw, in the per-tick mode `Wander` uses.**
///
/// `sample_standard_normal` feeds two consumers -- `sample_poisson`'s large-mean branch and every `Wander`
/// step -- and was validated by neither. Kurtosis is the load-bearing moment here: mean and spread alone
/// would pass a uniform draw (kurtosis 1.8) or a two-point distribution, and Box-Muller's whole job is to
/// turn uniforms into something with normal tails.
#[test]
fn the_normal_draw_is_normal_in_the_per_tick_mode() {
    let draws: Vec<f64> = (1..=DRAWS)
        .map(|k| sample_standard_normal_for_audit(&mut XorShift64::new(SEED ^ (TICK_SECS * k))))
        .collect();
    let n = draws.len() as f64;
    let mean = draws.iter().sum::<f64>() / n;
    let sd = (draws.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    let kurtosis = draws.iter().map(|x| ((x - mean) / sd).powi(4)).sum::<f64>() / n;

    eprintln!(
        "  [instrument] normal draw, per-tick: mean {mean:.4}, sd {sd:.4}, kurtosis {kurtosis:.3}"
    );

    assert!(mean.abs() < 0.02, "mean should be 0; got {mean:.4}");
    assert!((sd - 1.0).abs() < 0.02, "sd should be 1; got {sd:.4}");
    assert!(
        (kurtosis - 3.0).abs() < 0.2,
        "kurtosis should be 3; got {kurtosis:.3}. A uniform draw gives 1.8 and would pass the mean and \
         spread checks above, which is why this moment is the one that matters"
    );
}

/// **The generator's stream mode -- draws beyond the first.**
///
/// `the_generator_mixes_consecutive_seeds` covers the *first* draw from each per-tick seed, because that is
/// where the defect was. But `sample_poisson`'s Knuth loop draws repeatedly from one generator, so the
/// second draw onward is a second mode with its own exposure, and it was uncovered.
#[test]
fn the_generator_is_uniform_in_stream_mode() {
    let mut rng = XorShift64::new(SEED);
    let draws: Vec<f64> = (0..DRAWS).map(|_| rng.next_f64_for_audit()).collect();
    let mean = draws.iter().sum::<f64>() / draws.len() as f64;

    let mut buckets = [0usize; 10];
    for d in &draws {
        buckets[((d * 10.0) as usize).min(9)] += 1;
    }
    let expected = draws.len() as f64 / 10.0;
    let chi2: f64 = buckets
        .iter()
        .map(|&c| (c as f64 - expected).powi(2) / expected)
        .sum();

    eprintln!("  [instrument] stream mode: mean {mean:.4}, chi2 {chi2:.1} over 10 buckets");
    assert!(
        (mean - 0.5).abs() < 0.01,
        "stream-mode draws should average 0.5; got {mean:.4}"
    );
    assert!(
        chi2 < 16.9,
        "and be uniform: chi2 {chi2:.1} against 16.9 for 9 degrees of freedom at p=0.05"
    );
    // Draws must be in (0, 1]: a zero would make Box-Muller's `ln(u1)` infinite.
    assert!(
        draws.iter().all(|&d| d > 0.0 && d <= 1.0),
        "every draw must lie in (0, 1], or the normal draw's ln(u1) is unbounded"
    );
}

/// **The Poisson sampler is continuous across its own Knuth/normal seam.**
///
/// `sample_poisson` switches algorithm at `KNUTH_LIMIT = 30`, and a trajectory whose delivered rate crosses
/// that boundary changes sampling method mid-run. `both_poisson_branches_reproduce_their_mean` checks the
/// mean on each side; it does not check that the *distribution* is continuous through the seam, which is
/// what a mid-run switch actually needs.
#[test]
fn the_poisson_sampler_is_continuous_across_its_seam() {
    for lambda in [28.0f64, 29.9, 30.0, 30.1, 32.0] {
        let counts: Vec<f64> = (1..=DRAWS)
            .map(|k| sample_tick(SEED ^ lambda.to_bits(), TICK_SECS * k, lambda) as f64)
            .collect();
        let n = counts.len() as f64;
        let mean = counts.iter().sum::<f64>() / n;
        let variance = counts.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / n;

        eprintln!(
            "  [instrument] poisson at lambda {lambda}: mean {mean:.3}, var/mean {:.4}",
            variance / mean
        );
        assert!(
            (mean / lambda - 1.0).abs() < 0.02,
            "mean should track lambda across the seam; {mean:.3} against {lambda}"
        );
        assert!(
            (variance / mean - 1.0).abs() < 0.05,
            "and so should the variance, or the two branches deliver different statistics and a \
             trajectory crossing lambda = 30 changes its noise model mid-run: var/mean {:.4} at \
             lambda {lambda}",
            variance / mean
        );
    }
}

/// **The trajectory generators deliver what their names say.**
///
/// Every measurement in the suite is a statement about a trajectory, and the trajectory functions
/// themselves were never checked. A `declining` that floored at the wrong place, or an `intermittent` off
/// by a phase, would shift every reading built on it with nothing to catch it.
#[test]
fn the_trajectory_generators_deliver_what_they_claim() {
    let start = START_HASHRATE as f64;

    // Constant: flat forever.
    let c = constant();
    assert_eq!(c(0), start);
    assert_eq!(c(100 * 3600), start, "constant must not drift");

    // Declining: `rate_pct_per_hr` per hour, floored at `total_drop`.
    let d = declining(5.0, 0.20);
    for (elapsed, expected_fraction) in [(0u64, 1.0f64), (3600, 0.95), (4 * 3600, 0.80)] {
        let got = d(elapsed) / start;
        assert!(
            (got - expected_fraction).abs() < 1e-9,
            "declining(5, 0.20) at {elapsed}s should deliver {expected_fraction} of start; got {got}"
        );
    }
    assert!(
        (d(100 * 3600) / start - 0.80).abs() < 1e-9,
        "and must hold at the floor rather than continuing past it"
    );

    // Intermittent: dark for the first `down_secs` of each period, live for the rest, and periodic.
    let i = intermittent(1800, 900);
    assert_eq!(i(0), 0.0, "dark at the start of a cycle");
    assert_eq!(i(899), 0.0, "dark up to the boundary");
    assert_eq!(i(900), start, "live from the boundary");
    assert_eq!(i(1799), start, "live to the end of the cycle");
    assert_eq!(i(1800), 0.0, "and periodic");

    // Mis-declared: a flat multiple of the opening belief, which is what makes it a cold start.
    assert!((mis_declared(10.0)(0) / start - 10.0).abs() < 1e-9);
    assert!((mis_declared(0.01)(0) / start - 0.01).abs() < 1e-9);

    // Swinging: alternates between full and half every half period.
    let s = swinging(900);
    assert_eq!(s(0), start, "starts high");
    assert_eq!(s(900), start * 0.5, "half after one half-period");
    assert_eq!(s(1800), start, "and back");
}

/// **The belief-target round trip is exact, which every estimate depends on.**
///
/// `estimate` recovers a hashrate with `hash_rate_from_target` from a target the caller built with
/// `hash_rate_to_target`. If those are not mutual inverses, every estimate carries a systematic bias --
/// and it would look like a controller offset rather than an instrument error, which is the hardest kind
/// of defect to attribute. `target.rs`'s own tests cover the error cases and not the round trip.
///
/// Measured exact (zero relative error) over six orders of magnitude and the whole rate band, so this is a
/// coverage gap closed rather than a bug found.
#[test]
fn the_belief_target_round_trip_is_exact() {
    for spm in [2.0f64, 6.0, 12.0, 30.0, 60.0] {
        for hashrate in [1.0e9f64, 1.0e10, 1.0e11, 1.0e12, 1.0e13, 1.0e14, 1.0e15] {
            let target = hash_rate_to_target(hashrate, spm).expect("positive inputs convert");
            let back = hash_rate_from_target(target.to_le_bytes().into(), spm)
                .expect("a constructed target inverts");
            let relative = (back / hashrate - 1.0).abs();
            assert!(
                relative < 1e-9,
                "hash_rate_to_target and hash_rate_from_target must invert: {hashrate:.3e} H/s at \
                 {spm} spm came back as {back:.3e}, off by {relative:.3e}. A bias here reads as a \
                 controller offset, which is the hardest kind of defect to attribute"
            );
        }
    }
}

/// **The sampler's degenerate inputs are handled, because a controller under test will produce them.**
///
/// A non-finite or non-positive mean arises from a degenerate belief, which is behaviour under test rather
/// than a harness error -- so the sampler must absorb it instead of panicking or returning nonsense. The
/// large-mean ceiling matters for the same reason: the caller loops over the result, so an unbounded count
/// would hang the run rather than report the degenerate belief that caused it.
#[test]
fn the_sampler_absorbs_the_degenerate_means_a_controller_can_produce() {
    for lambda in [0.0f64, -1.0, f64::NAN, f64::NEG_INFINITY] {
        let mut rng = XorShift64::new(SEED);
        assert_eq!(
            sample_poisson(&mut rng, lambda),
            0,
            "a degenerate mean of {lambda} must yield no arrivals rather than panicking -- it comes from \
             a belief under test, not from a harness fault"
        );
    }
    // A vast but finite mean must be bounded, since the caller iterates over the count.
    let mut rng = XorShift64::new(SEED);
    let huge = sample_poisson(&mut rng, 1.0e12);
    assert!(
        huge <= 1_000_000,
        "a vast mean must be clamped so establishing a degenerate belief costs no more than a tick; got \
         {huge}"
    );
}

/// **The aggregation conventions, which are choices rather than facts and were never written down.**
///
/// `measure_arm` reports the **median across trials of each per-trial reading**, which is not the pooled
/// median of all ticks and does not claim to be. On a slow decline the retarget count is bimodal -- most
/// trials never adjust and a minority adjust many times -- and a mean over those two populations describes
/// neither, which is the stated reason for using a median at all. But median-of-medians has its own
/// property worth pinning: it is robust to a minority of divergent trials and therefore *hides* them, so a
/// cell where a third of trials behave completely differently reads as if none did.
///
/// Pinned here so the convention is explicit. A reading is a statement about the typical trial, never about
/// the worst one.
#[test]
fn the_aggregator_reports_a_median_and_hides_a_minority() {
    // Even count: the mean of the two middle values.
    assert_eq!(median_for_audit(vec![1.0, 2.0, 3.0, 4.0]), 2.5);
    // Odd count: the middle value.
    assert_eq!(median_for_audit(vec![1.0, 2.0, 3.0]), 2.0);
    // Unsorted input must be sorted first, or every reading depends on trial order.
    assert_eq!(median_for_audit(vec![9.0, 1.0, 5.0]), 5.0);

    // The property that matters and is easy to forget: a large minority is invisible.
    let mut mostly_small = vec![1.0; 12];
    mostly_small.extend(vec![1000.0; 8]);
    assert_eq!(
        median_for_audit(mostly_small),
        1.0,
        "40% of trials diverging by three orders of magnitude leaves the median untouched. That is the \
         convention working as intended, and it is why a median reading must not be read as a bound"
    );
}

/// **The percentile index convention, which is off-by-one-prone and silently so.**
///
/// `p95_error_pct` is `errors[len * 95 / 100]` on a sorted vector -- integer arithmetic, floor, zero-based.
/// For the 12-hour runs this suite uses that lands within one sample of the 95th percentile, but the
/// convention has never been stated and the failure mode is a quiet one: an off-by-one here shifts every
/// p95 figure in the suite by one sample and nothing would look wrong.
#[test]
fn the_percentile_index_convention_is_floor_and_zero_based() {
    // 100 samples, values 0..99 sorted: index 95 -> value 95.
    let sorted: Vec<f64> = (0..100).map(|v| v as f64).collect();
    assert_eq!(sorted[sorted.len() * 95 / 100], 95.0);
    // 720 samples, as a 12-hour run at one tick a minute: index 684.
    assert_eq!(720 * 95 / 100, 684, "floor, not round: 684 of 720, not 685");
    // And the median index is the same convention, which is why an even-length run's "median" here is the
    // upper of the two middle samples rather than their mean -- unlike `median` above. The two differ, and
    // that difference is deliberate: per-tick readings are indexed, per-trial readings are interpolated.
    assert_eq!(sorted[sorted.len() / 2], 50.0);
}

/// **A trial that never converges is counted as the whole span, not dropped.**
///
/// `measure_arm` maps a `None` convergence time to `f64::MAX` before taking the median, so a cell where
/// most trials never converge reads as `usize::MAX` rather than as the fast time of the minority that did.
/// Dropping them instead would report "converged in 14 ticks" for a cell that mostly does not converge,
/// which is the more dangerous error -- and it is exactly the error the retracted cold-start lock finding
/// would have produced in reverse.
///
/// Consumers must therefore filter the sentinel rather than trust the value, which `cold_start.rs` does.
///
/// **This test found a real defect on its first run**, which is the audit paying for itself. The
/// aggregator computed an even-length midpoint as `(a + b) / 2.0`, so two `f64::MAX` middle values
/// overflowed to `inf` rather than staying at `MAX`. It was harmless -- the caller's `as usize`
/// saturates `inf` to the same `usize::MAX` the sentinel wanted -- but harmless *by accident*, and a
/// sentinel that survives only because of a saturating cast is not a sentinel. `median` now uses the
/// stable `a + (b - a) / 2` form.
#[test]
fn a_trial_that_never_converges_is_counted_not_dropped() {
    // Majority never converge: the median must be the sentinel, not the minority's time.
    let mut mostly_never: Vec<f64> = vec![f64::MAX; 12];
    mostly_never.extend(vec![14.0; 8]);
    assert_eq!(
        median_for_audit(mostly_never),
        f64::MAX,
        "a cell where 60% of trials never converge must not report the 40%'s time -- that is the shape of \
         error the retracted lock finding made in reverse"
    );

    // Majority do converge: the sentinel must not drag the reading away from them.
    let mut mostly_converge: Vec<f64> = vec![14.0; 12];
    mostly_converge.extend(vec![f64::MAX; 8]);
    assert_eq!(median_for_audit(mostly_converge), 14.0);
}

/// **`final_error_pct` is not fit to be compared against an absolute threshold at these trial counts.**
///
/// The suite's decline-safety readings are `final_error_pct` medians over 40 trials, checked against a `5 %`
/// gate on over-difficulty. That comparison assumes the reading resolves the threshold, and at the corner
/// that fixes the memory -- a `10 %/hr` decline with hot wander -- it does not.
///
/// Measured over 2560 trials of the binding cell, with the standard error obtained by **block resampling**
/// (medians of disjoint 40-trial blocks) rather than by re-running windows, so it is the estimator's own
/// spread and not a proxy for it:
///
/// | statistic | per-trial sd | median | SE of a 40-trial median | relative |
/// |---|---|---|---|---|
/// | `final_error_pct` | `10.21 pp` | `+1.85` | `2.16 pp` | **`117 %`** |
/// | `worst_over_pct` | `4.86 pp` | `+23.26` | `0.84 pp` | `3.6 %` |
///
/// Sixty-four blocks estimate a standard error to about a tenth of itself: twelve independent samples of
/// 2560 trials put the first at `1.57-2.43 pp`, `2.00` on average, and the second at `0.75-1.06 pp`, `0.93`
/// on average.
///
/// The per-trial spread *exceeds the gate itself*, so one trial cannot resolve it at all, and the 40-trial
/// median still carries a third of the gate or more as noise.
///
/// **The `117 %` against `3.6 %` comparison flatters `worst_over_pct` and should not be leaned on.** Relative
/// error divides by the mean, and these two quantities have very different means (`+1.85` against `+23.26`),
/// so any ratio of relative errors favours the larger one. Against an *absolute* `5 pp` gate what matters is
/// absolute standard error, and there the advantage is `2.159` against `0.841` -- a factor of `2.57`, not
/// `32`, and `1.75-2.62x` across the twelve samples, `2.1x` on their averages.
///
/// The argument that does survive is **phase stability**, and it is not a mean artefact: across eight adjacent
/// window lengths the 30 spm reading moved `2.83 pp` on `final_error_pct` and `0.43 pp` on `worst_over_pct`. A
/// max over the span is already an extreme-value statistic and self-averages the within-run phase that a
/// single-instant reading samples. Its own limitation, stated rather than discovered later: a running max is
/// monotone in window length, so it cannot be compared across runs of different duration.
///
/// # Why this is an instrument finding and not a tuning note
///
/// Three separate hypotheses for the instability died on measurement before this one survived, and each is
/// recorded because the dead ones are what make the surviving explanation credible:
///
/// * **common-mode phase** -- that trials share the deterministic trajectory, so the median cannot average
///   the retarget phase out. False: the swing falls as `1/sqrt(n)` across 10, 40, 160 and 640 trials.
/// * **right skew** -- that a small-sample median sits above the population's. False: skew is `-0.25`, the
///   wrong sign for it, and negative in all twelve samples.
/// * **systematic bias with trial count** -- read into four noisy decreasing points. False: the same
///   `1/sqrt(n)` scaling explains them, and the 640-trial median is `+1.93`.
///
/// What remains is ordinary sampling error, which is the dullest explanation and the correct one.
///
/// **The consequence is not dull**, though it is narrower than the first version of this note claimed.
/// `worst_over_pct` is the reproducible statistic and should carry any *comparison between arms*. But the
/// crate's stated gate is a bound on the settled position, and swapping the quantity a criterion is applied
/// to is not a fix -- `tau_split.rs` records a conclusion this note's first version helped justify, where an
/// unbounded excursion comparison was used to refuse a configuration that passes the actual gate. Improving a
/// reading's precision and changing what it measures are different moves, and only the first is an
/// instrument decision.
///
/// # How far the damage reaches, checked rather than assumed
///
/// Every comparison of a `final_error_pct` median against an absolute threshold in the suite was re-measured
/// at 400 trials against its own file's count. **Two were underpowered and both are fixed; the rest are
/// sound**, which bounds the finding rather than leaving it as a general worry:
///
/// | pattern | where | powered reading | verdict |
/// |---|---|---|---|
/// | gate crossing count | `wander.rs` headroom | 1 or 3 of 4 at 40 trials by seed, **1 of 4** at 400 | fixed -- now asserts the excursion at every rate |
/// | gate crossing, two cells | `tau_split.rs` at the crossing memory | gap `+5.57` at 40, `+4.74` at 400; at 40 one seed set in twelve puts the dense cell inside | fixed -- now at 400 |
/// | sign of settled error | `outage.rs` flapping, 8 cells | `-12.07` to `-100.62`, stable | sound |
/// | sign of settled error | `cold_start.rs` under-declared, 4 cells | `-6.16` to `-19.49`, stable | sound |
/// | ratio of two medians | `cold_start.rs` sparse/dense offset | `4.75x` at 20 trials, `3.17x` at 400 | sound; the *figure* was stale and is corrected |
/// | `\|error\|` against the controller's own bar | `regimes.rs`, two sites | tightest cells `4.7x` and `6.3x` inside it | sound |
///
/// The pattern that fails is specific: a **count of near-threshold binary decisions**, where each decision
/// has a fraction of a standard error of margin and the count is then compared to an integer. Sign tests and
/// ratios survive because their margins are multiples of the spread rather than fractions of it. A ratio of
/// two medians is the one shape that moved materially while still clearing its threshold.
#[test]
fn the_settled_reading_is_underpowered_against_an_absolute_gate() {
    /// Trials to characterise the estimator. Sixty-four disjoint blocks of 40, which is the trial count the
    /// suite's gate readings use. At sixteen, twelve independent samples put the standard error anywhere
    /// from `24 %` to `61 %` of the gate.
    const TRIALS: u64 = 2560;
    /// Block size, matching the suite's own.
    const BLOCK: usize = 40;
    /// The gate these readings are compared against, in percent of over-difficulty.
    const GATE_PCT: f64 = 5.0;
    /// Least fraction of the gate the 40-trial standard error must reach for this finding to hold. Measured
    /// about `43 %`, and `31-49 %` across twelve independent samples.
    const MIN_SE_FRACTION_OF_GATE: f64 = 0.15;
    /// Least advantage in *absolute* standard error `worst_over_pct` must show. Measured `2.159` against
    /// `0.841`, so `2.57x`, and `1.75-2.62x` across twelve independent samples. Set at parity, because the
    /// claim is the direction -- that the excursion is the less noisy reading against an absolute gate --
    /// and not its size. Deliberately not the relative-error ratio, which reads `32x` only because the two
    /// quantities have different means -- see the note above.
    const MIN_ABSOLUTE_ADVANTAGE: f64 = 1.0;

    let runs: Vec<_> = (0..TRIALS)
        .map(|i| {
            run_trial(
                &declining(10.0, 0.40),
                30.0,
                3600,
                3 * 3600,
                trial_seed(i),
                Arm::Shipped,
                Some(Wander::HOT_AIR),
            )
        })
        .collect();

    let spread = |values: &[f64]| {
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt()
    };
    // Standard error of a 40-trial median, from the spread of disjoint 40-trial block medians.
    let characterise = |values: Vec<f64>| {
        let blocks: Vec<f64> = values
            .chunks(BLOCK)
            .map(|c| median_for_audit(c.to_vec()))
            .collect();
        let (sd, median, se) = (spread(&values), median_for_audit(values), spread(&blocks));
        (sd, median, se, se / median.abs())
    };

    let (final_sd, final_median, final_se, final_relative) =
        characterise(runs.iter().map(|r| r.final_error_pct).collect());
    let (worst_sd, worst_median, worst_se, worst_relative) =
        characterise(runs.iter().map(|r| r.worst_over_pct).collect());

    eprintln!(
        "  [instrument] final_error_pct: per-trial sd {final_sd:.2}, median {final_median:+.2}, SE of a \
         {BLOCK}-trial median {final_se:.3} pp ({:.1}% relative)",
        final_relative * 100.0
    );
    eprintln!(
        "  [instrument] worst_over_pct:  per-trial sd {worst_sd:.2}, median {worst_median:+.2}, SE of a \
         {BLOCK}-trial median {worst_se:.3} pp ({:.1}% relative)",
        worst_relative * 100.0
    );

    assert!(
        final_sd > GATE_PCT,
        "the per-trial spread should exceed the gate itself, which is what makes a single trial unable to \
         resolve it: sd {final_sd:.2} pp against a {GATE_PCT} pp gate"
    );
    assert!(
        final_se > GATE_PCT * MIN_SE_FRACTION_OF_GATE,
        "and a {BLOCK}-trial median should still carry a substantial fraction of the gate as noise: SE \
         {final_se:.3} pp against {:.2} pp, which is {:.0}% of it. If this ever fails the readings have \
         become powerful enough to compare directly and this finding is obsolete",
        GATE_PCT * MIN_SE_FRACTION_OF_GATE,
        MIN_SE_FRACTION_OF_GATE * 100.0
    );
    assert!(
        final_se > worst_se * MIN_ABSOLUTE_ADVANTAGE,
        "and the excursion statistic should carry less absolute noise, which is the comparison that bears on \
         an absolute gate: SE {final_se:.3} pp for final_error_pct against {worst_se:.3} pp for \
         worst_over_pct, a factor of {:.2}x against the {MIN_ABSOLUTE_ADVANTAGE}x asserted. The relative \
         figures ({:.1}% against {:.1}%) are reported but not asserted on, because they differ mostly \
         through the means",
        final_se / worst_se,
        final_relative * 100.0,
        worst_relative * 100.0
    );
}
