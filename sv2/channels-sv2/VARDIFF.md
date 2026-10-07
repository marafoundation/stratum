# Vardiff: what the controller does, and where it fails

> ## Retraction (supersedes every trajectory figure below)
>
> **The share sampler this document's measurements rest on was defective for the whole life of the
> branch, and two headline findings do not survive its repair.**
>
> `run_trial` builds a fresh `XorShift64` per tick from `seed ^ elapsed`. One xorshift round does not
> carry a low-bit seed change into the top 53 bits `next_f64` reads, so the *first* draw was pinned:
> across 2880 per-tick seeds it spanned `0.385849` to `0.385864`. Since a Knuth count of zero requires
> that first draw to fall below `exp(-lambda)`, the consequences were graded by delivered rate:
>
> | delivered lambda | what the feed did |
> |---|---|
> | `<= 0.952` | **exactly zero arrivals, every tick, forever** |
> | `1` | mean inflated 4.8%, and `P(0) = 0` against a true 36.8% |
> | `2` | mean inflated 2.3%, and `P(0) = 0` against a true 13.5% |
> | `20` | mean inflated 0.2%, `P(0) = 0` |
>
> So zero-share ticks were suppressed on *every* live trajectory and the feed collapsed to constant
> zero below one arrival per tick. **The `pending_shares == 0` branch — the whole silence apparatus —
> was unreachable on any live channel.** The claim in section 5 that it is "routinely entered, 13.5% of
> ticks at 2 spm" was arithmetic the harness was structurally incapable of confirming.
>
> **Retracted outright:**
> - **The cold-start lock.** There was no lock. `shares_fed = 0` across 20 seeds over 48 hours, so the
>   controller was fed nothing; the "48-hour invariance" read as proof of a latch was proof of an empty
>   input. Repaired, the same cell converges: `-25.21%` at 6 h, `-23.29%` at 48 h, 15 then 48
>   retargets, 812 then 7101 shares.
> - **The estimator-memory coupling.** At `tau = 720` the settled errors are now `-4.05%` (2 spm),
>   `-0.66%` (4) and `-0.72%` (12) — no cell crosses the safety gate, and a sleepier memory is *better*
>   at every rate. The measured claim was `+5.84% / +4.65% / +0.64%`. "Dense channels run a memory
>   chosen to keep sparse ones safe" is withdrawn. **Re-derived below, and the replacement claim is its
>   inverse.** The `12 spm` figure quoted here is itself superseded: it reads `+1.30%` on the current
>   controller, because the tighten cap exemption landed after this bullet was written and 12 spm is above
>   the seam where the cap applies. The `2 spm` figure is unchanged at `-4.05%`, the cap never having
>   applied below the seam — which is the exemption's own signature, visible in this bullet's drift.
> - **The sparse window cap's second rejection reason.** Capping a sparse window now changes about one
>   sparse trajectory in 25 and moves the median by a fraction of a point. The rate-aware cap's rejection
>   rests solely on the crossing.
> - The stable-channel churn baselines, and therefore the flapping-churn multiples; the wander box's
>   "3 of 9 gate failures"; and the "shipped margin is thin" reading.
>
> **Survives repair:** the flapping ratchet, essentially unchanged (`-126.71%` against `-123.69%`,
> median `170.03%` against `168.27%`, 98 retargets either way) — its up-phase sat at `lambda = 2`,
> where the broken count happened to equal the mean. And the sparse/champion parity gap, which widens
> to `+13.3` to `+25.2 pp`.
>
> **Unaffected:** everything arithmetic — the constant relations and compile-time assertions, the sigma
> conventions, the seam-versus-cap disagreement, and the 27 mechanism tests, which drive the controller
> with explicit share counts rather than through the harness.
>
> Where this block sets a broken-feed reading beside its repaired one, both are as taken at the repair.
> The controller has changed since, so a repaired reading here may not be the current one, which the tests
> print and §§1–8 quote.
>
> The 14 affected tests are `#[ignore]`d with this reason rather than recalibrated, because a commit
> whose message says "find a lock" is now a false record and patching numbers under a false headline
> would leave that in the history. Re-derivation is outstanding; **treat no trajectory figure below as
> load-bearing until it is done.**
>
> **14 of the 14 re-derived. None outstanding.** The two `#[ignore]`s left in the crate are the
> pre-registered hardware assertions in `test::actuator_granularity`, which await an A/B run and are
> unrelated to this.
>
> Of the fourteen, **five conclusions inverted, two were withdrawn outright, one was found to have a
> correct numerator and a broken denominator, four survived with their numbers moved, and two survived
> with their *instrument* replaced.** That distribution is the useful summary: a defect in the instrument
> did not simply add noise, it changed which end of the rate band the findings pointed at.
>
> ### Four threads the re-derivation left open, now closed
>
> - **The memory has no headroom, and that settles `tau`.** The re-derivation read every rate as inside
>   the gate at `720` on a 5 %/hr decline, which looked like a factor of two above the shipped value.
>   `wander::the_shallow_declines_memory_headroom_does_not_survive_a_steeper_one` runs the corner neither
>   file covered -- a 10 %/hr decline *and* hot wander, where a sleepier memory and a deeper disturbance
>   push the settled position the same way. There the shipped memory holds all four rates inside the gate
>   (worst `+2.62%`) and one step sleepier puts 30 spm outside it (`+5.31%`), while worsening the
>   over-difficulty *excursion* at all four rates. **This bullet previously read "three of four outside" on
>   the settled statistic, which was a 40-trial artefact**: each crossing had about half a standard error of
>   margin, and the assertion's threshold of two sat between the low-trial reading of three and the powered
>   reading of one. Re-measured at 400 trials and re-asserted on the excursion, where it is four of four.
>   The headroom was a property of the shallow decline alone. `EWMA_TAU_SECS` stays at `360` for a
>   measured reason now, and the remaining live option is a **split at 3 spm**, not a raise -- the
>   steadiness gain that motivated raising it is sparse-only, vanishing at the dense end under hot wander
>   (`0.97x` at 12 spm, a slight loss, against `1.11x` at 2).
>
>   **The split, by contrast, passes the gate under hot wander and is live.** This bullet briefly recorded
>   the split as closed too; that was wrong and the error was in the criterion rather than in a reading. The
>   gate bounds *settled* over-difficulty, and at the corner a split of `720` below 3 spm and `360` at or
>   above it reads `+0.93 / +2.73 / -2.99 / +2.62` across 1, 2, 6 and 30 spm -- **inside the gate at every
>   rate, where a uniform `720` fails at 30 spm (`+5.31`)**. Both passes, the shipped memory's and the
>   split's, are readings under hot wander, which pulls the settled position in (next bullet). On a steady
>   miner and the same decline the margin is under half a point: `+4.57%` at 30 spm and `+4.86%` for the
>   split at 2 spm at 12,800 trials, and at 400 each lands on either side of the gate according to the seeds
>   (`tau_split.rs::on_a_steady_miner_the_gate_holds_by_under_half_a_point`, ignored for its run time).
>   **Both margins hold at those grid points and not beside them**: the split reads `+5.31%` at 2.5 spm and
>   `+5.23%` at 2.9, inside its own sparse band, and the shipped memory `+5.62%` at 40 spm and `+8.55%` at
>   90, past `MAX_ABSOLUTE_SETTLED_PCT`. All of these are the 10 %/hr decline, read on the tick its ramp
>   reaches the floor -- the lag at the ramp's end rather than a steady state.
>   The split also delivers what motivated it: on a stable channel under hot wander, chronic
>   over-delivery falls `1.267x -> 1.172x` at 1 spm and `1.194x -> 1.136x` at 2, churn falls a third,
>   and every dense cell is byte-identical.
>
>   Its price is the transient **excursion** at sparse, which roughly doubles (`16.44% -> 26.60%` at 1 spm).
>   **Nothing in this crate bounds the excursion**, and the closure claim came from treating a comparison on
>   it as a threshold -- the same error class as the instrument finding below, a criterion applied to the wrong
>   quantity. For scale rather than as a criterion, the split leaves sparse's excursion slightly worse than
>   dense's shipped `23.54%`, so it is in-family.
>
>   A related correction: the `2-3x` steeper sparse penalty is real but is mostly **sparse having more to
>   lose**. At the shipped memory sparse has the *lowest* excursion on the band (`16.44%` against `23.54%` at
>   30 spm) and at `1080` every rate converges to `26-31%`. "Sparse is the tightest cell" was the opposite of
>   true on this axis.
>
> - **"Steady is the conservative reading" holds for the settled position and not for the excursion.**
>   The third inversion above is narrower than stated. At the shipped memory hot wander pulls the settled
>   reading *in* (`+4.74% -> +2.62%` at 30 spm) while roughly doubling the worst excursion
>   (`+11.74% -> +23.54%` at 30 spm, `+7.14% -> +18.75%` at 2). Steady is conservative for the gate and
>   optimistic
>   for the transient; the two must be quoted separately.
>
> - **The over-correction now has a price.** `MIN_ABSOLUTE_SETTLED_PCT` bounded how far easy the
>   controller settles without saying what being there costs, and the assertion's own message said nothing
>   bounded it. Measured as submission load against the intended rate, the worst cell is **`1.15x`** over
>   the 3 h span -- so the cost is modest, and far below the flapping channel's `3.9x`. It is the average
>   of a load still rising when the span ends (that cell's last hour costs `1.18x`, and 2 spm on the
>   40 %/hr decline `1.26x`), and not all of it is the decline's: a steady miner at 2 spm costs `1.17x`
>   over the same span. It is not monotone in the settled reading (`-23.66%` costs `1.12x`, `-15.27%`
>   costs `1.15x`) because the load integrates the span and the settled figure is its last tick. Held at
>   `1.30x` per cell.
>
> - **The sparse/champion gap spans zero at every cell**, which is sharper than its width: shipped settles
>   under at all four (`-20.02%` to `-8.53%`) and the champion over at all four (`+0.29%` to `+7.70%`).
>   The arms differ in *which failure mode they carry*, not only in distance from correct. On the two
>   declines the champion is also past the gate (`+7.70%` at 2 spm and `+6.78%` at 4, and `+6.2%` to
>   `+7.9%` at 400 trials on three seed sets), so there the choice is between a cost and a fault.
>   `sparse_parity.rs`'s own documented gaps were stale against the widening recorded above (`+7.08` to
>   `+15.63` where the measurement was `+13.26` to `+25.20`) and its mutation note named the wrong cell;
>   both corrected, and `14.0` re-verified as the value that fails. The gaps now read `+15.24` to
>   `+23.73`, so `14.0` no longer does.
>
> The last two re-derivations are the pair in `test::regimes`, and both failed in a way a recalibration
> would have hidden:
>
> - `a_decline_is_detected_at_every_share_rate` — the conclusion holds, the **bound's shape does not.**
>   Two of six cells fail the two-sided `|residual| < 0.25 * reference`, both at 2 spm and both by
>   *over*-correcting (`-11.51%` against a `22.31%` do-nothing reading, `-23.66%` against `69.31%`).
>   Recalibrating would mean loosening to `0.52` — past the `0.5` that the magnitude form was introduced
>   to replace — which is the tell that the instrument is wrong rather than mis-set: a symmetric bound on
>   `|residual|` cannot separate *decline uncorrected* from *over-corrected*. Replaced by the fraction of
>   the do-nothing gap closed, `(reference - settled) / reference`, vacuous in neither direction.
>   Measured `0.975` to `1.516` across the grid, so the controller removes essentially all of the
>   do-nothing over-difficulty at every cell and what it leaves instead is overshoot at the sparse end.
>   The overshoot's absolute size is deliberately *not* bounded here — it does not scale with the
>   disturbance, so it belongs in `the_absolute_settled_error...`, which holds it absolutely.
>
> - `a_disturbance_inside_the_dead_zone_is_a_noise_test_not_a_decline_test` — its **premise** moved, not
>   its bound, which is why it needed re-checking rather than re-measuring. The dead zone was hardcoded in
>   prose as `18.72%` at 2 spm; it is **`112.84%`**, against the same `8.34%` disturbance. So the premise
>   holds by a far wider margin than when written, and the flat `10%` drift bound failed not because the
>   controller wandered further but because the zone it may wander inside grew sixfold. Both readings are
>   now taken from the trial — the premise is an assertion, and drift is bounded by the threshold the
>   controller actually applied — so the test is self-checking against any future move in the bar. The
>   excursion bound's `+ threshold` slack is gone: at this width it was unfailable, and the readings say
>   the controller does not reach the over-difficulty side of correct on an invisible disturbance at all
>   (worst `-2.78%` at 2 spm).
>
> `test::cold_start`'s lock claim is **deleted rather than recalibrated.** It asserted an over-declared
> sparse channel pins at `START/9` forever; that cell was fed **zero shares across 20 seeds over 48
> hours**, so the 48-hour invariance read as a fixed point was an empty input. Repaired, it converges at
> every rate -- it reaches the truth in 79 ticks at 2 spm, 14 at 12 -- and is still retargeting at 48 h. A test name stating a
> retracted finding is a false record, so the name went with the claim.
>
> `test::wander` is **retracted wholesale, and for a second reason the note above did not give: the
> disturbance was itself deterministic.** `Wander::advance` drew from the same pinned per-tick generator,
> so the log-OU received an identical shock every step and drifted to a fixed point -- a constant offset,
> not a disturbance, which made every "survives wander" claim a claim about nothing. Four of its five
> conclusions inverted. The sleepier-memory steadiness gain survives at *sparse* rates and **vanishes at
> dense ones** under hot wander (`1.11x` at 2 spm against `0.97x` at 12), which the floor decomposition
> predicts: a longer memory averages counting noise and cannot average wander slower than itself, and
> those cross near 6 shares a minute. And wander now *pulls back* the cells sitting over-difficulty, so a
> steady-miner reading is the **conservative** one -- the reverse of what this document used to warn.
>
> `test::outage` came through almost untouched, because its up-phase sits at `lambda = 2` where the broken
> count happened to equal the mean. The one number that moved was the churn **comparator**: stable
> baselines of `0.25` and `0.33` retargets an hour were themselves measured on the broken feed, so the
> flapping multiple was recorded as `33x`/`55x` when it is `10x`/`20x`. The numerator was right and the
> denominator was not.
>
> `test::window_cap_form`'s sparse-cap cost is **withdrawn**, not narrowed a second time: the matched cap
> changes about one sparse trajectory in 25 and moves the median by a fraction of a point, because with real
> sampling noise the controller fires often enough that the window seldom reaches it. Where it does, it
> moves the settled position by several points: rarely reached, not harmless.
>
> `test::regimes::the_absolute_settled_error_is_held_not_merely_the_relative_one` is re-enabled, and it
> is the **second** finding whose cause changed ends of the band. At the test's 40 trials the gate is
> still exceeded, at `+5.53%` against the recorded `+5.64%` — a number that barely moved while the cell
> producing it moved from the *sparsest* rate to the *densest*. That is about half a standard error of
> margin, and with more trials the reading falls to `+4.4` to `+5.0%` at 400, by seed, and `+4.57%` at
> 12,800, just inside the gate
> (`tau_split.rs::on_a_steady_miner_the_gate_holds_by_under_half_a_point`); what survives is the
> cell's position at the top of the over-difficulty side. Recalibrating the constant alone would have
> passed and kept a false attribution, which is the trap this whole retraction exists to avoid. The grid
> sampled only 2 and 12 spm and was therefore blind to it; `30` is now in it, the same cell `tau_split`
> independently found fixes the memory. `MAX_ABSOLUTE_SETTLED_PCT` is unchanged at `7.0`. The under side
> is what the repair moved: worst `-23.66%` against `-11.10%`, so `MIN_ABSOLUTE_SETTLED_PCT` goes
> `-15.0 -> -30.0`. Settled error rises monotonically with rate in all four decline cells, so the two
> bounds are the two ends of the band rather than two views of one failure.
>
> Two doc citations of the settled offset went with it. `MIN_THRESHOLD_FRACTION` and
> `test::classic::test_asymmetry_leaves_a_dead_zone_of_uncorrected_over_delivery` both gave it as a
> single `-6.12%` in simulation; it is a **fraction of the threshold in force**, bounded at `0.206` of
> it (at 6 spm); the largest in percent is `19.35%`, at 2 spm against a `116%` bar. One rate's reading
> had been standing in for a rate-dependent quantity — the same shape §6 names for the rate-flat
> constants. The `42%` dead-zone band itself is unaffected, being a mechanism test on explicit share
> counts, and so is the `-6.96%` hardware figure, which came off a rig rather than the harness.
>
> `test::tau_split` is re-enabled, and the re-derivation
> **inverted its conclusion rather than recalibrating it**. The mechanism survives — settled error rises
> monotonically with `tau` at every rate — but repaired, the sparse cells start well *below* zero, so a
> longer memory walks them toward correct instead of out of the gate. Settled error on a sustained 5 %/hr
> decline at the shipped `tau` rises monotonically with rate, `-12.49%` (2 spm) to `+1.90%` (30), so the
> only cell on the over-difficulty side is the densest; it is also the first to cross the 5 % gate as
> `tau` grows, at `1080`, with every cell inside it at `720`. Steadiness improves with `tau` at every
> rate and most at the sparse end (2 spm p95 `1.46x` better at one step sleepier, against `1.24x` at
> 12 spm). **So "sparse channels run a memory chosen to keep dense ones decline-safe"** — the
> withdrawn claim with its ends exchanged, and the binding cell is the rate `REFERENCE_SPM` calibrates at.
> `EWMA_TAU_SECS` is left at `360`: the headroom is measured on a shallow decline, and on a steeper
> 10 %/hr one the same dense cell is already within half a point of the gate there (`+4.57%` at 12,800
> trials, `+5.53%` at 40).
>
> `test::regimes::a_stable_channel_is_left_largely_alone` is
> re-enabled: churn budget restated as `3600 / WINDOW_CAP_SECS`, the standing offset bounded both as a
> fraction of the threshold in force and absolutely, all three verified to bind by mutation. Its absolute
> offset bound moved `12% -> 25%` because at 2 spm the repaired feed settles at `20.45%` against a
> threshold of `83%` — the old figure was measured where the broken feed delivered nothing at all.
>
> Why nothing caught it: the harness's two self-validations both drive a *persistent* generator, a mode
> no trajectory uses, so they validated the wrong configuration. And mutation testing proves an
> assertion binds against a code change; it is orthogonal to whether the input exercises the code at
> all. Every mutation on this branch was compared against the same dead feed.

A reference for `src/vardiff/classic.rs`. Every number here is produced by a test in
`src/vardiff/test/`, named inline. Run them with:

```
cargo test -p channels_sv2 --lib vardiff -- --nocapture
```

The controller's job: choose a difficulty so a miner submits `r*` shares a minute. It cannot measure
hashrate, only its own tracking error, and it must decide from a share stream that is Poisson.

**What in this document is guaranteed, and what is not.** Every reference of the form
`<file>.rs::<test>` is resolved
against the test tree by `record.rs::every_test_named_by_the_record_exists`, so a renamed test breaks the
build rather than the reader. The *figures* carry no such guarantee and are **illustrative**: the suite
asserts inequalities with headroom -- `> 5`, `gap > 3`, `> 1.2x` -- because at twenty to forty trials a
point value is not a claim, so the numbers quoted here are printed by those tests rather than asserted by
them, and a drift inside an assertion's headroom would leave the suite green and these figures stale. The
assertions are the contract; treat the numbers as the reading that produced them.

---

## 1. The whole mechanism, in one coordinate

Everything follows from **`N`, the shares in the estimator's memory**: `N = r*·τ/60`, which at the
shipped `τ = 360 s` is 12 at 2 shares a minute, 36 at 6, 72 at 12 and 180 at 30. A 15× range in
evidence per decision across the operating band.

Three quantities, and one relation between them that governs everything:

| | what it is | shipped form |
|---|---|---|
| **σ** | what the estimator can resolve | `1/√(λ·n_eff)` → `√(30/(r*·τ))`: 20.4% at 2 spm, 11.8% at 6, 5.3% at 30 |
| **b** | the bar a deviation must clear | two branches, split at `SPARSE_SPM_SEAM`; then `×8` to tighten, less a persistence discount |
| **f** | how much of the gap one retarget closes | 0.2, growing 0.05 per same-direction retarget to 0.6 |

**The relation is `b > σ`.** A bar below the estimator's own spread is a bar the controller's wobble
clears, so it fires on itself. That single comparison produces all three failure modes:

| condition | failure |
|---|---|
| `b < σ` | churn — the controller chases its own estimate |
| `b` held high too long | entrenchment — a real change goes unseen |
| `f` too large / too small | overshoot / lag |

The settled position is inside the dead zone `[−b_ease, +b_tighten]`. That zone is wide: the
thresholds actually in force at a settle point run **42–92%**, not the 11–19% the asymptotic formula
suggests, because the window runs from the last *retarget* and the recorded value includes the
tighten multiplier. No criterion asking for settled error tighter than that can be met.
(`regimes.rs::the_settled_position_is_inside_the_threshold_that_produced_it`.)

`σ` is exact rather than the closed form, matching the research instrument's `ratio_std_for`; the
closed form is its limit, good to 0.115% at a 60 s cadence and 17.6% off at 900 s
(`relations.rs::the_closed_form_for_sigma_holds_at_the_nominal_cadence`).

## 2. The noise floor has a second term, and no share rate touches it

Counting noise falls as `1/√(r*·τ)`. Physical hashrate wander does not fall at all: measured per unit,
an immersion miner wanders 3.00% of its rate on a 301 s timescale and an air-cooled one that ran hot
wanders 12.58% on 2269 s. Wander slower than `τ` is not averaged away — the estimator removes only
`√(T_c/(T_c+τ))`, so **93% of the hot unit's arrives intact**
(`wander.rs::the_wander_model_matches_the_measured_units`).

So the floor is `√(σ_count² + σ_wander²)`, and the two cross at about **6 shares a minute**. Below
that, raising `r*` buys precision. Above it, nothing does: on an unstable unit a decline shallower
than ~12% is not resolvable at any share rate.

## 3. Where the sparse regime ends: 6–8 shares a minute

Two derivations bracket it, and they share their `σ` term so they are not independent:

- wander overtakes counting noise at **6.09** spm
  (`wander.rs::counting_noise_and_the_hot_units_wander_cross_near_six_shares_a_minute`)
- the window cap is `σ`-matched at **8.03** spm (`relations.rs`)

The shipped `SPARSE_SPM_SEAM = 6.00` is an input, not a derivation — it sits at the conservative end
of that interval. `WINDOW_CAP_SECS = 900` is the `σ`-matched cap at ~8 spm: too loose below, tighter
than needed above (`relations.rs::the_seam_and_the_window_cap_disagree_about_the_same_rate`).

**`r*` is not freely choosable below the wall.** A difficulty floor makes `r*` unreachable below
`H_min = r*·D_min·2³²`; at `D_min = 23000`, six shares a minute needs **9.9 TH/s**, and a 1 TH/s
miner tops out at 0.61. So the sparse regime is where the small-miner fleet permanently lives, and
the lever that moves it is `D_min`, which is outside this crate. **This crate imposes no fleet-wide floor of
its own**: `min_allowed_hashrate` is a per-channel constructor argument on `VardiffState`, so a slow unit and a
fast one can carry different floors and a mixed fleet does not force the strictest. Whether the pool-side
`D_min` is per-channel or global is unread, and that -- not fleet composition -- is what decides how far it
could be lowered. The hard limit is the ASIC's ticket-mask floor (`1024` on the hardware here), about 4.5
octaves below `D_min`, and below it a miner produces no shares while the pool silently undercounts. Note also
that the branch selector
and the sparse bar both read the *configured* rate: forbidding a low `r*` without raising the miner's
reachable rate puts the dense branch on sparse data, which is worse than the honest configuration.

## 4. The four disturbances, measured

| disturbance | what happens | where |
|---|---|---|
| **sustained decline** | acted on at every rate; on the grid, ends the ramp within 5% over (`+4.57%`, 30 spm, 10 %/hr) and 23.7% under; past the gate above 30 spm | `regimes.rs`, `tau_split.rs` |
| **single outage** | eased to the 9× displacement bound and stops there, at every rate; the bound is anchored on the declared hashrate, so it holds across repeated outages, not only within one | `regimes.rs` |
| **steady channel** | ≤4 retargets an hour, standing offset within 25% | `regimes.rs` |
| **swing above `τ`** | read less accurately at 2.5τ than at 20τ — the long flank only; `regimes.rs` does not bracket the memory | `regimes.rs` |
| **cold start, under-declared** | converges in 7–12 retargets for 10× and 100× errors; geometric | `cold_start.rs` |
| **cold start, over-declared** | converges at every rate and settles in the dead zone — see below | `cold_start.rs` |
| **repeated outage** | chronic under-difficulty, median error up to 133% at 2 spm — see below | `outage.rs` |

## 5. Two failure modes worth knowing about

One is the controller's asymmetry and one is the actuator's granularity. They are not the pair this
section used to name: the other half was the cold-start lock, which is **retracted** -- see §Retraction.
A channel that over-declares now converges at every rate, and its settled offset is the dead zone rather
than a latch (`cold_start.rs`).

### A quantised egress parks the belief below an octave and the excess multiplies

Measured on hardware: a belief settled 9.4% below a power-of-two boundary, held for **17.9 hours**, served
one octave low, and delivered `2.27x` its target share rate throughout -- derived from the served difficulty
at the miner's ~160 TH/s. A second stall counted `2.36x`: 70.9 shares a minute against a target of 30.

**Which instrument those numbers come from, and why it now matters.** Every hardware figure in this document
is miner-side: served difficulty and share counts from the rig poller, cross-validated against the serving
port on each record. None of them comes from the A/B tick's *fires per hour* attribution -- which is
important, because that attribution is defective. It separates the miner's channel from idle ones by a
belief threshold of `5e13`, justified on the premise that idle channels sit near the deployment default of
`10 TH/s`. The deployment overrides the per-channel floor to `80 TH/s`, so **every** channel clears the
threshold and the band separates nothing. Measured on the live candidate arm: 723 band entries produce
**460** attributed "fires" while only five distinct beliefs exist, so at most **four** real changes
occurred -- a `115x` overcount. Worse than a scale error, the sign tracks miner activity: a slot whose
miner is idle reads a uniform band and scores zero, so the bias does not cancel between arms.

Two consequences. **No claim in this document depends on it** -- the quantiser witnesses, the settled
share-rate ratios, the reject rates and the octave crossings are poller-side, and the one log-derived
reading (the threshold trace that shows the window cap exceeded) excludes idle channels by an exact
`100.00%` deviation, which is exact by construction because an idle channel computes a new hashrate of
zero against the floor. What is affected is any fires-per-hour rate, and any arm comparison built on one.

**And it is recoverable rather than lost.** The translator log already emits the downstream identifier
immediately before every belief line, and idle channels compute a new hashrate of exactly zero, so
attribution by identity needs no code change at all; a tool that recounts every block from the logs on each
run can re-derive its history correctly. The defect is a value heuristic used where an identifier was
already present.

Demonstrated on the same log rather than asserted. Attributing by identifier gives three clean series --
the mining channel with 6 distinct beliefs and **5** changes over 244 evaluations, and two idle channels
with one belief, zero changes, and a computed new hashrate of exactly zero on **100%** of evaluations
against **0%** for the miner. So the zero test separates the channels perfectly with no threshold at all.

**The sharper point is that no value threshold can be made to work here.** A deliberately well-placed one
just above the floor still reads `4` against the true `5`: it drops the transition into the miner's lowest
belief, because the miner's own belief range overlaps the idle floor. It has to -- a cold belief *starts* at
that floor and climbs through it, which is the same `2.4 h` climb the reject-window analysis measures. So
this is not a mis-tuned threshold to be re-tuned; it is the wrong kind of discriminator for a quantity whose
range includes the value it is trying to exclude.

The mechanism is worth naming because it is not a coding error in either place: **a configuration override
in one repository silently falsified an instrument's documented premise in another**, with nothing linking
them. The threshold was correct against the default it was written for.

The bound is the product of two mechanisms, and the product is what binds
(`quantisation.rs::the_dead_zone_and_the_quantiser_multiply`):

```text
  r_obs / r*  =  (H_true / belief) x (D_intended / D_served)
                  <= 1 + bar           <  2
```

Neither factor is a defect alone -- the dead zone is the threshold working, and round-down is deliberate
because rounding *up* makes a miner's own filter discard shares the pool would have paid for, up to half
its hashrate. Exempting the tighten direction from the window cap moves the bound `2.96x -> 2.32x`. That puts
the `2.36x` stall **outside** it; the `2.27x` stall is still inside, so the bound does not exclude an excess
of that size. What the exemption cannot touch is the quantiser's factor of two, which is then
**76%** of the excess that remains (`1.00` of `1.32`) -- so the remaining lever is the rounding threshold,
not the bar
(`actuator_granularity.rs`, pre-registered).

And of the counted `2.36x`, the quantiser contributed `1.882x` (the intended `61,680` over the `32,768`
served) against the controller's `1.237x` (truth over belief), and their product, `2.33x`, reproduces the
count to 1.4% (`quantisation.rs::the_measured_stall_sits_inside_the_bound_and_decomposes`). On a
quantised egress most of the excess is the actuator.


### A channel that flaps sits persistently under-difficulty

Median absolute error over 12 h at 50% duty, 2 spm: 133% on a 30 min cycle, 98% on 60, 61% on 120. At
12 spm the same cells are 1.1× to 2.3× milder, not a constant factor. The median trial ends
under-difficulty at every rate and cycle, though not every trial: across twelve independent seed sets,
one or two of the eight cells have some trial over during the run, both at 12 spm, by up to +12%. On
the 120 min cycle at 12 spm up to three trials in 20 end over, and in the other cells at most one does.
Medians rather than final readings, because a cyclic trajectory's last tick lands at a fixed phase that
differs between cells -- comparing finals across cells would manufacture an effect out of phase alone.

The tighten asymmetry drives it, confirmed by mutation: weakening `TIGHTEN_MULTIPLIER` from 8.0 to 3.0
shrinks every cell's under-difficulty at the end of the run, by 1.3× to 4.7×, while the worst cell's
retargets rise from 87 to 139. The 30 min cycle shrinks least, 1.4× at 2 spm (−101% to −71%) and 1.3×
at 12, so there the asymmetry is not all of it. Easing on an outage is cheap and tightening on a return
is expensive, so the belief ratchets down. The
displacement bound does not stop the ratchet: it is anchored on the declared hashrate, so it bounds the
fall across outages at a ninth of the declaration, and the ratchet comes to rest well above that, which
is asserted separately. Within an outage it does clamp: at 2 spm on the 120 min cycle, removing it
raises the retargets from 59 to 250 and the median error from 61% to 77%, and moves where the channel
rests up by 26% (by +15% to +37% across twelve independent seed sets).

Cost, in the two units an operator pays in and neither of them payout: **3.9× the intended share
load** at 2 spm, 4.4× at 12, and 7.2–23.8 retargets an hour against the **0.83 a stable channel
actually makes** at either rate -- a churn multiple of 9× to 29×. (Against `regimes.rs`'s `<= 4` an
hour *ceiling* it would read 1.8–6×, which understates it; and it was first recorded as 33×–55× against
stable baselines that were themselves measured on the broken feed, which overstated it. The numerator
was right both times.)

It is a band rather than a general degradation, with an **interior peak at 5τ** -- 90% at `τ`, 133% at
`5τ`, 49% at `40τ`, falling away on both flanks. First recorded as "peaks near `τ`" from a sweep whose
shortest cycle was already `5τ`, so the monotone falloff seen was the far flank of a maximum that had
not been located; the position was wrong by a factor of five.

## 6. Constants that are not free choices

Four of the nineteen are determined by others. Two are enforced at compile time; two are tests.

- `MIN_EFFECTIVE_TIGHTEN_MULTIPLIER = TIGHTEN_MULTIPLIER·(1 − MAX_DIRECTION_DISCOUNT)` = 3.2. Above a
  discount ceiling of **0.875** the asymmetry *inverts* — a sustained run makes tightening cheaper
  than loosening. Compile-time assertion; it has already refused a real edit.
- `MAX_STEP_RATIO`'s easing arm needs `f > 1/ratio = 0.5`, so it first becomes reachable at retarget
  run **8** of a clamp of 9, and then needs a ~6× estimate collapse. Near-dead.
- `EVIDENCE_AT_ONE_MINUTE` and `REFERENCE_SPM` enter only as `E/√REF = 0.2739`. **One** degree of
  freedom. `REFERENCE_SPM` reads like a documentation constant and is not: moving it rescales
  sensitivity at every rate.
- `SPARSE_SPM_SEAM` and `WINDOW_CAP_SECS` answer one question and disagree (§3).

A recurring shape worth naming: `WINDOW_CAP_SECS`, `MAX_SILENT_DISPLACEMENT` and the seam are all
rate-flat constants standing in for rate-dependent quantities. `MAX_SILENT_DISPLACEMENT` in
particular bounds a *ratio* where its own rationale is about a share *flood* — the same 9× permits
18 shares a minute on return at `r* = 2` and 540 at `r* = 60`. And the ratio is to whatever the
channel opened at: a translator opens each SV1 miner, which declares nothing, at its configured
minimum hashrate, so a miner running at twice that minimum can be eased to 18× below its truth.

## 7. What is not known

- **The wander model is one archive, two units, one uncharacterized sensor.** Closing it needs a second
  measured archive; no amount of simulation reaches it, because the quantity that matters at an aggregate
  is the *covariance* between units and one archive cannot show a covariance.
  *(Re-derived. The 3×3 sensitivity box previously cited here — "the memory coupling survives as an
  ordering", "the shipped controller exceeds the gate in three of nine" — was measured when the wander
  disturbance was itself deterministic, and is withdrawn with the rest of §Retraction. What replaces it:
  `wander.rs::the_wander_process_reproduces_its_own_parameters` checks the disturbance is a process at
  all, and `wander.rs::wander_pulls_back_the_cells_that_sit_over_difficulty` records that wander is now
  **protective** with respect to the over-difficulty gate — so a steady-miner reading is the conservative
  one, which is the reverse of what this section used to warn about.)*
- **Statistical power.** Mostly 20–400 trials on 4–12 hour runs, medians, and 12,800 where a margin is
  under half a point. Figures are quoted to three
  significant digits; the tests assert robust inequalities instead, and those are what to rely on.
- **`τ` is chosen by the *dense* band, and the coupling runs the other way.** Settled error on a
  sustained 5 %/hr decline rises monotonically with rate — −12.49% at 2 spm to +1.90% at 30 — so the only
  cell on the over-difficulty side at the shipped memory is the densest, and it is the first to cross the
  5% gate as `τ` grows (at 1080, with every cell inside at 720). Steadiness meanwhile improves with `τ` at
  every rate and most at the sparse end (2 spm p95 1.46x better one step sleepier, against 1.24x at
  12 spm). **So sparse channels run a memory chosen to keep dense ones decline-safe**, and the
  binding cell is the rate `REFERENCE_SPM` calibrates at (`tau_split.rs`, `wander.rs`). Every figure in this
  bullet is the shallow decline; the corner adds that a sleepier memory costs the sparse band two to three
  times what it costs dense on the *excursion*, which prices a split without refusing one. **A uniform raise
  is closed by the gate; a split passes it at every rate under hot wander and is a live calibration
  decision** with both sides now measured — `5–7.5%` less chronic over-delivery and a third less churn at
  sparse, against roughly double the transient excursion there. On a steady miner it clears the gate by
  under half a point at the 2 and 30 spm grid points, and not between or beyond them (§Retraction).
  *(Re-derived. This bullet previously asserted the exact inverse — "refused by 2–4 spm", "dense channels
  run a memory chosen to keep sparse ones safe" — measured when the share feed starved the sparse cells.
  The inversion holds under the worst measured wander too: at 720 s, 2 spm reads −4.74% and 30 spm
  +0.60%.)*
- **No second disturbance shape** beyond those above: no ramp, no partial recovery, no
  correlated fleet-wide event. The last is the one that matters, because it is also what sets the
  aggregate noise floor the second archive is needed for.

## 8. The honest statement of what this controller can promise

Not "converges to `r*`". It converges to *within a dead zone of order σ* of `r*`; it bounds the
excursion during each outage, though not the fall across repeated ones; and disturbances below the
hardware's own wander are invisible to it at any share rate. Those three are verifiable. The first
formulation is not achievable by any controller, and the reason is the information floor rather than
this implementation.
