//! Reproducible gate/admissibility decisions for the `context-evolve` loop.
//! Each input file is newline-delimited `RunResult` JSON (one per eval run).
//!
//!   eval_gate gate  <champion.jsonl> <candidate.jsonl>
//!   eval_gate admit <favorable.jsonl> <realistic.jsonl> [--overflowed]
//!
//! Prints a one-line verdict. Exit 0 == Promote/Admitted, 1 == rejected/not-admitted,
//! 2 == usage/IO error.
use agent_runtime_config::eval::{admit, gate, Admissibility, BatchResult, RunResult, Verdict};
use std::process::exit;

fn load(path: &str) -> BatchResult {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("read {path}: {e}");
        exit(2)
    });
    let runs = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str::<RunResult>(l).unwrap_or_else(|e| {
                eprintln!("parse: {e}");
                exit(2)
            })
        })
        .collect();
    BatchResult { runs }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gate") if args.len() >= 4 => {
            let (champ, cand) = (load(&args[2]), load(&args[3]));
            match gate(&champ, &cand) {
                Verdict::Promote => {
                    println!("Promote");
                    exit(0);
                }
                Verdict::Reject { reason } => {
                    println!("Reject: {reason}");
                    exit(1);
                }
            }
        }
        Some("admit") if args.len() >= 4 => {
            let (fav, real) = (load(&args[2]), load(&args[3]));
            let overflowed = args.iter().any(|a| a == "--overflowed");
            let verdict = admit(&fav, &real, overflowed);
            println!("{verdict:?}");
            exit(if verdict == Admissibility::Admitted {
                0
            } else {
                1
            });
        }
        _ => {
            eprintln!("usage: eval_gate <gate|admit> <a.jsonl> <b.jsonl> [--overflowed]");
            exit(2);
        }
    }
}
