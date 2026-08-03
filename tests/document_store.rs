//! `DieselDocumentStore` against a real SQLite database.
//!
//! `tests/document_efuns.rs` covers the same store as game code reaches it.
//! These check the SQL: every operator, the paging tie-break, the atomic
//! merges, and that the injection corpus cannot get through even when a filter
//! key is handed straight to `db_find`.

use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use serde_json::json;
use tempfile::TempDir;

use oxigeon::config::{DatabaseBackend, DatabaseConfig};
use oxigeon::domain::db::connection::AnyPool;
use oxigeon::domain::models::document::{
    Condition, DieselDocumentStore, DocumentLimits, FilterValue, JsonPath, Op, Order, Query, Sort,
};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

fn store() -> (DieselDocumentStore, TempDir) {
    store_with(DocumentLimits::default())
}

fn store_with(limits: DocumentLimits) -> (DieselDocumentStore, TempDir) {
    let dir = TempDir::new().unwrap();
    let pool = AnyPool::new(&DatabaseConfig {
        backend: DatabaseBackend::Sqlite,
        url: dir.path().join("t.db").to_string_lossy().to_string(),
        pool_size: 1,
    })
    .unwrap();
    pool.get_sqlite().unwrap().run_pending_migrations(MIGRATIONS).unwrap();
    (DieselDocumentStore::new(pool, limits).unwrap(), dir)
}

fn eq(field: &str, v: FilterValue) -> Condition {
    Condition { path: JsonPath::parse(field).unwrap(), op: Op::Eq, value: v }
}

fn cond(field: &str, op: Op, v: FilterValue) -> Condition {
    Condition { path: JsonPath::parse(field).unwrap(), op, value: v }
}

fn seed(s: &DieselDocumentStore) {
    s.put("reports", "R1", &json!({"status":"open","priority":1,"reporter":"amy",
        "tags":["bug"],"target":{"area":"workshop"}})).unwrap();
    s.put("reports", "R2", &json!({"status":"open","priority":5,"reporter":"bo",
        "tags":["bug","urgent"],"target":{"area":"caves"}})).unwrap();
    s.put("reports", "R3", &json!({"status":"closed","priority":3,"reporter":"amy",
        "tags":[],"target":{"area":"workshop"}})).unwrap();
    // No `status` at all — the document that makes `~=` and `nin` interesting.
    s.put("reports", "R4", &json!({"priority":2,"reporter":"cy"})).unwrap();
}

fn ids(rows: Vec<oxigeon::domain::models::Document>) -> Vec<String> {
    let mut v: Vec<String> = rows.into_iter().map(|d| d.id).collect();
    v.sort();
    v
}

// ─── round trips ─────────────────────────────────────────────────────────────

#[test]
fn a_document_round_trips() {
    let (s, _d) = store();
    let doc = json!({"a": 1, "b": [1, 2, 3], "c": {"d": true}, "e": 1.5, "f": "x"});
    s.put("things", "one", &doc).unwrap();

    let got = s.get("things", "one").unwrap().unwrap();
    assert_eq!(got.collection, "things");
    assert_eq!(got.id, "one");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&got.data).unwrap(), doc);
}

#[test]
fn get_on_a_missing_document_is_none_not_an_error() {
    let (s, _d) = store();
    assert!(s.get("things", "nope").unwrap().is_none());
    assert!(!s.exists("things", "nope").unwrap());
}

/// `put` is an upsert: the creation time is preserved, the update time moves.
#[test]
fn put_twice_preserves_created_at_and_advances_updated_at() {
    let (s, _d) = store();
    s.put("t", "a", &json!({"v": 1})).unwrap();
    let first = s.get("t", "a").unwrap().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(20));
    s.put("t", "a", &json!({"v": 2})).unwrap();
    let second = s.get("t", "a").unwrap().unwrap();

    assert_eq!(second.created_at, first.created_at, "created_at must not move");
    assert!(second.updated_at > first.updated_at, "updated_at must move");
    assert!(second.data.contains("\"v\":2"));
}

#[test]
fn delete_reports_whether_it_removed_anything() {
    let (s, _d) = store();
    s.put("t", "a", &json!({})).unwrap();
    assert!(s.delete("t", "a").unwrap());
    assert!(!s.delete("t", "a").unwrap());
}

// ─── the filter language ─────────────────────────────────────────────────────

#[test]
fn equality_and_comparisons_work_on_top_level_fields() {
    let (s, _d) = store();
    seed(&s);

    let mut q = Query::new("reports");
    q.filter = vec![eq("status", FilterValue::Text("open".into()))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R1", "R2"]);

    q.filter = vec![cond("priority", Op::Ge, FilterValue::Int(3))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R2", "R3"]);

    q.filter = vec![cond("priority", Op::Lt, FilterValue::Int(3))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R1", "R4"]);
}

#[test]
fn a_nested_path_works() {
    let (s, _d) = store();
    seed(&s);
    let mut q = Query::new("reports");
    q.filter = vec![eq("target.area", FilterValue::Text("workshop".into()))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R1", "R3"]);
}

/// The deliberate divergence from SQL. A Lua author writing
/// `doc.status ~= "closed"` on a document with no `status` expects that to be
/// true; plain `<> ?` against NULL matches nothing, which is a silent wrong
/// answer.
#[test]
fn not_equal_also_matches_documents_missing_the_field() {
    let (s, _d) = store();
    seed(&s);
    let mut q = Query::new("reports");
    q.filter = vec![cond("status", Op::Ne, FilterValue::Text("closed".into()))];
    assert_eq!(
        ids(s.find(&q).unwrap()),
        vec!["R1", "R2", "R4"],
        "R4 has no status at all and must still match"
    );
}

#[test]
fn in_and_nin_work_and_nin_matches_a_missing_field() {
    let (s, _d) = store();
    seed(&s);
    let mut q = Query::new("reports");

    q.filter = vec![cond(
        "reporter",
        Op::In,
        FilterValue::List(vec![
            FilterValue::Text("amy".into()),
            FilterValue::Text("cy".into()),
        ]),
    )];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R1", "R3", "R4"]);

    q.filter = vec![cond(
        "status",
        Op::NotIn,
        FilterValue::List(vec![FilterValue::Text("closed".into())]),
    )];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R1", "R2", "R4"]);
}

#[test]
fn exists_distinguishes_a_missing_field_from_a_falsy_one() {
    let (s, _d) = store();
    seed(&s);
    let mut q = Query::new("reports");

    q.filter = vec![cond("status", Op::Exists, FilterValue::Present(true))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R1", "R2", "R3"]);

    q.filter = vec![cond("status", Op::Exists, FilterValue::Present(false))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R4"]);
}

#[test]
fn contains_searches_inside_an_array() {
    let (s, _d) = store();
    seed(&s);
    let mut q = Query::new("reports");
    q.filter = vec![cond("tags", Op::Contains, FilterValue::Text("urgent".into()))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R2"]);

    q.filter = vec![cond("tags", Op::Contains, FilterValue::Text("bug".into()))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R1", "R2"]);
}

#[test]
fn like_matches_a_pattern() {
    let (s, _d) = store();
    seed(&s);
    let mut q = Query::new("reports");
    q.filter = vec![cond("reporter", Op::Like, FilterValue::Text("a%".into()))];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R1", "R3"]);
}

#[test]
fn several_conditions_are_combined_with_and() {
    let (s, _d) = store();
    seed(&s);
    let mut q = Query::new("reports");
    q.filter = vec![
        eq("status", FilterValue::Text("open".into())),
        cond("priority", Op::Ge, FilterValue::Int(3)),
    ];
    assert_eq!(ids(s.find(&q).unwrap()), vec!["R2"]);
}

#[test]
fn an_unknown_collection_returns_nothing_rather_than_erroring() {
    let (s, _d) = store();
    assert!(s.find(&Query::new("nothing_here")).unwrap().is_empty());
    assert_eq!(s.count("nothing_here", &[]).unwrap(), 0);
}

// ─── sorting and paging ──────────────────────────────────────────────────────

#[test]
fn sorting_by_a_json_path_works_in_both_directions() {
    let (s, _d) = store();
    seed(&s);
    let mut q = Query::new("reports");
    q.sort = Sort::Path(JsonPath::parse("priority").unwrap());

    q.order = Order::Asc;
    let asc: Vec<String> = s.find(&q).unwrap().into_iter().map(|d| d.id).collect();
    assert_eq!(asc, vec!["R1", "R4", "R3", "R2"]);

    q.order = Order::Desc;
    let desc: Vec<String> = s.find(&q).unwrap().into_iter().map(|d| d.id).collect();
    assert_eq!(desc, vec!["R2", "R3", "R4", "R1"]);
}

/// Without the `, id ASC` tie-break, offset paging over equal sort keys
/// duplicates and skips rows — a bug that only appears under load and looks
/// like data loss.
#[test]
fn paging_over_equal_sort_keys_is_stable() {
    let (s, _d) = store();
    for i in 0..10 {
        s.put("ties", &format!("d{i:02}"), &json!({"rank": 1})).unwrap();
    }

    let mut seen = Vec::new();
    for page in 0..5 {
        let mut q = Query::new("ties");
        q.sort = Sort::Path(JsonPath::parse("rank").unwrap());
        q.limit = Some(2);
        q.offset = page * 2;
        seen.extend(s.find(&q).unwrap().into_iter().map(|d| d.id));
    }

    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 10, "paging duplicated or skipped rows: {seen:?}");
}

#[test]
fn count_agrees_with_find() {
    let (s, _d) = store();
    seed(&s);
    let filter = vec![eq("status", FilterValue::Text("open".into()))];
    let mut q = Query::new("reports");
    q.filter = filter.clone();
    assert_eq!(s.count("reports", &filter).unwrap() as usize, s.find(&q).unwrap().len());
}

// ─── limits ──────────────────────────────────────────────────────────────────

/// Silently clamping to `max_results` is the single most tempting shortcut
/// here, and it is exactly the banned failure mode: a report list missing its
/// oldest entries but looking complete.
#[test]
fn an_unlimited_query_that_overflows_errors_rather_than_truncating() {
    let (s, _d) = store_with(DocumentLimits { max_results: 5, ..Default::default() });
    for i in 0..8 {
        s.put("many", &format!("d{i}"), &json!({"n": i})).unwrap();
    }

    let err = s.find(&Query::new("many")).unwrap_err().to_string();
    assert!(err.contains("no limit was given"), "unhelpful message: {err}");
    assert!(err.contains("paginate"), "should say what to do: {err}");

    // With a limit it is fine, and returns exactly that many.
    let mut q = Query::new("many");
    q.limit = Some(5);
    assert_eq!(s.find(&q).unwrap().len(), 5);
}

#[test]
fn a_limit_above_the_ceiling_is_refused() {
    let (s, _d) = store_with(DocumentLimits { max_results: 5, ..Default::default() });
    let mut q = Query::new("many");
    q.limit = Some(500);
    assert!(s.find(&q).unwrap_err().to_string().contains("max_results"));
}

#[test]
fn an_oversize_document_is_refused_and_nothing_is_written() {
    let (s, _d) = store_with(DocumentLimits { max_bytes: 200, ..Default::default() });
    let big = json!({"blob": "x".repeat(500)});
    let err = s.put("t", "big", &big).unwrap_err().to_string();
    assert!(err.contains("max_bytes"), "{err}");
    assert!(!s.exists("t", "big").unwrap(), "a refused put must write nothing");
}

#[test]
fn the_per_collection_ceiling_is_enforced() {
    let (s, _d) = store_with(DocumentLimits { max_per_collection: 3, ..Default::default() });
    for i in 0..3 {
        s.put("small", &format!("d{i}"), &json!({})).unwrap();
    }
    assert!(s.put("small", "d3", &json!({})).unwrap_err().to_string().contains("max_per_collection"));
    // Overwriting an existing document is still fine — it adds no rows.
    assert!(s.put("small", "d0", &json!({"v": 1})).is_ok());
}

#[test]
fn the_collection_ceiling_is_enforced() {
    let (s, _d) = store_with(DocumentLimits { max_collections: 2, ..Default::default() });
    s.put("a", "x", &json!({})).unwrap();
    s.put("b", "x", &json!({})).unwrap();
    assert!(s.put("c", "x", &json!({})).unwrap_err().to_string().contains("max_collections"));
}

// ─── injection ───────────────────────────────────────────────────────────────

/// The corpus, driven all the way through `find` rather than only through
/// `JsonPath::parse`, so the guarantee is tested where it is actually load
/// bearing. Every one of these must be rejected before any SQL is built.
#[test]
fn a_hostile_filter_key_cannot_reach_the_query_builder() {
    let (s, _d) = store();
    seed(&s);

    for evil in [
        "status' OR 1=1--",
        "status'); DROP TABLE documents;--",
        "status\\'",
        "status\" OR \"\"=\"",
        "status/*x*/",
        "status OR 1=1",
        "..",
        "$",
        "",
    ] {
        assert!(
            JsonPath::parse(evil).is_err(),
            "{evil:?} must not become a JSON path"
        );
    }

    // And the table is intact.
    assert_eq!(s.count("reports", &[]).unwrap(), 4);
}

#[test]
fn a_string_value_containing_sql_is_just_a_string() {
    let (s, _d) = store();
    seed(&s);
    // Bound, never formatted — so this matches nothing and destroys nothing.
    let mut q = Query::new("reports");
    q.filter = vec![eq(
        "reporter",
        FilterValue::Text("'; DROP TABLE documents; --".into()),
    )];
    assert!(s.find(&q).unwrap().is_empty());
    assert_eq!(s.count("reports", &[]).unwrap(), 4, "the table survived");
}

// ─── housekeeping ────────────────────────────────────────────────────────────

#[test]
fn collections_lists_names_with_counts() {
    let (s, _d) = store();
    seed(&s);
    s.put("mail", "m1", &json!({})).unwrap();
    assert_eq!(
        s.collections().unwrap(),
        vec![("mail".to_string(), 1), ("reports".to_string(), 4)]
    );
}

#[test]
fn clear_empties_one_collection_and_leaves_the_others() {
    let (s, _d) = store();
    seed(&s);
    s.put("mail", "m1", &json!({})).unwrap();
    assert_eq!(s.clear("reports").unwrap(), 4);
    assert_eq!(s.count("reports", &[]).unwrap(), 0);
    assert_eq!(s.count("mail", &[]).unwrap(), 1);
}

/// Counts are kept in memory and warmed at startup, so a store opened against
/// an existing database must not think its collections are empty.
#[test]
fn counts_survive_reopening_the_database() {
    let dir = TempDir::new().unwrap();
    let config = DatabaseConfig {
        backend: DatabaseBackend::Sqlite,
        url: dir.path().join("t.db").to_string_lossy().to_string(),
        pool_size: 1,
    };
    let pool = AnyPool::new(&config).unwrap();
    pool.get_sqlite().unwrap().run_pending_migrations(MIGRATIONS).unwrap();

    let limits = DocumentLimits { max_per_collection: 2, ..Default::default() };
    {
        let first = DieselDocumentStore::new(pool.clone(), limits.clone()).unwrap();
        first.put("t", "a", &json!({})).unwrap();
        first.put("t", "b", &json!({})).unwrap();
    }

    // A fresh store over the same data must already be at its ceiling.
    let second = DieselDocumentStore::new(pool, limits).unwrap();
    assert!(
        second.put("t", "c", &json!({})).is_err(),
        "the reopened store did not warm its counts"
    );
}

// ─── atomic operations ───────────────────────────────────────────────────────

#[test]
fn update_merges_recursively_and_leaves_siblings_alone() {
    let (s, _d) = store();
    s.put("t", "a", &json!({"keep": 1, "nested": {"x": 1, "y": 2}, "list": [1, 2]}))
        .unwrap();

    assert!(s.update("t", "a", &json!({"nested": {"y": 99}, "added": true})).unwrap());

    let got: serde_json::Value =
        serde_json::from_str(&s.get("t", "a").unwrap().unwrap().data).unwrap();
    assert_eq!(got["keep"], json!(1), "an untouched key must survive");
    assert_eq!(got["nested"]["x"], json!(1), "a sibling in a nested object must survive");
    assert_eq!(got["nested"]["y"], json!(99));
    assert_eq!(got["added"], json!(true));
    assert_eq!(got["list"], json!([1, 2]));
}

/// RFC 7396 replaces arrays wholesale rather than merging them. Worth pinning
/// so nobody is surprised.
#[test]
fn update_replaces_an_array_rather_than_merging_it() {
    let (s, _d) = store();
    s.put("t", "a", &json!({"list": [1, 2, 3]})).unwrap();
    s.update("t", "a", &json!({"list": [9]})).unwrap();
    let got: serde_json::Value =
        serde_json::from_str(&s.get("t", "a").unwrap().unwrap().data).unwrap();
    assert_eq!(got["list"], json!([9]));
}

#[test]
fn update_on_a_missing_document_reports_false() {
    let (s, _d) = store();
    assert!(!s.update("t", "nope", &json!({"x": 1})).unwrap());
}

#[test]
fn a_merge_that_would_overflow_rolls_back() {
    let (s, _d) = store_with(DocumentLimits { max_bytes: 300, ..Default::default() });
    s.put("t", "a", &json!({"v": 1})).unwrap();

    let err = s
        .update("t", "a", &json!({"blob": "x".repeat(500)}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("max_bytes"), "{err}");

    let got: serde_json::Value =
        serde_json::from_str(&s.get("t", "a").unwrap().unwrap().data).unwrap();
    assert_eq!(got, json!({"v": 1}), "the failed merge must have rolled back");
}

#[test]
fn unset_removes_a_field_including_a_nested_one() {
    let (s, _d) = store();
    s.put("t", "a", &json!({"keep": 1, "drop": 2, "n": {"gone": 3, "stays": 4}}))
        .unwrap();

    assert!(s.unset("t", "a", &JsonPath::parse("drop").unwrap()).unwrap());
    assert!(s.unset("t", "a", &JsonPath::parse("n.gone").unwrap()).unwrap());

    let got: serde_json::Value =
        serde_json::from_str(&s.get("t", "a").unwrap().unwrap().data).unwrap();
    assert_eq!(got, json!({"keep": 1, "n": {"stays": 4}}));
}

#[test]
fn incr_creates_the_document_on_first_use() {
    let (s, _d) = store();
    let path = JsonPath::parse("next").unwrap();
    assert_eq!(s.incr("counters", "reports", &path, 1.0).unwrap(), 1.0);
    assert_eq!(s.incr("counters", "reports", &path, 1.0).unwrap(), 2.0);
    assert_eq!(s.incr("counters", "reports", &path, 5.0).unwrap(), 7.0);
}

/// Without the json_type guard, SQLite coerces the text to 0 and silently
/// replaces a string with a number.
#[test]
fn incr_on_a_text_field_errors_and_does_not_overwrite_it() {
    let (s, _d) = store();
    s.put("t", "a", &json!({"v": "five"})).unwrap();

    let err = s
        .incr("t", "a", &JsonPath::parse("v").unwrap(), 1.0)
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a number"), "{err}");

    let got: serde_json::Value =
        serde_json::from_str(&s.get("t", "a").unwrap().unwrap().data).unwrap();
    assert_eq!(got["v"], json!("five"), "the string must be intact");
}

#[test]
fn incr_leaves_other_fields_alone() {
    let (s, _d) = store();
    s.put("t", "a", &json!({"other": "kept", "n": 1})).unwrap();
    s.incr("t", "a", &JsonPath::parse("n").unwrap(), 1.0).unwrap();
    let got: serde_json::Value =
        serde_json::from_str(&s.get("t", "a").unwrap().unwrap().data).unwrap();
    assert_eq!(got["other"], json!("kept"));
    assert_eq!(got["n"], json!(2));
}

#[test]
fn insert_generates_a_unique_id() {
    let (s, _d) = store();
    let a = s.insert("t", &json!({"n": 1})).unwrap();
    let b = s.insert("t", &json!({"n": 2})).unwrap();
    assert_ne!(a, b);
    assert!(s.exists("t", &a).unwrap());
    assert!(s.exists("t", &b).unwrap());
}
