//! This game's own GMCP packages.
//!
//! `game/daemons/gmcp_game_d.lua` registers `Game.Quest`, `Game.Quest.Request`
//! and `Game.Quest.Track`. The dispatcher that routes to them is mudlib and is
//! tested in `tests/gmcp_inbound.rs`; the packages themselves are content.

use crate::common::RealVm;
/// A custom package: registered by the game, dispatched by the mudlib, and the
/// mudlib's dispatcher never changed to allow it.
#[test]
fn the_game_can_add_its_own_package() {
    let mut vm = RealVm::boot_real_mudlib_with_probe();

    for package in ["game.quest.request", "game.quest.track"] {
        assert_eq!(
            vm.eval(&format!("return tostring(DAEMON.gmcp._handlers['{package}'] ~= nil)"))
                .unwrap(),
            "true",
            "'{package}' should be registered by the game layer"
        );
    }

    // Tracking an unknown quest is refused rather than stored.
    vm.eval("DAEMON.gmcp.receive('s4', 'Game.Quest.Track', { id = 'no_such_quest' })").unwrap();
    assert_eq!(
        vm.eval("return tostring((DAEMON.gmcp_game._tracking or {})['s4'])").unwrap(),
        "nil"
    );

    // A real one is.
    vm.eval("DAEMON.gmcp.receive('s4', 'Game.Quest.Track', { id = 'thin_the_crawlers' })")
        .unwrap();
    assert_eq!(
        vm.eval("return tostring(DAEMON.gmcp_game._tracking['s4'])").unwrap(),
        "thin_the_crawlers"
    );
}

