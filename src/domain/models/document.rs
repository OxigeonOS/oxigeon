//! A generic JSON document store, so persisting a new type needs no Rust.
//!
//! Adding a Diesel model today means a migration, a hand-edited `schema.rs`, a
//! model, a store, module re-exports, an `EfunContext` field and driver
//! wiring — and because `embed_migrations!` bakes the migration directory into
//! the binary at compile time, the hot-reloadable `game/` layer can never ship
//! a table at all. This is the escape hatch: one table, one migration, and
//! every collection a game author invents lives in it.
//!
//! Use a real model when you need indexed columns, joins or foreign keys. Use
//! this for everything else — reports, mail, quest state, preferences.

use std::collections::HashMap;
use std::sync::RwLock;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Integer, Text};
use serde_json::Value as JsonValue;

use crate::core::lock::RwLockExt;
use crate::domain::db::connection::AnyPool;
use crate::domain::db::schema::documents;
use crate::error::{OxigeonError, Result};

/// A stored document, with the envelope a caller sees.
#[derive(Debug, Clone, Queryable, Selectable, QueryableByName)]
#[diesel(table_name = documents)]
pub struct Document {
    pub collection: String,
    pub id: String,
    /// JSON text. Parsed at the Lua boundary, not here.
    pub data: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Insertable)]
#[diesel(table_name = documents)]
struct NewDocument<'a> {
    collection: &'a str,
    id: &'a str,
    data: &'a str,
    created_at: &'a str,
    updated_at: &'a str,
}

// ─── injection, made unrepresentable ─────────────────────────────────────────

/// A JSON path that is safe to interpolate into SQL.
///
/// Filter keys come from game code and become path *text* inside a query —
/// `json_extract(data, '$.status')` — because SQLite will only use an
/// expression index when the query text matches the index expression exactly,
/// and a bound parameter never matches.
///
/// So the safety is structural rather than reviewed: there is no way to
/// construct one of these except through [`JsonPath::parse`], and `parse`
/// admits only `[A-Za-z_][A-Za-z0-9_]*` segments and `[0-9]+` subscripts. That
/// charset excludes `'`, `"`, `\`, whitespace, NUL and every SQL
/// metacharacter, so [`JsonPath::as_sql_literal`] cannot return anything but a
/// closed, inert string literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonPath(String);

/// Longest path accepted, in segments and in characters.
const MAX_PATH_SEGMENTS: usize = 8;
const MAX_SEGMENT_LEN: usize = 64;

impl JsonPath {
    /// Parse a dotted path such as `status`, `target.name` or `history[0].actor`.
    pub fn parse(key: &str) -> Result<Self> {
        if key.is_empty() || key.len() > 256 {
            return Err(bad_path(key, "must be between 1 and 256 characters"));
        }

        let mut rendered = String::from("'$");
        let mut segments = 0;

        for part in key.split('.') {
            if part.is_empty() {
                return Err(bad_path(key, "has an empty segment"));
            }
            // `history[0]` — a name followed by any number of subscripts.
            let (name, mut rest) = match part.find('[') {
                Some(i) => (&part[..i], &part[i..]),
                None => (part, ""),
            };

            if name.is_empty() {
                return Err(bad_path(key, "has a subscript with no field name"));
            }
            if name.len() > MAX_SEGMENT_LEN {
                return Err(bad_path(key, "has a segment longer than 64 characters"));
            }
            let mut chars = name.chars();
            let first = chars.next().unwrap();
            if !(first.is_ascii_alphabetic() || first == '_')
                || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(bad_path(
                    key,
                    "may only contain letters, digits and underscores, and may not start \
                     with a digit",
                ));
            }
            segments += 1;
            if segments > MAX_PATH_SEGMENTS {
                return Err(bad_path(key, "has more than 8 segments"));
            }
            rendered.push('.');
            rendered.push_str(name);

            while !rest.is_empty() {
                let Some(close) = rest.find(']') else {
                    return Err(bad_path(key, "has an unclosed '['"));
                };
                let index = &rest[1..close];
                if index.is_empty()
                    || index.len() > 9
                    || !index.chars().all(|c| c.is_ascii_digit())
                {
                    return Err(bad_path(key, "has a subscript that is not a number"));
                }
                rendered.push('[');
                rendered.push_str(index);
                rendered.push(']');
                rest = &rest[close + 1..];
            }
        }

        rendered.push('\'');
        Ok(JsonPath(rendered))
    }

    /// The path as a complete, quoted SQL string literal — quotes included.
    pub fn as_sql_literal(&self) -> &str {
        &self.0
    }
}

fn bad_path(key: &str, why: &str) -> OxigeonError {
    OxigeonError::Internal(format!("'{key}' is not a valid document field: it {why}"))
}

// ─── filters ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    In,
    NotIn,
    Like,
    Exists,
    Contains,
}

impl Op {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "==" | "eq" => Self::Eq,
            "~=" | "ne" => Self::Ne,
            ">" | "gt" => Self::Gt,
            ">=" | "ge" => Self::Ge,
            "<" | "lt" => Self::Lt,
            "<=" | "le" => Self::Le,
            "in" => Self::In,
            "nin" => Self::NotIn,
            "like" => Self::Like,
            "exists" => Self::Exists,
            "contains" => Self::Contains,
            _ => return None,
        })
    }
}

/// A value a filter compares against. Always bound, never formatted.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterValue {
    Text(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    List(Vec<FilterValue>),
    /// The argument to `exists`.
    Present(bool),
}

/// One validated filter term. The only constructor takes a [`JsonPath`], so an
/// unvalidated key cannot reach the query builder.
#[derive(Debug, Clone)]
pub struct Condition {
    pub path: JsonPath,
    pub op: Op,
    pub value: FilterValue,
}

#[derive(Debug, Clone)]
pub enum Sort {
    /// A real column: `id`, `created_at`, `updated_at`.
    Column(&'static str),
    Path(JsonPath),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Asc,
    Desc,
}

impl Order {
    fn as_sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Query {
    pub collection: String,
    pub filter: Vec<Condition>,
    pub sort: Sort,
    pub order: Order,
    pub limit: Option<i64>,
    pub offset: i64,
}

impl Query {
    pub fn new(collection: impl Into<String>) -> Self {
        Self {
            collection: collection.into(),
            filter: Vec::new(),
            sort: Sort::Column("created_at"),
            order: Order::Asc,
            limit: None,
            offset: 0,
        }
    }
}

// ─── limits ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DocumentLimits {
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_max_per_collection")]
    pub max_per_collection: usize,
    #[serde(default = "default_max_collections")]
    pub max_collections: usize,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_max_bytes() -> usize { 65_536 }
fn default_max_per_collection() -> usize { 100_000 }
fn default_max_collections() -> usize { 256 }
fn default_max_results() -> usize { 500 }

impl Default for DocumentLimits {
    fn default() -> Self {
        Self {
            max_bytes: default_max_bytes(),
            max_per_collection: default_max_per_collection(),
            max_collections: default_max_collections(),
            max_results: default_max_results(),
        }
    }
}

// ─── the store ───────────────────────────────────────────────────────────────

pub struct DieselDocumentStore {
    pool: AnyPool,
    limits: DocumentLimits,
    /// collection -> live row count. Warmed once at startup, then maintained
    /// in memory: this process is the only writer, and a stale count would
    /// only ever affect an abuse ceiling, never a query result.
    counts: RwLock<HashMap<String, usize>>,
}

/// Collection names are lowercase by rule. They are always *bound*, so case is
/// not a safety issue — it is a correctness one: a `Reports`/`reports` split
/// would silently produce two collections, one of which looks empty.
fn validate_collection(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(OxigeonError::Internal(format!(
            "collection '{name}' must be between 1 and 64 characters"
        )));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(OxigeonError::Internal(format!(
            "collection '{name}' may only contain lowercase letters, digits and \
             underscores, and must start with a letter"
        )));
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 128 {
        return Err(OxigeonError::Internal(format!(
            "document id '{id}' must be between 1 and 128 characters"
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
    {
        return Err(OxigeonError::Internal(format!(
            "document id '{id}' may only contain letters, digits and . _ - :"
        )));
    }
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl DieselDocumentStore {
    pub fn new(pool: AnyPool, limits: DocumentLimits) -> Result<Self> {
        let store = Self { pool, limits, counts: RwLock::new(HashMap::new()) };
        store.warm_counts()?;
        Ok(store)
    }

    fn warm_counts(&self) -> Result<()> {
        #[derive(QueryableByName)]
        struct Row {
            #[diesel(sql_type = Text)]
            collection: String,
            #[diesel(sql_type = BigInt)]
            n: i64,
        }
        let mut conn = self.pool.get_sqlite()?;
        let rows: Vec<Row> = diesel::sql_query(
            "SELECT collection, COUNT(*) AS n FROM documents GROUP BY collection",
        )
        .load(&mut conn)?;
        let mut counts = self.counts.write_recover();
        for row in rows {
            counts.insert(row.collection, row.n as usize);
        }
        Ok(())
    }

    /// Insert or replace a document under a caller-chosen id.
    pub fn put(&self, collection: &str, id: &str, data: &JsonValue) -> Result<()> {
        validate_collection(collection)?;
        validate_id(id)?;
        let text = serde_json::to_string(data)
            .map_err(|e| OxigeonError::Internal(format!("document is not serializable: {e}")))?;
        if text.len() > self.limits.max_bytes {
            return Err(OxigeonError::Internal(format!(
                "document '{collection}/{id}' is {} bytes, over the {}-byte ceiling in \
                 [documents] max_bytes",
                text.len(),
                self.limits.max_bytes
            )));
        }

        let is_new = !self.exists(collection, id)?;
        if is_new {
            self.check_room_for_one_more(collection)?;
        }

        let stamp = now();
        let mut conn = self.pool.get_sqlite()?;
        diesel::insert_into(documents::table)
            .values(NewDocument {
                collection,
                id,
                data: &text,
                created_at: &stamp,
                updated_at: &stamp,
            })
            .on_conflict((documents::collection, documents::id))
            // `created_at` is untouched on conflict, so it stays the creation
            // time rather than becoming the last-write time.
            .do_update()
            .set((documents::data.eq(&text), documents::updated_at.eq(&stamp)))
            .execute(&mut conn)?;

        if is_new {
            *self.counts.write_recover().entry(collection.to_string()).or_insert(0) += 1;
        }
        Ok(())
    }

    fn check_room_for_one_more(&self, collection: &str) -> Result<()> {
        let counts = self.counts.read_recover();
        if !counts.contains_key(collection) && counts.len() >= self.limits.max_collections {
            return Err(OxigeonError::Internal(format!(
                "cannot create collection '{collection}': already at the {} collection \
                 ceiling in [documents] max_collections",
                self.limits.max_collections
            )));
        }
        if counts.get(collection).copied().unwrap_or(0) >= self.limits.max_per_collection {
            return Err(OxigeonError::Internal(format!(
                "collection '{collection}' is at the {}-document ceiling in \
                 [documents] max_per_collection",
                self.limits.max_per_collection
            )));
        }
        Ok(())
    }

    /// Store a document under a server-generated id.
    pub fn insert(&self, collection: &str, data: &JsonValue) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.put(collection, &id, data)?;
        Ok(id)
    }

    /// Recursively merge `patch` into a document (RFC 7396).
    ///
    /// Objects merge key by key; arrays are replaced wholesale. One statement,
    /// so a read-modify-write from Lua needs no transaction — which matters,
    /// because a transaction spanning a Lua callback would pin a pooled
    /// connection across arbitrary game code and deadlock.
    pub fn update(&self, collection: &str, id: &str, patch: &JsonValue) -> Result<bool> {
        let text = serde_json::to_string(patch)
            .map_err(|e| OxigeonError::Internal(format!("patch is not serializable: {e}")))?;
        let stamp = now();
        let max = self.limits.max_bytes;
        let mut conn = self.pool.get_sqlite()?;

        // In a transaction so an over-size merge rolls back rather than
        // committing a document that violates the ceiling.
        conn.transaction::<bool, OxigeonError, _>(|conn| {
            let n = diesel::sql_query(
                "UPDATE documents SET data = json_patch(data, ?), updated_at = ? \
                 WHERE collection = ? AND id = ?",
            )
            .bind::<Text, _>(text)
            .bind::<Text, _>(stamp)
            .bind::<Text, _>(collection.to_string())
            .bind::<Text, _>(id.to_string())
            .execute(conn)?;

            if n == 0 {
                return Ok(false);
            }

            #[derive(QueryableByName)]
            struct Size {
                #[diesel(sql_type = BigInt)]
                n: i64,
            }
            let size: Size = diesel::sql_query(
                "SELECT length(data) AS n FROM documents WHERE collection = ? AND id = ?",
            )
            .bind::<Text, _>(collection.to_string())
            .bind::<Text, _>(id.to_string())
            .get_result(conn)?;

            if size.n as usize > max {
                return Err(OxigeonError::Internal(format!(
                    "the merge would make '{collection}/{id}' {} bytes, over the \
                     {max}-byte ceiling in [documents] max_bytes — nothing was changed",
                    size.n
                )));
            }
            Ok(true)
        })
    }

    /// Remove one field. Lua tables cannot hold `nil`, so RFC 7396's
    /// delete-by-null is unreachable from Lua — hence a separate call rather
    /// than a gap nobody notices.
    pub fn unset(&self, collection: &str, id: &str, path: &JsonPath) -> Result<bool> {
        let mut conn = self.pool.get_sqlite()?;
        let n = diesel::sql_query(format!(
            "UPDATE documents SET data = json_remove(data, {}), updated_at = ? \
             WHERE collection = ? AND id = ?",
            path.as_sql_literal()
        ))
        .bind::<Text, _>(now())
        .bind::<Text, _>(collection.to_string())
        .bind::<Text, _>(id.to_string())
        .execute(&mut conn)?;
        Ok(n > 0)
    }

    /// Add `delta` to a numeric field, atomically, creating the document if
    /// needed. Returns the new value.
    ///
    /// The `WHERE` on the conflict branch is the important part: without it,
    /// incrementing a field holding `"five"` makes SQLite coerce the text to 0
    /// and **silently replace a string with a number**. With it the update
    /// matches nothing and the caller is told what the field actually holds.
    pub fn incr(&self, collection: &str, id: &str, path: &JsonPath, delta: f64) -> Result<f64> {
        validate_collection(collection)?;
        validate_id(id)?;
        let p = path.as_sql_literal();
        let stamp = now();

        #[derive(QueryableByName)]
        struct Val {
            #[diesel(sql_type = Double)]
            value: f64,
        }

        let sql = format!(
            "INSERT INTO documents (collection, id, data, created_at, updated_at) \
             VALUES (?, ?, json_set('{{}}', {p}, ?), ?, ?) \
             ON CONFLICT(collection, id) DO UPDATE \
               SET data = json_set(data, {p}, COALESCE(json_extract(data, {p}), 0) + ?), \
                   updated_at = ? \
               WHERE json_type(documents.data, {p}) IN ('integer','real') \
                  OR json_type(documents.data, {p}) IS NULL \
             RETURNING CAST(json_extract(data, {p}) AS REAL) AS value"
        );

        // An integral delta is bound as an integer so SQLite's `+` stays in
        // integer arithmetic and the stored counter stays `7` rather than
        // becoming `7.0`. Lua cannot tell the two apart — its numbers are all
        // doubles — but anything else reading the database can, and a
        // sequence number rendered as "7.0" is a papercut nobody should have
        // to debug.
        let integral = delta.fract() == 0.0 && delta.abs() < 9.0e15;
        let bind_delta = |q: Boxed<'static>| -> Boxed<'static> {
            if integral {
                q.bind::<BigInt, _>(delta as i64)
            } else {
                q.bind::<Double, _>(delta)
            }
        };

        // Scoped, so the connection is back in the pool before the error path
        // asks for another one. With `pool_size = 1` — which the tests use
        // precisely to catch this — holding both at once deadlocks.
        let mut conn = self.pool.get_sqlite()?;
        let mut q = diesel::sql_query(sql)
            .into_boxed::<diesel::sqlite::Sqlite>()
            .bind::<Text, _>(collection.to_string())
            .bind::<Text, _>(id.to_string());
        q = bind_delta(q);
        q = q
            .bind::<Text, _>(stamp.clone())
            .bind::<Text, _>(stamp.clone());
        q = bind_delta(q);
        let updated: Option<Val> = q
            .bind::<Text, _>(stamp)
            .get_result(&mut conn)
            .optional()?;
        drop(conn);

        match updated {
            Some(v) => Ok(v.value),
            None => {
                // The guard refused it. Say what the field actually holds.
                let kind = self.json_type_of(collection, id, path)?;
                Err(OxigeonError::Internal(format!(
                    "cannot increment '{collection}/{id}' field {}: it holds a {} , not a number",
                    path.as_sql_literal(),
                    kind.unwrap_or_else(|| "missing value".to_string())
                )))
            }
        }
    }

    fn json_type_of(&self, collection: &str, id: &str, path: &JsonPath) -> Result<Option<String>> {
        #[derive(QueryableByName)]
        struct Ty {
            #[diesel(sql_type = diesel::sql_types::Nullable<Text>)]
            t: Option<String>,
        }
        let mut conn = self.pool.get_sqlite()?;
        let row: Option<Ty> = diesel::sql_query(format!(
            "SELECT json_type(data, {}) AS t FROM documents WHERE collection = ? AND id = ?",
            path.as_sql_literal()
        ))
        .bind::<Text, _>(collection.to_string())
        .bind::<Text, _>(id.to_string())
        .get_result(&mut conn)
        .optional()?;
        Ok(row.and_then(|r| r.t))
    }

    pub fn get(&self, collection: &str, id: &str) -> Result<Option<Document>> {
        let mut conn = self.pool.get_sqlite()?;
        documents::table
            .find((collection, id))
            .first::<Document>(&mut conn)
            .optional()
            .map_err(Into::into)
    }

    pub fn exists(&self, collection: &str, id: &str) -> Result<bool> {
        let mut conn = self.pool.get_sqlite()?;
        let n: i64 = documents::table
            .filter(documents::collection.eq(collection))
            .filter(documents::id.eq(id))
            .count()
            .get_result(&mut conn)?;
        Ok(n > 0)
    }

    pub fn delete(&self, collection: &str, id: &str) -> Result<bool> {
        let mut conn = self.pool.get_sqlite()?;
        let n = diesel::delete(
            documents::table
                .filter(documents::collection.eq(collection))
                .filter(documents::id.eq(id)),
        )
        .execute(&mut conn)?;
        if n > 0 {
            if let Some(count) = self.counts.write_recover().get_mut(collection) {
                *count = count.saturating_sub(1);
            }
        }
        Ok(n > 0)
    }

    pub fn clear(&self, collection: &str) -> Result<usize> {
        let mut conn = self.pool.get_sqlite()?;
        let n = diesel::delete(documents::table.filter(documents::collection.eq(collection)))
            .execute(&mut conn)?;
        self.counts.write_recover().remove(collection);
        Ok(n)
    }

    pub fn collections(&self) -> Result<Vec<(String, i64)>> {
        #[derive(QueryableByName)]
        struct Row {
            #[diesel(sql_type = Text)]
            collection: String,
            #[diesel(sql_type = BigInt)]
            n: i64,
        }
        let mut conn = self.pool.get_sqlite()?;
        let rows: Vec<Row> = diesel::sql_query(
            "SELECT collection, COUNT(*) AS n FROM documents GROUP BY collection \
             ORDER BY collection",
        )
        .load(&mut conn)?;
        Ok(rows.into_iter().map(|r| (r.collection, r.n)).collect())
    }

    /// Run a query.
    ///
    /// A query with no explicit limit that matches more than `max_results`
    /// **errors** rather than silently returning the first N — a truncated
    /// report list that looks complete is exactly the silent failure this
    /// project forbids. One extra row is fetched to detect it.
    pub fn find(&self, query: &Query) -> Result<Vec<Document>> {
        let ceiling = self.limits.max_results as i64;
        if let Some(limit) = query.limit {
            if limit > ceiling {
                return Err(OxigeonError::Internal(format!(
                    "limit {limit} is above the {ceiling} ceiling in [documents] max_results"
                )));
            }
        }
        let probe = query.limit.unwrap_or(ceiling) + 1;

        let mut conn = self.pool.get_sqlite()?;
        let rows: Vec<Document> = self.build(query).sql(" LIMIT ? OFFSET ?")
            .bind::<BigInt, _>(probe)
            .bind::<BigInt, _>(query.offset)
            .load(&mut conn)?;

        if query.limit.is_none() && rows.len() as i64 > ceiling {
            return Err(OxigeonError::Internal(format!(
                "more than {ceiling} documents in '{}' match and no limit was given. \
                 Pass {{ limit = n }} and paginate with {{ offset = n }}",
                query.collection
            )));
        }
        Ok(rows.into_iter().take(probe as usize - 1).collect())
    }

    pub fn count(&self, collection: &str, filter: &[Condition]) -> Result<i64> {
        #[derive(QueryableByName)]
        struct Row {
            #[diesel(sql_type = BigInt)]
            n: i64,
        }
        let mut q = diesel::sql_query("SELECT COUNT(*) AS n FROM documents WHERE collection = ?")
            .into_boxed::<diesel::sqlite::Sqlite>()
            .bind::<Text, _>(collection.to_string());
        for cond in filter {
            q = push_condition(q, cond);
        }
        let mut conn = self.pool.get_sqlite()?;
        let row: Row = q.get_result(&mut conn)?;
        Ok(row.n)
    }

    /// The SELECT and its WHERE clause, without paging.
    fn build<'a>(
        &self,
        query: &'a Query,
    ) -> diesel::query_builder::BoxedSqlQuery<'a, diesel::sqlite::Sqlite, diesel::query_builder::SqlQuery>
    {
        let mut q = diesel::sql_query(
            "SELECT collection, id, data, created_at, updated_at FROM documents \
             WHERE collection = ?",
        )
        .into_boxed::<diesel::sqlite::Sqlite>()
        // Bound first, because BoxedSqlQuery walks binds in push order after
        // the whole SQL string — every `.sql(" ... ?")` must be followed
        // immediately by its `.bind`, or every parameter is off by one.
        .bind::<Text, _>(query.collection.clone());

        for cond in &query.filter {
            q = push_condition(q, cond);
        }

        // `, id ASC` is not decoration: without a tie-break, offset paging over
        // equal sort keys duplicates and skips rows, which only shows up under
        // load and looks like data loss.
        let order = query.order.as_sql();
        match &query.sort {
            Sort::Column(col) => q.sql(format!(" ORDER BY {col} {order}, id ASC")),
            Sort::Path(path) => q.sql(format!(
                " ORDER BY json_extract(data, {}) {order}, id ASC",
                path.as_sql_literal()
            )),
        }
    }
}

type Boxed<'a> =
    diesel::query_builder::BoxedSqlQuery<'a, diesel::sqlite::Sqlite, diesel::query_builder::SqlQuery>;

fn bind_value<'a>(q: Boxed<'a>, v: &FilterValue) -> Boxed<'a> {
    match v {
        FilterValue::Text(s) => q.bind::<Text, _>(s.clone()),
        FilterValue::Int(i) => q.bind::<BigInt, _>(*i),
        FilterValue::Real(f) => q.bind::<Double, _>(*f),
        // json_extract yields 1/0 for JSON booleans.
        FilterValue::Bool(b) => q.bind::<Integer, _>(i32::from(*b)),
        FilterValue::List(_) | FilterValue::Present(_) => q,
    }
}

fn push_condition<'a>(q: Boxed<'a>, cond: &Condition) -> Boxed<'a> {
    let p = cond.path.as_sql_literal();
    match (cond.op, &cond.value) {
        (Op::Exists, FilterValue::Present(want)) => {
            let test = if *want { "IS NOT NULL" } else { "IS NULL" };
            q.sql(format!(" AND json_extract(data, {p}) {test}"))
        }
        (Op::In | Op::NotIn, FilterValue::List(items)) => {
            let holes = std::iter::repeat("?").take(items.len()).collect::<Vec<_>>().join(",");
            // `nin` deliberately also matches documents where the key is
            // absent: a Lua author writing `doc.status ~= "closed"` on a table
            // with no `status` expects true, and plain SQL `NOT IN` against
            // NULL yields NULL and matches nothing — a silent wrong answer.
            let mut q = if cond.op == Op::In {
                q.sql(format!(" AND json_extract(data, {p}) IN ({holes})"))
            } else {
                q.sql(format!(
                    " AND (json_extract(data, {p}) IS NULL OR json_extract(data, {p}) NOT IN ({holes}))"
                ))
            };
            for item in items {
                q = bind_value(q, item);
            }
            q
        }
        (Op::Ne, v) => bind_value(
            // Same reasoning as `nin`.
            q.sql(format!(
                " AND (json_extract(data, {p}) IS NULL OR json_extract(data, {p}) <> ?)"
            )),
            v,
        ),
        (Op::Contains, v) => bind_value(
            q.sql(format!(
                " AND EXISTS (SELECT 1 FROM json_each(documents.data, {p}) \
                  WHERE json_each.value = ?)"
            )),
            v,
        ),
        (op, v) => {
            let sql_op = match op {
                Op::Eq => "=",
                Op::Gt => ">",
                Op::Ge => ">=",
                Op::Lt => "<",
                Op::Le => "<=",
                Op::Like => "LIKE",
                // Handled above; unreachable in practice.
                _ => "=",
            };
            bind_value(q.sql(format!(" AND json_extract(data, {p}) {sql_op} ?")), v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_paths_parse_to_inert_literals() {
        assert_eq!(JsonPath::parse("status").unwrap().as_sql_literal(), "'$.status'");
        assert_eq!(
            JsonPath::parse("target.name").unwrap().as_sql_literal(),
            "'$.target.name'"
        );
        assert_eq!(
            JsonPath::parse("history[0].actor").unwrap().as_sql_literal(),
            "'$.history[0].actor'"
        );
        assert_eq!(JsonPath::parse("_x9").unwrap().as_sql_literal(), "'$._x9'");
    }

    /// The corpus that matters. Every one of these would be a SQL injection if
    /// it reached `as_sql_literal`, so none of them may construct a `JsonPath`.
    #[test]
    fn nothing_that_could_break_out_of_a_literal_parses() {
        for evil in [
            "a' OR 1=1--",
            "a'--",
            "a\\'",
            "a\";DROP TABLE documents;--",
            "a b",
            "a\tb",
            "a\nb",
            "a\0b",
            "..",
            ".a",
            "a.",
            "",
            "$",
            "[",
            "a[",
            "a[]",
            "a[x]",
            "a[-1]",
            "1abc",
            "a.b.c.d.e.f.g.h.i",
            "*",
            "a/*x*/",
            "a%",
            "a)",
        ] {
            assert!(
                JsonPath::parse(evil).is_err(),
                "{evil:?} must not parse into a JSON path"
            );
        }

        let long = "a".repeat(65);
        assert!(JsonPath::parse(&long).is_err());
        let deep = (0..12).map(|_| "a").collect::<Vec<_>>().join(".");
        assert!(JsonPath::parse(&deep).is_err());
    }

    /// Whatever a path renders as, it must be one closed literal with no way
    /// out of it — belt and braces over the charset rule.
    #[test]
    fn a_rendered_path_is_always_one_closed_literal() {
        for good in ["a", "a.b", "a[0]", "a[10].b_c", "_", "z9.y8[3]"] {
            let lit = JsonPath::parse(good).unwrap();
            let s = lit.as_sql_literal();
            assert!(s.starts_with('\'') && s.ends_with('\''), "{s}");
            assert_eq!(s.matches('\'').count(), 2, "{s} has an interior quote");
            assert!(!s.contains('\\') && !s.contains('"'), "{s}");
        }
    }

    #[test]
    fn collection_names_are_lowercase_by_rule() {
        assert!(validate_collection("reports").is_ok());
        assert!(validate_collection("quest_flags2").is_ok());
        // A Reports/reports split would silently make two collections.
        assert!(validate_collection("Reports").is_err());
        assert!(validate_collection("2reports").is_err());
        assert!(validate_collection("reports;drop").is_err());
        assert!(validate_collection("").is_err());
    }

    #[test]
    fn document_ids_allow_the_shapes_a_game_will_use() {
        for good in ["R0001", "quest.manasteel", "char:42", "a-b_c.d"] {
            assert!(validate_id(good).is_ok(), "{good}");
        }
        for bad in ["", "has space", "quote'", "semi;colon", "sl/ash"] {
            assert!(validate_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn operators_parse_from_their_lua_spellings() {
        assert_eq!(Op::parse("=="), Some(Op::Eq));
        assert_eq!(Op::parse("~="), Some(Op::Ne));
        assert_eq!(Op::parse(">="), Some(Op::Ge));
        assert_eq!(Op::parse("contains"), Some(Op::Contains));
        assert_eq!(Op::parse("drop"), None);
    }
}
