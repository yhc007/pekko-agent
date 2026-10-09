//! Parity check of the `math_answer_check` port against the Python original.
//!
//! The fixture records what `reasoning_from_scratch.ch03` returns for every
//! MATH-500 answer plus a set of hand-picked equivalence probes. Regenerate it
//! from that repo and point `CH03_FIXTURE` at the file:
//!
//! ```text
//! CH03_FIXTURE=/path/to/ch03_fixture.json cargo test -p pekko-agent-tools --test ch03_port_parity -- --nocapture
//! ```
//!
//! Without the variable the test skips, so CI does not need the dataset.

use pekko_agent_tools::builtin::math_answer_check::{
    extract_final_candidate, grade_answer, normalize_text, Fallback,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct StringCase {
    input: String,
    expected: String,
}

#[derive(Deserialize)]
struct GradeCase {
    pred: String,
    gt: String,
    expected: bool,
    #[serde(default)]
    probe: bool,
}

#[derive(Deserialize)]
struct Fixture {
    normalize: Vec<StringCase>,
    extract: Vec<StringCase>,
    grade: Vec<GradeCase>,
}

#[test]
fn matches_the_python_implementation() {
    let Ok(path) = std::env::var("CH03_FIXTURE") else {
        eprintln!("CH03_FIXTURE not set — skipping parity check");
        return;
    };
    let fixture: Fixture =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
            .expect("parse fixture");

    let mut failures = Vec::new();

    for (i, c) in fixture.normalize.iter().enumerate() {
        let got = normalize_text(&c.input);
        if got != c.expected {
            failures.push(format!(
                "normalize[{i}] {:?} -> {:?}, python said {:?}",
                c.input, got, c.expected
            ));
        }
    }
    let norm_failures = failures.len();

    for (i, c) in fixture.extract.iter().enumerate() {
        let got = extract_final_candidate(&c.input, Fallback::NumberThenFull);
        if got != c.expected {
            failures.push(format!(
                "extract[{i}] -> {:?}, python said {:?}",
                got, c.expected
            ));
        }
    }
    let extract_failures = failures.len() - norm_failures;

    let mut grade_divergences = Vec::new();
    for (i, c) in fixture.grade.iter().enumerate() {
        let got = grade_answer(&c.pred, &c.gt);
        if got != c.expected {
            grade_divergences.push(format!(
                "grade[{i}] pred={:?} gt={:?} -> rust {got}, python {}{}",
                c.pred,
                c.gt,
                c.expected,
                if c.probe { "  [probe]" } else { "" }
            ));
        }
    }

    println!("── parity ──");
    println!(
        "normalize: {}/{} match",
        fixture.normalize.len() - norm_failures,
        fixture.normalize.len()
    );
    println!(
        "extract  : {}/{} match",
        fixture.extract.len() - extract_failures,
        fixture.extract.len()
    );
    println!(
        "grade    : {}/{} match",
        fixture.grade.len() - grade_divergences.len(),
        fixture.grade.len()
    );

    if !failures.is_empty() {
        println!("\n── extraction/normalization mismatches ──");
        for f in failures.iter().take(25) {
            println!("  {f}");
        }
    }
    if !grade_divergences.is_empty() {
        println!("\n── grading divergences (expected where SymPy is needed) ──");
        for f in grade_divergences.iter().take(25) {
            println!("  {f}");
        }
    }

    // Extraction and normalization are pure string logic and must match exactly.
    assert!(
        failures.is_empty(),
        "{} extraction/normalization mismatches",
        failures.len()
    );
}
