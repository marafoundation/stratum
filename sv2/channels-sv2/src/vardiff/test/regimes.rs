//! What the controller does in four regimes, measured on trajectories rather than reasoned about.
//!
//! The mechanism tests elsewhere establish that each part does what it says. These establish what the
//! parts add up to, which is a different question and the one an operator cares about: a controller
//! can pass every mechanism test and still churn, sit at the wrong difficulty, or fail to notice a
//! miner leaving.
//!
//! Four regimes, chosen because they are the ones a channel is actually in. It is declining, it is
//! silent, it is steady, or it is varying faster than the estimator can follow. The first is where
//! the failures the design addresses live; the last is where a fixed memory should be expected to do
//! worst, and it is measured for that reason rather than despite it.

use std::sync::Arc;

use super::{
    trajectory::{
        constant, declining, goes_silent, measure, sample_poisson, swinging, XorShift64,
        START_HASHRATE,
    },
    TEST_SHARES_PER_MINUTE,
};
use crate::{
    target::hash_rate_to_target,
    vardiff::{
        classic::{VardiffState, WINDOW_CAP_SECS},
        clock::MockClock,
        Vardiff,
    },
};

const ONE_HOUR: u64 = 3600;

/// Over-difficulty a controller that never adjusted would settle at, after losing `drop` of the
/// hashrate. A reference point rather than a bound: a reading above it means difficulty moved the
/// wrong way, which is a different failure from failing to move.
fn never_adjusted_pct(drop: f64) -> f64 {
    (1.0 / (1.0 - drop)).ln() * 100.0
}

/// A decline is detected at every share rate, and the channel ends far from where never adjusting
/// would have left it.
///
/// This is the failure the design exists to fix, so it is stated as a distance from the do-nothing
/// outcome rather than as a distance from zero. The controller's own settled position is not zero and
/// is not meant to be: requiring more evidence to tighten than to loosen leaves it a little below the
/// truth, and that offset is deliberate. What matters here is that it is nowhere near the do-nothing
/// reading.
///
/// # Why the bound is on the gap closed, and not on the residual's magnitude
///
/// It was one-sided (`final_error_pct < reference * 0.5`) and that let six of eight readings pass
/// *trivially*, because a controller that over-corrects settles **negative** and any negative number is
/// below a positive bound. Over-correction is a real cost — it multiplies the miner's submission load
/// without changing its payout under difficulty-weighted accounting — so the bound moved to the
/// magnitude, at `0.25`.
///
/// **Re-derived against the repaired sampler, and the magnitude form does not survive it.** Two of the
/// six cells now fail `0.25`, both at 2 spm and both by over-correcting: `-11.51%` against a `22.31%`
/// do-nothing reading, and `-23.66%` against `69.31%`. Recalibrating would mean loosening to `0.52` —
/// *past the `0.5` the magnitude bound was introduced to replace* — which is the signal that the
/// instrument is wrong rather than mis-set. A symmetric bound on `|residual|` cannot distinguish the two
/// outcomes it spans: a residual left **over**-difficulty is the decline uncorrected, the failure this
/// design exists to fix, while one left **under** is over-correction, a different cost with a different
/// cause.
///
/// So the reading is the **fraction of the do-nothing gap the controller closes**,
/// `(reference - settled) / reference`, which is signed the right way round and vacuous in neither
/// direction — settling on the wrong side fails the lower bound, and overshooting fails the upper one.
/// Measured, the controller closes `0.975` to `1.516` of the gap across the grid: it removes essentially
/// all of the do-nothing over-difficulty at every cell, and what it leaves instead is overshoot at the
/// sparse end.
///
/// The overshoot's own absolute size is **not** bounded here, and deliberately. It does not scale with
/// the disturbance — at 2 spm the residuals are `-11.51%`, `-5.99%` and `-23.66%` against references of
/// `22.31%`, `51.08%` and `69.31%`, which is not monotone — so a disturbance-relative bound is the wrong
/// shape for it. `the_absolute_settled_error_is_held_not_merely_the_relative_one` holds it absolutely,
/// which is the shape that fits.
///
/// A single scalar works here **only because the shallowest cell moved out**. See
/// `a_disturbance_inside_the_dead_zone_is_a_noise_test_not_a_decline_test` for why, and for what that
/// cell actually measures.
#[test]
fn a_decline_is_detected_at_every_share_rate() {
    /// Cells: hashrate lost per hour, total lost, and the share rates to see it at.
    ///
    /// Each disturbance here is larger than the controller's threshold at the settle point, so the
    /// decline is visible to it as signal. `2 %/hr` losing `8%` is deliberately absent — at `8.34%`
    /// it is smaller than the bar and cannot be acted on by signal at all.
    const CELLS: [(f64, f64); 3] = [(5.0, 0.20), (10.0, 0.40), (40.0, 0.50)];
    /// Least of the do-nothing gap the controller must close. Measured worst `0.975` (10 %/hr at
    /// 12 spm), so this binds at 1.15x. Below `1.0` because a cell may legitimately stop just short of
    /// correct; far below it would mean the decline went uncorrected.
    const MIN_GAP_CLOSED: f64 = 0.85;
    /// And the most it may close, which is how overshoot is caught in the same scaled units. Measured
    /// worst `1.516` (5 %/hr at 2 spm), so this binds at 1.15x. Anything above `1.0` is over-correction.
    const MAX_GAP_CLOSED: f64 = 1.75;
    const TRIALS: usize = 40;

    for (rate_pct_per_hr, drop) in CELLS {
        let reference = never_adjusted_pct(drop);
        for spm in [2.0f32, 12.0] {
            let trial = measure(
                &declining(rate_pct_per_hr, drop),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                TRIALS,
            );
            let gap_closed = (reference - trial.final_error_pct) / reference;
            let report = format!(
                "{rate_pct_per_hr} %/hr losing {:.0}% at {spm} spm: ends {:+.2}%, worst \
                 over-difficulty {:+.2}%, {} retargets; never adjusting would end at \
                 {reference:+.2}%, so it closed {gap_closed:.3} of the gap",
                drop * 100.0,
                trial.final_error_pct,
                trial.worst_over_pct,
                trial.retargets
            );

            eprintln!("  [decline] {report}");

            assert!(
                trial.retargets > 0,
                "the decline should be acted on at all -- {report}"
            );
            assert!(
                gap_closed > MIN_GAP_CLOSED,
                "and should close most of the do-nothing gap, at least {MIN_GAP_CLOSED} of it -- \
                 {report}"
            );
            assert!(
                gap_closed < MAX_GAP_CLOSED,
                "without overshooting past {MAX_GAP_CLOSED} of it, which is the over-correction \
                 this reading exists to keep visible -- {report}"
            );
            assert!(
                trial.worst_over_pct < reference,
                "and never being worse than not acting at all, which is the only excursion that \
                 would mean difficulty moved the wrong way -- {report}"
            );
        }
    }
}

/// **A disturbance smaller than the threshold is a noise test, not a decline test.**
///
/// `2 %/hr` losing `8%` used to sit in the decline grid. It cannot belong there: the do-nothing
/// over-difficulty is `8.34%` and the controller's threshold at the settle point is wider than that at
/// every rate. **The whole disturbance is inside the dead zone**, so a threshold controller cannot act
/// on it by signal, and the retargets it does make are Poisson fluctuations crossing the bar rather than
/// the decline being detected.
///
/// In the decline grid all three of its assertions were vacuous in different ways: `retargets > 0`
/// was satisfied by noise, the relative bound by a negative residual, and the excursion bound was
/// 30 percentage points away.
///
/// What it *is* worth measuring is that being unable to see a disturbance does not make the
/// controller wander. So the assertion is on drift: whatever the noise-driven fires do, the settled
/// position must stay inside the dead zone that produced them rather than walking out of it.
///
/// # Re-derived: the premise is now asserted rather than asserted *about*
///
/// The re-derivation against the repaired sampler had to re-check this test's **premise**, not just move
/// its bound, and that is why it reads differently from the rest of the file. The old form hardcoded the
/// dead zone as `18.72%` at 2 spm and `11.32%` at 12 in a doc comment, then bounded drift at a flat `10%`
/// derived from those two figures. Both are quantities the repair moved — the threshold in force at a
/// sparse rate is now recorded as `116%` at 2 shares a minute — so a test whose premise lives in prose
/// and whose bound is a constant cannot tell you whether it still measures what it claims.
///
/// So both readings are taken from the trial: the premise (`disturbance < threshold`) is an assertion,
/// and drift is bounded by **the threshold the controller actually applied** rather than by a number
/// copied out of one. That makes the test self-checking against any future move in the bar — which is
/// the property it was missing, and the reason its bound had gone stale without failing. Measured, the
/// bar is `112.84%` at 2 spm and `27.12%` at 12, against the same `8.34%` disturbance — so the premise
/// holds by a wider margin than when it was written, and the old `10%` drift bound failed not because
/// the controller wandered further but because the zone it is entitled to wander inside grew sixfold.
///
/// The excursion bound is against the disturbance alone, and the `+ threshold` slack it used to carry is
/// gone: with the bar this wide that slack made the assertion unfailable. It is worth keeping tight
/// because the measured readings say the controller does not sit over-difficulty on an invisible
/// disturbance *at all* — worst over-difficulty is `-2.78%` at 2 spm, i.e. it never reached the
/// over-difficulty side of correct in the measured span, which is the asymmetry doing what §5 says.
#[test]
fn a_disturbance_inside_the_dead_zone_is_a_noise_test_not_a_decline_test() {
    const RATE_PCT_PER_HR: f64 = 2.0;
    const DROP: f64 = 0.08;
    const TRIALS: usize = 40;

    let reference = never_adjusted_pct(DROP);
    for spm in [2.0f32, 12.0] {
        let trial = measure(
            &declining(RATE_PCT_PER_HR, DROP),
            spm,
            ONE_HOUR,
            3 * ONE_HOUR,
            TRIALS,
        );
        let report = format!(
            "{RATE_PCT_PER_HR} %/hr losing {:.0}% at {spm} spm: ends {:+.2}%, worst \
             over-difficulty {:+.2}%, {} retargets; the whole disturbance is {reference:+.2}% \
             against a threshold in force of {:.2}%",
            DROP * 100.0,
            trial.final_error_pct,
            trial.worst_over_pct,
            trial.retargets,
            trial.final_threshold_pct
        );
        eprintln!("  [dead zone] {report}");

        assert!(
            reference < trial.final_threshold_pct,
            "the premise of this test is that the disturbance is invisible to the controller, so \
             the whole of it must sit inside the bar. If it does not, this is a decline test after \
             all and belongs in the grid above -- {report}"
        );
        assert!(
            trial.final_error_pct.abs() < trial.final_threshold_pct,
            "a disturbance the controller cannot see must not make it wander further than the bar \
             that hid the disturbance -- {report}"
        );
        assert!(
            trial.worst_over_pct < reference,
            "and it must not overshoot past the disturbance it cannot see -- {report}"
        );
    }
}

/// A channel that goes silent is eased to the displacement bound and stops there, at every share rate.
///
/// This is what lets the graduated no-shares divisors go. They existed because the old fixed ladder's
/// top rung fired on a zero estimate, which would otherwise have driven the belief at
/// `min_allowed_hashrate`; a threshold sized from the evidence reaches below 100% on its own once the
/// window is long enough, so the descent happens without them. What bounds where it stops is the
/// displacement floor, and that is the claim this checks -- across the whole share-rate band rather
/// than at the one rate the arithmetic was worked through.
///
/// The bound matters because a belief far below the truth serves a difficulty far too easy: a channel
/// eased to `min_allowed_hashrate` from a terahash belief returns submitting some fourteen orders of
/// magnitude over its target rate.
#[test]
fn a_silent_channel_rests_at_the_displacement_bound() {
    /// The bound the controller applies, as a ratio below the declared hashrate.
    const EXPECTED_DISPLACEMENT: f32 = 9.0;
    /// Tolerance, because the descent lands on the floor from above by one partial step.
    const TOLERANCE: f32 = 0.15;
    const TRIALS: usize = 20;

    for spm in [2.0f32, 6.0, 10.0, 12.0, 30.0, 60.0] {
        let trial = measure(&goes_silent(ONE_HOUR), spm, ONE_HOUR, 3 * ONE_HOUR, TRIALS);
        let displacement = START_HASHRATE / trial.final_hashrate;
        let report = format!(
            "at {spm} spm a silent channel rests at {:.3e} H/s, which is {displacement:.2}x below \
             where it started, after {} retargets",
            trial.final_hashrate, trial.retargets
        );

        assert!(
            trial.retargets > 0,
            "silence should be acted on at all -- {report}"
        );
        assert!(
            (displacement / EXPECTED_DISPLACEMENT - 1.0).abs() < TOLERANCE,
            "and should rest at the {EXPECTED_DISPLACEMENT}x bound -- {report}"
        );
    }
}

/// A channel that is not changing is left largely alone, and sits at a standing offset rather than at
/// zero.
///
/// The offset is the asymmetry's, not an error: tightening requires more evidence than loosening, so
/// the controller settles a little below the truth and stays there. It is bounded here rather than
/// removed, because removing it would mean removing the asymmetry.
///
/// The retarget count is what matters more, because each one is a message and a discontinuity for the
/// miner. A controller that scored well on the decline by reacting to everything would fail here.
///
/// **This is the test that prices what the window cap is for.** On a channel that is not changing,
/// every retarget is a move made on noise, and bounding those is the cap's entire justification. So
/// the churn budget is stated in the cap's own units -- one move per cap window -- which makes the
/// bound move if the cap does.
///
/// The offset is bounded twice, because either bound alone is satisfiable without the property
/// holding. As a fraction of the threshold in force, it says the controller rests inside the dead
/// zone its own evidence bought; a wide threshold would satisfy that with an arbitrarily large
/// offset, so an absolute ceiling says the dead zone is not itself unbounded. Neither is derivable
/// from the other, and the absolute one binds at the sparsest rate while the relative one binds in
/// the middle of the range.
#[test]
fn a_stable_channel_is_left_largely_alone() {
    /// Difficulty changes an hour, as one move per window cap: on a channel that is not changing the
    /// bar has had a full cap window to decay between moves. Measured worst 2.42/h at 6 spm.
    const MAX_RETARGETS_PER_HOUR: f64 = 3600.0 / WINDOW_CAP_SECS as f64;
    /// The standing offset, as a fraction of the threshold that produced it. Measured worst 0.206 at
    /// 6 spm.
    const MAX_OFFSET_OVER_THRESHOLD: f64 = 0.5;
    /// And in percent, because the ratio above is scaled by a threshold that is itself wide at sparse
    /// rates. An error of this size is a share-rate error of the same size, which share accounting
    /// does not care about, because a share is credited at the difficulty it was found at. Measured
    /// worst 19.35% at 2 spm, where the threshold in force is 116%.
    const MAX_MEDIAN_OFFSET_PCT: f64 = 25.0;
    const MEASURED_HOURS: u64 = 12;
    const TRIALS: usize = 9;

    for spm in [2.0f32, 6.0, 12.0, 60.0] {
        let trial = measure(
            &constant(),
            spm,
            ONE_HOUR,
            MEASURED_HOURS * ONE_HOUR,
            TRIALS,
        );
        let per_hour = trial.retargets as f64 / MEASURED_HOURS as f64;
        let offset_over_threshold = trial.median_error_pct / trial.final_threshold_pct;
        let report = format!(
            "at {spm} spm a stable channel saw {per_hour:.2} retargets an hour, sitting {:.2}% out \
             at the median and {:.2}% at the 95th percentile, against a threshold in force of \
             {:.2}% ({offset_over_threshold:.3} of it)",
            trial.median_error_pct, trial.p95_error_pct, trial.final_threshold_pct
        );

        assert!(
            per_hour <= MAX_RETARGETS_PER_HOUR,
            "a stable channel should not be retargeted more than \
             {MAX_RETARGETS_PER_HOUR} times an hour -- {report}"
        );
        assert!(
            offset_over_threshold <= MAX_OFFSET_OVER_THRESHOLD,
            "and should rest well inside the threshold that produced it, within \
             {MAX_OFFSET_OVER_THRESHOLD} of it -- {report}"
        );
        assert!(
            trial.median_error_pct <= MAX_MEDIAN_OFFSET_PCT,
            "and the standing offset should stay within {MAX_MEDIAN_OFFSET_PCT}% -- {report}"
        );
    }
}

/// A miner swinging on the estimator's own timescale is averaged rather than followed, and that is the
/// cost of a memory of fixed length.
///
/// A memory of `tau` averages over `tau`, so a swing on a comparable timescale puts the estimate
/// between the two levels, where it is wrong at both. This is the regime a fixed memory should be
/// expected to do worst in, and nothing else in the suite reaches it.
///
/// Two things bound the cost. It is a resonance rather than a general regression: far below the
/// memory's timescale both levels average to the same thing and the controller sits still, and far
/// above it each level is settled before it changes and is followed accurately. And the error is a
/// typical-case one -- the estimate is never far away, it is just never right either.
#[test]
fn a_miner_swinging_near_the_memory_is_averaged_rather_than_followed() {
    /// Two half periods, **both above** the estimator's 360 s memory: 2.5x it and 20x it.
    ///
    /// The names say "near" and "far above" and the comparison is between those two, so what this test
    /// establishes is the *long-side* falloff -- a swing closer to the memory is read less accurately than
    /// one far above it. It does not bracket the memory and does not locate a maximum. An earlier version
    /// of this comment said "either side of", which was wrong: nothing here is below 360 s.
    ///
    /// `outage.rs::the_cost_has_an_interior_peak_above_the_estimator_memory` sweeps both flanks for the
    /// related disturbance and finds the maximum at 5x the memory rather than at it, so "near the memory"
    /// should be read as shorthand for this pair of points and not as a claim about where the worst case is.
    const NEAR_MEMORY_SECS: u64 = 900;
    const FAR_ABOVE_MEMORY_SECS: u64 = 7200;
    /// How much worse the near-memory error must be for the resonance to be real.
    const MIN_RATIO: f64 = 2.0;
    const MEASURED_HOURS: u64 = 12;
    const TRIALS: usize = 9;

    let at = |half_period_secs: u64| {
        measure(
            &swinging(half_period_secs),
            TEST_SHARES_PER_MINUTE,
            ONE_HOUR,
            MEASURED_HOURS * ONE_HOUR,
            TRIALS,
        )
    };
    let near = at(NEAR_MEMORY_SECS);
    let far = at(FAR_ABOVE_MEMORY_SECS);
    let report = format!(
        "near the memory ({NEAR_MEMORY_SECS}s half period): {:.1}% median, {:.1}% p95; far above it \
         ({FAR_ABOVE_MEMORY_SECS}s): {:.1}% median, {:.1}% p95",
        near.median_error_pct, near.p95_error_pct, far.median_error_pct, far.p95_error_pct
    );

    assert!(
        near.median_error_pct > far.median_error_pct * MIN_RATIO,
        "a swing near the memory's timescale should be read materially less accurately than one far \
         above it, or this cost is not the resonance it is described as -- {report}"
    );
    assert!(
        far.median_error_pct < 15.0,
        "and a swing far above it should be followed accurately, which is what confines the cost to \
         the band -- {report}"
    );
}

/// **What the absolute settled criterion reads on the shipped controller.**
///
/// The criteria above are *relative* — a final error below half of what never
/// adjusting would have given. That scales with the disturbance, and on the
/// deepest cell here it permits **34.7%** settled over-difficulty, where the
/// criterion this controller's window and sensitivity were *selected* by is an
/// absolute **5%** (`DECLINE_SAFETY_GATE_PCT`, which lives on the simulation
/// branch and has no assertion here). So the selection criterion and the
/// regression criterion diverged, by up to 6.9x on these cells, and nothing on
/// this branch would notice the shipped configuration drifting between them.
///
/// This measures the absolute reading and holds it, so that drift is caught. It
/// is deliberately not phrased as the 5% gate: the number asserted is what the
/// controller actually achieves, so the test fails on a *regression* rather than
/// encoding a target it may or may not clear.
///
/// **Re-derived against the repaired sampler, and the divergence survives while its
/// cause changes ends of the band.** At this test's 40 trials the gate reads
/// exceeded, by +5.53% against the old +5.64% — a number that barely moved while
/// the cell producing it moved from the *sparsest* to the *densest* rate. Powered,
/// the cell reads +4.57% at 12,800 trials, just inside the gate
/// (`tau_split::on_a_steady_miner_the_gate_holds_by_under_half_a_point`); what
/// survives is its position at the top of the over-difficulty side. Recalibrating
/// the constant alone would therefore have passed, and preserved a false
/// attribution: the grid sampled only 2 and 12 shares a minute, and every
/// over-difficulty reading below the calibration rate is inside the gate. 30 spm is
/// now in the grid for that reason, and it is the same cell `tau_split.rs`
/// independently found fixes the estimator's memory.
///
/// The other half is that settled error rises monotonically with share rate in all
/// four decline cells: the sparse end settles in *under*-difficulty (worst −23.66%)
/// and only the dense end reaches over-difficulty at all. So the two bounds here
/// are not two views of one failure — they are the two ends of the band, and the
/// asymmetry puts each end on a different side of correct.
#[test]
fn the_absolute_settled_error_is_held_not_merely_the_relative_one() {
    /// Cells shared with `a_decline_is_detected_at_every_share_rate`.
    const CELLS: [(f64, f64); 4] = [(5.0, 0.20), (2.0, 0.08), (10.0, 0.40), (40.0, 0.50)];
    const TRIALS: usize = 40;
    /// The worst absolute settled over-difficulty this controller may show across
    /// the grid. Set from measurement with headroom, not from the selection gate —
    /// **measured +5.53% on the dense moderate-decline cell (30 spm, 10 %/hr)** at
    /// this test's trial count, and +4.57% at 12,800. The cell sits within half a
    /// point of the 5% selection gate, closer than a 40-trial median resolves, so
    /// asserting 5.0 here would make the verdict a draw on sampling noise;
    /// asserting 7.0 holds the line against regression. It holds it on the grid
    /// only: above 30 spm the settled error keeps rising, past the gate by 40 spm
    /// and past this bound by 90 (`tau_split.rs`, the steady-miner margin test).
    ///
    /// The bound itself did not move in the re-derivation; the *cell* did, from
    /// sparse-deep to dense-moderate, and `C10_EVALUATION_2026_09_21.md` §13
    /// records the superseded +5.64% reading against the old one.
    const MAX_ABSOLUTE_SETTLED_PCT: f64 = 7.0;
    /// And the *under*-difficulty side, which nothing else bounds.
    /// Measured worst −23.66% at 2 spm on the steepest decline; held so that
    /// over-correction is a regression too. Was −11.10% against the broken
    /// sampler, and this is the reading the repair moved most: a sparse channel on
    /// a steep decline eases freely and then refuses to tighten back, which is
    /// `TIGHTEN_MULTIPLIER` working as specified and is why the bound is this wide.
    const MIN_ABSOLUTE_SETTLED_PCT: f64 = -30.0;
    /// And what that over-correction costs, in the unit it is paid in.
    ///
    /// The under-difficulty bound above is a *symptom* bound: it says how far easy the controller
    /// settles, not what being there costs. A difficulty settled easy pays the miner exactly the same
    /// under difficulty-weighted accounting, so the cost is the submission load, and until this was
    /// measured the assertion above was the only thing standing where a cost should be.
    ///
    /// **Measured worst 1.15x** over the 3 h span, at 2 spm on the shallowest decline — so the cost is
    /// modest, and notably smaller than the flapping channel's 3.9x. It is the average of a load still
    /// rising when the span ends: by hour that cell costs 1.10x, 1.15x and 1.18x, and 2 spm on the
    /// 40 %/hr decline 0.97x, 1.21x and 1.26x. Nor is all of it the decline's: a steady miner at 2 spm
    /// costs 1.17x over the same span. It is also not monotone in the settled reading: the −23.66% cell
    /// costs 1.12x while the −15.27% one costs 1.15x, because the load is integrated over the span and
    /// the settled figure is the last tick of it. The dense cells run *under* 1.0x (0.95x to 1.00x),
    /// which is the over-difficulty side of the band being paid for in the same unit.
    const MAX_SHARE_LOAD: f64 = 1.30;
    /// Mirrors `MAX_REMAINING` in `a_decline_is_detected_at_every_share_rate`,
    /// which is local to that test. Used only to report what the relative
    /// criterion would have allowed, never to assert.
    const RELATIVE_PERMITTED: f64 = 0.5;

    let mut readings: Vec<(f64, String)> = Vec::new();
    for (rate_pct_per_hr, drop) in CELLS {
        // `30` is here because `tau_split.rs` identified it as the cell that fixes the estimator's
        // memory, and a grid sampling only 2 and 12 was blind to it: every over-difficulty reading
        // below the calibration rate is inside the selection gate, and the one at it is not.
        for spm in [2.0f32, 12.0, 30.0] {
            let trial = measure(
                &declining(rate_pct_per_hr, drop),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                TRIALS,
            );
            let reference = never_adjusted_pct(drop);
            // The cost of the under-difficulty side, in the unit it is actually paid in. A difficulty
            // settled easy is credited the same under difficulty-weighted accounting, so what
            // over-correction buys the miner is not less payout but more messages.
            let intended_shares = spm as f64 * (3 * ONE_HOUR) as f64 / 60.0;
            let share_load = trial.shares_fed as f64 / intended_shares;
            readings.push((
                trial.final_error_pct,
                format!(
                    "{rate_pct_per_hr} %/hr losing {:.0}% at {spm} spm: settled {:+.2}%, \
                     relative criterion permitted {:+.2}%, share load {share_load:.2}x",
                    drop * 100.0,
                    trial.final_error_pct,
                    reference * RELATIVE_PERMITTED
                ),
            ));
            assert!(
                share_load < MAX_SHARE_LOAD,
                "the submission load this over-correction buys should stay under \
                 {MAX_SHARE_LOAD}x the intended rate -- {} shares against {intended_shares:.0} \
                 intended at {spm} spm on a {rate_pct_per_hr} %/hr decline, {share_load:.2}x",
                trial.shares_fed
            );
        }
    }
    assert!(
        !readings.is_empty(),
        "no cell produced a reading -- the harness measured nothing"
    );

    // Collected first and asserted after, so a failure reports the whole grid. Asserting inside the
    // loop aborts at the first bad cell, which is the least informative one to be shown -- both
    // bounds are grid-wide claims, and which cell is worst is the reading that identifies a drift.
    let by_error =
        |a: &&(f64, String), b: &&(f64, String)| a.0.partial_cmp(&b.0).expect("errors are finite");
    let worst_over = readings.iter().max_by(by_error).expect("non-empty");
    let worst_under = readings.iter().min_by(by_error).expect("non-empty");
    let grid = format!(
        "\n    {}",
        readings
            .iter()
            .map(|(_, label)| label.as_str())
            .collect::<Vec<_>>()
            .join("\n    ")
    );
    eprintln!(
        "  [absolute settled] worst over {:+.2}%, worst under {:+.2}%, across:{grid}",
        worst_over.0, worst_under.0
    );

    assert!(
        worst_under.0 >= MIN_ABSOLUTE_SETTLED_PCT,
        "settled *under*-difficulty regressed past {MIN_ABSOLUTE_SETTLED_PCT:.1}% -- {}. \
         Over-correcting downward costs the miner nothing in payout under difficulty-weighted \
         accounting but multiplies its submission load, which `MAX_SHARE_LOAD` bounds per cell. \
         Whole grid:{grid}",
        worst_under.1
    );
    assert!(
        worst_over.0 <= MAX_ABSOLUTE_SETTLED_PCT,
        "absolute settled over-difficulty regressed past {MAX_ABSOLUTE_SETTLED_PCT:.1}% -- {}. \
         The relative criterion above would still pass here, which is the point of this test: it \
         is the only thing on this branch that would catch the shipped configuration drifting away \
         from the criterion it was selected by. Whole grid:{grid}",
        worst_over.1
    );
}

/// **The settled position is inside the threshold that produced it**, at every cell and share rate.
///
/// This is the property every settled-error criterion in this suite depends on and none of them
/// stated. A threshold controller does not act while the deviation is below its threshold, so
/// wherever it stops is inside that number — which means **the threshold, not any written bound, is
/// what limits settled error.** A criterion asking for a settled error tighter than the threshold is
/// asking the controller to be more precise than its own trigger, and no parameter choice satisfies
/// that.
///
/// It is not a tautology. The controller compares its *estimate* against its *belief*, while the
/// reading here is belief against *truth*. They coincide only while the estimator is tracking, so
/// this asserts that it is — a real property, and the one that makes threshold-relative reasoning
/// about settled error valid at all.
///
/// The threshold is read from [`VardiffState::last_threshold_pct`] rather than recomputed. Three
/// records' worth of analysis had been recomputing the formula alongside the implementation, which is
/// exactly the arrangement in which a copy drifts from what it checks.
#[test]
fn the_settled_position_is_inside_the_threshold_that_produced_it() {
    const CELLS: [(f64, f64); 4] = [(5.0, 0.20), (2.0, 0.08), (10.0, 0.40), (40.0, 0.50)];
    const TRIALS: usize = 40;

    let mut tightest = f64::MAX;
    for (rate_pct_per_hr, drop) in CELLS {
        for spm in [2.0f32, 12.0] {
            let trial = measure(
                &declining(rate_pct_per_hr, drop),
                spm,
                ONE_HOUR,
                3 * ONE_HOUR,
                TRIALS,
            );
            let report = format!(
                "{rate_pct_per_hr} %/hr losing {:.0}% at {spm} spm: settled {:+.2}%, threshold in \
                 force {:.2}%",
                drop * 100.0,
                trial.final_error_pct,
                trial.final_threshold_pct
            );
            assert!(
                trial.final_threshold_pct.is_finite(),
                "no evaluation recorded a threshold -- the accessor is not being reached -- {report}"
            );
            assert!(
                trial.final_error_pct.abs() < trial.final_threshold_pct,
                "the settled position escaped the threshold that produced it, which would mean the \
                 estimator is not tracking and threshold-relative reasoning about settled error is \
                 invalid -- {report}"
            );
            tightest = tightest.min(trial.final_threshold_pct / trial.final_error_pct.abs());
        }
    }
    // Containment is not vacuous: the tightest cell sits within a small multiple of its threshold.
    // Measured ratios run 3.94x to 22.36x, and twelve independent seed sets put the tightest cell at
    // 3.94x to 5.77x, so 10x binds with at least 1.7x of room. The guard is oriented to fail when
    // containment stops being *exercised* — if every cell moved far inside its threshold this test
    // would assert nothing, and that is the regression worth catching here.
    assert!(
        tightest < 10.0,
        "every cell settled more than 10x inside its threshold, so this test is not exercising \
         containment and its assertion is vacuous (tightest ratio {tightest:.1}x)"
    );
}

/// Difficulty a hashrate implies at a target share rate, continuous.
fn difficulty_for(hashrate: f64, spm: f32) -> f64 {
    hashrate / ((spm as f64 / 60.0) * 2f64.powi(32))
}

/// The SV1 egress's rounding: **down** to the largest power of two not exceeding the difficulty.
///
/// Down and never up, because upstream credits shares against its own unrounded target, so a miner
/// told a *higher* difficulty silently discards every share between that target and the next power of
/// two. Defined here because nothing in this crate quantises — the rounding lives in the translator.
fn round_down_pow2(difficulty: f64) -> f64 {
    if difficulty <= 0.0 || !difficulty.is_finite() {
        return difficulty;
    }
    2f64.powi(difficulty.log2().floor() as i32)
}

/// **The composed path reaches the right octave — the property whose absence stalled a channel for
/// 17.9 hours on hardware.**
///
/// No harness in this crate quantises the target — every one calls `hash_rate_to_target` directly and
/// feeds the controller a continuous difficulty. The power-of-two round-down lives in the translator,
/// in another repository, so **a defect that needs the quantiser is invisible to this entire suite.**
/// That is why an eighteen-hour stall on live hardware had no failing test.
///
/// # The topology this models
///
/// The pool holds a continuous belief and hands it to the controller. The *translator* derives a
/// difficulty from it and rounds **down** to a power of two before the miner sees it. So the miner
/// produces shares against the rounded difficulty while the controller reasons about the unrounded
/// belief — and when the rounded difficulty is lower than the belief implies, the miner over-delivers
/// and the estimator reads a hashrate above the belief. That gap is the deviation the controller sees,
/// and on hardware it sat at 25.7% against a threshold of 64% for eighteen hours.
///
/// # The four components, none of which is individually wrong
///
/// 1. the partial retarget converges toward the estimate **from below**, each step a fraction of a
///    shrinking gap, so the belief approaches the truth without reaching it;
/// 2. round-**down** means the served difficulty changes only when the belief crosses an octave, so
///    everything short of that crossing is invisible to the miner;
/// 3. `TIGHTEN_MULTIPLIER` raises the bar eightfold for exactly the direction that would close the gap;
/// 4. the window cap stops the bar decaying to meet the deviation, which turns a slow approach into a
///    permanent one.
///
/// Classic, on the identical path, crosses the octave on its first retarget because it retargets in
/// full. The stall was a property of this composition, not of quantised actuators.
///
/// # What this asserted before the fix, kept because the numbers are the evidence
///
/// Until the window cap was exempted on the tighten direction, this test asserted the **stall**: belief
/// 128.52 TH/s against a boundary of 140.74, serving 32,768, realizing 68.2/min against a 30/min
/// target — matching hardware's 128.65 TH/s to 0.1%. That is what the fix changed, and the assertion was
/// inverted rather than deleted so the before-figures stay attached to the test that measured them.
#[test]
fn the_quantised_path_crosses_the_octave_boundary() {
    /// Where `first_target` puts the opening belief on the rig: 80 TH/s.
    const OPENING_BELIEF: f32 = 80e12;
    /// What the miner actually runs at.
    const TRUE_HASHRATE: f64 = 160e12;
    const SPM: f32 = 30.0;
    const TICK_SECS: u64 = 60;
    /// Four hours, well past the point the hardware belief stopped moving.
    const TICKS: u64 = 240;

    let clock = Arc::new(MockClock::new(0));
    let mut vardiff = VardiffState::new_with_clock(1.0, clock.clone()).expect("constructs");
    let mut believed = OPENING_BELIEF;
    let mut rng = XorShift64::new(0xB0A7);
    let mut served_history = Vec::new();

    for tick in 1..=TICKS {
        let now = tick * TICK_SECS;
        // The miner produces against the ROUNDED difficulty, which is what it was told.
        let served = round_down_pow2(difficulty_for(believed as f64, SPM));
        served_history.push(served);
        let effective = served * (SPM as f64 / 60.0) * 2f64.powi(32);
        let lambda = (TRUE_HASHRATE / effective) * (SPM as f64) * (TICK_SECS as f64 / 60.0);
        for _ in 0..sample_poisson(&mut rng, lambda) {
            vardiff.increment_shares_since_last_update();
        }

        clock.set(now);
        // The controller holds an unrounded belief but SENT the rounded difficulty, so its estimator
        // scales against the rounded target. `hashrate` and `target` therefore describe different
        // difficulties, which `VardiffState::threshold`'s own doc comment anticipates as a real
        // configuration ("those agree only while the caller's target and hashrate describe the same
        // difficulty, which nothing in this signature enforces").
        let served_hashrate = (served * (SPM as f64 / 60.0) * 2f64.powi(32)) as f32;
        let target = hash_rate_to_target(served_hashrate.into(), SPM.into()).expect("positive");
        if let Ok(Some(next)) = vardiff.try_vardiff(believed, &target, SPM) {
            believed = next;
        }
    }

    let boundary = 65536.0 * (SPM as f64 / 60.0) * 2f64.powi(32);
    let final_served = *served_history.last().expect("ticks ran");
    let realized = TRUE_HASHRATE / (final_served * 2f64.powi(32)) * 60.0;
    let report = format!(
        "after {TICKS} ticks: belief {:.2} TH/s, boundary {:.2} TH/s, serving {final_served:.0}, \
         realized {realized:.1}/min against a {SPM:.0}/min target",
        believed as f64 / 1e12,
        boundary / 1e12
    );

    // Asserted the other way round since the fix. Deleting the old direction would have lost the
    // measurement; inverting it keeps the test pointed at the same property.
    assert!(
        (believed as f64) > boundary,
        "the belief failed to cross the octave boundary, which is the 17.9-hour hardware stall \
         returning -- before the window cap was exempted on the tighten direction this read 128.52 \
         TH/s against a 140.74 boundary: {report}"
    );
    assert!(
        realized < 1.5 * SPM as f64,
        "and the miner must not be left materially over-delivering; the stall left it at 2.27x: \
         {report}"
    );
    // Still a fixed point, but now the right one: it settles and stays.
    let last_quarter = &served_history[served_history.len() * 3 / 4..];
    assert!(
        last_quarter.windows(2).all(|w| w[0] == w[1]),
        "the served difficulty was still moving in the final quarter, so it has not settled: {report}"
    );
    eprintln!("  [stall] {report}");
}
