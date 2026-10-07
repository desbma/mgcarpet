//! `mgc-sprite-export <baked-dir> <out-dir> [variant ...]`: the world
//! sprites as PNG + JSON for working on the art outside the game.

fn main() -> std::process::ExitCode {
    mgc_app::sprite_export_main()
}
