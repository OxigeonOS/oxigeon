//! The demo world's own help pages.
//!
//! `game.example/docs/` is shipped content, so this is the bucket that goes
//! when `game.example/` goes. The mechanism — discovery, merging, the
//! Markdown — is asserted against the fixture world in `tests/mudlib/help.rs`.
//! What is left to ask here is whether the demo actually exercises the feature,
//! which is a question about the demo.

use crate::common::RealVm;

#[test]
fn the_shipped_docs_are_discovered_as_categories() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");
    let out = vm.command("help");

    // `lore` exists only because `game.example/docs/lore/` does — no command
    // declares that category.
    assert!(out.contains("Lore"), "{out}");
    assert!(out.contains("Combat"), "{out}");
}

#[test]
fn a_shipped_markdown_page_renders() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");
    let out = vm.command("help stances");

    assert!(out.contains("===[ Stances ]==="), "{out}");
    assert!(out.contains("=== Choosing one ==="), "{out}");
    assert!(out.contains("- **Balanced**"), "{out}");
}

/// One page with no extension, so the demo covers both halves of "a filename
/// may have `.md` or nothing". Its ASCII diagram is the reason `plain` does not
/// reflow paragraphs.
#[test]
fn the_extensionless_page_keeps_its_diagram() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");
    let out = vm.command("help parrying");

    assert!(out.contains("attacker"), "{out}");
    assert!(out.contains("--->"), "{out}");
    // Not parsed: the `====` underline is text, not a heading.
    assert!(!out.contains("===[ PARRYING ]==="), "{out}");
}

#[test]
fn the_combat_category_holds_both_commands_and_pages() {
    let mut vm = RealVm::boot_real_mudlib(0);
    vm.command("pagesize 0");
    let out = vm.command("help combat");

    assert!(out.contains("attack"), "{out}");
    assert!(out.contains("stances"), "{out}");
}
