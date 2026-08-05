//! The notice board — and with it, the half of the document store that no game
//! code had ever used.
//!
//! Twelve `db_*` efuns shipped, and three had a caller anywhere: `db_get`,
//! `db_put` and `db_delete`, all from `cache_d`. The entire query and
//! atomic-merge half — `db_find`, `db_count`, `db_insert`, `db_update`,
//! `db_unset`, `db_incr`, `db_exists` — had never been reached by anything a
//! player could do.
//!
//! Every filter operator gets used here at least once, because an operator with
//! no caller is an operator nobody has checked.


use crate::common::RealVm;

/// A player-shaped table with a name and a char id, which is all the board
/// wants. Going through a real login would test the login.
fn poster(vm: &mut RealVm, var: &str, char_id: i64, name: &str) {
    vm.eval(&format!(
        "{var} = {{ char_id = {char_id}, name = '{name}', \
                    send = function() end, send_lines = function() end }}"
    ))
    .unwrap();
}

#[test]
fn a_notice_can_be_posted_read_and_counted() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    poster(&mut vm, "_a", 401, "Alice");

    let id = vm
        .eval("return DAEMON.board.post(_a, 'news', 'The mine', 'Third level went.')")
        .unwrap();
    assert!(!id.is_empty() && id != "nil", "posting failed: {id}");

    assert_eq!(vm.eval("return DAEMON.board.count()").unwrap(), "1");
    assert_eq!(vm.eval("return DAEMON.board.count('news')").unwrap(), "1");
    assert_eq!(
        vm.eval("return DAEMON.board.count('trade')").unwrap(),
        "0",
        "a count filtered by category should not count another category's notices"
    );

    vm.eval(&format!("_n = DAEMON.board.read('{id}')")).unwrap();
    assert_eq!(vm.eval("return _n.subject").unwrap(), "The mine");
    assert_eq!(vm.eval("return _n.author").unwrap(), "Alice");
    assert_eq!(vm.eval("return _n.category").unwrap(), "news");

    // `db_incr`: reading counts a view, atomically. Two readers in one tick
    // must not lose one to a read-modify-write.
    assert_eq!(vm.eval("return _n.views").unwrap(), "1");
    vm.eval(&format!("_n2 = DAEMON.board.read('{id}')")).unwrap();
    assert_eq!(vm.eval("return _n2.views").unwrap(), "2");
}

/// A category that does not exist is refused by name, so a typo cannot create a
/// category nobody can find again.
#[test]
fn an_unknown_category_is_refused() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    poster(&mut vm, "_a", 402, "Alice");

    vm.eval("_id, _why = DAEMON.board.post(_a, 'nonsense', 'x', 'y')").unwrap();
    assert_eq!(vm.eval("return tostring(_id)").unwrap(), "nil");
    assert!(
        vm.eval("return _why").unwrap().contains("news"),
        "the refusal should list the categories that do exist"
    );

    // And so is an empty one, in either field.
    vm.eval("_id2, _why2 = DAEMON.board.post(_a, 'news', '', 'body')").unwrap();
    assert_eq!(vm.eval("return tostring(_id2)").unwrap(), "nil");
    assert_eq!(vm.eval("return _why2").unwrap(), "A notice needs a subject.");
}

/// Listing is newest first, filtered by category, and does not show what has
/// run out.
#[test]
fn listing_is_ordered_filtered_and_excludes_expired() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    poster(&mut vm, "_a", 403, "Alice");
    poster(&mut vm, "_b", 404, "Bob");

    vm.eval("DAEMON.board.post(_a, 'news', 'First', 'one') return 'ok'").unwrap();
    vm.eval("DAEMON.board.post(_b, 'trade', 'Second', 'two') return 'ok'").unwrap();
    vm.eval("_third = DAEMON.board.post(_a, 'news', 'Third', 'three')").unwrap();

    assert_eq!(vm.eval("return #DAEMON.board.list()").unwrap(), "3");
    assert_eq!(vm.eval("return #DAEMON.board.list('news')").unwrap(), "2");
    assert_eq!(vm.eval("return #DAEMON.board.list('trade')").unwrap(), "1");

    // Newest first. All three were posted in the same second, so the ordering
    // has to come from something the store can sort on rather than from luck —
    // and a tie is allowed to go either way, so this checks membership.
    vm.eval("_subjects = {} for _, n in ipairs(DAEMON.board.list('news')) do \
             _subjects[n.subject] = true end return 'ok'")
        .unwrap();
    assert_eq!(vm.eval("return tostring(_subjects.First and _subjects.Third)").unwrap(), "true");

    // Expire one by hand, through the store, and it stops being listed at once
    // rather than at the next sweep.
    vm.eval("db_update('board_notices', _third, { expires = 1 }) return 'expired'")
        .unwrap();
    assert_eq!(
        vm.eval("return #DAEMON.board.list('news')").unwrap(),
        "1",
        "an expired notice should stop being listed the moment it expires"
    );
    assert_eq!(vm.eval("return DAEMON.board.count()").unwrap(), "2");

    // The sweep is housekeeping, not correctness: it removes the row.
    assert_eq!(vm.eval("return DAEMON.board.sweep()").unwrap(), "1");
    assert_eq!(vm.eval("return db_count('board_notices')").unwrap(), "2");
}

/// `like`, `in`, `>`, `<=` and `exists` — every operator the board uses, and
/// each of them used at least once.
#[test]
fn search_uses_the_filter_language() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    poster(&mut vm, "_a", 405, "Alice");
    poster(&mut vm, "_b", 406, "Bob");

    vm.eval("DAEMON.board.post(_a, 'trade', 'Selling ore', 'Two silver a bar.') return 'ok'")
        .unwrap();
    vm.eval("DAEMON.board.post(_b, 'help', 'Lost in the marsh', 'Do not leave the stone.') return 'ok'")
        .unwrap();
    vm.eval("DAEMON.board.post(_a, 'news', 'Ore prices', 'Up again.') return 'ok'").unwrap();

    // `like` on the subject.
    assert_eq!(vm.eval("return #DAEMON.board.search('ore')").unwrap(), "2");
    // `like` on the body — and the same notice found twice is returned once.
    assert_eq!(vm.eval("return #DAEMON.board.search('stone')").unwrap(), "1");
    assert_eq!(vm.eval("return #DAEMON.board.search('nothing at all')").unwrap(), "0");

    // `in`, over authors.
    assert_eq!(vm.eval("return #DAEMON.board.by_authors({ 'Alice' })").unwrap(), "2");
    assert_eq!(
        vm.eval("return #DAEMON.board.by_authors({ 'Alice', 'Bob' })").unwrap(),
        "3"
    );
    assert_eq!(vm.eval("return #DAEMON.board.by_authors({ 'Nobody' })").unwrap(), "0");

    // `exists`, straight at the store: every notice has a body and none has a
    // `sticky` flag yet.
    assert_eq!(
        vm.eval("return #db_find('board_notices', { body = { exists = true } })").unwrap(),
        "3"
    );
    assert_eq!(
        vm.eval("return #db_find('board_notices', { sticky = { exists = true } })").unwrap(),
        "0"
    );

    // `limit` and `offset`, which is how a pager over a busy board would work.
    assert_eq!(vm.eval("return #DAEMON.board.list(nil, { limit = 2 })").unwrap(), "2");
    assert_eq!(
        vm.eval("return #DAEMON.board.list(nil, { limit = 2, offset = 2 })").unwrap(),
        "1"
    );
}

/// Editing is a recursive merge, so it must not disturb a field it did not
/// name — including one another process is incrementing at the same time.
#[test]
fn editing_merges_rather_than_replacing() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    poster(&mut vm, "_a", 407, "Alice");
    poster(&mut vm, "_b", 408, "Bob");

    vm.eval("_id = DAEMON.board.post(_a, 'news', 'Original', 'Body text.')").unwrap();
    vm.eval("DAEMON.board.read(_id) DAEMON.board.read(_id) return 'viewed'").unwrap();

    assert_eq!(
        vm.eval("return tostring(DAEMON.board.edit(_a, _id, 'Corrected', nil))").unwrap(),
        "true"
    );

    vm.eval("_n = DAEMON.board.read(_id)").unwrap();
    assert_eq!(vm.eval("return _n.subject").unwrap(), "Corrected");
    assert_eq!(
        vm.eval("return _n.body").unwrap(),
        "Body text.",
        "a merge must leave the field it was not told about alone"
    );
    assert_eq!(
        vm.eval("return _n.views").unwrap(),
        "3",
        "the view count survived the edit — writing the whole document back \
         would have raced with the reads"
    );
    assert_eq!(vm.eval("return tostring(_n.edited ~= nil)").unwrap(), "true");

    // Somebody else's notice is not yours to edit.
    assert_eq!(
        vm.eval("return tostring(DAEMON.board.edit(_b, _id, 'Vandalised', nil))").unwrap(),
        "false"
    );
    assert_eq!(vm.eval("return DAEMON.board.read(_id).subject").unwrap(), "Corrected");
}

/// `db_unset` exists because Lua tables cannot hold `nil`, so RFC 7396's
/// delete-by-null is unreachable through `db_update` from Lua. This is the one
/// place the board needs it.
#[test]
fn a_field_can_be_removed_outright() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    poster(&mut vm, "_a", 409, "Alice");

    vm.eval("_id = DAEMON.board.post(_a, 'news', 'Pinned', 'Read this.')").unwrap();
    vm.eval("db_update('board_notices', _id, { sticky = true }) return 'stuck'").unwrap();
    assert_eq!(
        vm.eval("return #db_find('board_notices', { sticky = { exists = true } })").unwrap(),
        "1"
    );

    assert_eq!(vm.eval("return tostring(DAEMON.board.unstick(_id))").unwrap(), "true");
    assert_eq!(
        vm.eval("return #db_find('board_notices', { sticky = { exists = true } })").unwrap(),
        "0",
        "the field is gone, not set to false — those are different states"
    );
}

/// Removal: your own, or anyone's with the permission. `db_exists` answers the
/// "is it still there" question without deserialising the notice.
#[test]
fn a_notice_can_be_taken_down_by_its_author_or_by_staff() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();
    poster(&mut vm, "_a", 410, "Alice");
    poster(&mut vm, "_b", 411, "Bob");

    vm.eval("_id = DAEMON.board.post(_a, 'rp', 'Mine', 'Body.')").unwrap();
    assert_eq!(vm.eval("return tostring(db_exists('board_notices', _id))").unwrap(), "true");

    // Not Bob's to remove.
    vm.eval("_ok, _why = DAEMON.board.remove(_b, _id, false)").unwrap();
    assert_eq!(vm.eval("return tostring(_ok)").unwrap(), "false");
    assert_eq!(vm.eval("return _why").unwrap(), "That is not your notice.");
    assert_eq!(vm.eval("return tostring(db_exists('board_notices', _id))").unwrap(), "true");

    // Staff may.
    assert_eq!(
        vm.eval("return tostring(DAEMON.board.remove(_b, _id, true))").unwrap(),
        "true"
    );
    assert_eq!(vm.eval("return tostring(db_exists('board_notices', _id))").unwrap(), "false");

    // And a notice that is not there is refused by name rather than silently.
    vm.eval("_ok2, _why2 = DAEMON.board.remove(_a, 'no-such-id', true)").unwrap();
    assert_eq!(vm.eval("return _why2").unwrap(), "There is no such notice.");
}

/// Through the command, as a player meets it. This is also the first game-layer
/// command at all — `game/cmds/board.lua` — so it proves command discovery
/// spans both roots.
#[test]
fn the_board_command_posts_and_reads() {
    let mut vm = RealVm::boot_real_mudlib(0);

    let empty = vm.command("board");
    assert!(empty.contains("Nothing on the board"), "{empty}");

    let out = vm.command("board post news Ore | The mine is shut.");
    assert!(out.contains("Posted as"), "posting through the command failed:\n{out}");

    let list = vm.command("board");
    assert!(list.contains("Ore"), "the notice is not listed:\n{list}");
    assert!(list.contains("news"), "{list}");

    // The id is generated, so read it back off the listing.
    let id = list
        .lines()
        .find(|l| l.contains("Ore"))
        .and_then(|l| l.split_whitespace().next())
        .expect("no id in the listing")
        .to_string();

    let one = vm.command(&format!("board read {id}"));
    assert!(one.contains("The mine is shut."), "the body is missing:\n{one}");
    assert!(one.contains("view"), "the view count is missing:\n{one}");

    assert!(vm.command("board mine").contains("Ore"));
    assert!(vm.command("board search mine").contains("Ore"));
    assert!(vm.command("board trade").contains("Nothing on the board"));

    assert!(vm.command(&format!("board remove {id}")).contains("Taken down"));
    assert!(vm.command("board").contains("Nothing on the board"));
}
