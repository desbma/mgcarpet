//! World-sprite impostor baking: the retail sprite bank tabulated as an
//! [`ImpostorPack`] (`impostors.json` + `impostors.bin` in every
//! graphics bundle) — the renderer's only source of world art.
//!
//! The bake (1) takes every visual family the game's sprite table can
//! name, (2) applies the bank's ASSET CORRECTIONS — retail data that
//! is plainly wrong art, fixed in the picture without touching the
//! sim — on a copy of the sprite index, (3) tabulates retail's view
//! dispatch over it ([`impostor::retail::build`]) and (4) repacks the
//! result into its own tight atlas. `sprites.json`/`.bin` stay the
//! uncorrected retail bank: they are the SIM's view of the art (sizes,
//! draw types), and the sim must keep reading retail's numbers.

use mgc_formats::Game;
use mgc_formats::bundle::SpriteIndex;
use mgc_formats::impostor::{self, ImpostorPack, ModelKey};

/// Every visual family the game's sprite table can name: the MC1/HW
/// sprite-stats rows `(sprite_base, draw_type)`, the MC2 sprite-param
/// rows (base = `word_0`, draw type = the bank's flags high byte — the
/// loader's boot fill, `MC2_SPRITE_DRAW_TYPE`, re-read per bank
/// exactly as the app's pose resolver reads it). Rows whose base is
/// missing from the bank are skipped (they never draw). Every
/// billboard the app makes resolves through one of these rows.
pub fn retail_keys(game: Game, index: &SpriteIndex) -> Vec<ModelKey> {
    match game {
        Game::MagicCarpet1 | Game::HiddenWorlds => mgc_sim::mc1::sprite_stats::SPRITE_STATS
            .iter()
            .map(|s| ModelKey {
                sprite_base: s.sprite_base,
                draw_type: s.draw_type,
            })
            .collect(),
        Game::MagicCarpet2 => mgc_sim::mc2::sprite_params::SPRITE_PARAMS
            .iter()
            .filter_map(|p| {
                let e = index.sprites.get(p.word_0 as usize)?;
                (e.width != 0 && e.height != 0).then_some(ModelKey {
                    sprite_base: p.word_0,
                    draw_type: (e.flags >> 8) as u8,
                })
            })
            .collect(),
    }
}

/// TMAPS 452 — the eighth (last) view of the MC2 troglodyte's
/// STANDING family 445..=452 (sprite-param row 336, draw type 17: it
/// is drawn for view sectors 7 and 8, the second one mirrored).
pub const MC2_TROGLODYTE_STAND_VIEW_7: usize = 452;

/// The bank's asset corrections, applied to a copy of its index (the
/// rects only — the texels are never touched).
///
/// **MC2: the troglodyte's uncropped canvas (TMAPS 452).** Every other
/// view of the standing troglodyte is a tight 50..58 x 72..74 cel;
/// view 7 is a 320x200 canvas (a whole VGA screen) whose only
/// non-transparent pixels are a 53x72 creature at (140,74). Retail's
/// rasteriser sizes a billboard as `height = projScale * rotSpeed_8 /
/// depth`, `width = height * sprite.width / sprite.height` (remc2
/// GameRenderOriginal.cpp:3770-72) — the whole canvas at the
/// creature's 450-unit height — so from that angle the troglodyte drew
/// at 72/200 of its size with its feet 27% of the way up the quad. It
/// only shows on the standing row (the (5,24) state 192 pose,
/// `sub_287B0`), i.e. on troglodytes that are not chasing anything,
/// which retail's 19-tile fog mostly kept out of sight
/// (player-reported 2026-09-30: "troglodytes occasionally visually
/// shrink to tiny size … a parked pose, or seeing them from a specific
/// angle"). The picture is cut to its bounding box. Shipped as a patch
/// option 2026-09-30, ruled an asset correction 2026-10-06 (player:
/// "no impact to the sim or doubt about what the correct way should
/// be").
///
/// A census of every bank (MC1 arctic/temperate, MC2 day/night/cave)
/// finds no second entry of the kind: the other mostly-empty cels are
/// frames of grow/sink families that share one canvas on purpose.
pub fn corrected_index(game: Game, index: &SpriteIndex, atlas: &[u8]) -> SpriteIndex {
    let mut out = index.clone();
    if game == Game::MagicCarpet2
        && let Some(e) = out.sprites.get_mut(MC2_TROGLODYTE_STAND_VIEW_7)
    {
        crop_authoring_canvas(e, atlas, index.atlas_width as usize);
    }
    out
}

/// Cut a single-frame 320x200 sprite entry down to the bounding box
/// of its non-transparent (non-zero) pixels. Anything else — another
/// size, an animated entry, an empty canvas — is left as it is, so a
/// bank that ships the cel properly cropped is untouched.
fn crop_authoring_canvas(e: &mut mgc_formats::bundle::SpriteEntry, atlas: &[u8], atlas_w: usize) {
    if (e.width, e.height) != (320, 200) || e.frames.len() != 1 {
        return;
    }
    let (fx, fy) = (e.frames[0].x as usize, e.frames[0].y as usize);
    let (w, h) = (e.width as usize, e.height as usize);
    let row = |r: usize| atlas.get((fy + r) * atlas_w + fx..(fy + r) * atlas_w + fx + w);
    let (mut r0, mut r1, mut c0, mut c1) = (h, 0, w, 0);
    for r in 0..h {
        let Some(px) = row(r) else { return };
        let Some(first) = px.iter().position(|p| *p != 0) else {
            continue;
        };
        let last = px.iter().rposition(|p| *p != 0).unwrap_or(first);
        r0 = r0.min(r);
        r1 = r;
        c0 = c0.min(first);
        c1 = c1.max(last);
    }
    if r0 > r1 || c0 > c1 {
        return; // fully transparent
    }
    e.frames[0].x += c0 as u32;
    e.frames[0].y += r0 as u32;
    e.width = (c1 - c0 + 1) as u16;
    e.height = (r1 - r0 + 1) as u16;
}

/// The bank's impostor pack: corrected, tabulated, repacked into an
/// atlas `atlas_width` wide (doubled until it fits `max_dim` tall).
pub fn bake(
    game: Game,
    index: &SpriteIndex,
    atlas: &[u8],
    atlas_width: u32,
    max_dim: u32,
) -> ImpostorPack {
    let keys = retail_keys(game, index);
    let corrected = corrected_index(game, index, atlas);
    let wide = impostor::retail::build(&corrected, atlas, &keys);
    let mut width = atlas_width;
    let mut pack = wide.repacked(width);
    while pack.index.atlas_height > max_dim && width < max_dim {
        width *= 2;
        pack = wide.repacked(width);
    }
    pack
}

#[cfg(test)]
mod tests {
    use super::*;
    use mgc_formats::bundle::{FramePos, SpriteEntry};

    /// The troglodyte correction: the standing troglodyte's 320x200
    /// authoring canvas is re-pointed at the creature inside it; MC1,
    /// every other entry and a properly cropped cel are left alone.
    #[test]
    fn the_troglodyte_canvas_is_cropped_to_its_creature() {
        let (aw, ah) = (400usize, 260usize);
        let mut atlas = vec![0u8; aw * ah];
        // The canvas sits at (20,30); the creature is a 53x72 block
        // at (140,74) inside it, as in the shipped bank.
        let (fx, fy) = (20usize, 30usize);
        for r in 74..146 {
            for c in 140..193 {
                atlas[(fy + r) * aw + fx + c] = 7;
            }
        }
        let entry = |id: u32, w: u16, h: u16| SpriteEntry {
            id,
            group: 445,
            width: w,
            height: h,
            flags: 0x1102,
            frames: vec![FramePos {
                x: fx as u32,
                y: fy as u32,
            }],
        };
        let mut sprites: Vec<SpriteEntry> = (0..453).map(|id| entry(id, 58, 72)).collect();
        sprites[452] = entry(452, 320, 200);
        sprites[451] = entry(451, 320, 200); // same shape, NOT the cel
        let index = SpriteIndex {
            atlas_width: aw as u32,
            atlas_height: ah as u32,
            sprites,
            pointer_base: 0,
        };
        let on = corrected_index(Game::MagicCarpet2, &index, &atlas);
        let e = &on.sprites[452];
        assert_eq!((e.width, e.height), (53, 72), "the creature's own box");
        assert_eq!(
            (e.frames[0].x, e.frames[0].y),
            (fx as u32 + 140, fy as u32 + 74),
            "same atlas pixels, only the rect moves"
        );
        assert_eq!(on.sprites[451], index.sprites[451], "only TMAPS 452");
        // MC1 keeps the bank's own index.
        assert_eq!(corrected_index(Game::MagicCarpet1, &index, &atlas), index);
        // A bank that ships the cel cropped is untouched.
        let mut fine = index.clone();
        fine.sprites[452] = entry(452, 58, 72);
        assert_eq!(corrected_index(Game::MagicCarpet2, &fine, &atlas), fine);
    }
}
