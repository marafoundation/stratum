//! The reference's golden traces, read from `$VARDIFF_GOLDEN_DIR`. Each JSON file is one trace: per evaluation, n, x, d before and after, whether
//! it fired, and phi. A port must match `fired` exactly, d within 1e-6 and phi within 1e-9.
use super::*;

fn golden_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("VARDIFF_GOLDEN_DIR").map(std::path::PathBuf::from)
}

/// A small JSON reader for the golden files, so the crate needs no new dev-dependency.
#[derive(Debug, Clone)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl std::ops::Index<&str> for J {
    type Output = J;
    fn index(&self, k: &str) -> &J {
        match self {
            J::Obj(v) => {
                &v.iter()
                    .find(|(kk, _)| kk == k)
                    .unwrap_or_else(|| panic!("no key {k}"))
                    .1
            }
            _ => panic!("not an object"),
        }
    }
}

impl J {
    fn as_f64(&self) -> Option<f64> {
        if let J::Num(x) = self {
            Some(*x)
        } else {
            None
        }
    }
    fn as_u64(&self) -> Option<u64> {
        self.as_f64().map(|x| x as u64)
    }
    fn as_bool(&self) -> Option<bool> {
        if let J::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }
    fn as_array(&self) -> Option<&Vec<J>> {
        if let J::Arr(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

impl std::fmt::Display for J {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            J::Num(x) => write!(f, "{x}"),
            J::Bool(b) => write!(f, "{b}"),
            J::Str(s) => write!(f, "{s}"),
            other => write!(f, "{other:?}"),
        }
    }
}

fn parse(s: &str) -> J {
    fn ws(b: &[u8], i: &mut usize) {
        while *i < b.len() && (b[*i] as char).is_whitespace() {
            *i += 1;
        }
    }
    fn val(b: &[u8], i: &mut usize) -> J {
        ws(b, i);
        match b[*i] {
            b'{' => {
                *i += 1;
                let mut v = vec![];
                loop {
                    ws(b, i);
                    if b[*i] == b'}' {
                        *i += 1;
                        break;
                    }
                    let J::Str(k) = val(b, i) else { panic!("key") };
                    ws(b, i);
                    *i += 1; // ':'
                    let x = val(b, i);
                    v.push((k, x));
                    ws(b, i);
                    if b[*i] == b',' {
                        *i += 1;
                    }
                }
                J::Obj(v)
            }
            b'[' => {
                *i += 1;
                let mut v = vec![];
                loop {
                    ws(b, i);
                    if b[*i] == b']' {
                        *i += 1;
                        break;
                    }
                    v.push(val(b, i));
                    ws(b, i);
                    if b[*i] == b',' {
                        *i += 1;
                    }
                }
                J::Arr(v)
            }
            b'"' => {
                *i += 1;
                let st = *i;
                while b[*i] != b'"' {
                    *i += 1;
                }
                let s = String::from_utf8_lossy(&b[st..*i]).into_owned();
                *i += 1;
                J::Str(s)
            }
            b't' => {
                *i += 4;
                J::Bool(true)
            }
            b'f' => {
                *i += 5;
                J::Bool(false)
            }
            b'n' => {
                *i += 4;
                J::Null
            }
            _ => {
                let st = *i;
                while *i < b.len() && b"+-0123456789.eEInfinityNa".contains(&b[*i]) {
                    *i += 1;
                }
                J::Num(std::str::from_utf8(&b[st..*i]).unwrap().parse().unwrap())
            }
        }
    }
    let mut i = 0;
    val(s.as_bytes(), &mut i)
}

fn num(v: &J) -> f64 {
    match v {
        J::Str(s) if s == "nan" => f64::NAN,
        J::Str(s) if s == "inf" => f64::INFINITY,
        v => v.as_f64().expect("a number"),
    }
}

#[test]
fn golden_traces() {
    let Some(dir) = golden_dir() else {
        eprintln!(
            "skipped: set VARDIFF_GOLDEN_DIR to a directory of the reference's golden traces"
        );
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no traces in {dir:?}");
    let mut failures = vec![];
    for f in &files {
        let t = parse(&std::fs::read_to_string(f).unwrap());
        let mut c = CountRegime::new(t["spm"].as_f64().unwrap());
        for r in t["rows"].as_array().unwrap() {
            let (k, n, x) = (
                r["k"].as_u64().unwrap(),
                r["n"].as_u64().unwrap(),
                num(&r["x"]),
            );
            let d0 = c.d();
            let fired = c.step(n, x);
            let bad = (d0 - num(&r["d_before"])).abs() > 1e-6
                || fired != r["fired"].as_bool().unwrap()
                || (c.d() - num(&r["d_after"])).abs() > 1e-6
                || (c.phi() - num(&r["phi"])).abs() > 1e-9;
            if bad {
                failures.push(format!("{} row {k}: d {d0:.9} -> {:.9} fired {fired} phi {:.12}, reference {} -> {} fired {} phi {}",
                    f.file_name().unwrap().to_string_lossy(), c.d(), c.phi(), r["d_before"], r["d_after"], r["fired"], r["phi"]));
                break;
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} traces diverge:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn special_functions() {
    // values from scipy.special (gammaln, gammainc, gammaincc, betainc)
    let close = |a: f64, b: f64, tol: f64| (a - b).abs() <= tol * b.abs().max(1.0);
    assert!(close(lgamma(0.5), 0.5723649429247001, 1e-14));
    assert!(close(lgamma(3.0), 0.6931471805599453, 1e-14));
    assert!(close(lgamma(1234.5), 7550.550901077895, 1e-14));
    assert!(close(gammainc(3.0, 2.5), 0.4561868841166703, 1e-13));
    assert!(close(gammaincc(1201.0, 1180.0), 0.7257138558122986, 1e-11));
    assert!(close(gammainc(0.7, 0.01), 0.043633739077886, 1e-12));
    assert!(close(betainc(3.0, 5.0, 0.4), 0.5800960000000001, 1e-13));
}

#[test]
fn round_half_even_matches_python() {
    assert_eq!(
        [0.5, 1.5, 2.5, 3.5, 2.4, 2.6].map(round_half_even),
        [0.0, 2.0, 2.0, 4.0, 2.0, 3.0]
    );
}

mod adapter {
    use super::*;
    use crate::target::hash_rate_to_target;
    use crate::vardiff::clock::MockClock;
    use crate::vardiff::Vardiff;
    use std::sync::Arc;

    fn target_for(h: f64, spm: f64) -> Target {
        let t = hash_rate_to_target(h, spm).unwrap();
        Target::from_le_bytes(t.to_le_bytes())
    }

    #[test]
    fn exposure_units() {
        // at the declared hashrate's own target, a 60 s interval is charged about spm shares
        let (h, spm) = (175e12, 6.0);
        let x = 60.0 * h * target_fraction(&target_for(h, spm));
        assert!((x / spm - 1.0).abs() < 1e-3, "{x}");
    }

    /// Until its first retarget the adapter sees one target, so it must decide exactly as the core did on the
    /// reference's own trace: the same evaluation fires, to the same belief.
    #[test]
    fn first_retarget_matches_the_core() {
        let Some(dir) = golden_dir() else { return };
        let t = parse(&std::fs::read_to_string(dir.join("rise_x3_at_30min_6spm.json")).unwrap());
        let (spm, h0) = (6.0f64, 175e12f64);
        let clock = Arc::new(MockClock::new(1_000_000));
        let mut v = CountRegimeVardiff::new_with_clock(spm as f32, 1.0, clock.clone()).unwrap();
        let target = target_for(h0, spm);
        let x_unit = 60.0 * h0 * target_fraction(&target) / spm; // the reference's x is spm at d = 0
        for r in t["rows"].as_array().unwrap() {
            clock.advance(60);
            for _ in 0..r["n"].as_u64().unwrap() {
                v.increment_shares_since_last_update();
            }
            let out = v.try_vardiff(h0 as f32, &target, spm as f32).unwrap();
            assert_eq!(
                out.is_some(),
                r["fired"].as_bool().unwrap(),
                "row {}",
                r["k"]
            );
            if let Some(new) = out {
                let d = ((new as f64) / h0).ln();
                assert!(
                    (d - num(&r["d_after"])).abs() < 1e-5 + (x_unit - 1.0).abs(),
                    "row {}: {d} against {}",
                    r["k"],
                    r["d_after"]
                );
                return;
            }
        }
        panic!("the reference fired in this trace; the adapter never did");
    }

    #[test]
    fn steady_miner_holds_and_rise_is_followed() {
        let (spm, h0) = (6.0f64, 175e12f64);
        let clock = Arc::new(MockClock::new(1_000_000));
        let mut v = CountRegimeVardiff::new_with_clock(spm as f32, 1.0, clock.clone()).unwrap();
        let mut h = h0 as f32;
        let mut fires = 0;
        let mut seed = 12345u64;
        let mut pois = |lam: f64| {
            // a small inverse-transform Poisson draw from an LCG
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u = (seed >> 11) as f64 / (1u64 << 53) as f64;
            let (mut k, mut p) = (0u64, (-lam).exp());
            let mut c = p;
            while u > c && k < 10_000 {
                k += 1;
                p *= lam / k as f64;
                c += p;
            }
            k
        };
        for minute in 0..360 {
            let truth = if minute < 180 { h0 } else { 3.0 * h0 };
            let lam = spm * truth / h as f64;
            for _ in 0..pois(lam) {
                v.increment_shares_since_last_update();
            }
            clock.advance(60);
            if let Some(new) = v
                .try_vardiff(h, &target_for(h as f64, spm), spm as f32)
                .unwrap()
            {
                h = new;
                if minute < 180 {
                    fires += 1;
                }
            }
        }
        // the reference makes about 0.5 an hour at 6 spm after 2 h, plus the first move to its easy cushion
        assert!(fires <= 4, "{fires} retargets on a steady miner in 3 h");
        assert!(
            (h as f64 / (3.0 * h0)).ln().abs() < 0.25,
            "after a 3x rise the belief is {:.2}x",
            h as f64 / h0
        );
    }
}
