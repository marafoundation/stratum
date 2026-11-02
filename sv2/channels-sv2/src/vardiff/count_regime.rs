//! The count-regime vardiff core, BP-G: a port of the reference implementation, with its parameters, at a 60 s
//! evaluation.
//!
//! The core keeps a posterior over the miner's share rate, in rate units of the declared hashrate (rate 1 = declared,
//! so `d = 0` at the declared hashrate). Each evaluation takes `n`, the valid shares since the last, and `x`, the
//! exposure: the shares expected at rate 1 for the targets in force. It returns whether the log belief `d` moved.
//!
//! What it is made of, step by step as the reference does it:
//! - a mixture of change-point hypotheses, one per log4 age bucket, each a Gamma posterior on the rate;
//! - per hypothesis, the probability that the miner is paused or holding its shares, with seven cases for what
//!   happened in an evaluation, and a learned rate of holds;
//! - a quasi-likelihood dispersion `phi`, a slope state for declines and a return hold;
//! - an expected-cost decision: a target too hard by e^z costs (1 - q) z, one too easy q (e^z - 1), the optimum found
//!   as the root of the loss's closed-form derivative, a switching cost, and a one-step lookahead.
//!
//! The reference's golden traces are the specification; the tests read them from
//! `$VARDIFF_GOLDEN_DIR` and are skipped when it isn't set.

use std::f64::consts::LN_2;

/// BP-G's constants. Every value is the reference's.
#[derive(Debug, Clone)]
pub struct Params {
    pub dt: f64,
    pub hazard_h: f64,
    pub mem_mult: f64,
    pub a0: f64,
    pub q: f64,
    pub k_cost: f64,
    pub cl: f64,
    pub defer_s: f64,
    pub pause_every_s: f64,
    pub pause_lomax: (f64, f64),
    pub hold_lomax: (f64, f64),
    pub hold_learn: (f64, f64),
    pub hold_quiet_s: f64,
    pub phi_cap: f64,
    pub phi_mem_h: f64,
    pub slope_z: f64,
    pub slope_stable: f64,
    pub slope_reset_p: f64,
    pub bucket_base: u64,
    pub look_q: [f64; 3],
}

impl Default for Params {
    fn default() -> Self {
        Params {
            dt: 60.0,
            hazard_h: 3.0,
            mem_mult: 4.0,
            a0: 3.0,
            q: 0.25,
            k_cost: 0.02,
            cl: 1e6f64.ln(),
            defer_s: 120.0,
            pause_every_s: 21600.0,
            pause_lomax: (40.0, 0.5),
            hold_lomax: (40.0, 0.5),
            hold_learn: (1.0, 604800.0),
            hold_quiet_s: 600.0,
            phi_cap: 10.0,
            phi_mem_h: 2.0,
            slope_z: 2.0,
            slope_stable: 0.8,
            slope_reset_p: 0.8,
            bucket_base: 4,
            look_q: [0.1, 0.5, 0.9],
        }
    }
}

/// One change-point hypothesis: its age in evaluations, its weight, its Gamma(a, b) posterior on the rate, its
/// probabilities that the miner is paused or holding shares, and how much of the held exposure it has been charged.
#[derive(Debug, Clone)]
struct Hyp {
    age: u64,
    w: f64,
    a: f64,
    b: f64,
    pp: f64,
    ph: f64,
    xc: f64,
}

#[derive(Debug, Clone)]
pub struct CountRegime {
    p: Params,
    spm: f64,
    d: f64,
    w: f64,
    w2: f64,
    tau: f64,
    hazard: f64,
    hyps: Vec<Hyp>,
    xh: f64,
    learn: (f64, f64),
    t1: [f64; 2],
    t2: [f64; 2],
    phi: f64,
    pearson: f64,
    win: f64,
    wn: f64,
    wx: f64,
    wt: f64,
    prev: Option<(f64, f64)>,
    pend_k: u64,
    pend_x: f64,
    quiet_s: f64,
    prev_n: u64,
    peek: bool,
    lookahead: bool,
    defer: bool,
    peek_out: Option<(f64, f64)>,
}

fn memory(spm: f64) -> f64 {
    (3600.0 / spm).max(600.0)
}

impl CountRegime {
    pub fn new(spm: f64) -> Self {
        Self::with_params(spm, Params::default())
    }

    pub fn with_params(spm: f64, p: Params) -> Self {
        let tau = p.mem_mult * memory(spm);
        let a = spm * tau / 60.0;
        CountRegime {
            spm,
            d: 0.0,
            w: (-p.dt / tau).exp(),
            w2: (-p.dt / (2.0 * tau)).exp(),
            tau,
            hazard: 1.0 - (-p.dt / (p.hazard_h * 3600.0)).exp(),
            hyps: vec![Hyp {
                age: 1_000_000,
                w: 1.0,
                a,
                b: a,
                pp: 0.0,
                ph: 0.0,
                xc: 0.0,
            }],
            xh: 0.0,
            learn: p.hold_learn,
            t1: [spm * tau / 60.0; 2],
            t2: [spm * 2.0 * tau / 60.0; 2],
            phi: 1.0,
            pearson: 1.0,
            win: p.dt.max(5.0 * 60.0 / spm),
            wn: 0.0,
            wx: 0.0,
            wt: 0.0,
            prev: None,
            pend_k: 0,
            pend_x: 0.0,
            quiet_s: 0.0,
            prev_n: 1,
            peek: false,
            lookahead: true,
            defer: true,
            peek_out: None,
            p,
        }
    }

    /// The log belief: ln(hashrate / declared), and the log target.
    pub fn d(&self) -> f64 {
        self.d
    }

    /// Follow a belief set from outside (an `UpdateChannel`), as the reference's adapter does on entry.
    pub fn set_d(&mut self, d: f64) {
        self.d = d;
    }

    pub fn phi(&self) -> f64 {
        self.phi
    }

    fn lomax_leave(&self, span: f64, (l, al): (f64, f64)) -> f64 {
        1.0 - ((l + self.quiet_s) / (l + self.quiet_s + span)).powf(al)
    }

    /// One evaluation: `n` shares against exposure `x`. Returns whether the belief moved.
    pub fn step(&mut self, n: u64, x: f64) -> bool {
        if !(x.is_finite() && x > 0.0) {
            return false; // no target in force, or a broken input: skip
        }
        // 1. Defer a short silence.
        let mut k = 1u64;
        let mut x = x;
        if self.defer {
            if n == 0 && ((self.pend_k + 1) as f64) * self.p.dt <= self.p.defer_s {
                self.pend_k += 1;
                self.pend_x += x;
                return false;
            }
            k = self.pend_k + 1;
            x += self.pend_x;
            self.pend_k = 0;
            self.pend_x = 0.0;
        }
        let (w, w2, h) = if k == 1 {
            (self.w, self.w2, self.hazard)
        } else {
            (
                self.w.powi(k as i32),
                self.w2.powi(k as i32),
                1.0 - (1.0 - self.hazard).powi(k as i32),
            )
        };
        // 2. Grow and start a hypothesis, with the mixture's pause and hold probabilities.
        let wsum: f64 = self.hyps.iter().map(|y| y.w).sum();
        let pp_mix = self.hyps.iter().map(|y| y.w * y.pp).sum::<f64>() / wsum;
        let ph_mix = self.hyps.iter().map(|y| y.w * y.ph).sum::<f64>() / wsum;
        let mut hyps = Vec::with_capacity(self.hyps.len() + 1);
        hyps.push(Hyp {
            age: 0,
            w: wsum * h,
            a: self.p.a0,
            b: self.p.a0,
            pp: pp_mix,
            ph: ph_mix,
            xc: 0.0,
        });
        for y in &self.hyps {
            hyps.push(Hyp {
                age: y.age + k,
                w: y.w * (1.0 - h),
                a: y.a * w,
                b: y.b * w,
                ..y.clone()
            });
        }
        // 3-5. The activity cases, the hold rate, the held exposure.
        let (lik, frac) = self.joint_cases(n, x, k, &mut hyps);
        if n == 0 {
            self.xh += x;
            for (y, f) in hyps.iter_mut().zip(frac.iter()) {
                y.xc += x * f;
            }
        } else {
            self.xh = 0.0;
            for y in hyps.iter_mut() {
                y.xc = 0.0;
            }
        }
        // 6. Dispersion, on the charged exposure.
        let gsum: f64 = hyps.iter().map(|y| y.w).sum();
        let mean_frac = hyps
            .iter()
            .zip(frac.iter())
            .map(|(y, f)| y.w / gsum * f)
            .sum::<f64>();
        let (nn, xx) = self.dispersion(n as f64, x * mean_frac, k);
        // 7. Reweight and update.
        let lmax = lik.iter().cloned().fold(f64::MIN, f64::max);
        for (y, l) in hyps.iter_mut().zip(lik.iter()) {
            y.w *= l / lmax;
        }
        let s: f64 = hyps.iter().map(|y| y.w).sum();
        let finite = hyps.iter().all(|y| (y.w / s).is_finite());
        let nh = hyps.len() as f64;
        for y in hyps.iter_mut() {
            y.w = if finite { y.w / s } else { 1.0 / nh };
        }
        for (y, f) in hyps.iter_mut().zip(frac.iter()) {
            y.a += nn;
            y.b += x * f / self.phi;
            if !(y.a.is_finite() && y.b.is_finite() && y.b > 0.0) {
                y.a = self.p.a0;
                y.b = self.p.a0 * (-self.d).exp();
            }
        }
        // 8. Prune: one per log4 age bucket, the heaviest keeping the bucket's weight.
        self.hyps = self.prune(hyps);
        // 9. The slope state.
        let mut c = 0.0;
        self.t1 = [w * self.t1[0] + nn, w * self.t1[1] + xx];
        self.t2 = [w2 * self.t2[0] + nn, w2 * self.t2[1] + xx];
        let j = argmax(self.hyps.iter().map(|y| y.w));
        if (self.hyps[j].age as f64) * self.p.dt < 2.0 * self.tau
            && self.hyps[j].w >= self.p.slope_reset_p
        {
            let r = self.hyps[j].a / self.hyps[j].b;
            self.t1 = [self.t1[1] * r, self.t1[1]];
            self.t2 = [self.t2[1] * r, self.t2[1]];
        }
        if self.t1[0] > 0.0 && self.t2[0] > 0.0 {
            let dl = (self.t1[0] / self.t1[1]).ln() - (self.t2[0] / self.t2[1]).ln();
            let stable: f64 = self
                .hyps
                .iter()
                .filter(|y| (y.age as f64) * self.p.dt > self.tau)
                .map(|y| y.w)
                .sum();
            if stable > self.p.slope_stable && dl < -self.p.slope_z / (12.0 * self.t1[0]).sqrt() {
                c = dl.max(-LN_2);
            }
        }
        // 10. The return hold.
        let returning = n > 0 && self.prev_n == 0 && self.quiet_s >= self.p.hold_quiet_s;
        self.quiet_s = if n == 0 {
            self.quiet_s + self.p.dt * k as f64
        } else {
            0.0
        };
        self.prev_n = n;
        // 11. Decide by expected cost.
        let (u, gain) = self.cost_move(c);
        if self.peek {
            self.peek_out = Some((u, gain));
            return false;
        }
        if !(gain > self.p.k_cost && !(returning && u < self.d)) {
            return false;
        }
        // 12. The lookahead.
        if self.lookahead && !self.stable_move(u) {
            return false;
        }
        // 13. Emit.
        let nd = self.d + (u - self.d).clamp(-self.p.cl, self.p.cl);
        if (nd - self.d).abs() < 1e-9 {
            return false;
        }
        self.d = nd;
        true
    }

    /// Returns each hypothesis's likelihood and charged fraction of x, and updates its pause and hold probabilities
    /// (and, outside the lookahead, the learned hold rate).
    fn joint_cases(&mut self, n: u64, x: f64, k: u64, hyps: &mut [Hyp]) -> (Vec<f64>, Vec<f64>) {
        let span = self.p.dt * k as f64;
        let enter = 1.0 - (-span / self.p.pause_every_s).exp();
        let leave = self.lomax_leave(span, self.p.pause_lomax);
        let rel = self.lomax_leave(span, self.p.hold_lomax);
        let eh = 1.0 - (-span * self.learn.0 / self.learn.1).exp();
        let phi = self.phi;
        let nn = n as f64 / phi;
        let xx = x / phi;
        let xa = (self.xh + x) / phi;
        let z = if n == 0 { 1.0 } else { 0.0 };
        let gsum: f64 = hyps.iter().map(|y| y.w).sum();
        let (mut lik, mut frac) = (
            Vec::with_capacity(hyps.len()),
            Vec::with_capacity(hyps.len()),
        );
        let (mut post_num, mut post_den) = (0.0, 0.0);
        for y in hyps.iter_mut() {
            let (a, b) = (y.a, y.b);
            let lam = (xx * a / b).max(1e-12);
            let nb = (lgamma(a + nn) - lgamma(a) - lgamma(nn + 1.0) - a * (xx / b).ln_1p()
                + nn * (xx.ln() - (b + xx).ln()))
            .exp();
            let nba = (lgamma(a + nn) - lgamma(a) - lgamma(nn + 1.0) - a * (xa / b).ln_1p()
                + nn * (xa.ln() - (b + xa).ln()))
            .exp();
            let t1 = gammainc(nn + 1.0, lam);
            let t2 = gammainc(nn + 2.0, lam);
            let part = t1 / lam;
            let eu = if t1 > 1e-300 {
                (nn + 1.0) * t2 / (lam * t1).max(1e-300)
            } else {
                1.0
            };
            let r = (1.0 - y.pp - y.ph).max(0.0);
            let rr = r * (1.0 - enter) * (1.0 - eh) * nb;
            let rp = r * enter * (1.0 - eh) * part;
            let pr = y.pp * leave * part;
            let pp = y.pp * (1.0 - leave) * z;
            let rh = r * eh * z;
            let hh = y.ph * (1.0 - rel) * z;
            let hr = y.ph * rel * nba;
            let l = rr + rp + pr + pp + rh + hh + hr;
            let ext = ((self.xh - y.xc).max(0.0) + x) / x;
            if l > 0.0 {
                frac.push((rr + (rp + pr) * eu + hr * ext) / l);
                post_num += y.w / gsum * hr;
                post_den += y.w / gsum * l;
                y.pp = (rp + pp) / l;
                y.ph = (rh + hh) / l;
                lik.push(l);
            } else {
                frac.push(1.0);
                lik.push(1e-300);
            }
        }
        if !self.peek {
            // sum_h post_h * hr_h / l_h, with post_h = P0_h l_h / (P0 . l): so sum_h P0_h hr_h / (P0 . l)
            self.learn.0 += post_num / post_den;
            self.learn.1 += span;
        }
        (lik, frac)
    }

    fn dispersion(&mut self, n: f64, x: f64, k: u64) -> (f64, f64) {
        self.wn += n;
        self.wx += x;
        self.wt += self.p.dt * k as f64;
        if self.wt >= self.win {
            if let Some((n0, x0)) = self.prev {
                if self.wn + n0 > 0.0 {
                    let th = (self.wn + n0) / (self.wx + x0);
                    let v =
                        (self.wn / self.wx - n0 / x0).powi(2) / (th * (1.0 / self.wx + 1.0 / x0));
                    let a = (-self.win / (self.p.phi_mem_h * 3600.0)).exp();
                    self.pearson = a * self.pearson + (1.0 - a) * v.min(20.0);
                }
            }
            self.prev = Some((self.wn, self.wx));
            self.wn = 0.0;
            self.wx = 0.0;
            self.wt = 0.0;
        }
        self.phi = self.pearson.max(1.0).min(self.p.phi_cap);
        (n / self.phi, x / self.phi)
    }

    fn bucket(&self, age: u64) -> u64 {
        // floor(log_base(age + 1)), in integers (the reference's floats are exact at the powers)
        let (mut m, mut j) = (age + 1, 0u64);
        while m >= self.p.bucket_base {
            m /= self.p.bucket_base;
            j += 1;
        }
        j
    }

    fn prune(&self, hyps: Vec<Hyp>) -> Vec<Hyp> {
        let mut buckets: Vec<u64> = hyps.iter().map(|y| self.bucket(y.age)).collect();
        let mut uniq = buckets.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let mut out = Vec::with_capacity(uniq.len());
        for bk in uniq {
            let idx: Vec<usize> = (0..hyps.len()).filter(|&i| buckets[i] == bk).collect();
            let best = idx
                .iter()
                .copied()
                .fold(idx[0], |b, i| if hyps[i].w > hyps[b].w { i } else { b });
            let total: f64 = idx.iter().map(|&i| hyps[i].w).sum();
            out.push(Hyp {
                w: total,
                ..hyps[best].clone()
            });
        }
        buckets.clear();
        out.sort_by_key(|y| y.age);
        out
    }

    /// The share loss's derivative at log target u, the log rate shifted by the slope lag c.
    fn dloss(&self, u: f64, c: f64) -> f64 {
        let t = (u - c).exp();
        let s: f64 = self.hyps.iter().map(|y| y.w).sum();
        let (mut below, mut upper) = (0.0, 0.0);
        for y in &self.hyps {
            below += y.w / s * gammainc(y.a, y.b * t);
            upper += y.w / s * (y.a / y.b) * gammaincc(y.a + 1.0, y.b * t);
        }
        (1.0 - self.p.q) * below - self.p.q * upper / t
    }

    /// The loss's optimum u by bisection on its derivative, and the gain L(d) - L(u) by Simpson's rule.
    fn cost_move(&self, c: f64) -> (f64, f64) {
        let (mut lo, mut hi) = (self.d - 30.0, self.d + 30.0);
        for _ in 0..48 {
            let mid = (lo + hi) / 2.0;
            if self.dloss(mid, c) < 0.0 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let u = (lo + hi) / 2.0;
        let k = 16;
        let h = (self.d - u) / k as f64;
        let f: Vec<f64> = (0..=k)
            .map(|i| self.dloss(linspace(u, self.d, k + 1, i), c))
            .collect();
        let odd: f64 = (1..k).step_by(2).map(|i| f[i]).sum();
        let even: f64 = (2..k).step_by(2).map(|i| f[i]).sum();
        (u, h / 3.0 * (f[0] + f[k] + 4.0 * odd + 2.0 * even))
    }

    /// Would the move to u still be wanted after one more evaluation, at the predictive's 10%, 50% and 90% counts?
    fn stable_move(&self, u: f64) -> bool {
        let x1 = self.spm * self.p.dt / 60.0 * (-self.d).exp();
        let xx = x1 / self.phi;
        let s: f64 = self.hyps.iter().map(|y| y.w).sum();
        let mean = self.hyps.iter().map(|y| y.w / s * y.a / y.b).sum::<f64>() * xx;
        let upper = (mean * 4.0 + 10.0 * (mean + 1.0).sqrt() + 30.0) as u64;
        let (ks, cdf): (Vec<u64>, Vec<f64>) = if upper * self.hyps.len() as u64 <= 2_000_000 {
            let ks: Vec<u64> = (0..upper).collect();
            let mut pmf = vec![0.0; ks.len()];
            for y in &self.hyps {
                let (a, b, zp) = (y.a, y.b, y.pp + y.ph);
                let (lb, lx) = ((b / (b + xx)).ln(), (xx / (b + xx)).ln());
                for (i, &kk) in ks.iter().enumerate() {
                    let kf = kk as f64;
                    let v =
                        (lgamma(a + kf) - lgamma(a) - lgamma(kf + 1.0) + a * lb + kf * lx).exp();
                    pmf[i] += y.w / s * ((1.0 - zp) * v + if kk == 0 { zp } else { 0.0 });
                }
            }
            let mut acc = 0.0;
            let cdf = pmf
                .iter()
                .map(|v| {
                    acc += v;
                    acc
                })
                .collect();
            (ks, cdf)
        } else {
            // a collapsed belief: a bounded grid, P(N <= k) = I_p(a, k + 1)
            let mut ks: Vec<u64> = (0..2000).collect();
            let (l0, l1) = (2000f64.ln(), (upper as f64).ln());
            for i in 0..4000 {
                ks.push((l0 + (l1 - l0) * i as f64 / 3999.0).exp() as u64);
            }
            ks.sort_unstable();
            ks.dedup();
            let cdf = ks
                .iter()
                .map(|&kk| {
                    self.hyps
                        .iter()
                        .map(|y| {
                            let zp = y.pp + y.ph;
                            y.w / s
                                * ((1.0 - zp) * betainc(y.a, kk as f64 + 1.0, y.b / (y.b + xx))
                                    + zp)
                        })
                        .sum::<f64>()
                })
                .collect();
            (ks, cdf)
        };
        for &q in &self.p.look_q {
            let i = cdf
                .iter()
                .position(|&v| v >= q)
                .unwrap_or(ks.len())
                .min(ks.len() - 1);
            let n1 = round_half_even(ks[i] as f64 * self.phi) as u64;
            let mut c = self.clone();
            c.peek = true;
            c.defer = false;
            c.lookahead = false;
            c.step(n1, x1);
            let Some((u2, g2)) = c.peek_out else {
                return false;
            };
            if !(g2 > self.p.k_cost && (u2 - self.d) * (u - self.d) > 0.0) {
                return false;
            }
        }
        true
    }
}

fn argmax(v: impl Iterator<Item = f64>) -> usize {
    let mut best = (0usize, f64::MIN);
    for (i, x) in v.enumerate() {
        if x > best.1 {
            best = (i, x);
        }
    }
    best.0
}

/// numpy.linspace's i-th point of n on [a, b], computed as numpy does (step, with the end point exact).
fn linspace(a: f64, b: f64, n: usize, i: usize) -> f64 {
    if i == n - 1 {
        return b;
    }
    let step = (b - a) / (n - 1) as f64;
    a + i as f64 * step
}

fn round_half_even(x: f64) -> f64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - x.signum()
    } else {
        r
    }
}

// ---- special functions ---------------------------------------------------------------------------------------------

/// ln Gamma(x) for x > 0: Lanczos (g = 7, 9 terms) with the reflection-free form, about 1e-15 relative.
pub fn lgamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - lgamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// The regularised lower incomplete gamma P(a, x).
pub fn gammainc(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if !x.is_finite() {
        return 1.0;
    }
    if x < a + 1.0 {
        gser(a, x)
    } else {
        1.0 - gcf(a, x)
    }
}

/// The regularised upper incomplete gamma Q(a, x) = 1 - P(a, x).
pub fn gammaincc(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    if !x.is_finite() {
        return 0.0;
    }
    if x < a + 1.0 {
        1.0 - gser(a, x)
    } else {
        gcf(a, x)
    }
}

fn gser(a: f64, x: f64) -> f64 {
    let (mut ap, mut sum) = (a, 1.0 / a);
    let mut del = sum;
    for _ in 0..100_000 {
        ap += 1.0;
        del *= x / ap;
        sum += del;
        if del.abs() < sum.abs() * f64::EPSILON {
            break;
        }
    }
    sum * (-x + a * x.ln() - lgamma(a)).exp()
}

fn gcf(a: f64, x: f64) -> f64 {
    const FPMIN: f64 = 1e-300;
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / FPMIN;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..100_000 {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = b + an / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < f64::EPSILON {
            break;
        }
    }
    (-x + a * x.ln() - lgamma(a)).exp() * h
}

/// The regularised incomplete beta I_x(a, b), by its continued fraction (used only for a collapsed belief).
pub fn betainc(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bt = (lgamma(a + b) - lgamma(a) - lgamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * betacf(a, b, x) / a
    } else {
        1.0 - bt * betacf(b, a, 1.0 - x) / b
    }
}

fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const FPMIN: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FPMIN {
        d = FPMIN;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..100_000 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FPMIN {
            d = FPMIN;
        }
        c = 1.0 + aa / c;
        if c.abs() < FPMIN {
            c = FPMIN;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < f64::EPSILON {
            break;
        }
    }
    h
}

// ---- the Vardiff trait ---------------------------------------------------------------------------------------------

use super::{
    clock::{Clock, SystemClock},
    error::VardiffError,
    Vardiff,
};
use bitcoin::Target;
use std::sync::Arc;

/// T / 2^256 for a target T: the chance that one hash meets it.
pub fn target_fraction(t: &Target) -> f64 {
    let b = t.to_le_bytes();
    let mut v = 0.0f64;
    for i in (0..32).rev() {
        v = v * 256.0 + b[i] as f64;
    }
    v / 2f64.powi(256)
}

/// BP-G behind the `Vardiff` trait, for a caller that evaluates every 60 s (the translator's vardiff loop).
///
/// - **Units:** the first evaluation's `hashrate` is the declared hashrate, rate 1 of the core.
/// - **Exposure:** each interval is charged Δt · declared · T / 2^256. T is the average of the target passed at this
///   evaluation and the one passed at the last. The caller passes the target in force (the translator updates it when a
///   new difficulty goes out with a notify), so a switch inside an interval costs at most that interval's error.
/// - **The belief** is set from `hashrate` on every evaluation, so an outside `UpdateChannel` is followed.
/// - **The output** is floored at `min_allowed_hashrate`, and a move that doesn't survive the f32 round trip isn't a
///   retarget (the strict test).
#[derive(Debug)]
pub struct CountRegimeVardiff {
    core: CountRegime,
    min_allowed_hashrate: f32,
    declared: Option<f64>,
    shares: u32,
    timestamp_of_last_update: u64,
    last_evaluation: u64,
    last_target: Option<f64>,
    clock: Arc<dyn Clock>,
}

impl CountRegimeVardiff {
    pub fn new_with_min(spm: f32, min_allowed_hashrate: f32) -> Result<Self, VardiffError> {
        Self::new_with_clock(spm, min_allowed_hashrate, Arc::new(SystemClock))
    }

    pub fn new_with_clock(
        spm: f32,
        min_allowed_hashrate: f32,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, VardiffError> {
        let now = clock.now_secs()?;
        Ok(CountRegimeVardiff {
            core: CountRegime::new(spm as f64),
            min_allowed_hashrate,
            declared: None,
            shares: 0,
            timestamp_of_last_update: now,
            last_evaluation: now,
            last_target: None,
            clock,
        })
    }
}

impl Vardiff for CountRegimeVardiff {
    fn last_update_timestamp(&self) -> u64 {
        self.timestamp_of_last_update
    }

    fn shares_since_last_update(&self) -> u32 {
        self.shares
    }

    fn set_timestamp_of_last_update(&mut self, timestamp: u64) {
        self.timestamp_of_last_update = timestamp;
        self.last_evaluation = timestamp;
    }

    fn increment_shares_since_last_update(&mut self) {
        self.shares += 1;
    }

    fn reset_counter(&mut self) -> Result<(), VardiffError> {
        let now = self.clock.now_secs()?;
        self.set_timestamp_of_last_update(now);
        self.shares = 0;
        Ok(())
    }

    fn try_vardiff(
        &mut self,
        hashrate: f32,
        target: &Target,
        _spm: f32,
    ) -> Result<Option<f32>, VardiffError> {
        let now = self.clock.now_secs()?;
        let dt = now.saturating_sub(self.last_evaluation).max(1) as f64;
        self.last_evaluation = now;
        let t_now = target_fraction(target);
        let t_prev = self.last_target.unwrap_or(t_now);
        self.last_target = Some(t_now);
        let h = (hashrate as f64).max(1.0);
        let declared = *self.declared.get_or_insert(h);
        let x = dt * declared * (t_prev + t_now) / 2.0;
        let n = std::mem::take(&mut self.shares) as u64;
        self.core.set_d((h / declared).ln());
        self.timestamp_of_last_update = now;
        if !self.core.step(n, x) {
            return Ok(None);
        }
        let new = ((declared * self.core.d().exp()) as f32).max(self.min_allowed_hashrate);
        if new == hashrate {
            return Ok(None); // the strict test: no move survives the f32 round trip
        }
        Ok(Some(new))
    }

    fn min_allowed_hashrate(&self) -> f32 {
        self.min_allowed_hashrate
    }
}

#[cfg(test)]
mod tests;
