//! A harness for measuring the controller over time, and the measurements that need one.
//!
//! Every test outside this module pins a mechanism: given this state and this input, the controller
//! does that. Those are the right tests for a mechanism and they are not enough for this controller,
//! because its failures are properties of a trajectory. Difficulty stays where it was last set, so a
//! channel whose hashrate falls is over-difficulty; fewer shares arrive, less evidence accumulates,
//! and the correction that would bring difficulty back down is slower for exactly that reason.
//! Whether the controller escapes that loop cannot be read off a single evaluation.
//!
//! Two things make the harness worth trusting rather than merely running.
//!
//! Shares are sampled, not supplied. The mean is `spm * delivered / believed`, which is the rate a
//! miner delivering `delivered` produces against a difficulty set for `believed`. That coupling *is*
//! the self-reinforcing loop, so a harness that fed the target rate regardless of the difficulty in
//! force would make any controller look safe. `the_share_feed_responds_to_the_difficulty_in_force`
//! establishes that this one does not.
//!
//! Figures are medians across trials rather than means. The retarget count is bimodal on a slow
//! decline -- most trials never adjust and a minority adjust many times, depending on whether an
//! early window happens to cross the threshold -- and a mean of those two populations describes
//! neither.

use super::{TEST_MIN_ALLOWED_HASHRATE, TEST_SHARES_PER_MINUTE};
use crate::{
    target::hash_rate_to_target,
    vardiff::{classic::VardiffState, clock::MockClock, Vardiff},
};
use std::sync::Arc;

/// Evaluation interval.
pub(super) const TICK_SECS: u64 = 60;
/// The on-target hashrate a trajectory starts from.
pub(super) const START_HASHRATE: f32 = 1.0e12;
/// Fixed seed base. Every figure a test asserts on comes from it.
pub(super) const SEED: u64 = 0x5EED_1234_5678_9ABC;
/// Clock origin, well clear of zero so a backwards step has room.
const CLOCK_ORIGIN: u64 = 1_000_000;

/// XorShift64, so sampling is reproducible without a dev-dependency.
pub(super) struct XorShift64(u64);

impl XorShift64 {
    /// Seeds the generator, substituting for the one value it cannot use.
    ///
    /// Zero is a fixed point of this recurrence: a generator started there stays there and returns
    /// the same value forever, which `sample_poisson` reads as no arrivals at any rate. A trial fed
    /// by it would report a channel that never submits whatever the difficulty in force.
    ///
    /// Only that one value is replaced. Forcing a bit instead -- `seed | 1` -- would map every even
    /// seed onto its odd neighbour, and trials are numbered with consecutive seeds, so each trial
    /// would duplicate the one beside it and every cell would have half the sample size it reports.
    pub(super) fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_f64(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        ((x >> 11) as f64 + 1.0) / ((1u64 << 53) as f64 + 1.0)
    }
}

/// Knuth's method below `KNUTH_LIMIT`, a normal approximation above it.
///
/// Both branches are reachable. The mean is `spm * delivered / believed`, which stays near `spm`
/// while the controller tracks and grows without bound when it drives its belief below the delivered
/// rate. Knuth's cost is linear in the mean, so the second branch bounds cost as well as error.
const KNUTH_LIMIT: f64 = 30.0;

/// Ceiling on one interval's sampled arrivals, because the caller loops over the result. A mean this
/// large only arises from a degenerate belief, and establishing that must not cost more than a tick.
const MAX_SAMPLED_SHARES: u32 = 1_000_000;

pub(super) fn sample_poisson(rng: &mut XorShift64, lambda: f64) -> u32 {
    // A non-finite mean would reach the normal branch and produce a meaningless count; a zero one has
    // no shares to draw. Both arise from a degenerate belief, which is behaviour under test.
    if !lambda.is_finite() || lambda <= 0.0 {
        return 0;
    }
    if lambda < KNUTH_LIMIT {
        let limit = (-lambda).exp();
        let mut k = 0u32;
        let mut p = 1.0;
        while p > limit {
            p *= rng.next_f64();
            k += 1;
        }
        // A mean small enough that `exp(-lambda)` rounds to 1.0 exits before the first multiply,
        // leaving `k` at zero, and an unchecked decrement would read as four billion arrivals.
        return k.saturating_sub(1);
    }
    let (u1, u2) = (rng.next_f64(), rng.next_f64());
    let z = (-2.0 * u1.ln()).sqrt() * (core::f64::consts::TAU * u2).cos();
    let raw = lambda + lambda.sqrt() * z;
    if raw > 0.0 {
        raw.min(MAX_SAMPLED_SHARES as f64).round() as u32
    } else {
        0
    }
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("no NaN readings"));
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// The target a caller sets for a belief. Floored, because a controller under test may emit a
/// degenerate value and that must not become a panic in the harness.
pub(super) fn target_for(hashrate: f32, spm: f32) -> bitcoin::Target {
    hash_rate_to_target(hashrate.max(1.0) as f64, spm.max(0.001) as f64)
        .expect("positive inputs convert")
}

/// One trajectory, and what the controller did on it.
pub(super) struct Trial {
    /// Median absolute distance from the delivered rate, in percent, over the measured span.
    pub(super) median_error_pct: f64,
    /// The same at the 95th percentile, which is the figure a miner would notice.
    pub(super) p95_error_pct: f64,
    /// Signed distance at the last evaluation. Positive is over-difficulty.
    pub(super) final_error_pct: f64,
    /// Worst over-difficulty reached at any point.
    pub(super) worst_over_pct: f64,
    /// Difficulty changes over the measured span.
    pub(super) retargets: usize,
    /// Shares delivered over the measured span, which is the harness's own output.
    pub(super) shares_fed: u64,
    /// Where difficulty ended up.
    pub(super) final_hashrate: f32,
    /// The decision threshold in force at the last evaluation, in percent, read from the controller
    /// rather than recomputed.
    ///
    /// The controller does not act below its threshold, so the settled position is inside this
    /// number. Tests that want to reason about settled error need it, and recomputing the formula
    /// here would let the copy drift from the implementation being checked.
    pub(super) final_threshold_pct: f64,
}

/// Drives one trajectory: the miner delivers whatever `delivered` says, the feed samples the share
/// count that implies against the difficulty in force, and each returned belief is threaded back in.
///
/// `warm_up_secs` of the run are driven but not measured, so a trajectory can be given a settled
/// starting point without the settling appearing in the figures.
pub(super) fn run_trial(
    delivered: &impl Fn(u64) -> f64,
    spm: f32,
    warm_up_secs: u64,
    measured_secs: u64,
    seed: u64,
) -> Trial {
    let clock = Arc::new(MockClock::new(CLOCK_ORIGIN));
    let mut vardiff = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("a positive floor and a mock clock construct");

    let mut believed = START_HASHRATE;
    let mut elapsed = 0;
    let mut retargets = 0;
    let mut shares_fed = 0u64;
    let mut errors: Vec<f64> = Vec::new();
    let mut worst_over_pct = f64::MIN;
    let mut final_error_pct = 0.0;
    let mut final_threshold_pct = f64::NAN;

    while elapsed < warm_up_secs + measured_secs {
        elapsed += TICK_SECS;
        clock.advance(TICK_SECS);

        let delivered_now = delivered(elapsed);
        let lambda = (delivered_now / believed as f64) * spm as f64 * (TICK_SECS as f64 / 60.0);
        let shares = sample_poisson(&mut XorShift64::new(seed.wrapping_add(elapsed)), lambda);
        for _ in 0..shares {
            vardiff.increment_shares_since_last_update();
        }

        let measuring = elapsed > warm_up_secs;
        // Read against the belief that was in force for this interval, which is the one the shares
        // above were produced against.
        let error_pct = (believed as f64 / delivered_now).ln() * 100.0;
        if measuring {
            shares_fed += shares as u64;
            errors.push(error_pct.abs());
            worst_over_pct = worst_over_pct.max(error_pct);
            final_error_pct = error_pct;
        }

        let target = target_for(believed, spm);
        let fired = vardiff.try_vardiff(believed, &target, spm);
        if measuring {
            // After the call, so it is the threshold this evaluation used.
            final_threshold_pct = vardiff.last_threshold_pct().unwrap_or(f64::NAN);
        }
        if let Ok(Some(next)) = fired {
            believed = next;
            if measuring {
                retargets += 1;
            }
        }
    }

    errors.sort_by(|a, b| a.partial_cmp(b).expect("no NaN readings"));
    Trial {
        median_error_pct: errors[errors.len() / 2],
        p95_error_pct: errors[errors.len() * 95 / 100],
        final_error_pct,
        worst_over_pct,
        retargets,
        shares_fed,
        final_hashrate: believed,
        final_threshold_pct,
    }
}

/// Medians of each reading across `trials` trajectories that differ only in seed.
pub(super) fn measure(
    delivered: &impl Fn(u64) -> f64,
    spm: f32,
    warm_up_secs: u64,
    measured_secs: u64,
    trials: usize,
) -> Trial {
    let runs: Vec<Trial> = (0..trials)
        .map(|i| {
            run_trial(
                delivered,
                spm,
                warm_up_secs,
                measured_secs,
                SEED.wrapping_add(i as u64),
            )
        })
        .collect();
    let pick = |values: Vec<f64>| median(values);
    Trial {
        median_error_pct: pick(runs.iter().map(|r| r.median_error_pct).collect()),
        p95_error_pct: pick(runs.iter().map(|r| r.p95_error_pct).collect()),
        final_error_pct: pick(runs.iter().map(|r| r.final_error_pct).collect()),
        worst_over_pct: pick(runs.iter().map(|r| r.worst_over_pct).collect()),
        retargets: pick(runs.iter().map(|r| r.retargets as f64).collect()) as usize,
        shares_fed: pick(runs.iter().map(|r| r.shares_fed as f64).collect()) as u64,
        final_hashrate: pick(runs.iter().map(|r| r.final_hashrate as f64).collect()) as f32,
        final_threshold_pct: pick(runs.iter().map(|r| r.final_threshold_pct).collect()),
    }
}

/// A miner that never changes.
pub(super) fn constant() -> impl Fn(u64) -> f64 {
    |_elapsed| START_HASHRATE as f64
}

/// A miner losing `rate_pct_per_hr` of its hashrate each hour, floored at `total_drop`.
pub(super) fn declining(rate_pct_per_hr: f64, total_drop: f64) -> impl Fn(u64) -> f64 {
    move |elapsed| {
        let lost = ((rate_pct_per_hr / 100.0) * (elapsed as f64 / 3600.0)).min(total_drop);
        START_HASHRATE as f64 * (1.0 - lost)
    }
}

/// A miner that stops delivering entirely after `silent_after_secs`.
pub(super) fn goes_silent(silent_after_secs: u64) -> impl Fn(u64) -> f64 {
    move |elapsed| {
        if elapsed > silent_after_secs {
            0.0
        } else {
            START_HASHRATE as f64
        }
    }
}

/// A miner alternating between its hashrate and half of it every `half_period_secs`.
pub(super) fn swinging(half_period_secs: u64) -> impl Fn(u64) -> f64 {
    move |elapsed| {
        let low = (elapsed / half_period_secs) % 2 == 1;
        START_HASHRATE as f64 * if low { 0.5 } else { 1.0 }
    }
}

/// The share feed must respond to the difficulty in force, because that coupling is the effect every
/// measurement here depends on.
///
/// Stated on the shares the harness delivers, which is the only reading that can distinguish a
/// coupled feed from an uncoupled one. Error cannot: it is computed from the belief, so it would
/// differ between these two runs whether or not the feed responded.
#[test]
fn the_share_feed_responds_to_the_difficulty_in_force() {
    // Two runs of the same trajectory, differing only in the belief the feed is measured against.
    // A controller is not involved: the point is the harness.
    let spm = TEST_SHARES_PER_MINUTE;
    let minutes = 120u64;

    let count_at = |believed: f64| -> u64 {
        let mut rng = XorShift64::new(SEED);
        let mut total = 0u64;
        for minute in 1..=minutes {
            let lambda = (START_HASHRATE as f64 / believed) * spm as f64;
            total += sample_poisson(&mut rng, lambda) as u64;
            let _ = minute;
        }
        total
    };

    let on_target = count_at(START_HASHRATE as f64);
    let quadrupled = count_at(START_HASHRATE as f64 * 4.0);

    assert!(
        on_target > quadrupled * 3,
        "quadrupling the difficulty in force should roughly quarter the shares delivered; got \
         {on_target} against {quadrupled}, a ratio of {:.2}",
        on_target as f64 / quadrupled as f64
    );
}

/// Both sampling branches must reproduce their mean, or a trajectory that crosses `KNUTH_LIMIT`
/// silently changes its share statistics partway through.
#[test]
fn both_poisson_branches_reproduce_their_mean() {
    for lambda in [1.0f64, 12.0, KNUTH_LIMIT - 1.0, KNUTH_LIMIT + 1.0, 200.0] {
        let mut rng = XorShift64::new(SEED ^ lambda.to_bits());
        let draws = 20_000;
        let total: u64 = (0..draws)
            .map(|_| sample_poisson(&mut rng, lambda) as u64)
            .sum();
        let mean = total as f64 / draws as f64;
        assert!(
            (mean / lambda - 1.0).abs() < 0.05,
            "lambda {lambda} sampled a mean of {mean:.3} over {draws} draws, off by more than 5%"
        );
    }
}
