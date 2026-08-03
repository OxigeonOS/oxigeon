//! Moving a Lua value between two VMs.
//!
//! The compute bridge runs a second LuaJIT VM on another thread, and mlua's
//! `Lua` is `!Send` — no table, function or string may cross. Everything has
//! to be copied through an owned, `Send` representation.
//!
//! **Not JSON.** `efuns::lua_to_json` exists and is `pub`, but JSON's data
//! model cannot express a Lua table: JSON has one composite type where Lua has
//! one that is a sequence and a map at the same time, its keys are strings
//! only, and it cannot tell `1` from `1.0`. `lua_to_json` therefore *refuses*
//! anything ambiguous, which is right for persisting a character but wrong
//! here — a compute job's arguments and its result are ordinary Lua values
//! that never leave Lua, so they should survive the trip exactly as they went
//! in. [`LuaData`] keeps the sequence and the map parts apart and preserves
//! integer keys, so `from_lua ∘ to_lua` is an identity.
//!
//! It is *stricter* than `lua_to_json` in one direction: a function or userdata
//! in an argument table is a hard error rather than a `null`. Silently dropping
//! a closure from a job's arguments produces a failure inside the worker that
//! nobody can explain from the call site.

use std::collections::BTreeMap;

use mlua::prelude::*;

/// A Lua value that can cross a thread boundary without losing its shape.
#[derive(Clone, Debug, PartialEq)]
pub enum LuaData {
    Nil,
    Bool(bool),
    /// Kept distinct from [`LuaData::Num`] so `1` does not come back as `1.0`.
    Int(i64),
    Num(f64),
    /// Bytes, not `String` — Lua strings are byte strings and need not be UTF-8.
    Str(Vec<u8>),
    Table(Table),
}

/// A Lua table, with its two halves kept apart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// The contiguous `1..=n` part, in order.
    pub seq: Vec<LuaData>,
    /// Everything else. A `BTreeMap` so the type has a stable ordering and
    /// comparisons in tests are reproducible; Lua tables are unordered, so
    /// sorting loses nothing.
    pub map: BTreeMap<Key, LuaData>,
}

/// A non-sequence table key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Int(i64),
    Str(Vec<u8>),
}

/// Ceilings on one conversion.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum nesting. Doubles as cycle detection — a self-referential table
    /// has no bottom, so it trips this instead of exhausting the Rust stack.
    pub depth: usize,
    /// Maximum values visited. Bounds a table that is shallow but enormous,
    /// and a shared subtree copied once per reference.
    pub nodes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { depth: 64, nodes: 100_000 }
    }
}

#[derive(Debug, PartialEq)]
pub enum MarshalError {
    /// Nesting past the limit, which is also what a cycle looks like.
    TooDeep { limit: usize },
    TooManyNodes { limit: usize },
    /// A function, thread, or userdata. Nothing that lives in one VM's heap
    /// can be meaningful in another's.
    Unsupported(&'static str),
}

impl std::fmt::Display for MarshalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooDeep { limit } => write!(
                f,
                "value nests deeper than {limit} — a table that refers to itself \
                 will always hit this"
            ),
            Self::TooManyNodes { limit } => {
                write!(f, "value holds more than {limit} entries")
            }
            Self::Unsupported(ty) => write!(
                f,
                "a value of type '{ty}' cannot cross to a compute worker — it belongs \
                 to the VM that created it"
            ),
        }
    }
}

impl std::error::Error for MarshalError {}

impl From<MarshalError> for LuaError {
    fn from(e: MarshalError) -> Self {
        LuaError::RuntimeError(e.to_string())
    }
}

/// Copy a Lua value out of its VM.
pub fn from_lua(value: &LuaValue, limits: &Limits) -> Result<LuaData, MarshalError> {
    let mut budget = limits.nodes;
    from_lua_inner(value, 0, limits, &mut budget)
}

fn from_lua_inner(
    value: &LuaValue,
    depth: usize,
    limits: &Limits,
    budget: &mut usize,
) -> Result<LuaData, MarshalError> {
    if *budget == 0 {
        return Err(MarshalError::TooManyNodes { limit: limits.nodes });
    }
    *budget -= 1;

    Ok(match value {
        LuaValue::Nil => LuaData::Nil,
        LuaValue::Boolean(b) => LuaData::Bool(*b),
        LuaValue::Integer(i) => LuaData::Int(*i),
        LuaValue::Number(n) => LuaData::Num(*n),
        LuaValue::String(s) => LuaData::Str(s.as_bytes().to_vec()),
        LuaValue::Table(t) => {
            if depth >= limits.depth {
                return Err(MarshalError::TooDeep { limit: limits.depth });
            }

            let mut table = Table::default();
            // Walk the sequence part first so `seq` comes out in order without
            // sorting, then take whatever `pairs` reports beyond it.
            let seq_len = t.raw_len();
            for i in 1..=seq_len {
                let v: LuaValue = t.raw_get(i).map_err(|_| MarshalError::Unsupported("table"))?;
                table.seq.push(from_lua_inner(&v, depth + 1, limits, budget)?);
            }

            for pair in t.clone().pairs::<LuaValue, LuaValue>() {
                let (k, v) = pair.map_err(|_| MarshalError::Unsupported("table"))?;
                let key = match &k {
                    // Already captured above.
                    LuaValue::Integer(i) if *i >= 1 && (*i as usize) <= seq_len => continue,
                    LuaValue::Integer(i) => Key::Int(*i),
                    LuaValue::String(s) => Key::Str(s.as_bytes().to_vec()),
                    // A float or boolean key is legal Lua. Carrying it would
                    // need a third `Key` variant whose ordering is a mess
                    // (NaN), and nothing in the mudlib uses one.
                    other => return Err(MarshalError::Unsupported(other.type_name())),
                };
                let converted = from_lua_inner(&v, depth + 1, limits, budget)?;
                table.map.insert(key, converted);
            }

            LuaData::Table(table)
        }
        other => return Err(MarshalError::Unsupported(other.type_name())),
    })
}

/// Rebuild a value inside a VM.
pub fn to_lua(lua: &Lua, data: &LuaData) -> LuaResult<LuaValue> {
    Ok(match data {
        LuaData::Nil => LuaValue::Nil,
        LuaData::Bool(b) => LuaValue::Boolean(*b),
        LuaData::Int(i) => LuaValue::Integer(*i),
        LuaData::Num(n) => LuaValue::Number(*n),
        LuaData::Str(s) => LuaValue::String(lua.create_string(s)?),
        LuaData::Table(t) => {
            let table = lua.create_table_with_capacity(t.seq.len(), t.map.len())?;
            for (i, v) in t.seq.iter().enumerate() {
                table.raw_set(i + 1, to_lua(lua, v)?)?;
            }
            for (k, v) in &t.map {
                let value = to_lua(lua, v)?;
                match k {
                    Key::Int(i) => table.raw_set(*i, value)?,
                    Key::Str(s) => table.raw_set(lua.create_string(s)?, value)?,
                }
            }
            LuaValue::Table(table)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convert `src` out of a VM, back into a *different* VM, and out again.
    /// If the two `LuaData` agree, the pair of conversions is an identity —
    /// which is the whole contract, and which a one-way test cannot show.
    fn round_trip(src: &str) -> LuaData {
        let a = Lua::new();
        let value: LuaValue = a.load(src).eval().unwrap();
        let out = from_lua(&value, &Limits::default()).unwrap();

        let b = Lua::new();
        let rebuilt = to_lua(&b, &out).unwrap();
        let again = from_lua(&rebuilt, &Limits::default()).unwrap();

        assert_eq!(out, again, "round trip changed the value");
        out
    }

    fn refuse(src: &str) -> MarshalError {
        let lua = Lua::new();
        let value: LuaValue = lua.load(src).eval().unwrap();
        from_lua(&value, &Limits::default()).expect_err("expected a refusal")
    }

    #[test]
    fn scalars_survive() {
        assert_eq!(round_trip("return nil"), LuaData::Nil);
        assert_eq!(round_trip("return true"), LuaData::Bool(true));
        assert_eq!(round_trip("return 42"), LuaData::Int(42));
        assert_eq!(round_trip("return 'hi'"), LuaData::Str(b"hi".to_vec()));
    }

    /// The distinction JSON cannot make. `1` must not come back as `1.0`.
    #[test]
    fn an_integer_stays_an_integer_and_a_float_stays_a_float() {
        assert_eq!(round_trip("return 3"), LuaData::Int(3));
        assert_eq!(round_trip("return 3.5"), LuaData::Num(3.5));
    }

    #[test]
    fn a_list_keeps_its_order() {
        let LuaData::Table(t) = round_trip("return {10, 20, 30}") else {
            panic!("expected a table")
        };
        assert_eq!(t.seq, vec![LuaData::Int(10), LuaData::Int(20), LuaData::Int(30)]);
        assert!(t.map.is_empty());
    }

    /// The case that motivated not using JSON: a table that is a list *and* a
    /// map. `lua_to_json` refuses this; here it must simply work.
    #[test]
    fn a_table_that_is_both_a_list_and_a_map_survives_intact() {
        let LuaData::Table(t) = round_trip("return {'a', 'b', name = 'x', level = 3}") else {
            panic!("expected a table")
        };
        assert_eq!(t.seq, vec![LuaData::Str(b"a".to_vec()), LuaData::Str(b"b".to_vec())]);
        assert_eq!(t.map.get(&Key::Str(b"name".to_vec())), Some(&LuaData::Str(b"x".to_vec())));
        assert_eq!(t.map.get(&Key::Str(b"level".to_vec())), Some(&LuaData::Int(3)));
    }

    /// Integer keys outside the sequence are keys, not list entries, and JSON
    /// would have turned them into strings.
    #[test]
    fn integer_keys_outside_the_sequence_stay_integers() {
        let LuaData::Table(t) = round_trip("local t = {} t[0] = 'z' t[99] = 'n' return t") else {
            panic!("expected a table")
        };
        assert!(t.seq.is_empty());
        assert_eq!(t.map.get(&Key::Int(0)), Some(&LuaData::Str(b"z".to_vec())));
        assert_eq!(t.map.get(&Key::Int(99)), Some(&LuaData::Str(b"n".to_vec())));
    }

    #[test]
    fn nesting_survives() {
        let LuaData::Table(t) = round_trip("return { inner = { deep = { 1, 2 } } }") else {
            panic!("expected a table")
        };
        let LuaData::Table(inner) = &t.map[&Key::Str(b"inner".to_vec())] else {
            panic!("expected a nested table")
        };
        let LuaData::Table(deep) = &inner.map[&Key::Str(b"deep".to_vec())] else {
            panic!("expected a nested table")
        };
        assert_eq!(deep.seq, vec![LuaData::Int(1), LuaData::Int(2)]);
    }

    #[test]
    fn an_empty_table_survives() {
        assert_eq!(round_trip("return {}"), LuaData::Table(Table::default()));
    }

    /// Lua strings are byte strings; a job returning binary data must not be
    /// mangled by a UTF-8 assumption.
    #[test]
    fn non_utf8_strings_survive() {
        assert_eq!(
            round_trip(r"return '\255\254\000ok'"),
            LuaData::Str(vec![255, 254, 0, b'o', b'k'])
        );
    }

    // ─── refusals ────────────────────────────────────────────────────────

    #[test]
    fn a_cycle_is_refused_rather_than_recursed() {
        assert_eq!(
            refuse("local t = {} t.self = t return t"),
            MarshalError::TooDeep { limit: 64 }
        );
    }

    #[test]
    fn a_function_is_refused_by_name() {
        assert_eq!(refuse("return { f = function() end }"), MarshalError::Unsupported("function"));
    }

    /// A closure passed as an argument used to be the sort of thing JSON
    /// turned into `null`. Here it stops the call at the call site instead.
    #[test]
    fn a_bare_function_is_refused() {
        assert_eq!(refuse("return function() end"), MarshalError::Unsupported("function"));
    }

    #[test]
    fn a_coroutine_is_refused() {
        assert_eq!(
            refuse("return coroutine.create(function() end)"),
            MarshalError::Unsupported("thread")
        );
    }

    #[test]
    fn too_many_nodes_is_refused() {
        let lua = Lua::new();
        let value: LuaValue = lua
            .load("local t = {} for i = 1, 500 do t[i] = i end return t")
            .eval()
            .unwrap();
        assert_eq!(
            from_lua(&value, &Limits { depth: 64, nodes: 100 }),
            Err(MarshalError::TooManyNodes { limit: 100 })
        );
    }

    #[test]
    fn nesting_inside_the_limit_is_fine() {
        let src = "local t = {} local c = t for _ = 1, 40 do c.n = {} c = c.n end return t";
        assert!(from_lua(
            &Lua::new().load(src).eval::<LuaValue>().unwrap(),
            &Limits::default()
        )
        .is_ok());
    }

    /// The shape a real job returns: a plan plus some metadata.
    #[test]
    fn a_realistic_job_result_round_trips() {
        let LuaData::Table(t) = round_trip(
            "return { path = {'town.square', 'town.road', 'mine.mouth'}, \
                      cost = 12.5, visited = 340, ok = true }",
        ) else {
            panic!("expected a table")
        };
        let LuaData::Table(path) = &t.map[&Key::Str(b"path".to_vec())] else {
            panic!("expected a list")
        };
        assert_eq!(path.seq.len(), 3);
        assert_eq!(t.map[&Key::Str(b"cost".to_vec())], LuaData::Num(12.5));
        assert_eq!(t.map[&Key::Str(b"visited".to_vec())], LuaData::Int(340));
        assert_eq!(t.map[&Key::Str(b"ok".to_vec())], LuaData::Bool(true));
    }
}
