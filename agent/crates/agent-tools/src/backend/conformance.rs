//! PUBLIC conformance surface (spec J2 gate amendment): the acceptance test a
//! custom-backend author runs against their implementation, generic over
//! `Arc<dyn Backend>`. Kept dependency-light so external crates can call it
//! from their own test harnesses.
use super::{Backend, FsError};
use std::sync::Arc;

/// Run the full behavioral contract against a fresh backend. `fresh` must
/// return an empty backend each call.
pub async fn assert_backend_conformance<F>(fresh: F)
where
    F: Fn() -> Arc<dyn Backend>,
{
    // write → read round trip, parents auto-created
    let b = fresh();
    b.write("dir/sub/a.txt", "alpha").await.expect("write");
    assert_eq!(b.read("dir/sub/a.txt").await.expect("read"), "alpha");

    // read missing → NotFound
    let b = fresh();
    assert!(matches!(
        b.read("nope.txt").await,
        Err(FsError::NotFound(_))
    ));

    // edit unique replaces once and reports before/after
    let b = fresh();
    b.write("e.txt", "foo bar baz").await.unwrap();
    let ed = b.edit("e.txt", "bar", "QUX").await.expect("edit");
    assert_eq!(ed.before, "foo bar baz");
    assert_eq!(ed.after, "foo QUX baz");
    assert_eq!(b.read("e.txt").await.unwrap(), "foo QUX baz");

    // edit ambiguous → EditConflict naming the count
    let b = fresh();
    b.write("e.txt", "x x").await.unwrap();
    match b.edit("e.txt", "x", "y").await {
        Err(FsError::EditConflict(msg)) => assert!(msg.contains("2 times"), "{msg}"),
        other => panic!("expected EditConflict, got {other:?}"),
    }

    // ls: name-sorted, dirs flagged
    let b = fresh();
    b.write("d/inner.txt", "1").await.unwrap();
    b.write("b.txt", "2").await.unwrap();
    let entries = b.ls("").await.expect("ls");
    let names: Vec<(String, bool)> = entries.into_iter().map(|e| (e.name, e.is_dir)).collect();
    assert_eq!(
        names,
        vec![("b.txt".to_string(), false), ("d".to_string(), true)]
    );

    // glob
    let b = fresh();
    b.write("a.rs", "").await.unwrap();
    b.write("a.txt", "").await.unwrap();
    let hits = b.glob("*.rs").await.expect("glob");
    assert_eq!(hits, vec!["a.rs".to_string()]);

    // grep: 1-based line numbers, scoped and unscoped
    let b = fresh();
    b.write("g.txt", "one\nneedle here\nthree").await.unwrap();
    let hits = b.grep("needle", None).await.expect("grep");
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].path.as_str(), hits[0].line), ("g.txt", 2));
    assert!(b
        .grep("needle", Some("elsewhere"))
        .await
        .unwrap()
        .is_empty());

    // delete then read → NotFound; delete missing → NotFound
    let b = fresh();
    b.write("del.txt", "x").await.unwrap();
    b.delete("del.txt").await.expect("delete");
    assert!(matches!(b.read("del.txt").await, Err(FsError::NotFound(_))));
    assert!(matches!(
        b.delete("del.txt").await,
        Err(FsError::NotFound(_))
    ));
}
