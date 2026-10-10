//! ⭐ MULTIPLAYER STAGE 3, SLICE 5c — TWO HUMANS IN ONE WORLD.
//!
//! A second human seated at wizard colour 1 ([`World::add_human_seat`]
//! before the roster is seated) gets his own carpet record, his own
//! flight tier ([`Simulation::with_world`]) and his own input
//! ([`Simulation::step_seats`]); the walk dispatches each seat's carpet
//! at its own record's slot. One seat stays the single-player tick.
//!
//! Self-skips without baked mc2 data (game data is optional).

use mgc_formats::LevelPackage;
use mgc_sim::engine::features::{FeatureAssets, Planes};
use mgc_sim::engine::world::World;
use mgc_sim::ids::GameId;
use mgc_sim::mc2::rivals::Mc2RivalConfig;
use mgc_sim::{FlightInput, Hx, Simulation};
use std::path::Path;

fn load(level: &str, extra_seat: Option<u8>) -> Option<World> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../baked");
    let root = root.as_path();
    let bundle = mgc_formats::bundle::Bundle::load(&root.join("assets/mc2-night")).ok()?;
    let assets = FeatureAssets::parse(
        bundle.search.as_ref()?,
        bundle.build_tab.as_ref()?,
        bundle.build_dat.as_ref()?,
    )
    .ok()?
    .with_bldgprm(bundle.bldgprm.as_deref().unwrap_or_default());
    let assets = match bundle.mc2_extent_dims(&root.join("assets")) {
        Some(dims) => assets.with_mc2_sprite_ext(mgc_sim::mc2::derive_sprite_extents(&dims)),
        None => assets,
    };
    let assets = match bundle.spells.as_deref() {
        Some(sp) => assets.with_spells(sp).ok()?,
        None => assets,
    };
    let file = std::fs::File::open(root.join("mc2").join(format!("{level}.mgcl"))).ok()?;
    let pkg: LevelPackage = mgc_formats::mgcl::read(file).ok()?;
    let terrain = pkg.terrain.as_ref()?;
    let planes = Planes {
        height: terrain.height.clone(),
        tile_type: terrain.tile_type.clone(),
        shading: terrain.shading.clone()?,
        angle: terrain.angle.clone()?,
        ceiling: terrain.ceiling.clone().unwrap_or_default(),
    };
    let seed = pkg.gen_params.as_ref().map_or(0, |g| g.seed);
    let mut w = World::new_for_game(planes, &pkg.things.things, seed, assets, GameId::Mc2);
    w.set_mc2_night_shade(true);
    let (cfgs, count) = rival_configs(&pkg);
    if let Some(colour) = extra_seat {
        w.add_human_seat(colour).expect("a free colour");
    }
    w.seat_mc2_wizards(&cfgs, count);
    Some(w)
}

fn rival_configs(pkg: &LevelPackage) -> ([Option<Mc2RivalConfig>; 8], u16) {
    let mut out: [Option<Mc2RivalConfig>; 8] = Default::default();
    let (Some(wz), Some(h)) = (pkg.wizards.as_ref(), pkg.header.as_ref()) else {
        return (out, 1);
    };
    let count = h.number_of_players.clamp(1, 8) as u16;
    for (slot, cfg) in wz.wizards.iter().enumerate().take(8).skip(1) {
        let (Some(reflexes), Some(perception)) = (cfg.reflexes, cfg.perception) else {
            continue;
        };
        let mut start = [false; 26];
        let mut start_level = [0u8; 26];
        let mut blocked = [false; 26];
        for s in 0..26 {
            start[s] = cfg.starting_spells.get(s).copied().unwrap_or(0) != 0;
            start_level[s] = cfg
                .starting_spell_levels
                .get(s)
                .copied()
                .unwrap_or(0)
                .min(2);
            blocked[s] = cfg.blocked_spells.get(s).copied().unwrap_or(0) != 0;
        }
        out[slot] = Some(Mc2RivalConfig {
            aggression: cfg.aggression.clamp(0, 255) as u8,
            perception: perception.clamp(0, 255) as u8,
            reflexes: reflexes.clamp(0, 255) as u8,
            life: cfg.life.unwrap_or(0).max(0) as u16,
            castle_level: h.players[slot].max(0) as u8,
            start,
            start_level,
            blocked,
        });
    }
    (out, count)
}

const LEVEL: &str = "level-022";

fn forward() -> FlightInput {
    FlightInput {
        thrust: 1.0,
        mc1_move_byte: Some(1),
        ..FlightInput::default()
    }
}

/// Fly `ticks` ticks: seat 0 idle, seat 1 thrusting. Returns the sim.
fn fly(ticks: usize) -> Option<Simulation> {
    let w = load(LEVEL, Some(1))?;
    let mut sim = Simulation::with_world(w);
    for _ in 0..ticks {
        sim.step_seats(&[FlightInput::default(), forward()]);
    }
    Some(sim)
}

#[test]
fn a_second_human_seat_flies_its_own_carpet() {
    let Some(w) = load(LEVEL, Some(1)) else {
        eprintln!("skipping: no baked mc2 gamedata");
        return;
    };
    assert_eq!(w.seat_ixs().count(), 2, "two human seats");
    let mut sim = Simulation::with_world(w);
    let s1 = Hx(1);
    let start0 = (sim.carpet[Hx::FIRST].x, sim.carpet[Hx::FIRST].y);
    let start1 = (sim.carpet[s1].x, sim.carpet[s1].y);
    assert_ne!(start0, start1, "each seat stands on its own start marker");
    for _ in 0..120 {
        sim.step_seats(&[FlightInput::default(), forward()]);
    }
    let d = |a: (u16, u16), b: (u16, u16)| {
        let dx = a.0.wrapping_sub(b.0) as i16 as i32;
        let dy = a.1.wrapping_sub(b.1) as i16 as i32;
        ((dx * dx + dy * dy) as f64).sqrt()
    };
    let end0 = (sim.carpet[Hx::FIRST].x, sim.carpet[Hx::FIRST].y);
    let end1 = (sim.carpet[s1].x, sim.carpet[s1].y);
    assert!(
        d(start1, end1) > 1000.0,
        "seat 1 flew: {start1:?} -> {end1:?}"
    );
    assert!(
        d(start0, end0) < 256.0,
        "seat 0 held still: {start0:?} -> {end0:?}"
    );
}

#[test]
fn two_seats_are_deterministic() {
    let (Some(a), Some(b)) = (fly(200), fly(200)) else {
        eprintln!("skipping: no baked mc2 gamedata");
        return;
    };
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn one_seat_step_seats_is_step() {
    let (Some(wa), Some(wb)) = (load(LEVEL, None), load(LEVEL, None)) else {
        eprintln!("skipping: no baked mc2 gamedata");
        return;
    };
    let (mut a, mut b) = (Simulation::with_world(wa), Simulation::with_world(wb));
    for _ in 0..150 {
        a.step(&forward());
        b.step_seats(&[forward()]);
    }
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn a_two_seat_save_restores_every_seat() {
    let Some(a) = fly(150) else {
        eprintln!("skipping: no baked mc2 gamedata");
        return;
    };
    let bytes = a.snapshot();
    // A freshly built two-seat world takes the save, and re-saving it
    // reproduces the stream byte for byte (every seat's lanes landed).
    // (The live world itself is not the comparand: retail's load
    // empties the allocator's victim list, see `Gen::mc2_recycle`.)
    let restore = || {
        let mut s = Simulation::with_world(load(LEVEL, Some(1)).expect("level"));
        s.restore(&bytes).expect("restore");
        s
    };
    let (mut b, mut c) = (restore(), restore());
    assert_eq!(b.snapshot(), bytes, "round trip");
    for _ in 0..60 {
        b.step_seats(&[forward(), FlightInput::default()]);
        c.step_seats(&[forward(), FlightInput::default()]);
    }
    assert_eq!(b.state_hash(), c.state_hash(), "restores run in lockstep");
    // …and a one-seat world refuses it.
    let mut d = Simulation::with_world(load(LEVEL, None).expect("level"));
    assert!(d.restore(&bytes).is_err(), "seat count is identity");
}
