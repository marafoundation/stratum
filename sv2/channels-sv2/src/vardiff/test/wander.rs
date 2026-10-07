//! What the measurements look like once the miner's own rate is allowed to wander.
//!
//! Every other trajectory in this suite holds the miner perfectly steady between disturbances. Real ones do
//! not: measured per unit on the truth-poller archive, an immersion miner wanders 3.00% of its rate with a
//! timescale of 301 s and an air-cooled one that ran to 94 C wanders 12.58% at 2269 s. The second is the one
//! that matters, because wander slower than the estimator's memory is not averaged away.
//!
//! # Re-derived against the repaired sampler, and four of five conclusions inverted
//!
//! The pre-repair version of this file is retracted wholesale, and not only because its numbers were
//! measured against a feed that suppressed zero-share ticks. **The wander disturbance itself was
//! deterministic**: `Wander::advance` drew from a per-tick generator whose first output was pinned, so the
//! log-OU process received the same shock every step and drifted to a fixed point. It was a constant
//! offset, not a disturbance, and every "survives wander" claim was therefore a claim about nothing.
//!
//! `the_wander_process_reproduces_its_own_parameters` exists so that cannot recur: it checks the realized
//! spread *and* the lag-1 autocorrelation against the values the process is configured with, which a
//! constant fails on both counts.
//!
//! What the repair changed, claim by claim:
//!
//! | pre-repair claim | now |
//! |---|---|
//! | a sleepier memory is refused by the *sparse* band | **inverted** -- no cell crosses the gate at 720 s, and the cell nearest it is the densest |
//! | the dense wobble gain survives wander | **inverted** -- it survives at *sparse* and vanishes at dense under hot wander |
//! | wander makes the settled position worse, so steady readings are optimistic | **inverted** -- wander pulls the over-difficulty cells back, so steady is now the conservative reading for the gate |
//! | the sparse cap costs nothing only under wander | **moot** -- with or without wander it changes about one sparse trajectory in 25, because the window seldom reaches the matched value |
//! | the shipped controller exceeds the gate in 3 of 9 box cells | withdrawn with the cell it was measured at |
//!
//! The one conclusion that holds in *form* is the mechanism: a longer memory averages counting noise and
//! cannot average wander slower than itself. That is now the explanation for the second inversion rather
//! than a background note, and it predicts where the gain disappears.

use crate::vardiff::classic::VardiffState;

use super::trajectory::{
    changed_against_shipped, constant, declining, measure_arm_wander, Arm, Trial, Wander,
    XorShift64, SEED,
};

const ONE_HOUR: u64 = 3600;
const MEASURED_HOURS: u64 = 12;
const TRIALS: usize = 40;
/// Trials for the steadiness readings, whose 95th percentiles are noisier than a median.
const STABLE_TRIALS: usize = 120;
/// As `tau_split.rs`. The criterion the estimator's memory was selected by.
const DECLINE_SAFETY_GATE_PCT: f64 = 5.0;
/// The shipped memory, in seconds, for the reduction the estimator applies to wander.
const SHIPPED_TAU_SECS: u64 = 360;

fn declining_under(spm: f32, arm: Arm, wander: Option<Wander>) -> Trial {
    measure_arm_wander(
        &declining(5.0, 0.20),
        spm,
        ONE_HOUR,
        3 * ONE_HOUR,
        TRIALS,
        arm,
        wander,
    )
}

fn stable_under(spm: f32, arm: Arm, wander: Option<Wander>) -> Trial {
    measure_arm_wander(
        &constant(),
        spm,
        ONE_HOUR,
        MEASURED_HOURS * ONE_HOUR,
        STABLE_TRIALS,
        arm,
        wander,
    )
}

/// **The model reproduces the units it claims to model, before anything is concluded from it.**
///
/// Two halves. The *reduction* the estimator applies must match the three rows measured on hardware, which
/// is arithmetic and was never affected by the sampler.
#[test]
fn the_wander_model_matches_the_measured_units() {
    // The three measured rows, as (stationary spread, timescale, what the estimator saw).
    const ROWS: [(f64, u64, f64); 3] = [
        (0.0358, 1047, 0.0309),
        (0.0300, 301, 0.0203),
        (0.1258, 2269, 0.1168),
    ];

    for (sigma, t_c_secs, measured_seen) in ROWS {
        let seen = Wander { sigma, t_c_secs }.seen_by_estimator(SHIPPED_TAU_SECS);
        assert!(
            (seen / measured_seen - 1.0).abs() < 0.01,
            "a unit wandering {:.2}% at {t_c_secs}s should present {:.2}% to the estimator, as measured; \
             the model says {:.2}%. If this is off by about sqrt of the ratio, the reduction has reverted \
             to `T_c/(T_c+tau)`, which is what the corpus's prose says and what its own three rows \
             contradict",
            sigma * 100.0,
            measured_seen * 100.0,
            seen * 100.0
        );
    }

    let hot = Wander::HOT_AIR.seen_by_estimator(SHIPPED_TAU_SECS);
    let cool = Wander::IMMERSION.seen_by_estimator(SHIPPED_TAU_SECS);
    assert!(
        hot > cool * 5.0,
        "the hot unit should present several times the immersion unit's disturbance ({:.2}% against \
         {:.2}%); if they are comparable, there is no worst case to judge the gate at",
        hot * 100.0,
        cool * 100.0
    );
}

/// **Counting noise and the hot unit's wander cross at 6.09 shares a minute.**
///
/// Below that rate the estimator's own spread is the larger term of the floor, so a higher rate or a longer
/// memory buys precision. Above it the surviving wander is the larger term, and neither buys much. The
/// spread is the controller's exact one; the closed form `sqrt(30 / (r * tau))` puts the crossing at 6.10.
#[test]
fn counting_noise_and_the_hot_units_wander_cross_near_six_shares_a_minute() {
    let wander = Wander::HOT_AIR.seen_by_estimator(SHIPPED_TAU_SECS);

    // The spread falls monotonically with rate, so bisect for where it meets the wander.
    let (mut below, mut above) = (1.0f32, 30.0f32);
    for _ in 0..40 {
        let mid = (below + above) / 2.0;
        if VardiffState::estimator_spread(mid) > wander {
            below = mid;
        } else {
            above = mid;
        }
    }
    assert!(
        (below - 6.09).abs() < 0.05,
        "counting noise should meet the hot unit's surviving {:.2}% wander at 6.09 spm, the figure \
         VARDIFF.md quotes as where the sparse regime begins to end; it meets it at {below:.2}",
        wander * 100.0
    );
}

/// **And the process is actually a process: it reproduces its own spread and its own memory.**
///
/// The guard the pre-repair file needed and did not have. `Wander::advance` draws from a generator
/// constructed per tick, and while that generator's first output was pinned the log-OU received an
/// identical shock every step -- so it drifted to a fixed point and delivered a **constant offset**, not a
/// disturbance. Every claim of the form "X survives wander" was then a claim about a constant.
///
/// A constant fails both halves of this: its realized spread is zero and its autocorrelation is undefined.
/// Checking the spread alone would not be enough -- a white-noise bug would pass that and fail this.
#[test]
fn the_wander_process_reproduces_its_own_parameters() {
    const DRAWS: u64 = 20_000;
    const DT_SECS: u64 = 60;

    for w in [Wander::IMMERSION, Wander::HOT_AIR] {
        let mut log_state = 0.0f64;
        let logs: Vec<f64> = (1..=DRAWS)
            .map(|k| {
                let mut rng = XorShift64::new(SEED ^ (DT_SECS * k));
                w.advance_pub(&mut log_state, DT_SECS, &mut rng).ln()
            })
            .collect();

        let n = logs.len() as f64;
        let mean = logs.iter().sum::<f64>() / n;
        let sd = (logs.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n).sqrt();
        let lag1: f64 = logs
            .windows(2)
            .map(|p| (p[0] - mean) * (p[1] - mean))
            .sum::<f64>()
            / logs.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
        let expected_lag1 = (-(DT_SECS as f64) / w.t_c_secs as f64).exp();

        eprintln!(
            "  [wander] sigma {:.4} T_c {}: realized SD {sd:.4}, lag-1 {lag1:.3} (want {expected_lag1:.3})",
            w.sigma, w.t_c_secs
        );

        assert!(
            (sd / w.sigma - 1.0).abs() < 0.1,
            "the process must reproduce its stationary spread: {sd:.4} against {:.4}. Zero here means the \
             disturbance is a constant, which is what it was before the sampler was repaired",
            w.sigma
        );
        assert!(
            (lag1 - expected_lag1).abs() < 0.02,
            "and its memory: lag-1 {lag1:.3} against {expected_lag1:.3}. Spread alone would not catch a \
             white-noise bug, which is why both are asserted"
        );
    }
}

/// **The memory coupling's inversion survives wander: the cell nearest the gate is the densest.**
///
/// `tau_split.rs` re-derives, steady, that settled error rises with `tau` at every rate but that only the
/// dense cell is on the over-difficulty side at the shipped memory -- so it is dense channels that fix
/// `tau`, not sparse ones. This is that result under the worst measured wander, and it holds.
///
/// Measured at 720 s, one step sleepier: 2 spm `-4.74%`, 4 spm `-3.51%`, 12 spm `-1.82%`, 30 spm `+0.60%`.
/// Monotone in rate, no cell outside the `5%` gate, and the ordering is the exact reverse of what this file
/// asserted before the repair.
#[test]
fn the_cell_nearest_the_gate_is_the_densest_under_wander() {
    /// Trials for every reading here, ten times the file's default.
    ///
    /// Adjacent rates sit one to two points apart, against a `2 pp` standard error on a 40-trial median
    /// (`instruments::the_settled_reading_is_underpowered_against_an_absolute_gate`). At 40, three of twelve
    /// independent seed sets put at least one pair out of order. At this count all twelve are monotone,
    /// the narrowest step `0.79 pp`, and no cell reads above `+0.77%`.
    const NEAREST_TRIALS: usize = 400;

    let wander = Some(Wander::HOT_AIR);
    let readings: Vec<(f32, f64)> = [2.0f32, 4.0, 12.0, 30.0]
        .into_iter()
        .map(|spm| {
            let trial = measure_arm_wander(
                &declining(5.0, 0.20),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                NEAREST_TRIALS,
                Arm::Tau(720),
                wander,
            );
            (spm, trial.final_error_pct)
        })
        .collect();

    eprintln!(
        "  [wander] one step sleepier, hot unit: {}",
        readings
            .iter()
            .map(|(spm, pct)| format!("{spm} spm {pct:+.2}%"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    for pair in readings.windows(2) {
        let ((lo_spm, lo), (hi_spm, hi)) = (pair[0], pair[1]);
        assert!(
            hi > lo,
            "settled error should rise with the share rate under wander, so the densest cell is nearest \
             the gate: {lo_spm} spm reads {lo:+.2}% and {hi_spm} spm reads {hi:+.2}%. A fall here restores \
             the pre-repair ordering, which was an artifact of a feed that suppressed zero-share ticks"
        );
    }
    for (spm, pct) in &readings {
        assert!(
            *pct < DECLINE_SAFETY_GATE_PCT,
            "and no cell should cross the {DECLINE_SAFETY_GATE_PCT:.0}% gate one step sleepier: {spm} spm \
             reads {pct:+.2}%"
        );
    }
}

/// **The sleepier-memory steadiness gain survives at sparse rates and vanishes at dense ones.**
///
/// The second inversion, and the one with a mechanism that predicts it. A longer memory averages counting
/// noise, which falls as `1/sqrt(r*tau)`, and cannot average wander slower than itself, of which
/// `sqrt(T_c/(T_c+tau))` survives regardless. So the gain should persist exactly where counting noise
/// dominates and disappear where wander does -- and those cross near `6` shares a minute, where
/// `sigma_count` meets the hot unit's `11.7%`.
///
/// Measured p95 improvement from 360 s to 720 s, over [`STABLE_TRIALS`]: at 2 spm `1.46x / 1.33x / 1.11x`
/// for steady, immersion and hot; at 12 spm `1.24x / 1.12x / 0.97x`. The dense cell loses the gain entirely
/// under hot wander, which is the prediction. Twelve independent samples of that many trials put the hot
/// readings at `1.07-1.15x` and `0.97-1.00x`.
///
/// Without wander there is only counting noise to average, so the mechanism predicts a gain at both
/// rates and no ordering between them. The steady sparse reading does come out higher in those twelve
/// samples, but once by `0.01`, so the ordering is asserted only where wander predicts it.
#[test]
fn the_steadiness_gain_survives_at_sparse_rates_and_not_at_dense_ones() {
    let mut hot_dense = f64::NAN;
    for (label, wander) in [
        ("steady", None),
        ("immersion", Some(Wander::IMMERSION)),
        ("hot", Some(Wander::HOT_AIR)),
    ] {
        let gain = |spm: f32| {
            stable_under(spm, Arm::Shipped, wander).p95_error_pct
                / stable_under(spm, Arm::Tau(720), wander).p95_error_pct
        };
        let (sparse, dense) = (gain(2.0), gain(12.0));
        eprintln!("  [wander] p95 gain from a sleepier memory, {label}: {sparse:.2}x at 2 spm, {dense:.2}x at 12");

        assert!(
            sparse > 1.0,
            "the gain should persist at 2 spm under {label} wander, where counting noise dominates: \
             {sparse:.2}x"
        );
        if wander.is_some() {
            assert!(
                sparse > dense,
                "and under wander should be larger at the sparse rate than the dense one, because that is \
                 where the noise a longer memory can average is the larger term: {sparse:.2}x against \
                 {dense:.2}x"
            );
        }
        if label == "hot" {
            hot_dense = dense;
        }
    }

    // The prediction's sharp end: under the worst wander the dense gain is gone entirely.
    assert!(
        hot_dense < 1.1,
        "under hot wander at 12 spm the sleepier memory should buy essentially nothing, because the \
         disturbance is slower than either memory and dominates the counting noise it could average: \
         {hot_dense:.2}x. A gain here would mean wander is no longer the dominant term at this rate"
    );
}

/// **Where the controller sits on the over-difficulty side, wander pulls it back — so the steady reading
/// is the conservative one for the gate.**
///
/// The third inversion, and the one that changes how every other reading in the suite should be used.
/// Before the repair, wander appeared to push settled error toward over-difficulty, which made wander-free
/// measurement optimistic. That was the sampler: suppressing zero-share ticks over-counted shares, so the
/// controller read the rate high and leaned over-difficult, and adding a disturbance to an already-biased
/// loop moved it further the same way.
///
/// **Scoped to the over-difficulty side deliberately, because the effect is not reliably signed
/// elsewhere.** At 30 spm wander pulls the settled position down toward zero (`+2.90% -> +0.48%`), and on
/// these seeds it lowers the under-difficulty cells too (`-3.23% -> -4.22%` at 12 spm, `-8.53% -> -8.63%`
/// at 4), but twelve independent seed sets put the hot reading above the steady one at 12 spm in two of
/// them and at 4 spm in one, against none at 30.
///
/// What matters for the gate is only the over-difficulty side, and there it is protective. The case nearest
/// the gate is the binding cell on a 10 %/hr decline, where wander pulls `+5.53%` to `+3.71%`. The steady
/// reading there is outside the gate at this file's 40 trials and inside it powered (`+4.57%`,
/// `tau_split::on_a_steady_miner_the_gate_holds_by_under_half_a_point`), so what is asserted is the pull
/// and not the side of the gate either reading lands on. Twelve independent samples of 40 trials read
/// `+3.51` to `+5.57%` steady and `-0.35` to `+5.35%` hot, and the hot reading is the lower in all twelve.
#[test]
fn wander_pulls_back_the_cells_that_sit_over_difficulty() {
    /// The cell `tau_split.rs` identifies as fixing `tau`, and the only one on the over-difficulty side at
    /// the shipped memory.
    const BINDING_SPM: f32 = 30.0;

    let steady = declining_under(BINDING_SPM, Arm::Shipped, None).final_error_pct;
    let hot = declining_under(BINDING_SPM, Arm::Shipped, Some(Wander::HOT_AIR)).final_error_pct;
    eprintln!(
        "  [wander] 5 %/hr decline at {BINDING_SPM} spm: steady {steady:+.2}%, hot {hot:+.2}%"
    );

    assert!(
        steady > 0.0,
        "this test is about the over-difficulty side, so the steady reading must be on it: {steady:+.2}%. \
         If the binding cell has moved to the under-difficulty side, the scope here needs re-reading rather \
         than the threshold adjusting"
    );
    assert!(
        hot < steady,
        "and wander should pull it back: {hot:+.2}% against {steady:+.2}% steady. The reverse was measured \
         before the sampler repair and was an artifact of suppressed zero-share ticks; restoring it would \
         make every steady-only reading in this suite optimistic again"
    );

    // And on the steeper decline, where the binding cell sits nearest the gate.
    let steep = |wander| {
        measure_arm_wander(
            &declining(10.0, 0.40),
            BINDING_SPM,
            ONE_HOUR,
            3 * ONE_HOUR,
            TRIALS,
            Arm::Shipped,
            wander,
        )
        .final_error_pct
    };
    let (steep_steady, steep_hot) = (steep(None), steep(Some(Wander::HOT_AIR)));
    eprintln!(
        "  [wander] 10 %/hr decline at {BINDING_SPM} spm: steady {steep_steady:+.2}%, hot {steep_hot:+.2}%"
    );
    assert!(
        steep_hot < steep_steady,
        "and should pull it back on the steeper decline too, where the steady reading is nearest the \
         {DECLINE_SAFETY_GATE_PCT:.0}% gate -- steady {steep_steady:+.2}%, hot {steep_hot:+.2}%. That is \
         the argument for treating the steady reading as the conservative one for the gate"
    );
}

/// **The sparse window cap is reached on few trajectories, with or without wander, and moves the median
/// by a fraction of a point.**
///
/// `window_cap_form.rs` measures this steady; here it is under the hot unit's wander as well. Over
/// [`CAP_TRIALS`] on the 5 %/hr decline the `sigma`-matched cap changes 19 steady and 17 hot trajectories at
/// 2 spm, and 1 and none at 4.
/// Twelve independent samples of that many trials put the 2 spm counts at `9-19` steady and `10-21` hot,
/// and the 4 spm counts at `0-2`. Wander does not change the answer.
///
/// The median moves `+0.06 pp` steady and `+0.04 pp` hot, and `-0.30` to `+0.29` across the samples,
/// although the trajectories the cap does change move by several points (a median `+6.79 pp` steady and
/// `+5.79 pp` hot). That is why this counts trajectories rather than reading a median shift: over 40
/// trials the same comparison reads anywhere from `-0.56` to `+2.61 pp`, according to whether a changed
/// trajectory falls among them.
#[test]
fn the_sparse_window_cap_is_reached_rarely_rather_than_never() {
    /// Enough that a cap reached on one trajectory in 25 is reached about 20 times.
    const CAP_TRIALS: usize = 480;

    for (label, wander) in [("steady", None), ("hot", Some(Wander::HOT_AIR))] {
        let changed_at = |spm| {
            let (changed, cost) = changed_against_shipped(
                &declining(5.0, 0.20),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                CAP_TRIALS,
                Arm::SigmaMatchedCap,
                wander,
            );
            eprintln!(
                "  [wander] {label} {spm} spm: the sigma-matched cap changes {changed} of {CAP_TRIALS} \
                 trajectories and moves the median {cost:+.2} pp"
            );
            assert!(
                changed * 10 < CAP_TRIALS,
                "the sparse cap should change few trajectories at {spm} spm ({label}): {changed} of \
                 {CAP_TRIALS}. More means the window reaches the matched value routinely, and whether the \
                 cap helps or strands a decline becomes a live question"
            );
            changed
        };
        let sparsest = changed_at(2.0);
        changed_at(4.0);
        assert!(
            sparsest > 0,
            "and should change some at 2 spm ({label}): none of {CAP_TRIALS} trajectories reached it, \
             which would restore the claim this test replaced, that the window never gets there"
        );
    }
}

/// **The shallow decline's memory headroom does not survive a steeper one, and that settles `tau`.**
///
/// `tau_split.rs` measures, on a 5 %/hr decline, that every rate is inside the gate one step sleepier --
/// so there appeared to be about a factor of two of headroom above the shipped memory. This is the corner
/// neither file covered: the steeper 10 %/hr decline *and* hot wander together, where the sleepier memory
/// and the deeper disturbance push the settled position the same way.
///
/// The headroom is not there. Under hot wander on the steeper decline the shipped memory holds every rate
/// inside the gate (worst `+2.62%` at 30 spm) and one step sleepier puts 30 spm outside it (`+5.31%`). A
/// sleepier memory turns a configuration that passes into one that fails, on the same disturbance and the
/// same wander. So the apparent headroom was a property of the shallow decline alone, and `EWMA_TAU_SECS`
/// stays at `360` for a measured reason rather than for caution.
///
/// # This test used to claim three of four, and that was a trial-count artefact
///
/// The figures above are at [`GATE_TRIALS`]. At the file's default 40 they read `+6.14 / +4.32 / +6.01 /
/// +7.06` on one set of seeds -- three of four outside -- and `+2.65 / +2.92 / +4.97 / +7.15` on another,
/// one of four; the assertion below was a *count* of gate crossings with a threshold of two. A
/// `final_error_pct` median at this corner carries a `1.6-2.4 pp` standard error against a `5 pp` gate
/// (`instruments::the_settled_reading_is_underpowered_against_an_absolute_gate`), so each of those four
/// crossings is a coin flip with a fraction of a standard error of margin, and the threshold of two sat
/// *between* a low-trial reading of three and the powered reading of one. It passed for the wrong reason.
///
/// **The conclusion survives and is stronger on the right statistic.** One step sleepier the over-difficulty
/// *excursion* is worse at **all four** rates (`27.00 / 27.99 / 24.61 / 26.68` against `18.75 / 21.32 /
/// 21.21 / 23.54`), which is four of four rather than one of four, measured on a quantity with `3.6 %`
/// relative error instead of `117 %`. So this test now asserts that, and the gate count only where it has the
/// margin to be asserted -- the shipped configuration passing, which holds at both trial counts.
///
/// The excursion assertion holds at 40 trials too, which is the point of preferring it: [`GATE_TRIALS`] is
/// raised so the *figures reported here* are honest, not so the assertion passes. A count of gate crossings
/// needed the trials to survive; this does not.
///
/// `tau_split.rs` carries the rate-dependence of that penalty, which is what closed the split.
///
/// It also narrows the file's own third inversion. "Wander pulls the over-difficulty cells back, so steady
/// is the conservative reading" is true of the **settled** position and false of the **excursion**: at the
/// shipped memory hot wander roughly doubles worst over-difficulty against steady (`+23.54%` against
/// `+11.74%` at 30 spm, `+18.75%` against `+7.14%` at 2 spm) while pulling the settled reading from
/// `+4.74%` to `+2.62%`. Steady is conservative for the gate and optimistic for the transient, and the
/// two must be quoted separately.
#[test]
fn the_shallow_declines_memory_headroom_does_not_survive_a_steeper_one() {
    /// One step sleepier, as `tau_split.rs` names it.
    const SLEEPIER_TAU_SECS: u64 = 720;
    /// Trials for every reading here, ten times the file's default.
    ///
    /// Not caution: at 40 trials this test read "three of four rates outside" where the powered reading is
    /// one of four, and asserted a threshold that sat between them. See the module-level note above.
    const GATE_TRIALS: usize = 400;
    /// Least the excursion may be inflated by hot wander at the shipped memory. Measured `2.01x` at
    /// 30 spm and `2.63x` at 2 spm, so this binds at 1.8x on the tighter of them.
    const MIN_EXCURSION_INFLATION: f64 = 1.8;

    let steep = declining(10.0, 0.40);
    let at = |spm: f32, tau: u64, wander: Option<Wander>| {
        measure_arm_wander(
            &steep,
            spm,
            ONE_HOUR,
            3 * ONE_HOUR,
            GATE_TRIALS,
            Arm::Tau(tau),
            wander,
        )
    };
    let rates = [2.0f32, 4.0, 12.0, 30.0];
    let hot = Some(Wander::HOT_AIR);

    let shipped: Vec<(f32, Trial)> = rates
        .map(|spm| (spm, at(spm, SHIPPED_TAU_SECS, hot)))
        .into();
    let sleepier: Vec<(f32, Trial)> = rates
        .map(|spm| (spm, at(spm, SLEEPIER_TAU_SECS, hot)))
        .into();
    let describe = |label: &str, cells: &[(f32, Trial)]| {
        cells
            .iter()
            .map(|(spm, t)| format!("{spm:.0} spm {:+.2}%", t.final_error_pct))
            .collect::<Vec<_>>()
            .join(", ")
            + &format!(" ({label})")
    };
    eprintln!(
        "  [steep gate] hot wander on a 10 %/hr decline: {}; {}",
        describe("shipped", &shipped),
        describe("one step sleepier", &sleepier)
    );

    let outside = |cells: &[(f32, Trial)]| {
        cells
            .iter()
            .filter(|(_, t)| t.final_error_pct > DECLINE_SAFETY_GATE_PCT)
            .count()
    };
    assert_eq!(
        outside(&shipped),
        0,
        "the shipped memory should hold every rate inside the {DECLINE_SAFETY_GATE_PCT:.0}% gate on \
         this cell -- if it does not, the comparison below is between two failing configurations and \
         says nothing about the headroom: {}",
        describe("shipped", &shipped)
    );
    // What the absent headroom rests on, and it is the excursion rather than a count of gate crossings: a
    // sleepier memory is worse at *every* rate, which a count of four marginal decisions could not have
    // established even when it happened to read three.
    for ((spm, shipped_cell), (_, sleepier_cell)) in shipped.iter().zip(sleepier.iter()) {
        assert!(
            sleepier_cell.worst_over_pct > shipped_cell.worst_over_pct,
            "one step sleepier should worsen the over-difficulty excursion at every rate, which is what \
             makes the shallow decline's headroom absent rather than merely tighter: {spm} spm read \
             {:.2}% shipped against {:.2}% sleepier. This replaced a count of gate crossings that passed \
             only at 40 trials -- see the note on this test",
            shipped_cell.worst_over_pct,
            sleepier_cell.worst_over_pct
        );
    }
    // And the gate count only where it has the margin to carry an assertion.
    assert!(
        outside(&sleepier) > outside(&shipped),
        "and should still put more of the band outside the gate than the shipped memory does: {} against \
         {}. {}",
        outside(&sleepier),
        outside(&shipped),
        describe("one step sleepier", &sleepier)
    );

    // The excursion runs the other way from the settled reading, which is why both are measured here.
    for (spm, hot_trial) in &shipped {
        let steady = at(*spm, SHIPPED_TAU_SECS, None);
        let inflation = hot_trial.worst_over_pct / steady.worst_over_pct;
        assert!(
            inflation > MIN_EXCURSION_INFLATION,
            "hot wander should inflate the worst excursion at {spm} spm by more than \
             {MIN_EXCURSION_INFLATION}x -- steady reached {:+.2}% and hot {:+.2}%, a {inflation:.2}x \
             change. If this stops holding, 'steady is the conservative reading' has become true of \
             the excursion too and the scoping above is stale",
            steady.worst_over_pct,
            hot_trial.worst_over_pct
        );
    }
}
