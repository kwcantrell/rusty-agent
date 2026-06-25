use std::io::Write;
use std::process::Command;

fn write_jsonl(dir: &std::path::Path, name: &str, lines: &[&str]) -> std::path::PathBuf {
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    p
}

#[test]
fn admit_cli_reports_admitted_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let fav = write_jsonl(dir.path(), "fav.jsonl", &[r#"{"passed":true,"tokens":9,"turns":1}"#; 5]);
    let real = write_jsonl(dir.path(), "real.jsonl", &[r#"{"passed":false,"tokens":9,"turns":1}"#; 5]);
    let out = Command::new(env!("CARGO_BIN_EXE_eval_gate"))
        .args(["admit", fav.to_str().unwrap(), real.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Admitted"));
}

#[test]
fn gate_cli_rejects_correctness_regression_and_exits_one() {
    let dir = tempfile::tempdir().unwrap();
    let champ = write_jsonl(dir.path(), "champ.jsonl", &[r#"{"passed":true,"tokens":9,"turns":1}"#; 3]);
    let cand = write_jsonl(dir.path(), "cand.jsonl", &[r#"{"passed":false,"tokens":1,"turns":1}"#; 3]);
    let out = Command::new(env!("CARGO_BIN_EXE_eval_gate"))
        .args(["gate", champ.to_str().unwrap(), cand.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Reject"));
}
