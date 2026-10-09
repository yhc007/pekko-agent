//! Throughput of the math_answer_check stages, for comparison against the
//! Python original. Opt-in:
//!
//! ```text
//! CH03_BENCH=/path/to/ch03_full_fixture.json cargo test --release \
//!   -p pekko-agent-tools --test ch03_bench -- --nocapture
//! ```
//!
//! Reports the minimum per-call time over several rounds; the minimum is the
//! most robust statistic on a shared machine.

use pekko_agent_tools::builtin::math_answer_check::{
    extract_final_candidate, get_last_boxed, grade_answer, normalize_text, Fallback,
};
use serde::Deserialize;
use std::hint::black_box;
use std::time::Instant;

#[derive(Deserialize)]
struct StringCase {
    input: String,
    expected: String,
}

#[derive(Deserialize)]
struct GradeCase {
    pred: String,
    gt: String,
}

#[derive(Deserialize)]
struct Fixture {
    normalize: Vec<StringCase>,
    extract: Vec<StringCase>,
    grade: Vec<GradeCase>,
}

const WARMUP: usize = 2;
const ROUNDS: usize = 7;

fn bench<F: FnMut()>(label: &str, n: usize, mut f: F) {
    for _ in 0..WARMUP {
        f();
    }
    let mut best = f64::INFINITY;
    for _ in 0..ROUNDS {
        let t = Instant::now();
        f();
        let per = t.elapsed().as_secs_f64() / n as f64;
        if per < best {
            best = per;
        }
    }
    println!(
        "{label:<28} {:>9.2} us/call   {:>12.0} calls/s   (n={n})",
        best * 1e6,
        1.0 / best
    );
}

#[test]
fn throughput() {
    let Ok(path) = std::env::var("CH03_BENCH") else {
        eprintln!("CH03_BENCH not set — skipping benchmark");
        return;
    };
    let f: Fixture = serde_json::from_str(&std::fs::read_to_string(&path).expect("read"))
        .expect("parse");

    let norm_inputs: Vec<&str> = f.normalize.iter().map(|c| c.input.as_str()).collect();
    let ext_inputs: Vec<&str> = f.extract.iter().map(|c| c.input.as_str()).collect();
    let grade_inputs: Vec<(&str, &str)> =
        f.grade.iter().map(|c| (c.pred.as_str(), c.gt.as_str())).collect();

    let mean_len = ext_inputs.iter().map(|s| s.len()).sum::<usize>() / ext_inputs.len().max(1);
    println!("\n── rust ──");
    println!(
        "inputs: normalize {} (short), extract {} (mean {} bytes), grade {}",
        norm_inputs.len(),
        ext_inputs.len(),
        mean_len,
        grade_inputs.len()
    );

    bench("normalize_text", norm_inputs.len(), || {
        for s in &norm_inputs {
            black_box(normalize_text(s));
        }
    });
    bench("extract_final_candidate", ext_inputs.len(), || {
        for s in &ext_inputs {
            black_box(extract_final_candidate(s, Fallback::NumberThenFull));
        }
    });
    bench("  get_last_boxed only", ext_inputs.len(), || {
        for s in &ext_inputs {
            black_box(get_last_boxed(s));
        }
    });
    bench("grade_answer", grade_inputs.len(), || {
        for (p, g) in &grade_inputs {
            black_box(grade_answer(p, g));
        }
    });
}
