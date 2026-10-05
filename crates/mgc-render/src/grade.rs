//! The world colour path in RGB: retail's 8-bit shade remap
//! (`palette[shade_lut[shade][index]]`) re-expressed as a function of
//! a texel's COLOUR plus one material bit, so art that never had a
//! palette index (upscales, impostors) shades exactly like retail art.
//!
//! Per shade row the remap is fitted as a 3x3+offset colour matrix
//! (tint and desaturation are channel-mixing — a brightness/fog curve
//! tinted day blue and night beige, player review 2026-10-05) plus a
//! 33³ residual grid splatted from what retail actually does to every
//! palette colour (holds hues the matrix cannot: night's pure red
//! drifted orange). Both are baked into one table per row.
//!
//! ⭐ GLOW IS INDEX-ONLY INFORMATION. The night/cave palettes carry
//! duplicate colours with different fates — MC2 night index 224 and
//! 176 are both pure white, one fades to black, the other glows — so
//! no function of colour alone can shade both. Each texel therefore
//! carries an EMISSIVE bit (from its palette index, for retail art)
//! that selects between two tables: one fitted on the normal colours
//! only, one on the glowing ones. Glow is classified from the remap
//! itself (how much of its own colour an index keeps against the
//! normal fit). MC1 and MC2 day classify (near) nothing as glowing.
//!
//! Layout ([`GRADE_W`] x [`GRADE_H`] x 2 layers, RGBA8 sRGB bytes):
//! layer 0 = normal, layer 1 = glow; texel `(b * N + r, row * N + g)`
//! holds the remapped colour of grid node `(r, g, b)` at shade `row`.
//! The shader interpolates r/g with the hardware and b by hand.

/// Grid nodes per colour axis.
pub const GRADE_N: usize = 33;
/// Shade rows (the retail table's height).
pub const GRADE_ROWS: usize = 64;
pub const GRADE_W: usize = GRADE_N * GRADE_N;
pub const GRADE_H: usize = GRADE_N * GRADE_ROWS;
/// Material bit: the texel glows (layer 1 of the grade table).
pub const MAT_EMISSIVE: u8 = 1;
/// Material bit: the texel is retail water (terrain index < 0x0C —
/// the water raster's per-texel mirror law).
pub const MAT_WATER: u8 = 2;

const STEP: f64 = 255.0 / (GRADE_N - 1) as f64;
/// Residual splat radius (sRGB byte units) and its cutoff (3σ).
const SIGMA: f64 = 8.0;
/// Prior weight pulling the residual to zero away from any palette
/// colour (new art far from every retail colour gets the matrix).
const W0: f64 = 0.02;
/// Ridge on the per-class matrix fit (keeps the sparse glow class sane).
const RIDGE: f64 = 255.0 * 255.0 * 0.5;
/// Fit iterations of the glow classification.
const CLASSIFY_ITERS: usize = 4;

/// The grade tables of one palette + shade remap.
pub struct Grade {
    /// Per palette index: does it glow (layer 1)? Index 0 never.
    pub glow: [bool; 256],
    /// `GRADE_W * GRADE_H * 2` RGBA8 texels, layer 0 then layer 1.
    pub table: Vec<u8>,
}

type Mat = [[f64; 4]; 3];

fn apply(m: &Mat, c: [f64; 3]) -> [f64; 3] {
    let x = [c[0], c[1], c[2], 255.0];
    std::array::from_fn(|k| (0..4).map(|j| m[k][j] * x[j]).sum())
}

/// Solve the 4x4 system by Gaussian elimination with partial pivoting
/// (a singular column yields 0 for that coefficient).
fn solve(mut a: [[f64; 4]; 4], mut y: [f64; 4]) -> [f64; 4] {
    for c in 0..4 {
        let p = (c..4)
            .max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))
            .unwrap();
        a.swap(c, p);
        y.swap(c, p);
        if a[c][c].abs() < 1e-9 {
            continue;
        }
        for r in 0..4 {
            if r != c {
                let f = a[r][c] / a[c][c];
                for k in 0..4 {
                    a[r][k] -= f * a[c][k];
                }
                y[r] -= f * y[c];
            }
        }
    }
    std::array::from_fn(|i| {
        if a[i][i].abs() > 1e-9 {
            y[i] / a[i][i]
        } else {
            0.0
        }
    })
}

/// Weighted least-squares colour matrix of one shade row.
fn fit_row(
    pal: &[[f64; 3]; 256],
    target: impl Fn(usize) -> [f64; 3],
    weight: impl Fn(usize) -> f64,
    ridge: f64,
) -> Mat {
    let mut a = [[0.0; 4]; 4];
    let mut y = [[0.0; 4]; 3];
    for i in 1..256 {
        let w = weight(i);
        if w == 0.0 {
            continue;
        }
        let x = [pal[i][0], pal[i][1], pal[i][2], 255.0];
        let t = target(i);
        for p in 0..4 {
            for q in 0..4 {
                a[p][q] += w * x[p] * x[q];
            }
            for c in 0..3 {
                y[c][p] += w * x[p] * t[c];
            }
        }
    }
    for (d, row) in a.iter_mut().enumerate() {
        row[d] += ridge;
    }
    std::array::from_fn(|c| solve(a, y[c]))
}

/// Build the grade tables. `shade_lut` is [`GRADE_ROWS`] rows of 256.
pub fn build(palette: &[[u8; 3]; 256], shade_lut: &[u8]) -> Grade {
    assert_eq!(shade_lut.len(), GRADE_ROWS * 256);
    let pal: [[f64; 3]; 256] = std::array::from_fn(|i| palette[i].map(|v| v as f64));
    let target = |s: usize, i: usize| -> [f64; 3] { pal[shade_lut[s * 256 + i] as usize] };

    // Glow classification: alternate a normal-weighted matrix fit with
    // a per-index "keeps its own colour" amount `e` in 0..1.
    let mut emis = [0.0f64; 256];
    for _ in 0..CLASSIFY_ITERS {
        let mats: Vec<Mat> = (0..GRADE_ROWS)
            .map(|s| fit_row(&pal, |i| target(s, i), |i| 1.0 - emis[i], 0.0))
            .collect();
        for i in 1..256 {
            let (mut num, mut den) = (0.0, 0.0);
            for (s, m) in mats.iter().enumerate() {
                let fit = apply(m, pal[i]);
                let t = target(s, i);
                for c in 0..3 {
                    let d = pal[i][c] - fit[c];
                    num += (t[c] - fit[c]) * d;
                    den += d * d;
                }
            }
            emis[i] = if den > 1e-6 {
                (num / den).clamp(0.0, 1.0)
            } else {
                0.0
            };
        }
    }
    let glow: [bool; 256] = std::array::from_fn(|i| i != 0 && emis[i] > 0.5);

    let mut table = vec![0u8; GRADE_W * GRADE_H * 4 * 2];
    let normal: Vec<usize> = (1..256).filter(|&i| !glow[i]).collect();
    let glowing: Vec<usize> = (1..256).filter(|&i| glow[i]).collect();
    let layer_len = GRADE_W * GRADE_H * 4;
    fill_layer(&pal, &target, &normal, &mut table[..layer_len]);
    if glowing.is_empty() {
        let (a, b) = table.split_at_mut(layer_len);
        b.copy_from_slice(a);
    } else {
        fill_layer(&pal, &target, &glowing, &mut table[layer_len..]);
    }
    Grade { glow, table }
}

/// One class's table: per row, the class's own matrix plus the
/// Gaussian-splatted residual of its member colours, baked per node.
fn fill_layer(
    pal: &[[f64; 3]; 256],
    target: &impl Fn(usize, usize) -> [f64; 3],
    members: &[usize],
    out: &mut [u8],
) {
    let n = GRADE_N;
    let reach = (3.0 * SIGMA / STEP).ceil() as i64;
    let cut2 = 9.0 * SIGMA * SIGMA;
    let mut acc = vec![[0.0f64; 3]; n * n * n];
    let mut wsum = vec![0.0f64; n * n * n];
    let in_class: [bool; 256] = std::array::from_fn(|i| members.contains(&i));
    for s in 0..GRADE_ROWS {
        let m = fit_row(
            pal,
            |i| target(s, i),
            |i| if in_class[i] { 1.0 } else { 0.0 },
            RIDGE,
        );
        acc.iter_mut().for_each(|a| *a = [0.0; 3]);
        wsum.iter_mut().for_each(|w| *w = W0);
        for &i in members {
            let fit = apply(&m, pal[i]);
            let t = target(s, i);
            let res = [t[0] - fit[0], t[1] - fit[1], t[2] - fit[2]];
            let near = pal[i].map(|v| (v / STEP).round() as i64);
            for dr in -reach..=reach {
                for dg in -reach..=reach {
                    for db in -reach..=reach {
                        let node = [near[0] + dr, near[1] + dg, near[2] + db];
                        if node.iter().any(|&v| v < 0 || v >= n as i64) {
                            continue;
                        }
                        let d2: f64 = (0..3)
                            .map(|k| (pal[i][k] - node[k] as f64 * STEP).powi(2))
                            .sum();
                        if d2 > cut2 {
                            continue;
                        }
                        let w = (-d2 / (2.0 * SIGMA * SIGMA)).exp();
                        let at = (node[2] as usize * n + node[1] as usize) * n + node[0] as usize;
                        wsum[at] += w;
                        for k in 0..3 {
                            acc[at][k] += w * res[k];
                        }
                    }
                }
            }
        }
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let at = (b * n + g) * n + r;
                    let base = apply(&m, [r as f64 * STEP, g as f64 * STEP, b as f64 * STEP]);
                    let x = b * n + r;
                    let y = s * n + g;
                    let o = (y * GRADE_W + x) * 4;
                    for k in 0..3 {
                        out[o + k] =
                            (base[k] + acc[at][k] / wsum[at]).round().clamp(0.0, 255.0) as u8;
                    }
                    out[o + 3] = 255;
                }
            }
        }
    }
}

impl Grade {
    /// CPU mirror of the shader lookup (sRGB bytes in, sRGB out):
    /// trilinear over the row's grid in the texel's layer.
    pub fn lookup(&self, rgb: [u8; 3], row: usize, glow: bool) -> [f64; 3] {
        let layer = if glow { GRADE_W * GRADE_H * 4 } else { 0 };
        let u = rgb.map(|v| v as f64 / STEP);
        let i0 = u.map(|v| (v.floor() as usize).min(GRADE_N - 2));
        let f: [f64; 3] = std::array::from_fn(|k| u[k] - i0[k] as f64);
        let mut out = [0.0; 3];
        for corner in 0..8 {
            let d = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let w: f64 = (0..3)
                .map(|k| if d[k] == 1 { f[k] } else { 1.0 - f[k] })
                .product();
            if w == 0.0 {
                continue;
            }
            let (r, g, b) = (i0[0] + d[0], i0[1] + d[1], i0[2] + d[2]);
            let o = layer + ((row * GRADE_N + g) * GRADE_W + b * GRADE_N + r) * 4;
            for k in 0..3 {
                out[k] += w * self.table[o + k] as f64;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic palette: a coarse colour cube.
    fn cube_palette() -> [[u8; 3]; 256] {
        std::array::from_fn(|i| {
            let v = |k: usize| (((i >> (k * 2)) & 3) * 85) as u8;
            [v(0), v(1), v(2).wrapping_add(((i >> 6) * 20) as u8)]
        })
    }

    #[test]
    fn identity_remap_is_identity() {
        let pal = cube_palette();
        let lut: Vec<u8> = (0..GRADE_ROWS * 256).map(|k| (k % 256) as u8).collect();
        let g = build(&pal, &lut);
        assert!(g.glow.iter().all(|&x| !x));
        for i in 1..256 {
            let out = g.lookup(pal[i], 17, false);
            for k in 0..3 {
                assert!(
                    (out[k] - pal[i][k] as f64).abs() < 1.5,
                    "index {i}: {out:?}"
                );
            }
        }
    }

    /// A duplicate colour that keeps its brightness while its twin
    /// fades is classified as glowing, and each twin shades right.
    #[test]
    fn duplicate_glow_twin_is_separated() {
        let mut pal = cube_palette();
        pal[200] = [255, 255, 255];
        pal[201] = [255, 255, 255];
        // Row s scales every colour by s/63 toward black (index 0 is
        // black in the cube), except 201, which never fades.
        let mut lut = vec![0u8; GRADE_ROWS * 256];
        for s in 0..GRADE_ROWS {
            for i in 0..256 {
                let want = pal[i].map(|v| v as f64 * s as f64 / 63.0);
                let best = (0..256)
                    .filter(|&j| j != 201)
                    .min_by(|&a, &b| {
                        let d = |j: usize| -> f64 {
                            (0..3).map(|k| (pal[j][k] as f64 - want[k]).powi(2)).sum()
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .unwrap();
                lut[s * 256 + i] = if i == 201 { 201 } else { best as u8 };
            }
        }
        let g = build(&pal, &lut);
        assert!(g.glow[201], "the non-fading twin glows");
        assert!(!g.glow[200]);
        let dark_normal = g.lookup([255, 255, 255], 0, false);
        let dark_glow = g.lookup([255, 255, 255], 0, true);
        assert!(dark_normal.iter().all(|&v| v < 40.0), "{dark_normal:?}");
        assert!(dark_glow.iter().all(|&v| v > 200.0), "{dark_glow:?}");
    }

    /// Against the real baked variants (skipped when not baked): the
    /// approved accuracy of the 2026-10-05 review sheets.
    #[test]
    fn baked_variants_match_retail_remap() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../baked/assets");
        for (variant, max_mean) in [
            ("mc1-temperate", 2.0),
            ("mc2-day", 3.0),
            ("mc2-night", 3.0),
            ("mc2-cave", 3.5),
        ] {
            let dir = root.join(variant);
            let (Ok(p), Ok(lut)) = (
                std::fs::read(dir.join("palette.bin")),
                std::fs::read(dir.join("shade-lut.bin")),
            ) else {
                eprintln!("skip {variant}: not baked");
                continue;
            };
            let pal: [[u8; 3]; 256] =
                std::array::from_fn(|i| [p[i * 4], p[i * 4 + 1], p[i * 4 + 2]]);
            let g = build(&pal, &lut);
            let (mut err, mut n) = (0.0, 0.0);
            for s in 0..GRADE_ROWS {
                for i in 1..256 {
                    let out = g.lookup(pal[i], s, g.glow[i]);
                    let t = pal[lut[s * 256 + i] as usize];
                    for k in 0..3 {
                        err += (out[k] - t[k] as f64).abs();
                        n += 1.0;
                    }
                }
            }
            let mean = err / n;
            eprintln!("{variant}: mean |err| {mean:.2}/255");
            assert!(mean < max_mean, "{variant}: mean error {mean:.2}");
        }
    }
}
