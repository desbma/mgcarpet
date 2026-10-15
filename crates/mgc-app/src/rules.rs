//! Netplay session rules — the config paths the HOST decides for every
//! seat. One table ([`PINS`]) drives the whole contract: what the host
//! sends at the handshake ([`collect`]), what the joiner takes over
//! ([`adopt`]), which options the session holds fixed in play (the
//! app's option guard and the menu lock) and which values are never
//! written back to the player's own `mgcarpet.json`.
//!
//! Everything not in the table is the player's own (bindings, mouse,
//! audio, the spell selector, look-only render options): it stays
//! his on every peer.

use crate::config::Config;
use serde_json::Value;

/// Why the session holds a path fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    /// The worlds fork without it: every peer must step the same sim
    /// (plus the pacing — a faster peer would only stall on turns).
    Sync,
    /// A view or an aid that shows one seat more than another.
    Fair,
}

/// The pinned config paths. A path covers its whole subtree.
pub const PINS: &[(&str, Pin)] = &[
    ("sim", Pin::Sync),
    ("gameplay.patches", Pin::Sync),
    ("gameplay.cheat", Pin::Sync),
    ("controls.models", Pin::Sync),
    ("dev.plausible_spellbook", Pin::Sync),
    ("render.preference.fog_distance", Pin::Fair),
    ("render.preference.rival_tags", Pin::Fair),
    ("render.enhancement.map_owned_buildings", Pin::Fair),
    ("render.enhancement.expose_jar_spells", Pin::Fair),
    ("render.enhancement.map_extent_fog", Pin::Fair),
    ("render.enhancement.map_beyond_sight_areas", Pin::Fair),
    ("render.debug.health_bars", Pin::Fair),
    ("render.debug.autoaim_hints", Pin::Fair),
    ("render.debug.map_trigger_areas", Pin::Fair),
];

/// The pin holding `path` (a dotted config path), if any.
pub fn pin_of(path: &str) -> Option<Pin> {
    PINS.iter()
        .find(|(p, _)| {
            path == *p
                || path
                    .strip_prefix(p)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
        .map(|&(_, pin)| pin)
}

fn get<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, seg| v.get(seg))
}

fn get_mut<'a>(v: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    path.split('.').try_fold(v, |v, seg| v.get_mut(seg))
}

/// The session's rules from the host's config: `{path: value}` for
/// every pin.
pub fn collect(cfg: &Config) -> Value {
    let full = serde_json::to_value(cfg).expect("config serializes");
    Value::Object(
        PINS.iter()
            .map(|(p, _)| (p.to_string(), get(&full, p).cloned().unwrap_or(Value::Null)))
            .collect(),
    )
}

/// Take the host's rules over: every pinned path set from `rules`, the
/// rest of `cfg` untouched. A pin the host did not send is an error
/// (the build check makes it a corrupted Welcome).
pub fn adopt(cfg: &mut Config, rules: &Value) -> Result<(), String> {
    let mut full = serde_json::to_value(&*cfg).expect("config serializes");
    for (p, _) in PINS {
        let theirs = rules
            .get(*p)
            .ok_or_else(|| format!("net: the host's rules lack {p}"))?;
        let ours = get_mut(&mut full, p).ok_or_else(|| format!("net: no config path {p}"))?;
        *ours = theirs.clone();
    }
    *cfg = serde_json::from_value(full).map_err(|e| format!("net: the host's rules: {e}"))?;
    Ok(())
}

/// The pins where `rules` overrides `cfg` — what the joiner is told
/// the host's rules changed for him.
pub fn overridden(cfg: &Config, rules: &Value) -> Vec<&'static str> {
    let ours = collect(cfg);
    PINS.iter()
        .map(|&(p, _)| p)
        .filter(|p| ours.get(*p) != rules.get(*p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_is_a_real_config_path() {
        let full = serde_json::to_value(Config::default()).unwrap();
        for (p, _) in PINS {
            assert!(get(&full, p).is_some(), "{p} is not a config path");
        }
    }

    #[test]
    fn a_pin_covers_its_subtree_only() {
        assert_eq!(pin_of("sim.options.game_speed"), Some(Pin::Sync));
        assert_eq!(pin_of("gameplay.patches.no_spell_loss"), Some(Pin::Sync));
        assert_eq!(pin_of("render.preference.fog_distance"), Some(Pin::Fair));
        assert_eq!(pin_of("render.preference.fog_distance_x"), None);
        assert_eq!(pin_of("gameplay.enhancement.spell_selector"), None);
        assert_eq!(pin_of("controls.preferences.bindings"), None);
        assert_eq!(pin_of("render.debug.fps"), None);
    }

    /// The joiner ends with the host's pins and keeps his own rest.
    #[test]
    fn adopt_takes_the_pins_and_keeps_the_rest() {
        let mut host = Config::default();
        host.render.preference.fog_distance = 40;
        host.gameplay.cheat.invincible = true;
        host.audio.music_volume = 0.1;
        let mut joiner = Config::default();
        joiner.render.preference.fog_distance = 70;
        joiner.render.debug.health_bars = true;
        joiner.audio.music_volume = 0.9;
        joiner.controls.preferences.invert_y = true;
        adopt(&mut joiner, &collect(&host)).unwrap();
        assert_eq!(joiner.render.preference.fog_distance, 40);
        assert!(joiner.gameplay.cheat.invincible);
        assert!(!joiner.render.debug.health_bars);
        assert_eq!(joiner.audio.music_volume, 0.9);
        assert!(joiner.controls.preferences.invert_y);
        assert_eq!(collect(&joiner), collect(&host));
        assert!(overridden(&joiner, &collect(&host)).is_empty());
    }

    #[test]
    fn overridden_names_the_differing_pins() {
        let host = Config::default();
        let mut joiner = Config::default();
        joiner.render.preference.fog_distance = 70;
        joiner.render.debug.health_bars = true;
        joiner.audio.music_volume = 0.9;
        assert_eq!(
            overridden(&joiner, &collect(&host)),
            ["render.preference.fog_distance", "render.debug.health_bars"]
        );
    }

    #[test]
    fn a_missing_pin_is_refused() {
        let mut rules = collect(&Config::default());
        rules.as_object_mut().unwrap().remove("gameplay.cheat");
        assert!(adopt(&mut Config::default(), &rules).is_err());
    }
}
