# Vardiff: what the controller does, and where it fails

A reference for `src/vardiff/classic.rs`. Every number here is produced by a test in
`src/vardiff/test/`, named inline, so this file cannot drift from the code without a test failing.
Run it with:

```
cargo test -p channels_sv2 --lib vardiff -- --nocapture
```

The controller's job: choose a difficulty so a miner submits `r*` shares a minute. It cannot measure
hashrate, only its own tracking error, and it must decide from a share stream that is Poisson.

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

So the floor is `√(σ_count² + σ_wander²)`, and the two cross at about **7 shares a minute**. Below
that, raising `r*` buys precision. Above it, nothing does: on an unstable unit a decline shallower
than ~12% is not resolvable at any share rate.

## 3. Where the sparse regime ends: 6–8 shares a minute

Two derivations bracket it, and they share their `σ` term so they are not independent:

- wander overtakes counting noise at **7.07** spm
- the window cap is `σ`-matched at **8.03** spm (`relations.rs`)

The shipped `SPARSE_SPM_SEAM = 6.00` is an input, not a derivation — it sits at the conservative end
of that interval. `WINDOW_CAP_SECS = 900` is the `σ`-matched cap at ~8 spm: too loose below, tighter
than needed above (`relations.rs::the_seam_and_the_window_cap_disagree_about_the_same_rate`).

**`r*` is not freely choosable below the wall.** A difficulty floor makes `r*` unreachable below
`H_min = r*·D_min·2³²`; at `D_min = 23000`, six shares a minute needs **9.9 TH/s**, and a 1 TH/s
miner tops out at 0.61. So the sparse regime is where the small-miner fleet permanently lives, and
the lever that moves it is `D_min`, which is outside this crate. Note also that the branch selector
and the sparse bar both read the *configured* rate: forbidding a low `r*` without raising the miner's
reachable rate puts the dense branch on sparse data, which is worse than the honest configuration.

## 4. The four disturbances, measured

| disturbance | what happens | where |
|---|---|---|
| **sustained decline** | acted on at every rate; settles within 5.6% over / 11.1% under | `regimes.rs` |
| **single outage** | eased to the 9× displacement bound and stops there, at every rate | `regimes.rs` |
| **steady channel** | ≤4 retargets an hour, standing offset within 12% | `regimes.rs` |
| **swing near `τ`** | averaged rather than followed — a resonance, not a general regression | `regimes.rs` |
| **cold start, under-declared** | converges in 4–9 retargets for 10× and 100× errors; geometric | `cold_start.rs` |
| **cold start, over-declared** | **locks** at sparse rates — see below | `cold_start.rs` |
| **repeated outage** | chronic under-difficulty, 1.6–3.4× — see below | `outage.rs` |

## 5. Two failure modes worth knowing about

Both are the same inability — the controller cannot tell an absent miner from a wrong belief — and
they fail in **opposite directions**, with different causes and different fixes.

### A channel that over-declares by 100× at a sparse rate never recovers

It comes to rest at 11× over-difficulty and stays: identical error, hashrate and retarget count at 6,
12, 24 and 48 hours. The resting place names the cause — exactly `START/9`, the
`MAX_SILENT_DISPLACEMENT` floor. Confirmed by mutation: at `90` the same channel converges.

The mechanism: a channel set 100× too hard delivers `r*/100`, one share per fifty minutes at 2 spm.
Almost every evaluation sees none, so `evidenced_hashrate` never leaves the *opening* belief — a
number no evidence ever supported — and the ease floors at one ninth of it. **This falsifies the
constant's own former doc**, which said a miner that still submits is never held back by the bound.
This one submits and is held back indefinitely. The same over-declaration at 12 spm recovers in 77
ticks, which is what identifies the rate rather than the magnitude as the variable.

So the 9× bound is a bounded-regret policy in *both* directions: it bounds a returning miner's share
flood, and it also bounds how far an over-declaration can be corrected. At sparse rates the second
cost is unbounded in duration. **Unrepaired** — the fix is a choice among anchoring on something other
than an unverified opening belief, distinguishing silence from over-difficulty, and admitting fewer
sparse channels.

### A channel that flaps sits persistently under-difficulty

Settled error over 12 h, by cycle length at 50% duty: −123.69% at 30 min, −72.76% at 60, −20.60% at
120 (2 spm; 12 spm is 2–3× milder). Always under, never over, at every rate and cycle.

The cause is the tighten asymmetry, confirmed by mutation: weakening `TIGHTEN_MULTIPLIER` from 8.0 to
3.0 shrinks the worst cell by the same factor of 2.7, while retargets rise from 98 to 157. Easing on
an outage is cheap and tightening on a return is expensive, so the belief ratchets down. The
displacement bound is *not* what binds here, which is asserted separately — the two failures do not
share a fix.

Cost, in the two units an operator pays in and neither of them payout: **5.1× the intended share
load** at 2 spm, 4.1× at 12, and 8.2–18.0 retargets an hour against the 4 a stable channel is held
to. It is a resonance: the cost peaks near `τ` and falls off either side.

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
18 shares a minute on return at `r* = 2` and 540 at `r* = 60`.

## 7. What is not known

- **The wander model is one archive, two units, one uncharacterized sensor.** A 3×3 sensitivity box
  around it shows the memory coupling survives as an ordering (≥3.3 pp in all nine cells) but that
  "sparse fails the safety gate" holds in only six, **and that the shipped controller itself exceeds
  the gate in three**. That is sensitivity to a model's parameters, not evidence about hardware;
  closing it needs a second archive (`wander.rs::the_conclusions_that_survive_the_whole_wander_box`).
- **Statistical power.** 20–40 trials on 4–12 hour runs, medians. Figures are quoted to three
  significant digits; the tests assert robust inequalities instead, and those are what to rely on.
- **`τ` is chosen by the sparse band.** One step sleepier is refused by 2–4 spm and would be
  comfortably safe at 12, where it is worth p95 tracking error 12.95% → 9.68%. Dense channels run a
  memory chosen to keep sparse ones safe (`tau_split.rs`). Splitting it is two lines; which way each
  side should move is a policy call.
- **No second disturbance shape** beyond the seven above: no ramp, no partial recovery, no
  correlated fleet-wide event.

## 8. The honest statement of what this controller can promise

Not "converges to `r*`". It converges to *within a dead zone of order σ* of `r*`; it bounds the
excursion during an outage; and disturbances below the hardware's own wander are invisible to it at
any share rate. Those three are verifiable. The first formulation is not achievable by any
controller, and the reason is the information floor rather than this implementation.
