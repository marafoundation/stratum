//! A miner that drops out and returns repeatedly, which is the disturbance that drives the silence
//! machinery against itself.
//!
//! `regimes.rs` covers a single outage: the channel goes dark and stays dark, and what matters is where
//! the ease stops. A channel that *flaps* is a different problem, because each outage eases the belief and
//! each return has to tighten it back -- and those two moves are deliberately not symmetric. So the
//! question is not where one ease stops but where the pair comes to rest after many rounds.
//!
//! **It comes to rest under-difficulty, which is the mirror of the cold-start lock.** `cold_start.rs`
//! finds an over-declared sparse channel pinned *above* the truth by the displacement bound. A flapping
//! channel sits persistently *below* it, by 1.6x to 3.4x, for the opposite reason: easing on an outage is
//! cheap and tightening on a return costs `TIGHTEN_MULTIPLIER` times as much evidence, so the belief
//! ratchets down. Both failures are the controller unable to tell absence from a wrong belief; they run in
//! opposite directions.
//!
//! Under difficulty-weighted accounting that costs the pool nothing in payout -- a share is credited at the
//! difficulty it was found at. What it costs is load: four to five times the intended share bandwidth and
//! four times the retarget budget a stable channel is held to.
//!
//! **The asymmetry is the cause, confirmed by mutation, and the trade is visible in the same run.** With
//! `TIGHTEN_MULTIPLIER` weakened from `8.0` to `3.0` -- a factor of 2.7 -- the worst cell's settled
//! under-difficulty shrinks from `-123.69%` to `-46.44%`, also a factor of 2.7, while its retargets rise
//! from 98 to 157. So the under-difficulty is the price of the reluctance and the churn is the price of
//! giving it up; neither is free and this disturbance is where the exchange rate shows.
//!
//! *(`3.0` because `1.0` does not compile: the constant assertion beside `MIN_EFFECTIVE_TIGHTEN_MULTIPLIER`
//! refuses a multiplier the discount ceiling would invert, and `3.0` is the lowest value that clears it at
//! the shipped ceiling. That is the guard working on a real edit rather than on a hypothetical one.)*

use super::trajectory::{intermittent, measure_arm, Arm, Trial, START_HASHRATE};

const MEASURED_HOURS: u64 = 12;
const TRIALS: usize = 20;
/// Mirrors `MAX_SILENT_DISPLACEMENT`, which is private. Used to show the bound is *not* what binds here.
const EXPECTED_DISPLACEMENT: f64 = 9.0;
/// The budget `regimes.rs` holds a stable channel to, for contrast.
const STABLE_RETARGETS_PER_HOUR: f64 = 4.0;

/// `period_secs` long cycles, dark for `down_secs` of each.
fn flapping(period_secs: u64, down_secs: u64, spm: f32) -> Trial {
    measure_arm(
        &intermittent(period_secs, down_secs),
        spm,
        0,
        MEASURED_HOURS * 3600,
        TRIALS,
        Arm::Shipped,
    )
}

/// **A flapping miner settles under-difficulty at every rate and cycle, never over.**
///
/// The direction is the finding. A disturbance that looks like departure and is not should, naively, leave
/// the channel over-difficulty -- that is what a decline does. It does the opposite, because the two moves
/// the cycle demands are priced differently: the ease an outage triggers is the cheap direction and the
/// tighten a return demands is the expensive one, so each round loses a little ground and the belief
/// ratchets down.
///
/// Asserted as a sign rather than a magnitude, because the magnitude is cycle-dependent and the sign is
/// not. A cell that came out over-difficulty would mean the asymmetry had stopped being the dominant
/// effect, and everything below would need re-reading.
#[test]
fn a_flapping_miner_settles_under_difficulty_at_every_cycle() {
    for (period, down) in [(3600u64, 600u64), (3600, 1800), (1800, 900), (7200, 3600)] {
        for spm in [2.0f32, 12.0] {
            let trial = flapping(period, down, spm);
            assert!(
                trial.live_ticks > 0,
                "a cycle of {period}s dark for {down}s must leave live ticks to score, or the error \
                 readings mean nothing"
            );
            eprintln!(
                "  [outage] {}s cycle dark {}s at {spm} spm: ends {:+.2}%, worst over {:+.2}%, median \
                 {:.2}%, {} retargets, rests at {:.3e} H/s",
                period,
                down,
                trial.final_error_pct,
                trial.worst_over_pct,
                trial.median_error_pct,
                trial.retargets,
                trial.final_hashrate,
            );
            assert!(
                trial.final_error_pct < 0.0,
                "a flapping channel should end under-difficulty, not over: {:+.2}% on a {period}s cycle \
                 dark {down}s at {spm} spm. Over-difficulty here would mean the tighten asymmetry is no \
                 longer the dominant effect on this disturbance",
                trial.final_error_pct
            );
        }
    }
}

/// **And the displacement bound is not what binds, so the asymmetry is the thing to look at.**
///
/// Worth separating because the bound is the obvious suspect -- it is what governs a single outage, and
/// `cold_start.rs` shows it locking a channel when it does bind. Here it never does: an outage of ten to
/// thirty minutes does not ease far enough to reach one ninth of the last evidenced belief, so every cell
/// rests well above that floor.
///
/// If this reversed, the two failures would have one cause and one fix. They do not.
#[test]
fn the_displacement_bound_is_not_what_binds_on_a_flapping_channel() {
    let floor = START_HASHRATE as f64 / EXPECTED_DISPLACEMENT;

    for (period, down) in [(3600u64, 600u64), (1800, 900), (7200, 3600)] {
        let trial = flapping(period, down, 2.0);
        assert!(
            trial.final_hashrate as f64 > floor * 1.5,
            "a {period}s cycle dark {down}s rested at {:.3e} H/s, within reach of the {floor:.3e} \
             displacement floor. If the bound binds here too, this failure and the cold-start lock share \
             a cause and `cold_start.rs`'s mechanism claim needs widening",
            trial.final_hashrate
        );
    }
}

/// **What it costs: four to five times the intended share load, and four times the retarget budget.**
///
/// The price of the under-difficulty above, in the two units an operator pays in. Neither is a payout
/// cost -- a share is credited at the difficulty it was found at, so the miner gains nothing and the pool
/// loses nothing on that axis. Both are load: bandwidth and validation for the shares, and a message plus
/// a discontinuity for each retarget.
///
/// Measured against the intended load rather than against another arm, because there is no better
/// controller here to compare with -- the question is how far from the operator's own configuration a
/// flapping channel drags the deployment.
#[test]
fn a_flapping_miner_costs_several_times_the_intended_load() {
    /// Cycle nearest the estimator's memory, which the readings below show is the worst case.
    const PERIOD_SECS: u64 = 1800;
    const DOWN_SECS: u64 = 900;
    /// Least share-load multiple that counts as the effect being real rather than sampling.
    const MIN_SHARE_MULTIPLE: f64 = 3.0;

    for spm in [2.0f32, 12.0] {
        let trial = flapping(PERIOD_SECS, DOWN_SECS, spm);
        // What the operator asked for: the target rate, over the fraction of the span the miner was up.
        let duty = (PERIOD_SECS - DOWN_SECS) as f64 / PERIOD_SECS as f64;
        let intended = spm as f64 * (MEASURED_HOURS * 60) as f64 * duty;
        let multiple = trial.shares_fed as f64 / intended;
        let per_hour = trial.retargets as f64 / MEASURED_HOURS as f64;

        eprintln!(
            "  [outage] {PERIOD_SECS}s cycle at {spm} spm: {} shares against {intended:.0} intended \
             ({multiple:.1}x), {per_hour:.1} retargets an hour against a stable channel's \
             {STABLE_RETARGETS_PER_HOUR:.0}",
            trial.shares_fed
        );

        assert!(
            multiple > MIN_SHARE_MULTIPLE,
            "a flapping channel at {spm} spm delivered {multiple:.1}x the intended share load, under the \
             {MIN_SHARE_MULTIPLE}x this records. A smaller multiple would mean the under-difficulty above \
             is not actually reaching the wire"
        );
        assert!(
            per_hour > STABLE_RETARGETS_PER_HOUR,
            "and should exceed the retarget budget a stable channel is held to; it read {per_hour:.1} an \
             hour against {STABLE_RETARGETS_PER_HOUR:.0}"
        );
    }
}

/// **The cost peaks when the cycle is near the estimator's memory, which is the same resonance the
/// swinging regime finds.**
///
/// Not a new mechanism, and that is the point of measuring it: a memory of `tau` averages over `tau`, so a
/// disturbance on a comparable timescale is read at neither of its two levels. `regimes.rs` establishes
/// that for a miner alternating between two nonzero rates; this shows the same shape when one of the two
/// levels is zero, which is the case the silence machinery is involved in.
///
/// Confines the cost: a channel flapping much slower than the memory is tracked through each phase, so the
/// failure is a band rather than a general regression.
#[test]
fn the_cost_peaks_near_the_estimator_memory() {
    let near = flapping(1800, 900, 2.0);
    let far = flapping(7200, 3600, 2.0);
    eprintln!(
        "  [outage] 2 spm at 50% duty: 1800s cycle ends {:+.2}% (median {:.2}%), 7200s cycle ends \
         {:+.2}% (median {:.2}%)",
        near.final_error_pct, near.median_error_pct, far.final_error_pct, far.median_error_pct
    );

    assert!(
        near.final_error_pct < far.final_error_pct * 2.0,
        "a cycle near the memory should end materially further under-difficulty than one far above it: \
         {:+.2}% against {:+.2}%. Both are negative, so 'further' is more negative -- if this fails the \
         cost is not the resonance it is described as, and the band framing is wrong",
        near.final_error_pct,
        far.final_error_pct
    );
    assert!(
        near.median_error_pct > far.median_error_pct * 1.5,
        "and should track worse throughout, not only at the end: median {:.2}% against {:.2}%",
        near.median_error_pct,
        far.median_error_pct
    );
}
