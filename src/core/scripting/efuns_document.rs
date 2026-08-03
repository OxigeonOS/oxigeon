//! The `db_*` efuns — persisting anything from Lua, with no Rust involved.
//!
//! A sibling of `efuns_io.rs` rather than four hundred more lines in
//! `efuns.rs`.
//!
//! # Failure convention
//!
//! Three rules, applied uniformly:
//!
//! | Situation | Behaviour |
//! |---|---|
//! | Expected absence (`db_get` on an unknown id) | `nil` / `false` / `{}` |
//! | Author error (bad field name, oversize document, unknown operator) | **raise** |
//! | Infrastructure failure | **raise** |
//!
//! This departs from `save_character_data`, which logs a warning and returns
//! `false`. Commands are `pcall`-wrapped by `lib/commands.lua`, so raising is
//! safe and lands in the log — and a report that silently was not saved is
//! exactly the failure `CLAUDE.md` forbids. A boolean that can only ever be
//! `true` is worse than no return value, so writes return the id instead.

use mlua::prelude::*;

use crate::domain::models::document::{
    Condition, FilterValue, JsonPath, Op, Order, Query, Sort,
};

use super::efuns::{check_efun_permission, json_to_lua, lua_to_json, EfunContext};

/// Turn a store error into a Lua error. The message already names the offender.
fn fail<T>(e: crate::error::OxigeonError) -> LuaResult<T> {
    Err(LuaError::RuntimeError(e.to_string()))
}

/// The envelope every read returns.
///
/// A record rather than the bare document, so an id and timestamps come back
/// without inventing a reserved `_id` key that game authors would then have to
/// avoid.
fn record(lua: &Lua, doc: crate::domain::models::Document) -> LuaResult<LuaTable> {
    let t = lua.create_table()?;
    t.set("collection", doc.collection)?;
    t.set("id", doc.id)?;
    let parsed: serde_json::Value = serde_json::from_str(&doc.data)
        .map_err(|e| LuaError::RuntimeError(format!("stored document is not valid JSON: {e}")))?;
    t.set("data", json_to_lua(lua, &parsed)?)?;
    t.set("created_at", doc.created_at)?;
    t.set("updated_at", doc.updated_at)?;
    Ok(t)
}

/// One filter value, from whatever Lua passed.
fn filter_value(v: &LuaValue, field: &str) -> LuaResult<FilterValue> {
    Ok(match v {
        LuaValue::String(s) => FilterValue::Text(s.to_str()?.to_string()),
        LuaValue::Integer(i) => FilterValue::Int(*i),
        LuaValue::Number(n) => FilterValue::Real(*n),
        LuaValue::Boolean(b) => FilterValue::Bool(*b),
        other => {
            return Err(LuaError::RuntimeError(format!(
                "filter on '{field}' has a value of type '{}', which cannot be compared",
                other.type_name()
            )))
        }
    })
}

/// Parse `{ status = "open", priority = { [">="] = 3 } }` into conditions.
fn parse_filter(filter: Option<LuaTable>) -> LuaResult<Vec<Condition>> {
    let Some(filter) = filter else { return Ok(Vec::new()) };
    let mut out = Vec::new();

    for pair in filter.pairs::<String, LuaValue>() {
        let (field, value) = pair?;
        let path = JsonPath::parse(&field).or_else(|e| fail(e))?;

        match &value {
            // `{ [">="] = 3 }` — one operator per field.
            LuaValue::Table(spec) => {
                let mut seen = 0;
                for entry in spec.clone().pairs::<String, LuaValue>() {
                    let (op_name, operand) = entry?;
                    let Some(op) = Op::parse(&op_name) else {
                        return Err(LuaError::RuntimeError(format!(
                            "unknown operator '{op_name}' on field '{field}'. Use one of \
                             == ~= > >= < <= in nin like exists contains"
                        )));
                    };
                    let operand = match (op, &operand) {
                        (Op::Exists, LuaValue::Boolean(b)) => FilterValue::Present(*b),
                        (Op::Exists, _) => {
                            return Err(LuaError::RuntimeError(format!(
                                "'exists' on field '{field}' takes true or false"
                            )))
                        }
                        (Op::In | Op::NotIn, LuaValue::Table(list)) => {
                            let mut items = Vec::new();
                            for item in list.clone().sequence_values::<LuaValue>() {
                                items.push(filter_value(&item?, &field)?);
                            }
                            if items.is_empty() || items.len() > 64 {
                                return Err(LuaError::RuntimeError(format!(
                                    "'{op_name}' on field '{field}' needs between 1 and 64 values"
                                )));
                            }
                            FilterValue::List(items)
                        }
                        (Op::In | Op::NotIn, _) => {
                            return Err(LuaError::RuntimeError(format!(
                                "'{op_name}' on field '{field}' takes a list of values"
                            )))
                        }
                        (_, v) => filter_value(v, &field)?,
                    };
                    out.push(Condition { path: path.clone(), op, value: operand });
                    seen += 1;
                }
                if seen == 0 {
                    return Err(LuaError::RuntimeError(format!(
                        "filter on '{field}' is an empty table; give it an operator such \
                         as {{ [\">=\"] = 3 }}"
                    )));
                }
            }
            // Bare value: equality.
            v => out.push(Condition {
                path,
                op: Op::Eq,
                value: filter_value(v, &field)?,
            }),
        }
    }
    Ok(out)
}

fn parse_opts(collection: String, filter: Vec<Condition>, opts: Option<LuaTable>) -> LuaResult<Query> {
    let mut q = Query::new(collection);
    q.filter = filter;

    let Some(opts) = opts else { return Ok(q) };

    if let Ok(Some(limit)) = opts.get::<Option<i64>>("limit") {
        if limit < 0 {
            return Err(LuaError::RuntimeError("limit cannot be negative".into()));
        }
        q.limit = Some(limit);
    }
    if let Ok(Some(offset)) = opts.get::<Option<i64>>("offset") {
        if offset < 0 {
            return Err(LuaError::RuntimeError("offset cannot be negative".into()));
        }
        q.offset = offset;
    }
    if let Ok(Some(sort)) = opts.get::<Option<String>>("sort") {
        q.sort = match sort.as_str() {
            "id" => Sort::Column("id"),
            "created_at" => Sort::Column("created_at"),
            "updated_at" => Sort::Column("updated_at"),
            path => Sort::Path(JsonPath::parse(path).or_else(|e| fail(e))?),
        };
    }
    if let Ok(Some(order)) = opts.get::<Option<String>>("order") {
        q.order = match order.as_str() {
            "asc" => Order::Asc,
            "desc" => Order::Desc,
            other => {
                return Err(LuaError::RuntimeError(format!(
                    "order must be 'asc' or 'desc', not '{other}'"
                )))
            }
        };
    }
    Ok(q)
}

/// Register the twelve `db_*` efuns.
pub fn register_document_efuns(lua: &Lua, ctx: &EfunContext) -> LuaResult<()> {
    let globals = lua.globals();
    let store = ctx.document_store.clone();

    // Every db_* efun consults the permission table, which is a no-op HashMap
    // miss when the name is absent. That means an operator can gate any of
    // them from permissions.toml without a code change.
    macro_rules! gated {
        ($name:literal, $ctx:expr) => {{
            let perm = $ctx.permission_config.clone();
            let sh = $ctx.session_handler.clone();
            let gl = $ctx.game_logger.clone();
            move || check_efun_permission($name, &perm, &sh, &gl)
        }};
    }

    // db_put(collection, id, doc) -> id
    {
        let store = store.clone();
        let guard = gated!("db_put", ctx);
        let f = lua.create_function(move |lua, (collection, id, doc): (String, String, LuaValue)| {
            guard()?;
            let json = lua_to_json(lua, &doc)?;
            store.put(&collection, &id, &json).or_else(|e| fail(e))?;
            Ok(id)
        })?;
        globals.set("db_put", f)?;
    }

    // db_insert(collection, doc) -> id
    {
        let store = store.clone();
        let guard = gated!("db_insert", ctx);
        let f = lua.create_function(move |lua, (collection, doc): (String, LuaValue)| {
            guard()?;
            let json = lua_to_json(lua, &doc)?;
            store.insert(&collection, &json).or_else(|e| fail(e))
        })?;
        globals.set("db_insert", f)?;
    }

    // db_get(collection, id) -> record|nil
    {
        let store = store.clone();
        let guard = gated!("db_get", ctx);
        let f = lua.create_function(move |lua, (collection, id): (String, String)| {
            guard()?;
            match store.get(&collection, &id).or_else(|e| fail(e))? {
                Some(doc) => Ok(LuaValue::Table(record(lua, doc)?)),
                None => Ok(LuaValue::Nil),
            }
        })?;
        globals.set("db_get", f)?;
    }

    // db_exists(collection, id) -> boolean
    {
        let store = store.clone();
        let guard = gated!("db_exists", ctx);
        let f = lua.create_function(move |_, (collection, id): (String, String)| {
            guard()?;
            store.exists(&collection, &id).or_else(|e| fail(e))
        })?;
        globals.set("db_exists", f)?;
    }

    // db_delete(collection, id) -> boolean
    {
        let store = store.clone();
        let guard = gated!("db_delete", ctx);
        let f = lua.create_function(move |_, (collection, id): (String, String)| {
            guard()?;
            store.delete(&collection, &id).or_else(|e| fail(e))
        })?;
        globals.set("db_delete", f)?;
    }

    // db_find(collection, filter?, opts?) -> array of records
    {
        let store = store.clone();
        let guard = gated!("db_find", ctx);
        let f = lua.create_function(
            move |lua, (collection, filter, opts): (String, Option<LuaTable>, Option<LuaTable>)| {
                guard()?;
                let query = parse_opts(collection, parse_filter(filter)?, opts)?;
                let rows = store.find(&query).or_else(|e| fail(e))?;
                let out = lua.create_table_with_capacity(rows.len(), 0)?;
                for (i, doc) in rows.into_iter().enumerate() {
                    out.set(i + 1, record(lua, doc)?)?;
                }
                Ok(out)
            },
        )?;
        globals.set("db_find", f)?;
    }

    // db_count(collection, filter?) -> integer
    {
        let store = store.clone();
        let guard = gated!("db_count", ctx);
        let f = lua.create_function(move |_, (collection, filter): (String, Option<LuaTable>)| {
            guard()?;
            let conditions = parse_filter(filter)?;
            store.count(&collection, &conditions).or_else(|e| fail(e))
        })?;
        globals.set("db_count", f)?;
    }

    // db_update(collection, id, patch) -> boolean
    {
        let store = store.clone();
        let guard = gated!("db_update", ctx);
        let f = lua.create_function(
            move |lua, (collection, id, patch): (String, String, LuaValue)| {
                guard()?;
                let json = lua_to_json(lua, &patch)?;
                store.update(&collection, &id, &json).or_else(|e| fail(e))
            },
        )?;
        globals.set("db_update", f)?;
    }

    // db_unset(collection, id, field) -> boolean
    {
        let store = store.clone();
        let guard = gated!("db_unset", ctx);
        let f = lua.create_function(
            move |_, (collection, id, field): (String, String, String)| {
                guard()?;
                let path = JsonPath::parse(&field).or_else(|e| fail(e))?;
                store.unset(&collection, &id, &path).or_else(|e| fail(e))
            },
        )?;
        globals.set("db_unset", f)?;
    }

    // db_incr(collection, id, field, delta?) -> number
    {
        let store = store.clone();
        let guard = gated!("db_incr", ctx);
        let f = lua.create_function(
            move |_, (collection, id, field, delta): (String, String, String, Option<f64>)| {
                guard()?;
                let path = JsonPath::parse(&field).or_else(|e| fail(e))?;
                store
                    .incr(&collection, &id, &path, delta.unwrap_or(1.0))
                    .or_else(|e| fail(e))
            },
        )?;
        globals.set("db_incr", f)?;
    }

    // db_collections() -> array of {name, count}
    {
        let store = store.clone();
        let guard = gated!("db_collections", ctx);
        let f = lua.create_function(move |lua, ()| {
            guard()?;
            let rows = store.collections().or_else(|e| fail(e))?;
            let out = lua.create_table_with_capacity(rows.len(), 0)?;
            for (i, (name, count)) in rows.into_iter().enumerate() {
                let t = lua.create_table()?;
                t.set("name", name)?;
                t.set("count", count)?;
                out.set(i + 1, t)?;
            }
            Ok(out)
        })?;
        globals.set("db_collections", f)?;
    }

    // db_clear(collection) -> integer
    {
        let guard = gated!("db_clear", ctx);
        let f = lua.create_function(move |_, collection: String| {
            guard()?;
            store.clear(&collection).or_else(|e| fail(e)).map(|n| n as i64)
        })?;
        globals.set("db_clear", f)?;
    }

    Ok(())
}
