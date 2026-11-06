//! The release gate's diagnostic, before building the bundles (decisions.md R-SKIN-KEYS).

fn main() {
    if let Err(why) = skin_pack::keys::check_release_key(
        skin_pack::keys::IDLE_FOUNDRY_KEY,
        skin_pack::keys::TRUSTED_KEYS,
    ) {
        eprintln!("Release blocked: {why}. See docs/skin-packs.md, Signing first-party packs.");
        std::process::exit(1);
    }
    println!("Idle Foundry's first-party public key is ready for release.");
}
