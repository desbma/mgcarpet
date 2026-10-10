//! MC1 mob-speed runaway probe (player report 2026-10-10: vultures at
//! "mach speed" late in a level; worms shooting off cliffs on map 48
//! under `awake_range 0` with the clock sped up).
//!
//! Builds a baked MC1 level headless, parks a ghosted + invincible human
//! at the start marker (the player's own repro: invisibility +
//! immunity, then wait), ticks as fast as the CPU allows, and prints a
//! per-model SPEED census every `interval` ticks: live count, the
//! fastest creature's `+126` against its own `+128` max and `+130`
//! accel, its state and pack leader, and how many of the model are
//! above their own max. The final block lists the fastest slots so a
//! second run can attribute the writes (findings: docs/DEVIATIONS.md
//! `mc1_pack_speed_restore`):
//!
//! ```text
//! cargo run -p mgc-sim --release --example speed_runaway_probe -- \
//!     --level 48 --awake 0 --ticks 60000 --interval 5000
//! MGC_WRITE_TRACE=<slot>:f126 cargo run ... (same args)
//! ```
//!
//! `--awake 0` = always awake (`awake_gate_sq = i32::MAX`), any other
//! value = that radius in tiles (24 = retail). `--orbit R` flies the
//! human in a circle of R tiles instead of parking it. `--no-ghost`
//! drops the cloak/immunity. `--patch` arms `mc1_pack_speed_restore`
//! (the fix this probe measured); `--dump-model M` lists that model's
//! leader chains at every census.
use mgc_sim::engine::features::{FeatureAssets, Planes};
use mgc_sim::engine::world::{PlayerCommand, PlayerPose, World};
use mgc_sim::ids::GameId;
use mgc_sim::mc1::rivals::RivalConfig;
use std::collections::BTreeMap;

fn arg<T: std::str::FromStr>(name: &str, default: T) -> T {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn rival_configs(wizards: Option<&mgc_formats::Wizards>) -> ([Option<RivalConfig>; 8], u16) {
    let mut out: [Option<RivalConfig>; 8] = Default::default();
    let Some(w) = wizards else { return (out, 1) };
    let count = w.player_count.unwrap_or(1).min(8);
    for (slot, cfg) in w.wizards.iter().enumerate().take(8).skip(1) {
        let (Some(acc), Some(tempo), Some(allowed_mask)) =
            (cfg.accuracy, cfg.tempo, cfg.allowed_spells.as_ref())
        else {
            continue;
        };
        let mut book = [false; 24];
        let mut allowed = [false; 24];
        for s in 0..24 {
            let a = allowed_mask.get(s).copied().unwrap_or(0) != 0;
            allowed[s] = a;
            book[s] = a && cfg.starting_spells.get(s).copied().unwrap_or(0) != 0;
        }
        out[slot] = Some(RivalConfig {
            aggression: cfg.aggression.clamp(0, 255) as u8,
            accuracy: acc.clamp(0, 255) as u8,
            tempo: tempo.clamp(0, 255) as u8,
            castle_level: cfg.castle_level.unwrap_or(0),
            book,
            allowed,
        });
    }
    (out, count)
}

fn build_world(baked: &std::path::Path, game: &str, level: u32, awake_tiles: u32) -> World {
    let lp = baked.join(game).join(format!("level-{level:03}.mgcl"));
    let file = std::fs::File::open(&lp).unwrap_or_else(|e| panic!("{}: {e}", lp.display()));
    let pkg: mgc_formats::LevelPackage = mgc_formats::mgcl::read(file).expect("mgcl");
    let (variant, game_id) = if game == "mc1hw" {
        ("mc1-arctic", GameId::Mc1Hw)
    } else {
        ("mc1-temperate", GameId::Mc1)
    };
    let bundle =
        mgc_formats::bundle::Bundle::load(&baked.join("assets").join(variant)).expect("bundle");
    let terrain = pkg.terrain.as_ref().expect("terrain");
    let planes = Planes {
        height: terrain.height.clone(),
        tile_type: terrain.tile_type.clone(),
        shading: terrain.shading.clone().expect("shading"),
        angle: terrain.angle.clone().expect("angle"),
        ceiling: Vec::new(),
    };
    let mut assets = FeatureAssets::parse(
        bundle.search.as_ref().expect("search"),
        bundle.build_tab.as_ref().expect("build tab"),
        bundle.build_dat.as_ref().expect("build dat"),
    )
    .expect("assets");
    if let Some(prm) = bundle.bldgprm.as_deref() {
        assets = assets.with_bldgprm(prm);
    }
    if let Some(sp) = bundle.spells.as_deref() {
        assets = assets.with_spells(sp).expect("spells");
    }
    let seed = pkg.gen_params.as_ref().map_or(0, |g| g.seed);
    let mut chassis = game_id.chassis();
    chassis.awake_gate_sq = if awake_tiles == 0 {
        i32::MAX
    } else {
        let u = (awake_tiles * 256) as i32;
        u.saturating_mul(u)
    };
    let mut w = World::new_full_env(
        planes,
        &pkg.things.things,
        seed,
        assets,
        chassis,
        game_id,
        false,
    );
    if let Some(f) = pkg.gen_params.as_ref().and_then(|g| g.footer) {
        w.set_win_pct(f[0]);
    }
    let (wizards, player_count) = rival_configs(pkg.wizards.as_ref());
    w.set_wizards(&wizards, player_count);
    w
}

/// Every live class-5 creature as `(slot, model, state, speed @126,
/// max @128, accel @130, leader @52, awake @58)`, over all MC1 models.
fn rows(w: &World) -> Vec<(usize, u8, u8, i16, i16, i16, u16, i16)> {
    let mut out = Vec::new();
    for model in 0..=24u8 {
        for r in w.debug_flock_probe(5, model) {
            out.push((
                r.slot,
                model,
                r.state,
                r.speed,
                r.max_speed,
                r.min_speed,
                r.leader,
                r.awake,
            ));
        }
    }
    out
}

fn census(w: &World, t: u64) {
    // model -> (count, above_own_max, fastest row)
    let mut by: BTreeMap<u8, (usize, usize, (usize, u8, i16, i16, i16, u16, i16))> =
        BTreeMap::new();
    for (slot, model, state, f126, f128, f130, f52, f58) in rows(w) {
        let e = by
            .entry(model)
            .or_insert((0, 0, (slot, state, f126, f128, f130, f52, f58)));
        e.0 += 1;
        if f126 > f128 {
            e.1 += 1;
        }
        if f126 > e.2.2 {
            e.2 = (slot, state, f126, f128, f130, f52, f58);
        }
    }
    println!("t={t}");
    for (model, (n, above, (slot, state, f126, f128, f130, f52, f58))) in by {
        println!(
            "  m{model:<2} n={n:<4} above_max={above:<4} fastest slot {slot:<5} \
             speed {f126:<5} max {f128:<4} accel {f130:<3} state {state:<3} leader {f52:<5} awake {f58}"
        );
    }
    // `--dump-model M`: every creature of that model with its leader
    // chain (leader, the leader's speed/state, the leader's leader), to
    // see cycles and self-leaders in the pack graph.
    let dump: i32 = arg("--dump-model", -1);
    if dump >= 0 {
        let all = rows(w);
        let find = |slot: usize| all.iter().find(|r| r.0 == slot);
        for r in all.iter().filter(|r| r.1 == dump as u8) {
            let (slot, _, state, f126, _, _, f52, _) = *r;
            let (ls, lst, ll) = find(f52 as usize)
                .map(|l| (l.3, l.2, l.6))
                .unwrap_or((0, 0, 0));
            println!(
                "    slot {slot:<5} state {state:<3} speed {f126:<5} -> leader {f52:<5} \
                 (speed {ls:<5} state {lst:<3} leader {ll:<5}){}",
                if f52 as usize == slot { "  SELF" } else { "" }
            );
        }
    }
}

fn main() {
    let level: u32 = arg("--level", 48);
    let awake: u32 = arg("--awake", 0);
    let ticks: u64 = arg("--ticks", 60_000);
    let interval: u64 = arg("--interval", 5_000);
    let orbit: f32 = arg("--orbit", 0.0);
    let game: String = arg("--game", "mc1".to_string());
    let baked = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../baked");
    let mut w = build_world(&baked, &game, level, awake);
    if !flag("--no-ghost") {
        w.set_ghost(true);
        w.set_invincible(true);
    }
    // `--patch`: the `mc1_pack_speed_restore` arm (default = retail).
    if flag("--patch") {
        w.set_patches(mgc_sim::WorldPatches {
            mc1_pack_speed_restore: true,
            ..mgc_sim::WorldPatches::RETAIL
        });
    }
    let (sx, sz) = w
        .debug_start_marker(0)
        .map(|(x, y)| (x as f32 + 0.5, y as f32 + 0.5))
        .unwrap_or((128.0, 128.0));
    println!(
        "level {game}:{level} awake={awake} ticks={ticks} start=({sx:.1},{sz:.1}) \
         orbit={orbit} ghost={}",
        !flag("--no-ghost")
    );
    census(&w, 0);
    for t in 1..=ticks {
        let (x, z, yaw, speed) = if orbit > 0.0 {
            // One lap every ~1200 ticks (50 s), tangential heading.
            let a = t as f32 / 1200.0 * std::f32::consts::TAU;
            (
                sx + orbit * a.cos(),
                sz + orbit * a.sin(),
                a + std::f32::consts::FRAC_PI_2,
                orbit * std::f32::consts::TAU / 1200.0,
            )
        } else {
            (sx, sz, 0.0, 0.0)
        };
        let alt = w.ground_height_tiles(x, z) + 3.0;
        w.tick(
            PlayerPose::from_tiles(x, alt, z, yaw, 0.0, speed),
            PlayerCommand::default(),
        );
        if t % interval == 0 {
            census(&w, t);
        }
    }
    // Final: the fastest 15 creatures, for a MGC_WRITE_TRACE follow-up.
    let mut rows = rows(&w);
    rows.sort_by_key(|r| std::cmp::Reverse(r.3));
    println!("fastest at t={ticks}:");
    for (slot, model, state, f126, f128, f130, f52, f58) in rows.into_iter().take(15) {
        println!(
            "  slot {slot:<5} m{model:<2} speed {f126:<5} max {f128:<4} accel {f130:<3} \
             state {state:<3} leader {f52:<5} awake {f58}"
        );
    }
}
