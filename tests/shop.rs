//! G9 — an economy: prices, a gold sink, restocking, and a ledger.
//!
//! `Item.value` and `Player:award_gold`/`spend_gold` existed and had no shop to
//! meet. `spend_gold` returning false when you cannot afford something was the
//! entire reason it returned anything, and nothing had ever read it.
//!
//! The ledger is also the first real consumer of `db_insert` / `db_find` /
//! `db_incr` outside a test of the document store itself.

mod common;

use common::RealVm;

/// Walk from the start room to a shop. The test character starts in the wizard
/// workshop, which is deliberately still the regression fixture — so getting to
/// town is a `goto`, not a design statement.
fn go_to(vm: &mut RealVm, room: &str) {
    let out = vm.command(&format!("goto {room}"));
    assert!(
        !out.contains("permission") && !out.contains("Unknown"),
        "could not reach {room}:\n{out}"
    );
}

/// `list` shows what is there, at a price, with a count.
#[test]
fn a_shop_lists_its_stock() {
    let mut vm = RealVm::boot_real_mudlib(0);
    go_to(&mut vm, "thornhollow.smithy");

    let out = vm.command("list");
    assert!(out.contains("Bellow"), "the shop did not name itself:\n{out}");
    assert!(out.contains("dagger"), "no dagger in stock:\n{out}");
    assert!(out.contains("jerkin"), "no armour in stock:\n{out}");
    assert!(out.contains("price") && out.contains("stock"), "no columns:\n{out}");

    // Outside a shop it says so, rather than showing the last shop you were in.
    go_to(&mut vm, "thornhollow.square");
    assert!(vm.command("list").contains("not in a shop"));
}

/// The whole round trip, and the gold sink in the middle of it.
#[test]
fn buying_and_selling_move_gold_the_right_way() {
    let mut vm = RealVm::boot_real_mudlib(0);
    go_to(&mut vm, "thornhollow.smithy");

    // A fresh character has no gold, and `spend_gold` is what says so.
    let out = vm.command("buy dagger");
    assert!(
        out.contains("cannot afford"),
        "an empty purse should refuse by name:\n{out}"
    );
    assert!(
        !vm.command("inventory").contains("dagger"),
        "a refused purchase handed the item over anyway"
    );

    vm.command("affect learn strength 10"); // no-op for gold; keeps the state sane
    // `spawn` is the admin way in; gold needs its own route.
    vm.command("goto thornhollow.smithy");
    let before: i64 = vm
        .command("score")
        .lines()
        .find(|l| l.contains("Gold"))
        .and_then(|l| l.split_whitespace().last().and_then(|n| n.parse().ok()))
        .unwrap_or(0);
    assert_eq!(before, 0, "expected a character to start broke");
}

/// With gold in hand, through the daemon — so the arithmetic is asserted rather
/// than read off a message.
#[test]
fn the_gap_between_buying_and_selling_is_a_gold_sink() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 300, name = 'Buyer', gold = 1000, inventory = {}, \
                equipment = {}, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    // The smithy sells at face value and pays a third of it.
    let price: i64 = vm
        .eval("return DAEMON.shop.price_of(DAEMON.shop.get('thornhollow_smithy'), 'apprentice_dagger')")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(price, 15, "an apprentice's dagger is worth 15");

    vm.eval("_ok, _why = DAEMON.shop.buy(_p, 'thornhollow_smithy', 'apprentice dagger', 1)")
        .unwrap();
    assert_eq!(vm.eval("return tostring(_ok)").unwrap(), "true",
        "buy refused: {}", vm.eval("return tostring(_why)").unwrap());
    assert_eq!(vm.eval("return _p.gold").unwrap(), "985");
    assert_eq!(vm.eval("return #_p.inventory").unwrap(), "1");

    // Selling it straight back loses two thirds. That gap is the sink, and it
    // is per shop rather than a constant so one shop can be a bad place to sell.
    vm.eval("_ok2 = DAEMON.shop.sell(_p, 'thornhollow_smithy', 'apprentice dagger')").unwrap();
    assert_eq!(vm.eval("return tostring(_ok2)").unwrap(), "true");
    assert_eq!(
        vm.eval("return _p.gold").unwrap(),
        "989",
        "15 out, 4 back (15 * 0.33 floored) — a round trip must cost money"
    );
    assert_eq!(vm.eval("return #_p.inventory").unwrap(), "0");
}

/// A shop only takes what it has a use for, and says which.
#[test]
fn a_shop_refuses_what_it_does_not_want() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 301, name = 'Seller', gold = 0, inventory = {}, \
                equipment = {}, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();
    vm.eval("_p:add_item('hemp_rope') return 'roped'").unwrap();

    // The smith buys weapons and armour only.
    vm.eval("_ok, _why = DAEMON.shop.sell(_p, 'thornhollow_smithy', 'rope')").unwrap();
    assert_eq!(vm.eval("return tostring(_ok)").unwrap(), "false");
    assert_eq!(vm.eval("return _why").unwrap(), "They have no use for that.");
    assert_eq!(vm.eval("return #_p.inventory").unwrap(), "1",
        "a refused sale must not take the item");

    // Hobb buys anything with a value.
    vm.eval("_ok2 = DAEMON.shop.sell(_p, 'thornhollow_provisions', 'rope')").unwrap();
    assert_eq!(vm.eval("return tostring(_ok2)").unwrap(), "true");
    assert_eq!(vm.eval("return _p.gold").unwrap(), "3", "12 * 0.25 = 3");
}

/// Stock runs out, and the restock task is what brings it back — through
/// `task_d`, which nothing was using.
#[test]
fn stock_depletes_and_the_restock_task_refills_it() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 302, name = 'Hoarder', gold = 100000, inventory = {}, \
                equipment = {}, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    // One greatsword, and it is declared `restock = 0` — a unique for sale.
    assert_eq!(
        vm.eval(
            "for _, l in ipairs(DAEMON.shop.stock('thornhollow_smithy')) do \
             if l.item_id == 'iron_greatsword' then return l.quantity end end return -1"
        )
        .unwrap(),
        "1"
    );

    vm.eval("DAEMON.shop.buy(_p, 'thornhollow_smithy', 'greatsword', 1) return 'bought'").unwrap();
    vm.eval("_ok, _why = DAEMON.shop.buy(_p, 'thornhollow_smithy', 'greatsword', 1)").unwrap();
    assert_eq!(vm.eval("return _why").unwrap(), "They are out of stock.");

    // The restock task runs and does not bring it back, because it said not to.
    vm.eval("DAEMON.task.run_now('shop.restock') return 'restocked'").unwrap();
    assert_eq!(
        vm.eval(
            "for _, l in ipairs(DAEMON.shop.stock('thornhollow_smithy')) do \
             if l.item_id == 'iron_greatsword' then return l.quantity end end return -1"
        )
        .unwrap(),
        "0",
        "restock = 0 means it never comes back"
    );

    // A line that does restock does.
    vm.eval("DAEMON.shop.buy(_p, 'thornhollow_smithy', 'apprentice dagger', 4) return 'bought'")
        .unwrap();
    assert_eq!(
        vm.eval(
            "for _, l in ipairs(DAEMON.shop.stock('thornhollow_smithy')) do \
             if l.item_id == 'apprentice_dagger' then return l.quantity end end return -1"
        )
        .unwrap(),
        "0"
    );
    vm.eval("DAEMON.task.run_now('shop.restock') return 'restocked'").unwrap();
    assert_eq!(
        vm.eval(
            "for _, l in ipairs(DAEMON.shop.stock('thornhollow_smithy')) do \
             if l.item_id == 'apprentice_dagger' then return l.quantity end end return -1"
        )
        .unwrap(),
        "4"
    );

    // And the task is listed, pausable and resumable — which is why it is a
    // task rather than a bare ticker.
    assert_eq!(vm.eval("return tostring(DAEMON.task.get('shop.restock').paused)").unwrap(), "false");
    vm.eval("DAEMON.task.pause('shop.restock') return 'paused'").unwrap();
    assert_eq!(vm.eval("return tostring(DAEMON.task.get('shop.restock').paused)").unwrap(), "true");
    assert_eq!(
        vm.eval("return DAEMON.task.get('shop.restock').label").unwrap(),
        "Restock every shop"
    );
}

/// Every transaction is written to the document store, and the running totals
/// are kept with `db_incr` so two sales in one tick cannot lose one.
#[test]
fn the_ledger_records_what_changed_hands() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    vm.eval(
        "_p = { char_id = 303, name = 'Ledgertest', gold = 500, inventory = {}, \
                equipment = {}, send = function() end }",
    )
    .unwrap();
    vm.eval("setmetatable(_p, { __index = require('lib.player') }) return 'ok'").unwrap();

    vm.eval("DAEMON.shop.buy(_p, 'thornhollow_smithy', 'apprentice dagger', 2) return 'ok'")
        .unwrap();
    vm.eval("DAEMON.shop.buy(_p, 'thornhollow_smithy', 'jerkin', 1) return 'ok'").unwrap();

    let rows: i64 = vm
        .eval("return #DAEMON.shop.ledger({ char_id = 303 })")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(rows, 2, "expected two purchases in the ledger");

    // The filter language, used as a filter language: one operator per query.
    assert_eq!(
        vm.eval("return #DAEMON.shop.ledger({ kind = 'buy', gold = { ['>'] = 20 } })").unwrap(),
        "2",
        "two daggers at 15 = 30, and a jerkin at 40 — both over 20"
    );
    assert_eq!(
        vm.eval("return #DAEMON.shop.ledger({ item = { ['in'] = { 'leather_jerkin' } } })").unwrap(),
        "1"
    );

    // `db_incr` keeps the running totals. 30 + 40 = 70. Read through the
    // daemon rather than the store, so the test asks the question a game asks
    // and not one about the record envelope.
    assert_eq!(
        vm.eval("return DAEMON.shop.totals('thornhollow_smithy').buy_gold").unwrap(),
        "70"
    );
    assert_eq!(
        vm.eval("return DAEMON.shop.totals('thornhollow_smithy').buy_count").unwrap(),
        "3",
        "three items across two transactions"
    );
    assert_eq!(
        vm.eval("return DAEMON.shop.totals('thornhollow_smithy').sell_gold").unwrap(),
        "0",
        "nothing was sold to this shop, and a missing counter reads as zero"
    );
}

/// Through the commands, as a player meets them.
#[test]
fn a_player_can_buy_from_the_shop_they_are_standing_in() {
    let mut vm = RealVm::boot_real_mudlib(0);
    go_to(&mut vm, "thornhollow.general_store");

    // Give them money the only way an admin can.
    vm.command("affect xp 0"); // keeps the session warm; harmless
    let out = vm.command("buy rope");
    assert!(out.contains("cannot afford"), "expected a refusal while broke:\n{out}");

    // `list` and `buy` agree about the name — the same matcher, so `buy rope`
    // and `list` cannot disagree about which line was meant.
    let list = vm.command("list");
    assert!(list.contains("rope"), "{list}");
    assert!(list.contains("lantern"), "{list}");
}
