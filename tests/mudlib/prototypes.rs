//! Prototypes: authoring by inheritance.
//!
//! Six creatures in two areas differed by four numbers and repeated the other
//! twelve keys each. A prototype is that skeleton, named once, and a template
//! says which one it is and what differs.
//!
//! Three properties this file exists to pin down:
//!
//! * **the per-type merge matrix.** A map merges key-by-key and everything else
//!   replaces, and *the schema decides which is which* — "does it look like an
//!   array" gets an empty `exits` wrong, silently. `flatten` takes its chain as
//!   an argument precisely so the algebra can be driven directly here;
//! * **a broken prototype costs one stat block, not the area.** Every failure —
//!   missing parent, cycle, depth, wrong kind — leaves the record with its own
//!   data and carries on. Nothing raises and nothing is dropped;
//! * **`@none` never reaches a registered template.** The sentinel is consumed
//!   by the resolver, which is the only reason `item_d.resolve` and `mob_d.spawn`
//!   need to know nothing about it.
//!
//! Nothing here touches `game/`: the fixture prototype file is written into the
//! test's own temporary game root, the trick `tests/verify_lint.rs` already uses
//! for areas.

use crate::common::RealVm;

fn one_line(src: &str) -> String {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

struct Vm {
    vm: RealVm,
    game: std::path::PathBuf,
}

impl Vm {
    fn new() -> Self {
        let mut vm = RealVm::boot_fixture_with_probe();
        let game = vm.game_root().unwrap().to_path_buf();
        vm.eval(
            "P = require('lib.prototype') PR = require('prototypes') \
             S = require('lib.schema') return 'ready'",
        )
        .unwrap();
        Self { vm, game }
    }

    fn run(&mut self, src: &str) -> String {
        self.vm.eval(&one_line(src)).unwrap()
    }

    /// Write a prototype file into the temp game root and re-read the index.
    fn prototypes(&mut self, file: &str, lua: &str) {
        let dir = self.game.join("prototypes");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{file}.lua")), lua).unwrap();
        self.run("PR.flush_cache() return 'flushed'");
    }
}

/// `flatten` with a hand-built chain, rendered as sorted `key=value` pairs so a
/// single string can carry the whole answer back through the probe.
const RENDER: &str = r#"
    function render(t)
      local keys = {}
      for k in pairs(t) do keys[#keys+1] = k end
      table.sort(keys)
      local out = {}
      for _, k in ipairs(keys) do
        local v = t[k]
        if type(v) == "table" then
          local inner = {}
          local n = 0
          for _ in pairs(v) do n = n + 1 end
          if #v > 0 and #v == n then
            for _, x in ipairs(v) do inner[#inner+1] = tostring(type(x) == "table" and "{...}" or x) end
            out[#out+1] = k .. "=[" .. table.concat(inner, ",") .. "]"
          else
            local ks = {}
            for kk in pairs(v) do ks[#ks+1] = tostring(kk) end
            table.sort(ks)
            for _, kk in ipairs(ks) do inner[#inner+1] = kk .. ":" .. tostring(v[kk]) end
            out[#out+1] = k .. "={" .. table.concat(inner, ",") .. "}"
          end
        else
          out[#out+1] = k .. "=" .. tostring(v)
        end
      end
      return table.concat(out, " ")
    end
"#;

// ─── The merge matrix ────────────────────────────────────────────────────────

/// A `map` merges key-by-key; every other shape replaces.
///
/// This is the whole reason the feature pays for itself on mobs: five stat keys
/// in the prototype and two in the child produce a creature with five, so an
/// area file stops restating `strength`, `dexterity` and `constitution` for
/// every crawler that has the usual ones.
#[test]
fn a_map_merges_key_by_key_and_everything_else_replaces() {
    let mut vm = Vm::new();
    vm.run(RENDER);

    let out = vm.run(
        r#"
        local chain = { { id = "beast", data = {
            stats = { strength = 10, dexterity = 12, constitution = 11 },
            damage = { min = 1, max = 2 },
            tags = { "beast", "common" },
            patrol = { "a.one", "a.two" },
            loot_table = { { item_id = "hide", chance = 0.5 } },
            aggressive = true,
            faction = "wild",
        } } }
        local merged = P.flatten("mob", {
            prototype = "beast",
            stats = { dexterity = 16, level = 7 },
            damage = { min = 5, max = 11 },
            tags = { "beast", "mine" },
        }, chain)
        return render(merged)
        "#,
    );

    // `stats` merged: the prototype's strength and constitution survive, the
    // child's dexterity wins, and its level is added.
    assert!(
        out.contains("stats={constitution:11,dexterity:16,level:7,strength:10}"),
        "a map must merge key-by-key, got: {out}"
    );
    // `damage` is a range — one value, replaced wholesale. If it merged, the
    // prototype's max of 2 would leak into a creature that hits for 5-11.
    assert!(out.contains("damage={max:11,min:5}"), "a range replaces wholesale: {out}");
    // Arrays replace. `common` is gone, deliberately: union has no removal.
    assert!(out.contains("tags=[beast,mine]"), "an array replaces: {out}");
    // Untouched fields come through.
    assert!(out.contains("aggressive=true") && out.contains("faction=wild"), "{out}");
    assert!(out.contains("patrol=[a.one,a.two]"), "{out}");
    assert!(out.contains("loot_table=[{...}]"), "{out}");
}

/// A record inside an `of_record` map is replaced whole, never deep-merged.
///
/// An exit inheriting its `target` from a parent while the child supplies only
/// `hidden` is a passage whose destination is invisible in the file in front of
/// you. Merging at the key level and no deeper is what keeps a record readable
/// on its own.
#[test]
fn a_record_valued_map_entry_is_replaced_whole() {
    let mut vm = Vm::new();
    vm.run(RENDER);

    let out = vm.run(
        r#"
        local chain = { { id = "cell", data = { exits = {
            north = { target = "gaol.hall", hidden = false },
            south = "gaol.yard",
        } } } }
        local merged = P.flatten("room", {
            prototype = "cell",
            exits = { north = { hidden = true } },
        }, chain)
        return tostring(merged.exits.north.target) .. "|"
            .. tostring(merged.exits.north.hidden) .. "|"
            .. tostring(merged.exits.south)
        "#,
    );

    assert_eq!(
        out, "nil|true|gaol.yard",
        "the north record is replaced whole (target gone) while south, a sibling \
         key the child never named, is inherited untouched"
    );
}

/// A chain composes root-first, so the nearest ancestor is the last to speak
/// before the datum itself.
#[test]
fn a_three_deep_chain_composes_left_to_right() {
    let mut vm = Vm::new();
    vm.run(RENDER);

    let out = vm.run(
        r#"
        local chain = {
            { id = "beast",         data = { faction = "wild", xp_award = 10, race = "beast",
                                             stats = { strength = 8 } } },
            { id = "beast.crawler", data = { faction = "vermin", xp_award = 20,
                                             stats = { dexterity = 16 } } },
            { id = "beast.venom",   data = { xp_award = 30, stats = { strength = 12 } } },
        }
        local merged = P.flatten("mob", { prototype = "beast.venom", xp_award = 40 }, chain)
        return render(merged)
        "#,
    );

    assert!(out.contains("xp_award=40"), "the datum wins over every ancestor: {out}");
    assert!(out.contains("faction=vermin"), "the nearest ancestor to set it wins: {out}");
    assert!(out.contains("race=beast"), "the root still reaches through: {out}");
    assert!(
        out.contains("stats={dexterity:16,strength:12}"),
        "map keys accumulate across the chain with the nearest winning: {out}"
    );
}

/// `@none` removes an inherited field, and one inherited map key.
///
/// The sentinel exists because a prototyped record is incomplete by
/// construction: the value to remove is in the *parent's* file, so "take it out
/// in OLC" — `custom.lua`'s answer — is not available. Without it a child
/// needing one field fewer has to stop inheriting or make the prototype worse.
#[test]
fn none_strikes_an_inherited_field_and_an_inherited_map_key() {
    let mut vm = Vm::new();
    vm.run(RENDER);

    let out = vm.run(
        r#"
        local chain = { { id = "beast", data = {
            patrol = { "a.one" }, faction = "wild",
            stats = { strength = 10, fear = 3 },
        } } }
        local merged = P.flatten("mob", {
            prototype = "beast",
            patrol = P.NONE,
            stats = { fear = P.NONE },
        }, chain)
        return render(merged)
        "#,
    );

    assert!(!out.contains("patrol"), "a struck field is gone, not set to '@none': {out}");
    assert!(out.contains("stats={strength:10}"), "one map key struck, its sibling kept: {out}");
    assert!(out.contains("faction=wild"), "striking one field touches no other: {out}");
    assert!(!out.contains("@none"), "the sentinel is consumed, never emitted: {out}");
}

/// A strike in a root prototype deletes nothing and must not survive as a
/// literal string. There is simply nothing above it to remove.
#[test]
fn none_in_a_root_prototype_is_ignored_rather_than_stored() {
    let mut vm = Vm::new();
    vm.run(RENDER);

    let out = vm.run(
        r#"
        local chain = { { id = "beast", data = { faction = P.NONE, race = "beast" } } }
        return render(P.flatten("mob", { prototype = "beast" }, chain))
        "#,
    );

    assert!(!out.contains("@none"), "a strike with nothing above it is dropped: {out}");
    assert!(!out.contains("faction"), "{out}");
    assert!(out.contains("race=beast"), "{out}");
}

/// A field no schema declares is kept and replaces — CLAUDE.md's rule holds
/// here unchanged. With no descriptor there is no merge rule, and replace is
/// the safe default.
#[test]
fn a_field_no_schema_declares_is_kept_and_replaces() {
    let mut vm = Vm::new();
    vm.run(RENDER);

    let out = vm.run(
        r#"
        local chain = { { id = "beast", data = { mood = "surly", omen = "raven" } } }
        return render(P.flatten("mob", { prototype = "beast", mood = "placid" }, chain))
        "#,
    );

    assert!(out.contains("mood=placid"), "the child wins: {out}");
    assert!(out.contains("omen=raven"), "and an undeclared field still inherits: {out}");
}

/// Neither the datum nor any prototype table is mutated by a flatten.
///
/// A prototype's tables are shared by every template that inherits it. One
/// creature growing a tag must not give it to every creature in the game, and
/// nothing about the merge makes that obvious — `merge_one` used to write
/// straight into `data[key]`.
#[test]
fn flatten_mutates_neither_the_datum_nor_the_prototype() {
    let mut vm = Vm::new();

    let out = vm.run(
        r#"
        local proto = { stats = { strength = 10 }, tags = { "beast" } }
        local chain = { { id = "beast", data = proto } }
        local child = { prototype = "beast", stats = { level = 3 } }
        local a = P.flatten("mob", child, chain)
        a.stats.strength = 99
        a.tags[#a.tags+1] = "tainted"
        local b = P.flatten("mob", child, chain)
        return tostring(proto.stats.strength) .. "|" .. tostring(proto.stats.level)
            .. "|" .. tostring(#proto.tags) .. "|" .. tostring(b.stats.strength)
            .. "|" .. tostring(child.stats.strength)
        "#,
    );

    assert_eq!(
        out, "10|nil|1|10|nil",
        "the prototype keeps its own strength and tag count, the child gains no \
         inherited key, and a second flatten is unaffected by the first"
    );
}

// ─── Discovery ───────────────────────────────────────────────────────────────

/// The index finds `game/prototypes/*.lua` across the jail roots, and a flush
/// picks up an edit — which is what makes "editing a prototype takes effect on
/// area reload" true rather than aspirational.
#[test]
fn prototypes_are_discovered_from_the_game_root_and_reread_on_flush() {
    let mut vm = Vm::new();

    assert_eq!(
        vm.run("return tostring(PR.get('mob', 'beast'))"),
        "nil",
        "nothing before the file exists"
    );

    vm.prototypes(
        "fixture",
        r#"return { mobs = { ["beast"] = { faction = "wild", xp_award = 10 } } }"#,
    );
    assert_eq!(vm.run("return tostring(PR.get('mob', 'beast').faction)"), "wild");
    assert_eq!(vm.run("return table.concat(PR.ids('mob'), ',')"), "beast");

    vm.prototypes(
        "fixture",
        r#"return { mobs = { ["beast"] = { faction = "tamed", xp_award = 10 } } }"#,
    );
    assert_eq!(
        vm.run("return tostring(PR.get('mob', 'beast').faction)"),
        "tamed",
        "a flush must purge package.loaded, or require hands back the old table"
    );
}

/// A prototype carrying an `id` would merge it into every child, and every one
/// of them would register under the same name — which reads in the log as "the
/// last area loaded ate the others". Dropped, loudly.
#[test]
fn a_prototype_declaring_an_id_has_it_dropped_and_reported() {
    let mut vm = Vm::new();
    vm.prototypes(
        "fixture",
        r#"return { mobs = { ["beast"] = { id = "stowaway", faction = "wild" } } }"#,
    );

    assert_eq!(vm.run("return tostring(PR.get('mob', 'beast').id)"), "nil");
    let problems = vm.run(
        "local out = {} for _, p in ipairs(PR.problems()) do out[#out+1] = p.message end \
         return table.concat(out, '\\n')",
    );
    assert!(problems.contains("declares an `id`"), "it must be reported: {problems}");
}

// ─── Failure modes ───────────────────────────────────────────────────────────

/// Every failure leaves the record with its own data and carries on. A broken
/// prototype costs one creature's stat block, never the area.
#[test]
fn a_broken_chain_leaves_the_record_unresolved_and_never_raises() {
    let mut vm = Vm::new();
    vm.prototypes(
        "fixture",
        r#"return {
            mobs = {
                ["loop.a"] = { prototype = "loop.b", faction = "a" },
                ["loop.b"] = { prototype = "loop.a", faction = "b" },
                ["deep1"] = { faction = "deep" },
                ["deep2"] = { prototype = "deep1" }, ["deep3"] = { prototype = "deep2" },
                ["deep4"] = { prototype = "deep3" }, ["deep5"] = { prototype = "deep4" },
                ["deep6"] = { prototype = "deep5" }, ["deep7"] = { prototype = "deep6" },
                ["deep8"] = { prototype = "deep7" }, ["deep9"] = { prototype = "deep8" },
                ["deep10"] = { prototype = "deep9" },
            },
            items = { ["reagent"] = { weight = 0.5 } },
        }"#,
    );

    // Missing parent.
    let out = vm.run(
        "local m, e = P.resolve('mob', { id = 'x', prototype = 'nope', name = 'x' }) \
         return tostring(e) .. '|' .. tostring(m.name)",
    );
    assert!(out.starts_with("prototype 'nope' does not exist|"), "{out}");
    assert!(out.ends_with("|x"), "the record keeps its own data: {out}");

    // A cycle names the full path, because the path is the fix.
    let out = vm.run(
        "local m, e = P.resolve('mob', { id = 'marsh_lurker', prototype = 'loop.a' }) \
         return tostring(e)",
    );
    assert_eq!(
        out, "prototype cycle: marsh_lurker -> loop.a -> loop.b -> loop.a",
        "the message must be the path, not 'a cycle exists'"
    );

    // Depth.
    let out = vm.run("local _, e = P.resolve('mob', { id = 'x', prototype = 'deep10' }) return tostring(e)");
    assert!(out.starts_with("prototype chain deeper than 8"), "{out}");

    // A parent of the wrong kind says so, rather than "does not exist" — which
    // is the same message a typo gets.
    let out = vm.run("local _, e = P.resolve('mob', { id = 'x', prototype = 'reagent' }) return tostring(e)");
    assert_eq!(out, "prototype 'reagent' is a item prototype; this is a mob");
}

/// `resolve_list` reports what failed and resolves the rest of the file.
#[test]
fn one_broken_record_does_not_stop_the_others() {
    let mut vm = Vm::new();
    vm.prototypes("fixture", r#"return { mobs = { ["beast"] = { faction = "wild" } } }"#);

    let out = vm.run(
        r#"
        local list = {
            { id = "good", prototype = "beast" },
            { id = "bad",  prototype = "missing" },
            { id = "plain" },
        }
        local _, report = P.resolve_list("mob", list)
        return report.resolved .. "|" .. #report.failed .. "|" .. report.failed[1].id
            .. "|" .. tostring(list[1].faction) .. "|" .. tostring(list[2].faction)
        "#,
    );

    assert_eq!(out, "1|1|bad|wild|nil");
}

// ─── The invariant that keeps item_d and mob_d ignorant ──────────────────────

/// **`@none` never reaches a registered template.**
///
/// This is what lets `item_d.resolve`, `mob_d.spawn` and every downstream
/// consumer know nothing about the sentinel: it is a load-time authoring
/// construct that is gone before anything is registered.
#[test]
fn the_sentinel_never_survives_into_a_resolved_record() {
    let mut vm = Vm::new();
    vm.prototypes(
        "fixture",
        r#"return { mobs = { ["beast"] = {
            patrol = { "a.one" }, faction = "wild", stats = { strength = 10, fear = 3 },
        } } }"#,
    );

    let out = vm.run(
        r#"
        local list = { { id = "x", prototype = "beast", patrol = P.NONE,
                         stats = { fear = P.NONE }, faction = P.NONE } }
        P.resolve_list("mob", list)
        local found = {}
        local function walk(t, path)
          for k, v in pairs(t) do
            if v == P.NONE then found[#found+1] = path .. tostring(k) end
            if type(v) == "table" then walk(v, path .. tostring(k) .. ".") end
          end
        end
        walk(list[1], "")
        return table.concat(found, ",") .. "|" .. tostring(list[1].patrol)
            .. "|" .. tostring(list[1].faction) .. "|" .. tostring(list[1].stats.strength)
        "#,
    );

    assert_eq!(out, "|nil|nil|10", "no '@none' anywhere in the resolved record");
}

// ─── The components problem ─────────────────────────────────────────────────

/// A prototype that names a component makes its fields real on every child.
///
/// Before this, `fields_for` asked `components.claimed(data)` about the *raw*
/// child, so a child inheriting `weapon` had no `damage` field: `olc set damage`
/// refused it, `verify` skipped the whole block, the orderer misplaced it and
/// `unknown` reported it as declared by nothing. Four wrong answers, one cause.
#[test]
fn a_prototypes_components_make_their_fields_real_on_the_child() {
    let mut vm = Vm::new();
    vm.prototypes(
        "fixture",
        r#"return { items = { ["blade"] = {
            components = { "weapon" }, weight = 3, damage = { min = 2, max = 5 },
        } } }"#,
    );

    let out = vm.run(
        r#"
        local child = { id = "shiv", prototype = "blade" }
        local names = {}
        for _, f in ipairs(S.fields_for("item", child)) do names[f.name] = true end
        return tostring(names.damage) .. "|" .. tostring(S.field("item", "damage", child) ~= nil)
        "#,
    );
    assert_eq!(out, "true|true", "`damage` must be a field on the child before resolution");

    // `olc set` goes through `schema.set`, so this is the same question the
    // builder asks by typing.
    let out = vm.run(
        r#"
        local child = { id = "shiv", prototype = "blade" }
        local ok, err = S.set("item", child, "damage", "4-9")
        return tostring(ok) .. "|" .. tostring(err)
        "#,
    );
    assert_eq!(out, "true|nil", "a builder must be able to set an inherited component's field");
}

/// The invariant that keeps the shallow seed honest: whatever
/// `discovery_seed` believes about `components`, the real resolve must agree.
/// They are two code paths answering one question, and they are only allowed to
/// answer it the same way.
#[test]
fn the_discovery_seed_and_the_resolve_agree_about_components() {
    let mut vm = Vm::new();
    vm.prototypes(
        "fixture",
        r#"return { items = {
            ["base"]  = { components = { "weapon" }, weight = 1 },
            ["armed"] = { prototype = "base" },
            ["worn"]  = { prototype = "base", components = { "armour" } },
        } }"#,
    );

    let out = vm.run(
        r#"
        local function pair(child)
          local seed = P.discovery_seed("item", child)
          local full = P.resolve("item", child)
          return table.concat(seed.components or {}, ",") .. "/"
              .. table.concat(full.components or {}, ",")
        end
        return pair({ id = "a", prototype = "armed" }) .. "|"
            .. pair({ id = "b", prototype = "worn" }) .. "|"
            .. pair({ id = "c", prototype = "base", components = { "container" } })
        "#,
    );

    assert_eq!(
        out, "weapon/weapon|armour/armour|container/container",
        "nearest writer wins in both, because `components` is a string_array and \
         a string_array replaces"
    );
}

// ─── Origin ──────────────────────────────────────────────────────────────────

/// Where a value came from, which is what `olc show` marks each line with.
#[test]
fn origin_distinguishes_set_here_from_inherited_from_struck() {
    let mut vm = Vm::new();
    vm.prototypes(
        "fixture",
        r#"return { mobs = {
            ["beast"]         = { race = "beast", faction = "wild", xp_award = 5 },
            ["beast.crawler"] = { prototype = "beast", faction = "vermin" },
        } }"#,
    );

    let out = vm.run(
        r#"
        local child = { id = "x", prototype = "beast.crawler", name = "crawler",
                        xp_award = P.NONE }
        local function o(f) local a, b = P.origin("mob", child, f) return a .. ":" .. tostring(b) end
        return o("name") .. "|" .. o("faction") .. "|" .. o("race") .. "|"
            .. o("xp_award") .. "|" .. o("count") .. "|" .. o("title")
        "#,
    );

    assert_eq!(
        out,
        "self:nil|inherited:beast.crawler|inherited:beast|struck:nil|default:nil|unset:nil",
        "faction comes from the nearest ancestor that sets it, race reaches past it \
         to the root, and a field with a schema default but no value anywhere is \
         'default' rather than 'unset'"
    );
}

// ─── Thinning ────────────────────────────────────────────────────────────────

/// `olc thin` drops what only restates the prototype — and nothing else.
///
/// The *safe* form of subtraction: a human asked for it and sees what went.
/// Codegen must never do this on its own, because a builder who deliberately
/// sets a value equal to the inherited one is saying "this is mine now, and it
/// must not move if the prototype moves".
#[test]
fn thin_removes_only_what_restates_the_prototype() {
    let mut vm = Vm::new();
    vm.prototypes(
        "fixture",
        r#"return { mobs = { ["beast"] = {
            faction = "wild", xp_award = 10, aggressive = true,
            damage = { min = 1, max = 2 }, tags = { "beast" },
        } } }"#,
    );

    let out = vm.run(
        r#"
        local child = { id = "x", prototype = "beast", faction = "wild", xp_award = 40,
                        damage = { min = 1, max = 2 }, tags = { "beast" }, name = "thing" }
        local removed = P.thin("mob", child)
        local left = {}
        for k in pairs(child) do left[#left+1] = k end
        table.sort(left)
        return table.concat(removed, ",") .. "|" .. table.concat(left, ",")
        "#,
    );

    assert_eq!(
        out, "damage,faction,tags|id,name,prototype,xp_award",
        "a deep-equal range and array go too; `id` and `prototype` are never \
         candidates, and a value that differs is left alone"
    );
}
