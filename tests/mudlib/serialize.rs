//! `lib/serialize.lua` — Lua values back into Lua source.
//!
//! Three properties, and they are not the same property:
//!
//! * **P1 idempotence** — `emit(load(emit(v)))` byte-equals `emit(v)`. This is
//!   the one that catches formatting bugs: the indented closing `]]` that put
//!   four spaces inside a description and grew them on every write, the leading
//!   newline an opening long bracket eats, `%.17g` rendering `1.2` as
//!   `1.1999999999999999`.
//! * **P2 fidelity** — `load(emit(v))` deep-equals `v`.
//! * **P3 refusal** — everything it cannot write is refused *by name*, rather
//!   than emitted as something that will not compile. `codegen_d` concatenated
//!   strings, so a room title containing a quote produced a broken file and
//!   nothing said so until the next reload.
//!
//! Through `boot_fixture_with_probe` rather than a stubbed `mlua::Lua`: this is
//! mudlib code and it should be asked what it does inside the engine. It happens
//! to be pure, which is why the cases can be table-driven.

use crate::common::RealVm;

/// Probe source travels as one input line, so these snippets are written
/// readably and folded here. No `--` comments inside them: folded onto one line
/// a comment swallows everything after it.
fn one_line(src: &str) -> String {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A `deep_equal` and a round-trip harness, defined once in Lua.
///
/// Loading the emitted source with `load` rather than comparing strings is what
/// makes P2 a claim about *values* — a formatting change should not fail it.
/// `roundtrip` then emits a second time and compares bytes, which is P1.
const HARNESS: &str = r#"
    S = require('lib.serialize')
    function deep_equal(a, b)
        if type(a) ~= type(b) then return false end
        if type(a) ~= 'table' then return a == b end
        for k, v in pairs(a) do if not deep_equal(v, b[k]) then return false end end
        for k in pairs(b) do if a[k] == nil then return false end end
        return true
    end
    function roundtrip(value)
        local src, err = S.module(value)
        if not src then return 'refused: ' .. tostring(err) end
        local chunk, load_err = load(src)
        if not chunk then return 'did not compile: ' .. tostring(load_err) .. ' >> ' .. src end
        local ok, back = pcall(chunk)
        if not ok then return 'did not run: ' .. tostring(back) end
        if not deep_equal(value, back) then return 'value changed >> ' .. src end
        local again = S.module(back)
        if again ~= src then
            return 'not idempotent >> FIRST ' .. src .. ' SECOND ' .. again
        end
        return 'ok'
    end
"#;

struct Vm(RealVm);

impl Vm {
    fn new() -> Self {
        let mut vm = RealVm::boot_fixture_with_probe();
        vm.eval(&one_line(&format!("{HARNESS} return 'ready'"))).unwrap();
        Self(vm)
    }

    /// Evaluate a readably-written snippet.
    fn run(&mut self, src: &str) -> String {
        self.0.eval(&one_line(src)).unwrap()
    }
}

/// Every string that has ever broken a naive emitter, in one table so adding a
/// case is one line.
#[test]
fn hostile_strings_round_trip() {
    let mut vm = Vm::new();

    let out = vm.run(r#"
            local cases = {
                'plain',
                '',
                'a "quoted" title',
                "an apostrophe's place",
                'a backslash \\ here',
                'tab\there',
                'newline\nhere',
                'crlf\r\nhere',
                'trailing newline\n',
                '\nleading newline',
                '\n\ntwo leading newlines',
                'closing ]] bracket',
                'level one ]=] bracket',
                'level two ]==] bracket',
                'opening [[ bracket',
                'both [[ and ]] brackets',
                'ends with a bracket ]',
                'multi\nline with ]] inside',
                'nul\0byte',
                'byte \127 delete',
                'caf\195\169',
                'em \226\128\148 dash',
                'a percent %s format',
                'a {colour} tag',
            }
            local bad = {}
            for i, s in ipairs(cases) do
                local r = roundtrip({ value = s })
                if r ~= 'ok' then bad[#bad+1] = i .. ': ' .. r end
            end
            return #bad == 0 and 'ok' or table.concat(bad, '\n')
        "#,
        );
    assert_eq!(out, "ok");
}

/// The float trap. `%.17g` round-trips everything and renders every authored
/// `speed = 1.2` as `1.1999999999999999`; `%d` for an integral float silently
/// changes `math.type`.
#[test]
fn numbers_round_trip_without_growing_digits() {
    let mut vm = Vm::new();

    let out = vm.run(r#"
            local cases = { 0, 1, -1, 42, -42, 0.5, 1.2, 0.1, 2.0, -0.0,
                            1e15, 1e-15, math.pi, 2^53, -2^53, 1/3 }
            local bad = {}
            for i, n in ipairs(cases) do
                local r = roundtrip({ value = n })
                if r ~= 'ok' then bad[#bad+1] = i .. ' (' .. tostring(n) .. '): ' .. r end
            end
            return #bad == 0 and 'ok' or table.concat(bad, '\n')
        "#,
        );
    assert_eq!(out, "ok");

    // The specific regression: a value a builder typed should come back looking
    // like the value a builder typed.
    assert_eq!(vm.run("return S.number(1.2)"), "1.2");
    assert_eq!(vm.run("return S.number(0.1)"), "0.1");
    assert_eq!(vm.run("return S.number(42)"), "42");
}

/// `false` must survive. It is falsy, so every `if v then emit(v) end` drops it,
/// and a mob whose `aggressive` quietly became nil is a mob that stops fighting.
#[test]
fn false_and_empty_survive() {
    let mut vm = Vm::new();

    assert_eq!(
        vm.run("return roundtrip({ two_handed = false, aggressive = false })"),
        "ok"
    );
    assert_eq!(vm.run("return roundtrip({ tags = {}, exits = {} })"), "ok");

    let src = vm.run("return S.value({ two_handed = false })");
    assert!(src.contains("two_handed = false"), "{src}");
}

/// Keys: identifiers bare, everything else bracketed, keywords never bare.
#[test]
fn keys_are_emitted_in_a_form_that_compiles() {
    let mut vm = Vm::new();

    assert_eq!(
        vm.run("return roundtrip({ plain = 1, ['with space'] = 2, ['end'] = 3, \
                                ['1numeric'] = 4, [5] = 'five', ['a-dash'] = 6 })"
        ),
        "ok"
    );

    // `end = 1` is a syntax error, and a room with an `end` scenery keyword
    // would otherwise emit a file that does not compile.
    assert_eq!(vm.run("return tostring(S.is_identifier('end'))"), "false");
    assert_eq!(vm.run("return tostring(S.is_identifier('short'))"), "true");
    assert_eq!(vm.run("return tostring(S.is_identifier('with space'))"), "false");
}

/// A table that is both a list and a map is fine here, though `jsonsafe` refuses
/// one. That difference is the reason these are two modules.
#[test]
fn a_mixed_list_and_map_is_written() {
    let mut vm = Vm::new();

    assert_eq!(
        vm.run("return roundtrip({ 'a', 'b', name = 'both', count = 2 })"),
        "ok"
    );
    // …and `jsonsafe` still refuses it, so neither has quietly adopted the
    // other's rules.
    assert_eq!(
        vm.run("return tostring((require('lib.jsonsafe').check({ 'a', name = 'x' })))"),
        "false"
    );
}

/// Everything it cannot write is refused, and the reason names the path.
///
/// "Cannot serialize" without a location is a bug report you cannot act on, and
/// the whole point of refusing is that a builder can go and fix the field.
#[test]
fn what_cannot_be_written_is_refused_by_name() {
    let mut vm = Vm::new();

    let cases = [
        ("{ a = { b = function() end } }", "a.b", "function"),
        ("{ nan = 0/0 }", "nan", "NaN"),
        ("{ inf = math.huge }", "inf", "inf"),
        ("{ co = coroutine.create(function() end) }", "co", "thread"),
    ];
    for (expr, path, needle) in cases {
        let out = vm.run(&format!(
                "local ok, why = S.check({expr}) return tostring(ok) .. '|' .. tostring(why)"
            ));
        assert!(out.starts_with("false|"), "{expr} should be refused: {out}");
        assert!(out.contains(path), "the reason should name '{path}': {out}");
        assert!(
            out.to_lowercase().contains(&needle.to_lowercase()),
            "the reason should say what was wrong ({needle}): {out}"
        );
    }

    // A cycle, named. Detected on the *ancestor* path, so the same subtable
    // appearing twice is still legal.
    let out = vm.run("local t = { name = 'x' } t.self = t \
             local ok, why = S.check(t) return tostring(ok) .. '|' .. tostring(why)",
        );
    assert!(out.starts_with("false|"), "{out}");
    assert!(out.contains("contains itself"), "{out}");

    let shared = vm.run("local leaf = { 1, 2 } \
             return tostring((S.check({ a = leaf, b = leaf })))",
        );
    assert_eq!(shared, "true", "the same table twice is not a cycle");

    // And `M.value` refuses rather than emitting something broken.
    let out = vm.run("local src, err = S.value({ f = print }) return tostring(src) .. '|' .. tostring(err)");
    assert!(out.starts_with("nil|"), "{out}");
}

/// Prose uses a long bracket, at a level that does not collide with its content.
#[test]
fn multi_line_prose_uses_a_long_bracket_that_does_not_truncate() {
    let mut vm = Vm::new();

    let src = vm.run("return S.value({ description = 'Line one.\\nLine two.' })");
    assert!(src.contains("[["), "prose should use a long bracket:\n{src}");

    // A `]]` in the content forces a level up rather than truncating the string.
    let src = vm.run("return S.value({ description = 'Has ]] inside.\\nAnd more.' })");
    assert!(src.contains("[=["), "should step up a level:\n{src}");
    assert_eq!(
        vm.run("return roundtrip({ description = 'Has ]] inside.\\nAnd more.' })"),
        "ok"
    );

    // **Nothing is inserted around the content.** The old emitter wrote the
    // closing `]]` indented to match the field, which put that indentation
    // *inside* the string — so a description grew four spaces on every
    // read-and-rewrite, for ever. Nested two levels deep here, where the
    // temptation to indent is strongest.
    let src = vm.run("return S.value({ nested = { description = 'a\\nb\\n' } })");
    assert!(
        src.contains("\n]]") && !src.contains("    ]]"),
        "the closing bracket must not be indented:\n{src}"
    );
    // A string not ending in a newline closes flush against its last character,
    // for the same reason: a newline there would be a newline in the value.
    let src = vm.run("return S.value({ nested = { description = 'a\\nb' } })");
    assert!(src.contains("b]]"), "no newline may be invented before the close:\n{src}");

    // And both survive the trip, which is the property the formatting serves.
    assert_eq!(vm.run("return roundtrip({ d = 'a\\nb\\n' })"), "ok");
    assert_eq!(vm.run("return roundtrip({ d = 'a\\nb' })"), "ok");
}

/// Ordering is stable, so a file that has not changed produces no diff.
///
/// `codegen_d` sorted its exits and iterated its scenery items with `pairs`, so
/// half of every generated room reshuffled between writes and every diff was
/// noise.
#[test]
fn key_order_is_deterministic() {
    let mut vm = Vm::new();

    let once = vm.run("return S.value({ zebra = 1, apple = 2, mango = 3, ['_x'] = 4 })");
    for _ in 0..5 {
        let again = vm.run("return S.value({ zebra = 1, apple = 2, mango = 3, ['_x'] = 4 })");
        assert_eq!(once, again, "emission is not stable across calls");
    }
    // Sorted, so the order is predictable rather than merely repeatable.
    let apple = once.find("apple");
    let zebra = once.find("zebra");
    assert!(apple < zebra, "string keys should be sorted:\n{once}");
}

/// An explicit order wins, which is how codegen puts schema fields in schema
/// order rather than alphabetically.
#[test]
fn a_caller_can_impose_its_own_order() {
    let mut vm = Vm::new();

    let src = vm.run("return S.value({ description = 'd', id = 'x', short = 's' }, { \
                order = function() return { 'id', 'short', 'description' } end })",
        );
    let id = src.find("id ");
    let short = src.find("short ");
    let desc = src.find("description ");
    assert!(id < short && short < desc, "declared order should win:\n{src}");
}

/// A short array of scalars stays on one line. `tags = { "indoor", "damp" }` is
/// one fact; spread over four lines it reads as four.
#[test]
fn short_scalar_arrays_are_inline() {
    let mut vm = Vm::new();

    let src = vm.run("return S.value({ tags = { 'indoor', 'damp' } })");
    assert!(
        src.contains(r#"{ "indoor", "damp" }"#),
        "a short array should be inline:\n{src}"
    );

    // A long one is not, and still round-trips.
    assert_eq!(
        vm.run("local t = {} for i = 1, 40 do t[i] = 'tag_number_' .. i end \
             return roundtrip({ tags = t })"
        ),
        "ok"
    );
}

/// `M.module` produces a file, header comments and all.
#[test]
fn module_emits_a_loadable_file() {
    let mut vm = Vm::new();

    let src = vm.run("return S.module({ { id = 'crypt.hall' } }, \
                { header = { 'Generated by OLC.', '', 'DO NOT EDIT.' } })",
        );

    assert!(src.starts_with("-- Generated by OLC.\n--\n-- DO NOT EDIT.\n"), "{src}");
    assert!(src.contains("return {"), "{src}");
    assert!(src.ends_with("\n"), "a file should end with a newline");

    assert_eq!(
        vm.run("local src = S.module({ { id = 'crypt.hall' } }) \
             local t = load(src)() return t[1].id"
        ),
        "crypt.hall"
    );
}

/// The realistic shape: a room with every field type at once.
#[test]
fn a_whole_room_round_trips() {
    let mut vm = Vm::new();

    assert_eq!(
        vm.run(r#"return roundtrip({
                {
                    id          = 'crypt.hall',
                    short       = 'The "Great" Hall',
                    description = 'Bone stacked to the vault.\nFour hundred years of it.',
                    light       = 1,
                    smell       = 'Silt, and something older.',
                    tags        = { 'indoor', 'damp' },
                    exits       = { north = 'crypt.stair', down = 'crypt.ossuary' },
                    items       = { ['tide mark'] = 'A brown line, dead level.' },
                    stats       = { corruption = 3 },
                },
            })"#
        ),
        "ok"
    );
}
