//! **Pre-registration: make the actuator fine-grained, and what that should do.**
//!
//! Registered before the run, deliberately. Two instrument failures landed on this branch in one week --
//! a share sampler that fed a constant, and a record that claimed a guard it did not have -- and in both
//! cases the reading was believed because nothing had been committed to beforehand. So the predictions
//! below are written down, with thresholds and kill criteria, and the hardware assertions are `#[ignore]`d
//! until data exists rather than authored afterwards to fit it.
//!
//! # 1. Theory
//!
//! The actuator is the difficulty a miner is actually served. Define its **granularity** as
//!
//! ```text
//!   g = D_intended / D_served     in [1, 2) under power-of-two round-down
//! ```
//!
//! and recall the settled-excess bound from `quantisation.rs`: `excess <= (1 + bar) * g_max`, the product
//! of the controller's dead zone and the actuator's coarseness. At the shipped asymptotic tighten bar of
//! `16%` that gives `1.16 * 2 = 2.32x`, and the actuator is `1.00` of that `1.32` excess, `76%`.
//!
//! **The claim.** Granularity is not a property of the hardware or of SV1. It is a threshold in our own
//! translator, and above that threshold we round. Three independent reasons, all recorded:
//!
//! 1. the firmware's `mining.set_difficulty` handler stores the value and **never wires it to hardware**;
//! 2. units of the same firmware family run non-power-of-two difficulties on the SV2 path, so the hashing
//!    engine is demonstrably content with arbitrary integers;
//! 3. our own translator already emits fractional `f64` *below* the threshold, so the code path exists --
//!    though **not that it is exercised**, which is corrected in the premise check at the end of this file.
//!
//! **What remains after the change.** The firmware keeps whole difficulty, so the residual granularity is
//! one unit of difficulty: `g = 1 + 1/D`. At an operating difficulty near `500,000` that is
//! `1 + 2e-6` -- six orders of magnitude finer than the octave it replaces.
//!
//! So the bound should fall by a factor of two, to the controller's own dead zone and no further:
//!
//! | | actuator granularity | settled excess bound |
//! |---|---|---|
//! | today | `< 2` | `2.32x` |
//! | after | `1 + 2e-6` | **`1.16x`** |
//!
//! `1.16x` is then irreducible without changing the bar itself, which is a separate decision.
//!
//! # 2. The one hard constraint, which is genuinely hardware
//!
//! The ASIC's share threshold is a ticket mask, which selects by leading-zero count and therefore **can
//! only express a power of two**. It is fixed at init, to `1024` on the hardware here. Below it the miner
//! produces no shares at all and the pool **silently undercounts hashrate rather than erroring**.
//!
//! That is a *floor*, not a lattice, and it sits about nine octaves below the operating point -- which is
//! exactly why the pow2 setpoints we see are not the mask showing through. The experiment must stay clear
//! of it, and `the_operating_point_clears_the_ticket_mask_floor` asserts the margin.
//!
//! # 3. Procedure
//!
//! * **Change.** Raise `SV1_MIN_DIFFICULTY_FOR_INTEGER_POWER_OF_TWO_ROUNDING` above the served difficulty
//!   on one A/B candidate slot, so the translator emits the unrounded value.
//! * **Rollback.** See the premise check at the end of this file -- the claim this bullet used to make, that
//!   the change is a config edit with no rebuild, is **false for the deployed build**.
//! * **Scope.** One slot. Not fleet-wide, because this changes what a production-adjacent miner is served.
//! * **Arms.** Alternate against control on the existing schedule so the contrast is paired rather than
//!   historical.
//! * **Instrument.** The rig sampler already records `Last Share Difficulty`, realized share rate and
//!   reject rate every 30 s, and self-attributes the arm from the pool URL.
//!
//! # 4. Admissibility gate -- run BEFORE reading any outcome
//!
//! The sampler episode's lesson, applied: validate the instrument in the mode it is used in, first.
//!
//! * **G1.** The sampler must record a *changed* served difficulty within one polling interval of the
//!   config change. Until it does, a null result is indistinguishable from a config that never applied.
//! * **G2.** The advertised target and the target share validation uses must agree in the logs. Our
//!   translator stores the advertised target for exactly this reason; if they ever disagree, every share
//!   between the two is rejected and the run measures that instead.
//!
//! **No outcome below is admissible until G1 and G2 both hold.**
//!
//! # 5. Pre-registered predictions
//!
//! Stated with the threshold that would falsify each, so none can be read after the fact.
//!
//! | | prediction | falsified if |
//! |---|---|---|
//! | **P1** | ~~realized share rate falls from `~2.3x` target to within the dead zone, `<= 1.35x`~~ | **superseded -- already satisfied post-fix, see premise check #5** |
//! | **P2** | `Last Share Difficulty` reports a non-power-of-two value | reports a power of two |
//! | **P3** | ~~reject rate stays inside the measured `0.11-0.15%` baseline band, `<= 0.5%`~~ | **corrected -- floor is `0.000%` and the band was wrong, see premise check #5** |
//! | **P4** | `set_difficulty` message rate rises, but by less than `10x` | rises more than `10x` |
//! | **P5** | ~~no change below `SPARSE_SPM_SEAM`, where the threshold already emits fractional values~~ | **retired -- premise false, see below** |
//! | **P5'** | the *control* arm reports only powers of two throughout | any non-power-of-two on control |
//!
//! **P4 is the one I expect to bite and the reason this is registered rather than just done.** Today a
//! belief move smaller than an octave changes the served value not at all, so it never reaches the wire.
//! With a fine actuator every fire does. The cost is unknown, the corpus records a mid-job `SetTarget`
//! failure mode, and `~23 retargets/hour` was priced as unacceptable when dithering was evaluated. If P4
//! fails, the fix is a **minimum-change threshold on the egress** -- which reintroduces granularity, but
//! chosen deliberately and far finer than an octave.
//!
//! # 6. Kill criteria -- roll back immediately, do not wait for the window to end
//!
//! * reject rate above `1%` sustained over 15 minutes, **measured more than 60 minutes after any reconnect**
//!   -- unqualified, this criterion fires on the restart the change itself requires (premise check #5);
//! * served difficulty below `1024`;
//! * realized share rate *rising* rather than falling;
//! * any `set_difficulty` rate that trips the known mid-job `SetTarget` failure mode.
//!
//! # 7. What this cannot settle
//!
//! * **Other firmware families.** The read above is of one source tree and is not independently verified.
//!   A mixed fleet may contain a unit that does wire the value.
//! * **The pool's own SV1 server.** Still unread. It serves powers of two and may round independently of
//!   anything we do, in which case our threshold change has no effect on that path.
//! * **Payout.** Nothing here touches it: a share is credited at the difficulty it was found at, so the
//!   quantum is a load term, not a revenue term, in either direction.

use super::quantisation::{asymptotic_tighten_bar, over_delivery_bound, round_down_pow2, OCTAVE};

/// The ASIC share threshold, a ticket mask fixed at init, and the floor below which a miner produces no
/// shares while the pool silently undercounts.
const TICKET_MASK_FLOOR: f64 = 1024.0;

/// Representative served difficulty on the fleet, for sizing the margins above.
const OPERATING_DIFFICULTY: f64 = 500_000.0;

/// Residual granularity after the change: the firmware truncates to whole difficulty.
fn truncation_granularity(difficulty: f64) -> f64 {
    1.0 + 1.0 / difficulty
}

/// **The bound is linear in the actuator's granularity, so halving the quantum halves the excess.**
///
/// The theory the experiment tests, as arithmetic. Registered now so the predicted magnitude cannot drift
/// to match whatever the run produces.
#[test]
fn removing_the_quantum_halves_the_settled_excess_bound() {
    let bar = asymptotic_tighten_bar();
    let with_pow2 = over_delivery_bound(bar);
    let after = (1.0 + bar) * truncation_granularity(OPERATING_DIFFICULTY);

    eprintln!(
        "  [prereg] dead zone {:.0}%: bound {with_pow2:.3}x with a {OCTAVE}x quantum, {after:.3}x with \
         truncation at difficulty {OPERATING_DIFFICULTY:.0} -- a factor of {:.2}",
        bar * 100.0,
        with_pow2 / after
    );

    assert!(
        (with_pow2 / after - OCTAVE).abs() < 1e-3,
        "the bound is a product, so removing a factor-of-two quantum must divide it by two: \
         {with_pow2:.4} against {after:.4}"
    );
    assert!(
        (after - (1.0 + bar)).abs() < 1e-3,
        "and what remains must be the controller's own dead zone and nothing else: {after:.4} against \
         {:.4}",
        1.0 + bar
    );
    // The residual granularity must be negligible, or "fine" is the wrong word for it.
    assert!(
        truncation_granularity(OPERATING_DIFFICULTY) < 1.000_01,
        "truncation to whole difficulty should leave granularity under 1e-5 at the operating point; it is \
         {:.8}",
        truncation_granularity(OPERATING_DIFFICULTY)
    );
}

/// **The operating point clears the ticket-mask floor by nine octaves, which is why the change is safe.**
///
/// The floor is the one genuine hardware constraint and the one failure that is silent: below it the miner
/// produces nothing and the pool undercounts rather than erroring. Asserted rather than assumed, because a
/// later proposal that lowers the pool's minimum difficulty toward `1024` would hit it.
#[test]
fn the_operating_point_clears_the_ticket_mask_floor() {
    let octaves = (OPERATING_DIFFICULTY / TICKET_MASK_FLOOR).log2();
    eprintln!(
        "  [prereg] operating difficulty {OPERATING_DIFFICULTY:.0} sits {octaves:.1} octaves above the \
         {TICKET_MASK_FLOOR:.0} ticket-mask floor"
    );

    assert!(
        octaves > 8.0,
        "the operating point must clear the ticket-mask floor by a wide margin, or removing the pool-side \
         lattice brings the hardware one into range: {octaves:.1} octaves"
    );
    // And the floor itself is a power of two, which is the coincidence of form that made this confusing:
    // the mask can only express 2^N, so the hardware constraint looks like the server-side policy.
    assert!(
        (round_down_pow2(TICKET_MASK_FLOOR) - TICKET_MASK_FLOOR).abs() < f64::EPSILON,
        "the ticket-mask floor is itself a power of two -- that is why it was mistaken for the lattice"
    );
}

/// **P1: the realized share rate falls into the dead zone.** Un-ignore when the run produces data.
#[test]
#[ignore = "pre-registered; awaiting the A/B run. Do not un-ignore without G1 and G2 satisfied"]
fn p1_realized_rate_falls_into_the_dead_zone() {
    /// Fill in from the candidate arm once G1 and G2 hold: `None` -> `Some(x)`.
    const MEASURED_EXCESS: Option<f64> = None;
    /// Registered threshold: within the dead zone plus room for counting noise.
    const PREDICTED_AT_MOST: f64 = 1.35;
    /// Registered falsification: no material improvement on the 2.3x baseline.
    const FALSIFIED_ABOVE: f64 = 1.8;

    let Some(measured) = MEASURED_EXCESS else {
        panic!("no data yet -- this test is the registration, not the result")
    };
    assert!(
        measured <= PREDICTED_AT_MOST,
        "P1 predicted the excess falls to {PREDICTED_AT_MOST}x or below; measured {measured:.2}x. \
         Above {FALSIFIED_ABOVE}x the theory is falsified and the quantum was not the dominant term"
    );
}

/// **P4: the message rate rises, and this is the cost the whole change turns on.** Un-ignore with data.
///
/// Registered separately from P1 because it is the prediction most likely to fail, and because failing it
/// does not falsify the theory -- it identifies the follow-on work. A fine actuator means every fire
/// reaches the wire where sub-octave moves previously reached nothing.
#[test]
#[ignore = "pre-registered; awaiting the A/B run. Do not un-ignore without G1 and G2 satisfied"]
fn p4_the_message_rate_rise_is_tolerable() {
    /// Fill in as candidate/control, both in `set_difficulty` sends per hour: `None` -> `Some(x)`.
    const MEASURED_RATIO: Option<f64> = None;
    /// Registered threshold. Above this the remedy is a minimum-change threshold on the egress --
    /// granularity chosen deliberately, and still far finer than an octave.
    const PREDICTED_BELOW: f64 = 10.0;

    let Some(measured) = MEASURED_RATIO else {
        panic!("no data yet -- this test is the registration, not the result")
    };
    assert!(
        measured < PREDICTED_BELOW,
        "P4 predicted the message rate rises by less than {PREDICTED_BELOW}x; measured \
         {measured:.1}x. This does not falsify the bound -- it says a minimum-change threshold is \
         needed, and that its value is the next thing to choose"
    );
}

// ---------------------------------------------------------------------------------------------------------
// Premise check against the deployed build, 2026-09-23
// ---------------------------------------------------------------------------------------------------------

/// **The procedure above was wrong about the deployed build, and checking first is why this is known.**
///
/// The deployed build is `sv2-apps 8145cf88`, on branch `deploy/vardiff-cap-exemption`. Four things were
/// read before anything was changed, and three of them contradict what this file asserted:
///
/// **1. The threshold is a compile-time constant, not a config key.** On the deployed build it is
/// `const SV1_MIN_DIFFICULTY_FOR_INTEGER_POWER_OF_TWO_ROUNDING: f64 = 1.0` in
/// `miner-apps/translator/src/lib/sv1/sv1_server/mod.rs`. The commit this file cites as making it
/// configurable, `fc3bdb3b`, exists but is **not an ancestor of `HEAD`** -- it sits on an unmerged branch,
/// `feat/configurable-pow2-rounding`. So "one config edit, no rebuild, no redeploy" is false. The change
/// requires merging that branch or editing the constant, then a rebuild and redeploy of the translator, and
/// the rollback costs the same. That is a materially larger action than this file priced.
///
/// **2. Nothing is emitted unrounded in production, so `P5`'s premise is false.** The rounding policy in
/// `stratum-core/stratum-translation/src/sv2_to_sv1.rs` emits difficulties *below* the threshold unchanged and
/// rounds those *at or above* it, and rejects any threshold below `1.0`. The deployed value **is** `1.0` --
/// the minimum legal setting -- so every difficulty is at or above it and **everything is rounded**. There is
/// no unrounded regime to be unaffected. `P5` cannot be tested as written and is retired rather than
/// reinterpreted.
///
/// `P5'` replaces it and is worth more: at a threshold of `1.0` the control arm *must* report only powers of
/// two. If it ever reports otherwise, something else in the path is emitting unrounded values and the
/// experiment is measuring that instead. That makes it a third admissibility gate rather than a prediction.
///
/// **3. The deployed slot is already carrying a different live treatment.** It pins the window-cap tighten
/// exemption (`585fd805`) for a rig test, which is the whole purpose of `8145cf88`. Raising the rounding
/// threshold on that same slot would stack two treatments and confound both
/// -- the exemption moves the settled-excess bound `2.96x -> 2.32x`, and the actuator change is predicted to
/// halve whatever the bound is, so their effects are not separable after the fact. This file's `Scope` bullet
/// says "one slot" and never considered that the slot might not be idle.
///
/// **4. The one thing that checked out.** The never-round-*up* property is implemented and documented at
/// `sv2_to_sv1.rs:147`, with the reasoning this corpus recorded independently: upstream credits shares against
/// its own unrounded target, so telling a miner a *higher* difficulty silently discards everything between
/// the two -- up to half its hashrate. Rounding down only makes the miner submit some shares below the
/// upstream target, which the proxy filters. That is the safety property the experiment leans on, and it is
/// real.
///
/// # What this means for running it
///
/// The experiment is **blocked on a decision and a rebuild.**
/// Three things have to happen first, none of them measurement:
///
/// 1. either merge `feat/configurable-pow2-rounding` so the threshold really is config-only, or accept a
///    translator rebuild and redeploy on both the change and the rollback;
/// 2. pick a slot that is not mid-test, or wait for the cap-exemption rig test to conclude, so the two
///    treatments do not stack;
/// 3. confirm `G1`, `G2` and now `P5'` before reading any outcome.
///
/// ## Status: (1) is done, (2) is outstanding
///
/// A deploy candidate exists at `sv2-apps deploy/vardiff-cap-exemption-pow2-config`, which merges the
/// configurable-threshold branch onto `8145cf88`. The merge was textually clean -- the two parents touch
/// disjoint files -- and it is **behaviour-neutral until the key is set**, because the config default is
/// `1.0`, the value the constant already held. So it restores the one-edit rollback this file was designed
/// around rather than merely enabling the change. Verified: all three workspaces build, including under
/// `--locked`; the translator's 47 lib tests pass; and the library's seven rounding-threshold tests pass,
/// among them the unrounded path (`test_integer_power_of_two_difficulty_keeps_value_below_threshold`) and
/// the configurable threshold itself.
///
/// Two incidental defects were fixed to get there, both pre-existing and neither caused by the merge:
///
/// * a translator test asserting a vardiff update that the controller correctly declines to make -- it drove
///   one share in two minutes against five a minute expected, a `90%` deviation, against a threshold in force
///   of `91.46%`. It sat `1.46` points inside the sparse bar. That is this crate's own dead-zone finding
///   turning up as a red test in another repo, and it is the tightest illustration of it yet: a tenfold
///   difficulty error is *inside* the bar at 5 spm on one share.
/// * three lockfiles carrying three different stratum-core revisions (`58a8d641`, `99ca8d03`, `99ca8d03`)
///   against manifests requesting `7534f290`. Deployed binaries were unaffected, since an exact rev pin
///   resolves deterministically whatever the lock says, but the `--locked` CI job cannot have passed.
///
/// ## A fourth premise failure, and this one would have stopped the run
///
/// The three above were found by reading sv2-apps. A fourth was found by reading the *deployment tooling*,
/// which this file never considered at all: it said "raise the key on one A/B candidate slot" as though the
/// key were reachable from a config file. It was not.
///
/// The deployment tooling **generates** each slot's `translator.toml` from a template, and that template did
/// not emit the rounding threshold -- so every generated config omitted the key and fell back to the serde
/// default of `1.0`, round everything. Merging the sv2-apps branch made the threshold configurable *in the
/// code*; it did not make it settable *in a deployment*. Those are different claims and this file conflated
/// them. The template now emits the key.
///
/// That is the fourth time on this branch that checking a premise beat trusting it, and the first time the
/// payoff landed outside this repository.
///
/// # The run, concretely
///
/// Kept here rather than in a separate runbook so there is one source of truth; a second document describing
/// the same procedure is a doc-drift hazard, which this suite already guards against elsewhere.
///
/// **Step 0.** Deploy with a build of the deployment tooling whose template emits the key. An older build
/// emits a `translator.toml` without the key however recent the sv2-apps branch is. This is the failure the
/// fourth premise check found; it is easy to reproduce by forgetting this step.
///
/// **Step 1.** Deploy `deploy/vardiff-cap-exemption-pow2-config` to two free slots, a candidate and a
/// control. Identical binaries, both behaviour-neutral, because the key defaults to `1.0`. The slot running
/// the cap-exemption test is untouched, so that test stays interpretable.
///
/// **Step 2. Gates, before any outcome is read.** `G1` the sampler records a changed served difficulty within
/// one polling interval; `G2` the advertised target and the target share validation uses agree in the logs;
/// `P5'` the control slot reports only powers of two. `P5'` is forced at a threshold of `1.0`, so a violation
/// means something else in the path is un-rounding and the run would be measuring that instead.
///
/// **Step 3. The treatment**, on the candidate only, in `[downstream_difficulty_config]`:
///
/// ```text
///   minimum_difficulty_for_integer_power_of_two_rounding = 10000000
/// ```
///
/// Twenty times the operating difficulty, so every served value falls below it and is emitted exact. Restart
/// the translator. **Rollback is the same edit back to `1.0` and a restart** -- which is what merging the
/// config branch bought, and the reason that merge was worth doing first.
///
/// **Step 4.** Read `P1` through `P4` paired against the control, and stop on any kill criterion in section 6.
///
/// # Premise check #5, from the hardware handoff: two predictions and a kill criterion are wrong
///
/// The cap-exemption run settled on hardware at 20:44:23Z on 2026-09-23, and its numbers falsify three
/// things written above. All three were authored against the *pre-fix* world and none survived the fix they
/// were waiting on.
///
/// **`P1` is already satisfied, so it cannot be tested.** It predicts the realized share rate falls from
/// `~2.3x` to `<= 1.35x`. Post-fix the rig measures `34.91` shares a minute against a `30` target --
/// **`1.164x`** -- which is already inside `P1`'s own threshold before the actuator changes anything. The
/// cap exemption alone did what `P1` was written to detect.
///
/// Restated against the new baseline, and it is a far sharper test. The handoff reports the residual as
/// *entirely* quantiser: served difficulty `65,536`, so an intended difficulty near `76,284` sits `16.4%`
/// above the octave boundary it is rounded down to, and the controller's own dead-zone factor is about
/// `1.00`. Removing the quantiser should therefore take `1.164x` to `1.000x`, since
/// [`truncation_granularity`] at the operating point is `1 + 2e-6`.
///
/// | | prediction | falsified if |
/// |---|---|---|
/// | **`P1'`** | realized rate falls `1.164x -> <= 1.02x` | stays above `1.10x` |
///
/// **`P3` and kill criterion 1 are both wrong, and the kill criterion would fire on the change itself.**
/// `P3` asserts a `0.11-0.15%` reject baseline. Measured on both arms: **`2.954%` on candidate and `1.165%`
/// on control in the first ten minutes after a reconnect, decaying to exactly `0.000%` after sixty
/// minutes.** So the settled floor is `0.000%`, not `0.11%`, and the transient exceeds the `1%` kill
/// threshold by threefold.
///
/// Raising the rounding threshold requires a translator restart, which causes a reconnect. **The kill
/// criterion as written would have aborted this experiment as a direct consequence of starting it**, and the
/// rollback would have been read as a safety response rather than as an artefact. Corrected:
///
/// * evaluate rejects **only more than 60 minutes after any switch or reconnect**, against a floor of
///   `0.000%`;
/// * inside that window a reject rate up to `~3%` is expected on candidate and carries no information.
///
/// **And there is a second, longer settling window that nothing above accounts for.** A reconnect resets the
/// belief, and from a cold start at the `80 TH/s` floor the climb back to `65,536` takes **`~2.4 h`**. So
/// reject readings need 60 minutes and *difficulty* readings need about two and a half hours. Two windows,
/// different lengths, and the difficulty one is the binding constraint on how quickly this experiment can
/// produce a reading. Log the restart timestamp so the re-climb is not mistaken for a regression.
///
/// Incidentally this confirms that `min_individual_miner_hashrate` is genuinely where a cold belief starts,
/// so a slot configured orders of magnitude below the `80 TH/s` floor is not a cosmetic difference.
///
/// ## A prediction to revise before reading it
///
/// `P4` expects the `set_difficulty` rate to rise and bounds the rise at `10x`. It may barely rise at all. The
/// fleet's small miners sit in the sparse regime, where the threshold in force is measured near `100%`
/// (`quantisation::the_shipped_bound_covers_the_dense_branch_only`), so most belief moves never clear the bar
/// to reach the wire regardless of how fine the actuator is. **A small rise would confirm the dead-zone
/// finding rather than being a null result**, and it is worth writing that down now so it is not read as luck
/// afterwards.
///
/// This test asserts the arithmetic of the first point, which is the only part of it that is checkable here.
#[test]
fn the_deployed_rounding_threshold_rounds_everything() {
    /// The value compiled into the deployed build.
    const DEPLOYED_THRESHOLD: f64 = 1.0;

    // Every difficulty the fleet could plausibly be served is at or above the deployed threshold, so all of
    // it is rounded. This is the arithmetic behind "there is no unrounded regime in production".
    for difficulty in [
        TICKET_MASK_FLOOR,
        23_000.0,
        OPERATING_DIFFICULTY,
        1.0e9,
        DEPLOYED_THRESHOLD,
    ] {
        assert!(
            difficulty >= DEPLOYED_THRESHOLD,
            "a served difficulty of {difficulty} would fall below the deployed threshold of \
             {DEPLOYED_THRESHOLD} and be emitted unrounded. If this ever fails the premise check above is \
             stale and P5 may be testable again"
        );
    }

    // And the granularity that follows: a full octave, everywhere, rather than only above some threshold.
    let served = round_down_pow2(OPERATING_DIFFICULTY);
    let granularity = OPERATING_DIFFICULTY / served;
    eprintln!(
        "  [prereg] deployed threshold {DEPLOYED_THRESHOLD}: difficulty {OPERATING_DIFFICULTY:.0} is served \
         as {served:.0}, granularity {granularity:.3}x -- and nothing on the fleet is below the threshold, \
         so this applies band-wide"
    );
    assert!(
        granularity > 1.0 && granularity < OCTAVE,
        "the granularity must sit inside the octave: {granularity:.3}x"
    );
}
