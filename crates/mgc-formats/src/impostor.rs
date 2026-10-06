//! Impostor packs: the world-sprite drawing model.
//!
//! An impostor is a picture-based stand-in for a 3D object: a set of
//! pre-rendered VIEWS, picked per frame by where the camera stands
//! relative to the object, plus the object's ANIMATION. Every world
//! sprite the renderer draws goes through one; retail's art is the
//! most basic instance (16 yaw sectors folded onto 1/3/5/8/16
//! pictures, one elevation ring, the entity's animation byte and the
//! FLC loop), and richer packs (more views, pitch rings, baked from
//! meshes) are the same model with bigger axes. The normative
//! description is `docs/FORMAT.md` "Impostor packs".
//!
//! A pack is self-contained: an index ([`ImpostorIndex`], JSON) over
//! its own atlas ([`ImpostorPack::pixels`]). Lookup runs
//!
//! ```text
//! entity ──key──▶ model ──(pose, pitch ring, yaw sector)──▶ cell
//!        cell ──▶ clip (+ mirror) ──(clock / seek)──▶ image ──▶ atlas rect
//! ```
//!
//! The KEY is retail's visual family `(sprite_base, draw_type)` — what
//! the sim's sprite tables already name — so a replacement pack slots
//! in under the sim without the sim knowing. Sizing stays the sim's:
//! the quad is the entity's world height tall and the image's pixel
//! aspect wide, anchored at the image's [`ImpostorImage::anchor`].

use serde::{Deserialize, Serialize};

use crate::bundle::SpriteIndex;

/// Current [`ImpostorIndex::version`].
pub const IMPOSTOR_VERSION: u32 = 1;

/// Engine angle steps per turn: yaw sectors are quantized on retail's
/// 11-bit angles, so a 16-sector model reproduces the original's
/// `(rel >> 7) & 15` exactly.
const ANGLE_STEPS: f32 = 2048.0;

/// A visual family: what one entity draws as. Retail's own addressing
/// — the sprite id of the family's first picture and the original's
/// view-selection mode (sprite flags high byte / stats-table draw
/// type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ModelKey {
    pub sprite_base: u16,
    pub draw_type: u8,
}

/// How the atlas texels are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    /// One byte per texel, a palette index resolved through the level's
    /// palette and glow set at load (index 0 = transparent). Retail art.
    Indexed8,
    /// Four bytes per texel, sRGB RGBA (alpha 0 = transparent, other
    /// alphas are treated as opaque), followed by a second plane of one
    /// MATERIAL byte per texel (bit 0 = emissive: resists darkness,
    /// not fog — the renderer's `grade::MAT_EMISSIVE`).
    Rgba8,
}

impl PixelFormat {
    /// Bytes [`ImpostorPack::pixels`] must hold for a `w × h` atlas.
    pub fn byte_len(self, w: u32, h: u32) -> usize {
        let n = w as usize * h as usize;
        match self {
            PixelFormat::Indexed8 => n,
            PixelFormat::Rgba8 => n * 5,
        }
    }
}

/// One picture in the atlas.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ImpostorImage {
    /// Atlas rect, texels.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Where the entity's position (its feet) sits in the picture, as
    /// fractions of the drawn quad: x from the left edge, y UP from the
    /// bottom edge. Retail pictures are feet-centre: `[0.5, 0.0]`. A
    /// mirrored cell mirrors the anchor with the picture.
    pub anchor: [f32; 2],
}

/// How a clip's images advance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Playback {
    /// Always the first image.
    Still,
    /// Retail's FLC stream: image 0 is the base picture and image `k`
    /// the canvas after the k-th delta; free-running it loops images
    /// `1..len` one per animation turn, all streams in lockstep, and
    /// never returns to image 0 (the last delta is the ring back to
    /// the base, and retail wraps `FrameIndex > CountOfFrames` straight
    /// to delta 1). An entity may SEEK it to an explicit image
    /// (`Billboard::flc_frame` — the sim-positioned one-shots).
    FlcLoop,
}

/// A run of consecutive [`ImpostorIndex::images`] with a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clip {
    pub first: u32,
    pub len: u32,
    pub playback: Playback,
}

impl Clip {
    /// Which of the clip's images is on screen: `seek` = an explicit
    /// image the entity asks for, `turn` = the animation clock.
    pub fn image(&self, seek: Option<u8>, turn: usize) -> u32 {
        match self.playback {
            Playback::Still => self.first,
            Playback::FlcLoop => self.first + flc_image(self.len as usize, seek, turn) as u32,
        }
    }
}

/// Which image of an FLC stream of `frames` images (base + one per
/// delta) is on screen — see [`Playback::FlcLoop`].
pub fn flc_image(frames: usize, seek: Option<u8>, turn: usize) -> usize {
    let Some(count) = frames.checked_sub(1).filter(|c| *c > 0) else {
        return 0;
    };
    match seek {
        Some(k) => (k as usize).min(count),
        None => 1 + turn % count,
    }
}

/// One cell of a model's view table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    /// Index into [`ImpostorIndex::clips`]; `None` draws nothing (a
    /// retail family member that is missing or known-corrupt).
    pub clip: Option<u32>,
    /// Draw the picture flipped left-right (retail's back-half folds).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
}

/// A visual family's view table.
///
/// `cells` is indexed `(pose · pitches.len() + ring) · yaw_sectors +
/// sector`:
/// - **pose** — the entity's animation byte (retail's frame offset for
///   the animation draw types); a byte past the last pose shows the
///   last.
/// - **ring** — the elevation ring nearest the camera's elevation as
///   seen from the entity, `pitches` in degrees (0 = level, positive =
///   camera above). Retail has one ring.
/// - **sector** — relative yaw (entity facing − camera yaw) on the
///   engine's 2048-step circle, `floor(steps · yaw_sectors / 2048)`:
///   sector 0 starts AT relative yaw 0 (retail's convention), so a
///   view baked for sector `s` should be rendered at its middle,
///   `(s + ½) · 360° / yaw_sectors`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpostorModel {
    pub key: ModelKey,
    pub yaw_sectors: u16,
    pub pitches: Vec<f32>,
    pub poses: u16,
    pub cells: Vec<Cell>,
}

impl ImpostorModel {
    /// The yaw sector for a relative yaw in radians (any range).
    pub fn sector(&self, rel_yaw: f32) -> usize {
        let rel = rel_yaw.rem_euclid(std::f32::consts::TAU);
        let steps = (rel * (ANGLE_STEPS / std::f32::consts::TAU)) as i64;
        let n = self.yaw_sectors.max(1) as i64;
        ((steps * n) >> 11).clamp(0, n - 1) as usize
    }

    /// The elevation ring nearest `elevation_deg`.
    pub fn ring(&self, elevation_deg: f32) -> usize {
        self.pitches
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1 - elevation_deg)
                    .abs()
                    .total_cmp(&(b.1 - elevation_deg).abs())
            })
            .map_or(0, |(i, _)| i)
    }

    /// The cell for a pose, relative yaw (radians) and camera
    /// elevation (degrees).
    pub fn cell(&self, pose: u8, rel_yaw: f32, elevation_deg: f32) -> Option<Cell> {
        let pose = (pose as usize).min(self.poses.max(1) as usize - 1);
        let rings = self.pitches.len().max(1);
        let ring = self.ring(elevation_deg);
        let at = (pose * rings + ring) * self.yaw_sectors.max(1) as usize + self.sector(rel_yaw);
        self.cells.get(at).copied()
    }
}

/// `impostors.json`: a pack's index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpostorIndex {
    pub version: u32,
    pub atlas_width: u32,
    pub atlas_height: u32,
    pub pixels: PixelFormat,
    pub images: Vec<ImpostorImage>,
    pub clips: Vec<Clip>,
    /// Sorted by key (lookup is a binary search).
    pub models: Vec<ImpostorModel>,
}

/// What a view lookup resolved to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub image: ImpostorImage,
    pub mirror: bool,
}

impl ImpostorIndex {
    /// The model drawing `key`, if the pack has one.
    pub fn model(&self, key: ModelKey) -> Option<&ImpostorModel> {
        self.models
            .binary_search_by_key(&key, |m| m.key)
            .ok()
            .map(|i| &self.models[i])
    }

    /// Resolve one entity's picture: `pose` = its animation byte,
    /// `rel_yaw` = its facing minus the camera yaw (radians),
    /// `elevation_deg` = the camera's elevation seen from it, `seek` /
    /// `turn` = the clip clock ([`Clip::image`]).
    pub fn view(
        &self,
        key: ModelKey,
        pose: u8,
        rel_yaw: f32,
        elevation_deg: f32,
        seek: Option<u8>,
        turn: usize,
    ) -> Option<View> {
        let cell = self.model(key)?.cell(pose, rel_yaw, elevation_deg)?;
        let clip = self.clips.get(cell.clip? as usize)?;
        let image = *self.images.get(clip.image(seek, turn) as usize)?;
        Some(View {
            image,
            mirror: cell.mirror,
        })
    }

    /// Structural check: every reference in range, every axis non-empty,
    /// every image inside the atlas, models sorted and unique.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != IMPOSTOR_VERSION {
            return Err(format!("impostor version {} unsupported", self.version));
        }
        for (i, im) in self.images.iter().enumerate() {
            if im.x + im.w > self.atlas_width || im.y + im.h > self.atlas_height {
                return Err(format!("image {i} outside the atlas"));
            }
        }
        for (i, c) in self.clips.iter().enumerate() {
            if c.len == 0 || (c.first + c.len) as usize > self.images.len() {
                return Err(format!("clip {i} out of range"));
            }
        }
        for (i, m) in self.models.iter().enumerate() {
            if i > 0 && self.models[i - 1].key >= m.key {
                return Err(format!("models not sorted/unique at {:?}", m.key));
            }
            if m.yaw_sectors == 0 || m.poses == 0 || m.pitches.is_empty() {
                return Err(format!("model {:?}: empty axis", m.key));
            }
            let want = m.poses as usize * m.pitches.len() * m.yaw_sectors as usize;
            if m.cells.len() != want {
                return Err(format!(
                    "model {:?}: {} cells, axes want {want}",
                    m.key,
                    m.cells.len()
                ));
            }
            if m.cells
                .iter()
                .any(|c| c.clip.is_some_and(|k| k as usize >= self.clips.len()))
            {
                return Err(format!("model {:?}: clip out of range", m.key));
            }
        }
        Ok(())
    }
}

/// An index plus its atlas texels ([`PixelFormat::byte_len`] bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct ImpostorPack {
    pub index: ImpostorIndex,
    pub pixels: Vec<u8>,
}

impl ImpostorPack {
    /// File names inside a pack directory.
    pub const INDEX_FILE: &str = "impostors.json";
    pub const PIXELS_FILE: &str = "impostors.bin";

    pub fn validate(&self) -> Result<(), String> {
        self.index.validate()?;
        let want = self
            .index
            .pixels
            .byte_len(self.index.atlas_width, self.index.atlas_height);
        if self.pixels.len() != want {
            return Err(format!(
                "atlas holds {} bytes, want {want}",
                self.pixels.len()
            ));
        }
        Ok(())
    }

    /// The same pack over a fresh, tight atlas `atlas_width` texels
    /// wide: only the texels some image shows are kept, an atlas rect
    /// shared by several images is stored once, and the rects are
    /// shelf-packed tallest first (ties in first-use order —
    /// deterministic). Lookups resolve to the same pictures; only the
    /// atlas coordinates move.
    ///
    /// Panics if an image is wider than `atlas_width`.
    pub fn repacked(&self, atlas_width: u32) -> ImpostorPack {
        use std::collections::HashMap;
        let src = &self.index;
        let (bpp, plane) = match src.pixels {
            PixelFormat::Indexed8 => (1usize, false),
            PixelFormat::Rgba8 => (4, true),
        };
        // Unique source rects, in first-use order.
        let mut rects: Vec<(u32, u32, u32, u32)> = Vec::new();
        let mut slot_of: HashMap<(u32, u32, u32, u32), usize> = HashMap::new();
        let image_slot: Vec<usize> = src
            .images
            .iter()
            .map(|im| {
                let r = (im.x, im.y, im.w, im.h);
                *slot_of.entry(r).or_insert_with(|| {
                    rects.push(r);
                    rects.len() - 1
                })
            })
            .collect();
        let mut order: Vec<usize> = (0..rects.len()).collect();
        order.sort_by_key(|&i| (std::cmp::Reverse(rects[i].3), i));
        // Shelf packing.
        struct Shelf {
            y: u32,
            h: u32,
            x: u32,
        }
        let mut shelves: Vec<Shelf> = Vec::new();
        let mut height = 0u32;
        let mut placed = vec![(0u32, 0u32); rects.len()];
        for &i in &order {
            let (_, _, w, h) = rects[i];
            assert!(
                w <= atlas_width,
                "impostor image {w} wide > atlas {atlas_width}"
            );
            let shelf = match shelves
                .iter_mut()
                .find(|s| s.h >= h && s.x + w <= atlas_width)
            {
                Some(s) => s,
                None => {
                    shelves.push(Shelf { y: height, h, x: 0 });
                    height += h;
                    shelves.last_mut().expect("just pushed")
                }
            };
            placed[i] = (shelf.x, shelf.y);
            shelf.x += w;
        }
        // Copy the texels (and the material plane).
        let (sw, sh) = (src.atlas_width as usize, src.atlas_height as usize);
        let (dw, dh) = (atlas_width as usize, height as usize);
        let mut pixels = vec![0u8; PixelFormat::byte_len(src.pixels, atlas_width, height)];
        for (i, &(x, y, w, h)) in rects.iter().enumerate() {
            let (px, py) = placed[i];
            for row in 0..h as usize {
                let s = ((y as usize + row) * sw + x as usize) * bpp;
                let d = ((py as usize + row) * dw + px as usize) * bpp;
                let n = w as usize * bpp;
                pixels[d..d + n].copy_from_slice(&self.pixels[s..s + n]);
                if plane {
                    let (s0, d0) = (sw * sh * 4, dw * dh * 4);
                    let (s, d) = (s / 4, d / 4);
                    let n = w as usize;
                    pixels[d0 + d..d0 + d + n].copy_from_slice(&self.pixels[s0 + s..s0 + s + n]);
                }
            }
        }
        let images = src
            .images
            .iter()
            .zip(&image_slot)
            .map(|(im, &slot)| ImpostorImage {
                x: placed[slot].0,
                y: placed[slot].1,
                ..*im
            })
            .collect();
        ImpostorPack {
            index: ImpostorIndex {
                atlas_width,
                atlas_height: height,
                images,
                ..src.clone()
            },
            pixels,
        }
    }

    /// Write `impostors.json` + `impostors.bin` into `dir`.
    pub fn write_dir(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_vec(&self.index).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(Self::INDEX_FILE), json)?;
        std::fs::write(dir.join(Self::PIXELS_FILE), &self.pixels)
    }

    /// Read and validate a pack written by [`Self::write_dir`].
    pub fn read_dir(dir: &std::path::Path) -> std::io::Result<Self> {
        let index: ImpostorIndex =
            serde_json::from_slice(&std::fs::read(dir.join(Self::INDEX_FILE))?)
                .map_err(std::io::Error::other)?;
        let pack = Self {
            index,
            pixels: std::fs::read(dir.join(Self::PIXELS_FILE))?,
        };
        pack.validate().map_err(std::io::Error::other)?;
        Ok(pack)
    }
}

/// Retail art as impostors: the original's per-sprite view dispatch
/// (remc1 `DrawSprite3D_2F170`, remc2 GameRenderOriginal `LABEL_26`)
/// tabulated once per visual family.
pub mod retail {
    use super::*;

    /// 16 view sectors folded to 5 pictures (draw type 19,
    /// `byte_906E8`); the back half mirrored.
    pub const VIEW_FOLD_5: [u8; 16] = [0, 1, 1, 2, 2, 3, 3, 4, 4, 3, 3, 2, 2, 1, 1, 0];
    /// 16 view sectors folded to 3 pictures (draw type 20,
    /// `byte_906F8`); the back half mirrored.
    pub const VIEW_FOLD_3: [u8; 16] = [0, 0, 0, 1, 1, 1, 2, 2, 2, 2, 2, 1, 1, 1, 0, 0];

    /// Retail's feet-centre anchor.
    pub const ANCHOR: [f32; 2] = [0.5, 0.0];

    /// Animation poses of a draw type: the 2..=16 animation types carry
    /// their frame count in the type itself, MC2's 22..=36 band the
    /// same count offset by 20; everything else has one (the sim's
    /// `mc1::mobs::FRAME_COUNTS`).
    pub fn pose_count(draw_type: u8) -> u16 {
        match draw_type {
            2..=16 => draw_type as u16,
            22..=36 => draw_type as u16 - 20,
            _ => 1,
        }
    }

    /// Picture offset from the family base + mirror for a draw type in
    /// one of retail's 16 view sectors (view types) or at one pose
    /// (animation types).
    pub fn member(draw_type: u8, sector: u16, pose: u16) -> (u16, bool) {
        match draw_type {
            17 => {
                if sector < 8 {
                    (sector, false)
                } else {
                    (15 - sector, true)
                }
            }
            18 => (sector, false),
            19 => (VIEW_FOLD_5[sector as usize] as u16, sector >= 8),
            20 => (VIEW_FOLD_3[sector as usize] as u16, sector >= 8),
            2..=16 | 22..=36 => (pose, false),
            // 0/1/21 single view, and anything unknown: the base.
            _ => (0, false),
        }
    }

    /// Whether a draw type picks its picture by view sector.
    pub fn is_view_type(draw_type: u8) -> bool {
        matches!(draw_type, 17..=20)
    }

    /// Tabulate `keys` over a retail sprite bank (`index` + its 8-bit
    /// `atlas`). The pack shares the bank's texels verbatim; each
    /// sprite becomes one clip (its FLC stream, or its still), shared
    /// by every family that draws it.
    pub fn build(index: &SpriteIndex, atlas: &[u8], keys: &[ModelKey]) -> ImpostorPack {
        let mut keys = keys.to_vec();
        keys.sort();
        keys.dedup();
        let mut images = Vec::new();
        let mut clips = Vec::new();
        let mut clip_of = std::collections::HashMap::<usize, Option<u32>>::new();
        let mut clip_for = |id: usize| -> Option<u32> {
            *clip_of.entry(id).or_insert_with(|| {
                let entry = index.sprites.get(id)?;
                if entry.frames.is_empty() {
                    return None; // known-corrupt / placeholder source entry
                }
                let animated = entry.flags & 1 != 0;
                let frames = if animated {
                    &entry.frames[..]
                } else {
                    &entry.frames[..1]
                };
                let first = images.len() as u32;
                images.extend(frames.iter().map(|f| ImpostorImage {
                    x: f.x,
                    y: f.y,
                    w: entry.width as u32,
                    h: entry.height as u32,
                    anchor: ANCHOR,
                }));
                clips.push(Clip {
                    first,
                    len: frames.len() as u32,
                    playback: if animated {
                        Playback::FlcLoop
                    } else {
                        Playback::Still
                    },
                });
                Some(clips.len() as u32 - 1)
            })
        };
        let models = keys
            .iter()
            .map(|&key| {
                let poses = pose_count(key.draw_type);
                let yaw_sectors = if is_view_type(key.draw_type) { 16 } else { 1 };
                let mut cells = Vec::with_capacity((poses * yaw_sectors) as usize);
                for pose in 0..poses {
                    for sector in 0..yaw_sectors {
                        let (offset, mirror) = member(key.draw_type, sector, pose);
                        cells.push(Cell {
                            clip: clip_for(key.sprite_base as usize + offset as usize),
                            mirror,
                        });
                    }
                }
                ImpostorModel {
                    key,
                    yaw_sectors,
                    pitches: vec![0.0],
                    poses,
                    cells,
                }
            })
            .collect();
        ImpostorPack {
            index: ImpostorIndex {
                version: IMPOSTOR_VERSION,
                atlas_width: index.atlas_width,
                atlas_height: index.atlas_height,
                pixels: PixelFormat::Indexed8,
                images,
                clips,
                models,
            },
            pixels: atlas.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{FramePos, SpriteEntry};
    use std::f32::consts::TAU;

    /// The FLC loop: free-running it steps frames 1..=24 and wraps
    /// straight to 1 — retail's `FrameIndex > CountOfFrames` reset
    /// (remc2 `sub_72350`) — and a sim-seeked stream shows exactly the
    /// image it names.
    #[test]
    fn flc_streams_loop_their_deltas_and_honour_a_seek() {
        let run: Vec<usize> = (0..50).map(|t| flc_image(25, None, t)).collect();
        let want: Vec<usize> = (0..50).map(|t| 1 + t % 24).collect();
        assert_eq!(run, want);
        assert!(!run.contains(&0), "the base is never part of the loop");
        assert_eq!(flc_image(25, Some(0), 7), 0, "a seek can show the base");
        assert_eq!(flc_image(25, Some(24), 7), 24);
        assert_eq!(flc_image(25, Some(200), 7), 24, "clamped to the stream");
        // Frame-less and single-frame entries have nothing to step.
        assert_eq!(flc_image(0, None, 3), 0);
        assert_eq!(flc_image(1, Some(5), 3), 0);
    }

    /// The pose counts are the sim's `FRAME_COUNTS` table.
    #[test]
    fn retail_pose_counts_are_the_frame_count_table() {
        const FRAME_COUNTS: [u16; 37] = [
            1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, //
            1, 1, 1, 1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
        ];
        for (dt, &n) in FRAME_COUNTS.iter().enumerate() {
            assert_eq!(retail::pose_count(dt as u8), n, "draw type {dt}");
        }
    }

    /// A 16-sector model quantizes exactly like the engine's
    /// `(((entityYaw - camYaw) >> 3) & 0xF0) >> 4` on 11-bit angles.
    #[test]
    fn sixteen_sectors_are_the_engine_quantization() {
        let m = ImpostorModel {
            key: ModelKey {
                sprite_base: 0,
                draw_type: 18,
            },
            yaw_sectors: 16,
            pitches: vec![0.0],
            poses: 1,
            cells: vec![],
        };
        for k in 0..20000 {
            let rel = k as f32 * (2.0 * TAU / 20000.0) - 0.5;
            let r = rel.rem_euclid(TAU);
            let want = ((r * (2048.0 / TAU)) as i32 >> 7).clamp(0, 15) as usize;
            assert_eq!(m.sector(rel), want, "rel {rel}");
        }
    }

    fn bank() -> (SpriteIndex, Vec<u8>) {
        // 40 sprites, 2×3 each in a row; sprite 5 broken, sprite 30
        // an FLC stream of 4 images.
        let mut sprites = Vec::new();
        for id in 0..40u32 {
            let frames = match id {
                5 => vec![],
                30 => (0..4)
                    .map(|k| FramePos {
                        x: id * 2,
                        y: 3 * k,
                    })
                    .collect(),
                _ => vec![FramePos { x: id * 2, y: 0 }],
            };
            sprites.push(SpriteEntry {
                id,
                group: id,
                width: 2,
                height: 3,
                flags: if id == 30 { 1 } else { 0 },
                frames,
            });
        }
        let index = SpriteIndex {
            atlas_width: 80,
            atlas_height: 12,
            sprites,
            pointer_base: 0,
        };
        let atlas = (0..80 * 12).map(|i| (i % 251) as u8).collect();
        (index, atlas)
    }

    /// The retail tabulation picks the same picture the original's
    /// draw dispatch would, for every draw type, sector and pose.
    #[test]
    fn retail_pack_reproduces_the_draw_dispatch() {
        let (index, atlas) = bank();
        let keys: Vec<ModelKey> = (0..=36u8)
            .map(|dt| ModelKey {
                sprite_base: 2,
                draw_type: dt,
            })
            .chain([ModelKey {
                sprite_base: 30,
                draw_type: 0,
            }])
            .collect();
        let pack = retail::build(&index, &atlas, &keys);
        pack.validate().unwrap();
        for key in &keys {
            for pose in 0..retail::pose_count(key.draw_type) as u8 {
                for s in 0..16u16 {
                    let rel = (s as f32 + 0.5) * TAU / 16.0;
                    let (off, mirror) = retail::member(key.draw_type, s, pose as u16);
                    let id = (key.sprite_base + off) as usize;
                    let got = pack.index.view(*key, pose, rel, 30.0, None, 2);
                    if id == 5 {
                        assert_eq!(got, None);
                        continue;
                    }
                    let want_frame = if id == 30 { flc_image(4, None, 2) } else { 0 };
                    let v = got.unwrap_or_else(|| panic!("{key:?} s{s} p{pose}"));
                    assert_eq!(
                        (v.image.x, v.image.y, v.mirror),
                        (id as u32 * 2, 3 * want_frame as u32, mirror),
                        "{key:?} sector {s} pose {pose}"
                    );
                }
            }
        }
        // A seek reaches the stream's base.
        let flc = ModelKey {
            sprite_base: 30,
            draw_type: 0,
        };
        assert_eq!(
            pack.index
                .view(flc, 0, 0.0, 0.0, Some(0), 9)
                .unwrap()
                .image
                .y,
            0
        );
        // Unknown families draw nothing.
        assert!(
            pack.index
                .model(ModelKey {
                    sprite_base: 3,
                    draw_type: 17
                })
                .is_none()
        );
    }

    /// A pack survives its own files byte for byte.
    #[test]
    fn packs_round_trip_through_a_directory() {
        let (index, atlas) = bank();
        let pack = retail::build(
            &index,
            &atlas,
            &[ModelKey {
                sprite_base: 0,
                draw_type: 17,
            }],
        );
        let dir = std::env::temp_dir().join(format!("mgc-impostor-rt-{}", std::process::id()));
        pack.write_dir(&dir).unwrap();
        let back = ImpostorPack::read_dir(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(back, pack);
    }

    /// The texels a view shows, row-major (+ the material plane for
    /// RGBA packs).
    fn texels(pack: &ImpostorPack, im: &ImpostorImage) -> Vec<u8> {
        let ix = &pack.index;
        let (w, h) = (ix.atlas_width as usize, ix.atlas_height as usize);
        let mut out = Vec::new();
        for r in 0..im.h as usize {
            for c in 0..im.w as usize {
                let p = (im.y as usize + r) * w + im.x as usize + c;
                match ix.pixels {
                    PixelFormat::Indexed8 => out.push(pack.pixels[p]),
                    PixelFormat::Rgba8 => {
                        out.extend_from_slice(&pack.pixels[p * 4..p * 4 + 4]);
                        out.push(pack.pixels[w * h * 4 + p]);
                    }
                }
            }
        }
        out
    }

    /// Repacking keeps every lookup's picture, shrinks the atlas to
    /// the texels in use, and stores a shared rect once — for both
    /// pixel formats.
    #[test]
    fn repacking_keeps_every_picture() {
        let (index, atlas) = bank();
        let keys: Vec<ModelKey> = [(2, 17), (2, 18), (0, 19), (30, 0), (10, 5)]
            .iter()
            .map(|&(b, d)| ModelKey {
                sprite_base: b,
                draw_type: d,
            })
            .collect();
        let indexed = retail::build(&index, &atlas, &keys);
        let mut rgba = indexed.clone();
        rgba.index.pixels = PixelFormat::Rgba8;
        rgba.pixels = indexed
            .pixels
            .iter()
            .flat_map(|&q| [q, q ^ 0x55, q.wrapping_mul(3), 255])
            .chain(indexed.pixels.iter().map(|&q| q & 1))
            .collect();
        for pack in [indexed, rgba] {
            let small = pack.repacked(16);
            small.validate().unwrap();
            assert!(small.pixels.len() < pack.pixels.len());
            for key in &keys {
                for pose in 0..retail::pose_count(key.draw_type) as u8 {
                    for s in 0..16 {
                        let rel = (s as f32 + 0.5) * TAU / 16.0;
                        for turn in 0..4 {
                            let a = pack.index.view(*key, pose, rel, 0.0, None, turn);
                            let b = small.index.view(*key, pose, rel, 0.0, None, turn);
                            assert_eq!(a.is_some(), b.is_some());
                            if let (Some(a), Some(b)) = (a, b) {
                                assert_eq!(a.mirror, b.mirror);
                                assert_eq!((a.image.w, a.image.h), (b.image.w, b.image.h));
                                assert_eq!(texels(&pack, &a.image), texels(&small, &b.image));
                            }
                        }
                    }
                }
            }
        }
    }

    /// Pitch rings pick the nearest elevation.
    #[test]
    fn rings_pick_the_nearest_elevation() {
        let m = ImpostorModel {
            key: ModelKey {
                sprite_base: 0,
                draw_type: 0,
            },
            yaw_sectors: 1,
            pitches: vec![-45.0, 0.0, 45.0],
            poses: 1,
            cells: vec![],
        };
        assert_eq!(m.ring(-80.0), 0);
        assert_eq!(m.ring(10.0), 1);
        assert_eq!(m.ring(30.0), 2);
    }
}
