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
    vardiff::{
        classic::{VardiffState, WindowCapForm},
        clock::MockClock,
        Vardiff,
    },
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
/// Salt for the wander stream, so adding wander to a trial does not disturb its share sampling.
const WANDER_SEED_SALT: u64 = 0x_4AD3_0000_0000_0001;

/// XorShift64, so sampling is reproducible without a dev-dependency.
pub(super) struct XorShift64(u64);

impl XorShift64 {
    /// Seeds the generator, avalanching the seed first.
    ///
    /// **The avalanche is load-bearing and its absence was a silent, total defect.** This generator is
    /// constructed *per tick* from `seed ^ elapsed`, so consecutive constructions differ only in low
    /// bits — and one xorshift round does not carry a low-bit change into the top 53 bits that
    /// [`Self::next_f64`] reads. Without mixing, the first output across 2880 consecutive per-tick
    /// seeds spanned `0.385849` to `0.385864`: a constant to five decimal places.
    ///
    /// A constant uniform draw makes `sample_poisson` **deterministic**, and in the direction that
    /// hides itself. Knuth's loop then multiplies by the same factor each time, so the count is a
    /// fixed function of the mean: a channel expecting two shares a tick received exactly two, every
    /// tick, with **zero sampling noise**; one expecting fewer than 0.95 received exactly zero,
    /// forever. Every measurement over a trajectory therefore ran against a deterministic share feed,
    /// and rare-arrival cells were fed nothing at all.
    ///
    /// SplitMix64's finalizer fixes it — a standard avalanche with no dependency — restoring 57
    /// arrivals where 57 are expected at `lambda = 0.02`, uniform to `chi2 = 6.0` on 9 degrees of
    /// freedom. Pinned by `the_generator_mixes_consecutive_seeds`, which tests the *per-tick* mode the
    /// harness actually uses. The pre-existing sampler tests validated a persistent generator, which
    /// is a mode no trajectory runs.
    ///
    /// Zero needs no special case once the seed is avalanched: the finalizer maps it to a nonzero
    /// state, so the recurrence's one fixed point is unreachable from any seed.
    pub(super) fn new(seed: u64) -> Self {
        // SplitMix64 finalizer.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self(z ^ (z >> 31))
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

/// One tick's delivered share count, seeded the way a trial seeds it.
///
/// Extracted so `the_delivered_feed_is_poisson` exercises the path `run_trial` uses rather than a copy
/// of it. A statistical guard on a reimplementation of the feed would not have caught the defect this
/// one exists for, because the defect was in how the generator is *seeded* per tick, not in the
/// sampling arithmetic.
pub(super) fn sample_tick(seed: u64, elapsed: u64, lambda: f64) -> u32 {
    sample_poisson(&mut XorShift64::new(seed.wrapping_add(elapsed)), lambda)
}

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
    let raw = lambda + lambda.sqrt() * sample_standard_normal(rng);
    if raw > 0.0 {
        raw.min(MAX_SAMPLED_SHARES as f64).round() as u32
    } else {
        0
    }
}

/// One standard normal draw, Box-Muller.
///
/// Shared with `sample_poisson`'s large-mean branch rather than written twice, so a trajectory that
/// crosses `KNUTH_LIMIT` and one that carries wander draw from the same generator.
fn sample_standard_normal(rng: &mut XorShift64) -> f64 {
    let (u1, u2) = (rng.next_f64(), rng.next_f64());
    (-2.0 * u1.ln()).sqrt() * (core::f64::consts::TAU * u2).cos()
}

/// Physical hashrate wander: a miner whose delivered rate drifts on its own, independent of anything
/// the controller does.
///
/// **Every other trajectory in this module holds the miner perfectly steady between disturbances, and
/// real ones do not.** Measured on the truth-poller archive, per unit, detrended so slow drift is not
/// read as correlation: an immersion unit wanders 3.00% with an integral timescale of 301 s, and an
/// air-cooled unit that ran to 94 C wanders 12.58% at 2269 s. The second matters more than its size
/// suggests, because wander slower than the estimator's memory is not averaged away -- only
/// `T_c / (T_c + tau)` of it is removed, so essentially all of the hot unit's survives into the
/// estimate and sits on top of the counting noise.
///
/// Modelled as an Ornstein-Uhlenbeck process in log rate, which is the standard choice for a
/// mean-reverting disturbance and the one the research branch's own wander test used: it has exactly the
/// two parameters that were measured, a stationary spread and a timescale, and it cannot wander away
/// without bound.
#[derive(Debug, Clone, Copy)]
pub(super) struct Wander {
    /// Stationary relative standard deviation of the delivered rate.
    pub(super) sigma: f64,
    /// Integral timescale, in seconds. Wander slower than `tau` is what the estimator cannot remove.
    pub(super) t_c_secs: u64,
}

impl Wander {
    /// The air-cooled unit that ran hot: the worst measured, and the one the gate should be judged at.
    pub(super) const HOT_AIR: Self = Self {
        sigma: 0.1258,
        t_c_secs: 2269,
    };

    /// The immersion unit: the best measured, for contrast.
    pub(super) const IMMERSION: Self = Self {
        sigma: 0.0300,
        t_c_secs: 301,
    };

    /// How much of it survives the estimator: `sigma * sqrt(T_c / (T_c + tau))`.
    ///
    /// The reason the hot unit is the interesting one. At the shipped memory it keeps 93% of its wander
    /// while the immersion unit keeps 67% of a spread four times smaller — so the hot unit's disturbance
    /// arrives at the estimator essentially intact, on top of the counting noise.
    ///
    /// **The square root is the corpus's, against the corpus's own prose.** Its text says "the reduction
    /// is `T_c/(T_c+tau)`", but all three of its measured rows fit the root and none fit the bare ratio:
    /// 3.58% at `T_c` 1047 reads 3.09% (root 0.863, ratio 0.744), 3.00% at 301 reads 2.03% (0.675 against
    /// 0.455), and 12.58% at 2269 reads 11.68% (0.929 against 0.863). Three rows agreeing with the root
    /// and disagreeing with the sentence beside them settles it; `the_wander_model_matches_the_measured_units`
    /// pins that, so this cannot quietly revert to the prose.
    pub(super) fn seen_by_estimator(&self, tau_secs: u64) -> f64 {
        self.sigma * (self.t_c_secs as f64 / (self.t_c_secs + tau_secs) as f64).sqrt()
    }

    /// Advances the log-rate state by `dt_secs` and returns the new multiplier on the delivered rate.
    ///
    /// `x <- rho*x + eps`, with `rho = e^(-dt/T_c)` and `eps` scaled so the stationary spread is
    /// `sigma` whatever `dt` is -- which is what makes the disturbance a property of the miner rather
    /// than of how often the harness happens to look.
    fn advance(&self, log_state: &mut f64, dt_secs: u64, rng: &mut XorShift64) -> f64 {
        let rho = (-(dt_secs as f64) / (self.t_c_secs as f64)).exp();
        let step_sd = self.sigma * (1.0 - rho * rho).sqrt();
        *log_state = rho * *log_state + step_sd * sample_standard_normal(rng);
        log_state.exp()
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
    /// Measured ticks on which the miner delivered anything at all.
    ///
    /// **The three error readings above are meaningless when this is zero**, and are reported as `0.0`
    /// rather than left as the infinity that `ln(belief / 0)` produces. Tracking error is undefined while
    /// a miner is absent -- no difficulty is correct for a hashrate of zero -- so a dark tick is skipped
    /// rather than scored. A trajectory with dark ticks therefore reports its error over the ticks the
    /// miner was present for, which is the quantity that means something: how wrong the difficulty is
    /// when there is someone to serve it to.
    pub(super) live_ticks: usize,
    /// Ticks from the start of the measured span until the belief is first within 10% of the truth, or
    /// `None` if it never gets there.
    ///
    /// Only meaningful on a trajectory that starts wrong, which until now none of them did. Ten percent
    /// because it is inside every threshold the controller applies, so reaching it means the controller
    /// has stopped being the thing in the way.
    pub(super) ticks_to_converge: Option<usize>,
    /// The decision threshold in force at the last evaluation, in percent, read from the controller
    /// rather than recomputed.
    ///
    /// The controller does not act below its threshold, so the settled position is inside this
    /// number. Tests that want to reason about settled error need it, and recomputing the formula
    /// here would let the copy drift from the implementation being checked.
    pub(super) final_threshold_pct: f64,
}

/// Which controller a trajectory is driven with.
///
/// The two agree at or above [`SPARSE_SPM_SEAM`] and differ below it, so on a dense cell this is not a
/// choice. See `sparse_parity.rs`.
///
/// [`SPARSE_SPM_SEAM`]: crate::vardiff::classic
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Arm {
    /// What this crate ships: the directional multiplier and the persistence discount apply at every
    /// share rate.
    Shipped,
    /// What the simulation framework selected: below the seam, a bare symmetric Poisson interval with
    /// neither of them.
    ChampionSparse,
    /// The shipped boundary, with the window capped at the `sigma`-matched value at every rate instead
    /// of at a constant above the seam and nowhere below it. See `window_cap_form.rs`.
    SigmaMatchedCap,
    /// The shipped boundary, with the window cap never looser than the `sigma`-matched window.
    TighterOfBothCaps,
    /// The shipped controller, with the estimator's time constant set to this many seconds.
    ///
    /// Only the memory changes: the cap stays the constant it is, which is what varying `tau` alone
    /// would actually do. See `tau_split.rs`.
    Tau(u64),
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
    arm: Arm,
    wander: Option<Wander>,
) -> Trial {
    let clock = Arc::new(MockClock::new(CLOCK_ORIGIN));
    let fresh = VardiffState::new_with_clock(TEST_MIN_ALLOWED_HASHRATE, clock.clone())
        .expect("a positive floor and a mock clock construct");
    let mut vardiff = match arm {
        Arm::Shipped => fresh,
        Arm::ChampionSparse => fresh.with_champion_sparse_boundary(),
        Arm::SigmaMatchedCap => fresh.with_window_cap_form(WindowCapForm::SigmaMatched),
        Arm::TighterOfBothCaps => fresh.with_window_cap_form(WindowCapForm::TighterOfBoth),
        Arm::Tau(secs) => fresh.with_tau_secs(secs),
    };

    let mut believed = START_HASHRATE;
    let mut wander_log = 0.0f64;
    let mut elapsed = 0;
    let mut retargets = 0;
    let mut shares_fed = 0u64;
    let mut errors: Vec<f64> = Vec::new();
    let mut worst_over_pct = f64::MIN;
    let mut final_error_pct = 0.0;
    let mut final_threshold_pct = f64::NAN;
    let mut ticks_to_converge = None;
    let mut measured_ticks = 0usize;
    let mut live_ticks = 0usize;

    while elapsed < warm_up_secs + measured_secs {
        elapsed += TICK_SECS;
        clock.advance(TICK_SECS);

        // Wander multiplies the trajectory rather than replacing it, so a declining miner that also
        // wanders is both, which is the realistic case and the one the gate is judged on.
        let delivered_now = delivered(elapsed)
            * match wander {
                Some(w) => w.advance(
                    &mut wander_log,
                    TICK_SECS,
                    &mut XorShift64::new(seed ^ WANDER_SEED_SALT ^ elapsed),
                ),
                None => 1.0,
            };
        let lambda = (delivered_now / believed as f64) * spm as f64 * (TICK_SECS as f64 / 60.0);
        let shares = sample_tick(seed, elapsed, lambda);
        for _ in 0..shares {
            vardiff.increment_shares_since_last_update();
        }

        let measuring = elapsed > warm_up_secs;
        if measuring {
            measured_ticks += 1;
            shares_fed += shares as u64;
        }
        // Read against the belief that was in force for this interval, which is the one the shares above
        // were produced against. Skipped entirely while the miner is absent: no difficulty is correct for a
        // hashrate of zero, so the ratio is undefined rather than infinite, and scoring it would let one
        // dark tick swamp every real reading in the run.
        if measuring && delivered_now > 0.0 {
            let error_pct = (believed as f64 / delivered_now).ln() * 100.0;
            live_ticks += 1;
            if ticks_to_converge.is_none() && error_pct.abs() < 10.0 {
                ticks_to_converge = Some(measured_ticks);
            }
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
    // A span the miner was absent for the whole of has no tracking error to report. Zero rather than a
    // panic, flagged by `live_ticks`, because a trajectory may legitimately be dark throughout -- and NaN
    // would trip the no-NaN guard in `median` for every caller.
    let (median_error_pct, p95_error_pct) = if errors.is_empty() {
        (0.0, 0.0)
    } else {
        (errors[errors.len() / 2], errors[errors.len() * 95 / 100])
    };
    Trial {
        median_error_pct,
        p95_error_pct,
        final_error_pct,
        worst_over_pct: if live_ticks == 0 { 0.0 } else { worst_over_pct },
        retargets,
        shares_fed,
        final_hashrate: believed,
        live_ticks,
        ticks_to_converge,
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
    measure_arm(
        delivered,
        spm,
        warm_up_secs,
        measured_secs,
        trials,
        Arm::Shipped,
    )
}

/// [`measure`], on a named arm. Same seeds, so a pair of readings differs only by the controller.
pub(super) fn measure_arm(
    delivered: &impl Fn(u64) -> f64,
    spm: f32,
    warm_up_secs: u64,
    measured_secs: u64,
    trials: usize,
    arm: Arm,
) -> Trial {
    measure_arm_wander(
        delivered,
        spm,
        warm_up_secs,
        measured_secs,
        trials,
        arm,
        None,
    )
}

/// [`measure_arm`], with the miner's own rate wandering. See [`Wander`].
///
/// Same seeds as the wander-free call, and the wander stream is salted rather than shared, so a pair of
/// readings differs only by the disturbance.
pub(super) fn measure_arm_wander(
    delivered: &impl Fn(u64) -> f64,
    spm: f32,
    warm_up_secs: u64,
    measured_secs: u64,
    trials: usize,
    arm: Arm,
    wander: Option<Wander>,
) -> Trial {
    let runs: Vec<Trial> = (0..trials)
        .map(|i| {
            run_trial(
                delivered,
                spm,
                warm_up_secs,
                measured_secs,
                SEED.wrapping_add(i as u64),
                arm,
                wander,
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
        // A trial that never converged counts as the whole span rather than being dropped, so a median
        // over mostly-unconverged trials reads as slow rather than as fast-on-a-subset.
        live_ticks: pick(runs.iter().map(|r| r.live_ticks as f64).collect()) as usize,
        ticks_to_converge: Some(pick(
            runs.iter()
                .map(|r| r.ticks_to_converge.map_or(f64::MAX, |t| t as f64))
                .collect(),
        ) as usize),
        final_threshold_pct: pick(runs.iter().map(|r| r.final_threshold_pct).collect()),
    }
}

/// A miner that never changes.
pub(super) fn constant() -> impl Fn(u64) -> f64 {
    |_elapsed| START_HASHRATE as f64
}

/// A miner delivering `factor` times what its declared nominal said, from the first tick.
///
/// Cold start, which nothing else here measures. The harness opens every channel believing
/// [`START_HASHRATE`], so a miner delivering `factor` times that is exactly a channel whose declared
/// `nominal_hash_rate` was wrong by `factor` -- the case `MIN_EVALUATION_INTERVAL_SECS` exists to rate
/// limit and the only one that exercises [`MAX_STEP_RATIO`]'s tightening arm, which the easing arm's
/// analysis leaves untouched.
pub(super) fn mis_declared(factor: f64) -> impl Fn(u64) -> f64 {
    move |_elapsed| START_HASHRATE as f64 * factor
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

/// A miner that drops out entirely for `down_secs` of every `period_secs`, and delivers in full the rest
/// of the time.
///
/// Repeated outage, as distinct from [`goes_silent`]'s single one. The distinction matters because the
/// silence machinery is *stateful*: an outage eases the belief toward the displacement floor and a return
/// has to tighten it back, so a channel that flaps drives that pair of mechanisms against each other
/// rather than exercising either once. It is also the disturbance that most resembles the cold-start lock
/// -- silence-detection interacting with a belief that is already wrong -- driven harder.
pub(super) fn intermittent(period_secs: u64, down_secs: u64) -> impl Fn(u64) -> f64 {
    move |elapsed| {
        if elapsed % period_secs < down_secs {
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

/// **The generator must decorrelate consecutive seeds, because that is how the harness uses it.**
///
/// `run_trial` constructs a fresh generator per tick from `seed ^ elapsed`, so the only mode that
/// matters is the *first* output from nearby seeds. The sampler tests below use a persistent generator
/// and therefore could not see -- and did not see -- that one xorshift round left that first output
/// constant to five decimal places, which made every trajectory's share feed deterministic and starved
/// rare-arrival cells of shares entirely.
///
/// This tests the mode in use: 2880 per-tick seeds, as a 48-hour run produces.
#[test]
fn the_generator_mixes_consecutive_seeds() {
    const TICKS: u64 = 2880;
    let firsts: Vec<f64> = (1..=TICKS)
        .map(|k| XorShift64::new(SEED.wrapping_add(60 * k)).next_f64())
        .collect();

    let (lo, hi) = firsts
        .iter()
        .fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
    assert!(
        hi - lo > 0.9,
        "the first output across per-tick seeds spanned only {:.6} ({lo:.6} to {hi:.6}). A narrow span          makes sample_poisson deterministic: Knuth's loop multiplies by the same factor every time, so          the count becomes a fixed function of the mean and the feed carries no sampling noise at all",
        hi - lo
    );

    // Uniformity, because a wide span is not the same as a good distribution.
    let mut buckets = [0usize; 10];
    for &v in &firsts {
        buckets[((v * 10.0) as usize).min(9)] += 1;
    }
    let expected = TICKS as f64 / 10.0;
    let chi2: f64 = buckets
        .iter()
        .map(|&c| (c as f64 - expected).powi(2) / expected)
        .sum();
    assert!(
        chi2 < 16.9,
        "per-tick first outputs are not uniform: chi2 {chi2:.1} over 10 buckets {buckets:?}, against          16.9 for 9 degrees of freedom at p=0.05"
    );

    // And the consequence that actually bit: a rare-arrival cell must receive arrivals.
    let lambda = 0.02;
    let arrivals: u32 = (1..=TICKS)
        .map(|k| sample_poisson(&mut XorShift64::new(SEED.wrapping_add(60 * k)), lambda))
        .sum();
    let expected_arrivals = lambda * TICKS as f64;
    assert!(
        (arrivals as f64 / expected_arrivals - 1.0).abs() < 0.3,
        "a channel expecting {expected_arrivals:.0} arrivals over {TICKS} ticks received {arrivals}.          Zero here is the defect that produced a spurious 'lock' finding: the controller was not          locking, it was being fed nothing"
    );
}

/// **The counts a trajectory is actually fed must be Poisson: mean, variance, and zero fraction.**
///
/// The guard this harness most needed and did not have. Every other check here tests a *component* --
/// the generator's spread, the sampler's mean -- and the defect that invalidated the branch's whole
/// empirical tier slipped between them: the sampler reproduced its mean to within a few percent while
/// making the zero-count outcome **impossible** and, below one arrival a tick, returning nothing at all.
///
/// So this asserts the three moments that together pin a Poisson feed, on the code path a trial runs:
///
/// - **mean**, which the old checks covered;
/// - **variance equal to the mean**, which nothing covered, and which is what "there is sampling noise
///   at all" means;
/// - **the zero fraction**, `e^-lambda`, which nothing covered and which the defect drove to zero
///   everywhere. It is the load-bearing one: a zero-share tick is the *input* to the silence apparatus,
///   so suppressing it made an entire branch of the controller unreachable while every test still
///   passed.
///
/// Model-free by construction. It makes no claim about what the controller does with the feed, so it
/// cannot be confounded by a controller change -- which is what lets it stand as a permanent instrument
/// check rather than a calibration that drifts.
#[test]
fn the_delivered_feed_is_poisson() {
    /// Enough draws that the zero fraction is resolvable at the rates that matter.
    const DRAWS: u64 = 40_000;

    for lambda in [0.02f64, 0.2, 1.0, 2.0, 12.0, 29.0, 31.0, 60.0] {
        let counts: Vec<u32> = (1..=DRAWS)
            .map(|k| sample_tick(SEED ^ lambda.to_bits(), 60 * k, lambda))
            .collect();
        let n = counts.len() as f64;
        let mean = counts.iter().map(|&c| c as f64).sum::<f64>() / n;
        let variance = counts
            .iter()
            .map(|&c| (c as f64 - mean).powi(2))
            .sum::<f64>()
            / n;
        let zero_fraction = counts.iter().filter(|&&c| c == 0).count() as f64 / n;
        let expected_zero = (-lambda).exp();

        eprintln!(
            "  [feed] lambda {lambda:>5}: mean {mean:.4} var/mean {:.3} P(0) {zero_fraction:.4} (expected {expected_zero:.4})",
            variance / mean
        );

        assert!(
            (mean / lambda - 1.0).abs() < 0.05,
            "lambda {lambda} delivered a mean of {mean:.4}, off by more than 5%"
        );
        assert!(
            (variance / mean - 1.0).abs() < 0.15,
            "lambda {lambda} delivered variance/mean of {:.3}, not the 1.0 a Poisson count has. Below              1 the feed is under-dispersed -- the defect this guard exists for made it 0 at small              lambda, so the controller saw no sampling noise to respond to",
            variance / mean
        );
        // Absolute tolerance, because at large lambda the expected fraction is ~0 and a relative bound
        // would be meaningless.
        assert!(
            (zero_fraction - expected_zero).abs() < 0.02,
            "lambda {lambda} delivered zero shares on {:.2}% of ticks against the {:.2}% a Poisson              feed gives. A zero-share tick is the input to the silence apparatus, so getting this              wrong makes a whole branch of the controller unreachable without failing anything else",
            zero_fraction * 100.0,
            expected_zero * 100.0
        );
    }
}

/// Both sampling branches must reproduce their mean, or a trajectory that crosses `KNUTH_LIMIT`
/// silently changes its share statistics partway through.
///
/// **Note the mode.** This drives a persistent generator, which no trajectory does, so it cannot
/// detect a seeding defect. `the_generator_mixes_consecutive_seeds` covers the mode in use; keep both.
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
