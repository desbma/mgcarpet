//! The MC2 multiplayer lobby screen — retail HSCREEN0 case 7
//! (`sub_77680`, MenusAndIntros.cpp:5903-6125; recon
//! docs/traces/mc2-multiplayer-lobby.md), a VIEW over the
//! host-authoritative lobby (`lobby.rs`, `net::Host`/`net::Guest`).
//!
//! Retail law reproduced: the cave doorway background with its own
//! palette; six buttons whose art is part of the background, the
//! hovered one lit with its `byte_21` sprite (level arrows, Exit and
//! Play plaques, the two wizard totems); click sample 14; the level
//! name `"%d. %s"` (level − 49) in FONT1 at (246,14); your wizard's
//! portrait in the doorway (its own palette, faded in/out on a change);
//! the lintel windows lit in the members' colours, member order, from
//! retail's colour table (`str_BYTE_E1711`; retail left your own dark —
//! the port lights it too, so you can find your seat, player ruling
//! 10-15); the oval window in your own; lobby sprite 15 as the cursor.
//!
//! The port's additions (player rulings 2026-10-15): the level arrows
//! answer the HOST only; a client's Play plaque is his agreement
//! toggle and the host's starts the game only once everyone agreed; a
//! window/oval burns at full colour once its member agreed (the host's
//! always), dimmed before; a status line under the doorway says what
//! the lobby waits for (or, hovering a window, whose it is); the host
//! kicks by clicking a lit window (confirmed on a parchment scroll).
//! The lintel's two little boxes (retail: the session digit on the
//! right) show the player limit (left, by the figure — the host cycles
//! it) and the players in (right, by the ring). The session rules sit
//! on a parchment scroll at the right: the host steps each row
//! left/right, the clients read them; the passphrase is its first row
//! (the host's only).

use std::path::Path;

use mgc_render::UiQuad;

use crate::lobby::{LobbyView, WIZARDS};

const W: f32 = 640.0;
const H: f32 = 480.0;
const SND_CLICK: u8 = 14;
/// The parchment strip width (case-4 sprite 72).
const DIALOG_W: f32 = 114.0;

/// What the player asked the lobby for — drained by the app, which
/// knows the role and talks to the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LobbyAction {
    PrevLevel,
    NextLevel,
    /// Leave the lobby (the host: close it).
    Exit,
    /// Host: start (only offered once everyone agreed). Client: toggle
    /// the agreement.
    Play,
    PrevWizard,
    NextWizard,
    /// Host: show member `id` the door (confirmed).
    Kick(u32),
    /// Host: the new passphrase.
    SetPassphrase(String),
    /// Host: step rules row `row` (`forward` = the right half).
    StepRule {
        row: usize,
        forward: bool,
    },
    /// Host: the next player limit.
    CycleCapacity,
}

/// One row of the rules scroll: its label and current value text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleRow {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Btn {
    PrevLevel,
    NextLevel,
    Exit,
    Play,
    PrevWizard,
    NextWizard,
}

/// `str_E1BAC_0x1b8` (MI:437-444): hit rect + the lit sprite.
const BUTTONS: [(Btn, [f32; 4], usize); 6] = [
    (Btn::PrevLevel, [284.0, 2.0, 30.0, 10.0], 4),
    (Btn::NextLevel, [282.0, 34.0, 30.0, 10.0], 5),
    (Btn::Exit, [111.0, 101.0, 40.0, 40.0], 1),
    (Btn::Play, [449.0, 100.0, 40.0, 40.0], 2),
    (Btn::PrevWizard, [154.0, 259.0, 80.0, 150.0], 6),
    (Btn::NextWizard, [399.0, 258.0, 80.0, 150.0], 7),
];

/// The lintel's session-digit box (MI:6363-65): the players in.
const BOX: [f32; 4] = [382.0, 18.0, 16.0, 16.0];
/// Its mirror by the figure plaque: the player limit (the host cycles).
const LIMIT_BOX: [f32; 4] = [202.0, 18.0, 16.0, 16.0];
/// The rules scroll: (x1, y1, w), below the Play plaque (449,100 40×40);
/// its height follows the rows.
const RULES: (f32, f32, f32) = (452.0, 148.0, 184.0);
/// Rules rows' pitch and first row offset.
const ROW_H: f32 = 15.0;
const ROW_Y: f32 = 34.0;

/// Retail's wizard colours (`str_BYTE_E1711`, Network.cpp pre-rework
/// :33-53): 6-bit (light, dark) by lobby PORTRAIT index.
const COLOURS: [([u8; 3], [u8; 3]); 8] = [
    ([0x36, 0x36, 0x36], [0x22, 0x22, 0x22]), // grey
    ([0x00, 0x26, 0x00], [0x00, 0x16, 0x00]), // green
    ([0x2D, 0x00, 0x00], [0x1B, 0x00, 0x00]), // red
    ([0x23, 0x00, 0x21], [0x14, 0x00, 0x16]), // violet
    ([0x3F, 0x0F, 0x28], [0x31, 0x00, 0x19]), // pink
    ([0x16, 0x0F, 0x00], [0x0E, 0x0A, 0x00]), // brown
    ([0x00, 0x00, 0x17], [0x00, 0x00, 0x1E]), // blue
    ([0x3F, 0x1F, 0x00], [0x3C, 0x1C, 0x00]), // orange
];

/// A lobby portrait's in-game wizard — retail
/// `GetTrueWizardNumber_61790`'s multiplayer arm (Basic.cpp:3005-36):
/// portrait 1 is Rahn (wizard 2), 6 is Belix (3), … The port's seat IS
/// that wizard (name, colour, castle, carpet).
pub const PORTRAIT_SEAT: [u8; 8] = [0, 2, 1, 4, 5, 6, 3, 7];

/// The portrait showing wizard seat `seat`.
pub fn portrait_of_seat(seat: u8) -> usize {
    PORTRAIT_SEAT.iter().position(|&s| s == seat).unwrap_or(0)
}

/// An atlas rect (x, y, w, h).
type Rect = (f32, f32, f32, f32);

/// A white mask in the atlas and where it lands on the screen.
#[derive(Clone, Copy)]
struct Mask {
    uv: Rect,
    at: (f32, f32),
}

enum Modal {
    /// Host: confirm kicking member `id`.
    Kick { id: u32, name: String },
    /// Host: edit the passphrase.
    Passphrase { buf: String },
}

pub struct LobbyScreen {
    atlas: Vec<u8>,
    atlas_w: u32,
    atlas_h: u32,
    /// The lobby bank (case 7).
    rects: Vec<Option<Rect>>,
    /// The temple bank (case 4): the parchment scroll's parts.
    menu: Vec<Option<Rect>>,
    font: Vec<Option<Rect>>,
    /// Per portrait: its atlas rect and its doorway position.
    portraits: Vec<Mask>,
    /// Per window slot 0-7 (7 = the oval): the light and dark masks.
    windows: Vec<[Option<Mask>; 2]>,
    view: Option<LobbyView>,
    /// The portrait on show and its fade (0 = black, 1 = shown).
    shown: Option<usize>,
    fade: f32,
    anim: f32,
    modal: Option<Modal>,
    actions: Vec<LobbyAction>,
    sounds: Vec<u8>,
    /// The rules scroll's rows (the app reads them off the session).
    rules: Vec<RuleRow>,
    /// The host's passphrase (the scroll's first row; none for clients).
    passphrase: Option<String>,
}

/// A row-major 8bpp image to place in the atlas.
struct Piece {
    w: usize,
    h: usize,
    /// RGBA, w×h×4.
    rgba: Vec<u8>,
}

/// Shelf-pack `pieces` below `top` in an atlas `width` wide: their
/// (x, y) and the height used.
fn shelf(pieces: &[Piece], width: usize, top: usize) -> (Vec<(usize, usize)>, usize) {
    let (mut x, mut y, mut row) = (0, top, 0);
    let mut at = Vec::new();
    for p in pieces {
        if x + p.w > width {
            x = 0;
            y += row;
            row = 0;
        }
        at.push((x, y));
        x += p.w;
        row = row.max(p.h);
    }
    (at, y + row)
}

fn rgb6(c: [u8; 3]) -> [u8; 3] {
    [c[0] << 2, c[1] << 2, c[2] << 2]
}

impl LobbyScreen {
    /// Load the case-7 members (plus the temple bank's scroll parts and
    /// FONT1) out of the baked `assets/mc2-ui` bundle.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let read = |name: &str| -> Result<Vec<u8>, String> {
            std::fs::read(dir.join(name)).map_err(|e| {
                format!(
                    "{}: {e} (rebake — epoch 28 adds it)",
                    dir.join(name).display()
                )
            })
        };
        let index = |name: &str| -> Result<mgc_formats::bundle::SpriteIndex, String> {
            serde_json::from_slice(&read(name)?).map_err(|e| format!("{name}: {e}"))
        };
        let bg = read("lobby-bg.bin")?;
        let pal = read("lobby-pal.bin")?;
        if bg.len() != (W * H) as usize || pal.len() != 768 {
            return Err("lobby-bg.bin / lobby-pal.bin: wrong size".into());
        }
        let bank_px = read("lobby-sprites.bin")?;
        let bank = index("lobby-sprites.json")?;
        let menu_px = read("menu-sprites.bin")?;
        let menu_pal = read("menu-pal.bin")?;
        let menu = index("menu-sprites.json")?;
        let font_px = read("font.bin")?;
        let font = index("font.json")?;
        let portrait_px = read("lobby-portraits.bin")?;
        let portrait_pals = read("lobby-portrait-pals.bin")?;
        let portrait_rects: Vec<serde_json::Value> =
            serde_json::from_slice(&read("lobby-portraits.json")?)
                .map_err(|e| format!("lobby-portraits.json: {e}"))?;

        let rgb = |pal: &[u8], i: u8| {
            rgb6([
                pal[i as usize * 3],
                pal[i as usize * 3 + 1],
                pal[i as usize * 3 + 2],
            ])
        };
        let mut pieces = Vec::new();
        // 0: the background, palette-resolved, the windows left black
        // (shipped so; the masks below light them).
        pieces.push(Piece {
            w: W as usize,
            h: H as usize,
            rgba: bg
                .iter()
                .flat_map(|&p| {
                    let [r, g, b] = rgb(&pal, p);
                    [r, g, b, 255]
                })
                .collect(),
        });
        // The three sprite banks as packed atlases (0 = transparent).
        let bank_piece =
            |px: &[u8], ix: &mgc_formats::bundle::SpriteIndex, pal: &[u8], mask: bool| Piece {
                w: ix.atlas_width as usize,
                h: ix.atlas_height as usize,
                rgba: px
                    .iter()
                    .flat_map(|&p| match (p, mask) {
                        (0, _) => [0, 0, 0, 0],
                        (_, true) => [255, 255, 255, 255],
                        (p, false) => {
                            let [r, g, b] = rgb(pal, p);
                            [r, g, b, 255]
                        }
                    })
                    .collect(),
            };
        pieces.push(bank_piece(&bank_px, &bank, &pal, false));
        pieces.push(bank_piece(&menu_px, &menu, &menu_pal, false));
        pieces.push(bank_piece(&font_px, &font, &pal, true));
        // The portraits, each through its own palette (0/0xFF clear).
        let mut portrait_at = Vec::new();
        for (k, r) in portrait_rects.iter().enumerate() {
            let g = |f: &str| r[f].as_u64().unwrap_or(0) as usize;
            let (w, h, off) = (g("w"), g("h"), g("offset"));
            let ppal = portrait_pals
                .get(k * 768..k * 768 + 768)
                .ok_or("lobby-portrait-pals.bin: short")?;
            let px = portrait_px
                .get(off..off + w * h)
                .ok_or("lobby-portraits.bin: short")?;
            pieces.push(Piece {
                w,
                h,
                rgba: px
                    .iter()
                    .flat_map(|&p| {
                        if p == 0 || p == 0xFF {
                            [0, 0, 0, 0]
                        } else {
                            let [r, g, b] = rgb(ppal, p);
                            [r, g, b, 255]
                        }
                    })
                    .collect(),
            });
            portrait_at.push((g("x") as f32, g("y") as f32));
        }
        // The window masks: every bg pixel in palette entry 134+i
        // (i = 0..16 — window k light/dark = 2k / 2k+1, k 7 = the
        // oval), cut to its bounding box.
        let mut window_at = Vec::new();
        for i in 0..16u8 {
            let idx = 134 + i;
            let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
            for (n, &p) in bg.iter().enumerate() {
                if p == idx {
                    let (x, y) = (n % W as usize, n / W as usize);
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
            if x0 == usize::MAX {
                window_at.push(None);
                continue;
            }
            let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
            let mut rgba = vec![0u8; w * h * 4];
            for y in 0..h {
                for x in 0..w {
                    if bg[(y0 + y) * W as usize + x0 + x] == idx {
                        rgba[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[255; 4]);
                    }
                }
            }
            pieces.push(Piece { w, h, rgba });
            window_at.push(Some((x0 as f32, y0 as f32)));
        }

        let atlas_w = 1024usize;
        let (at, atlas_h) = shelf(&pieces, atlas_w, 0);
        let mut atlas = vec![0u8; atlas_w * atlas_h * 4];
        for (p, &(x, y)) in pieces.iter().zip(&at) {
            for row in 0..p.h {
                let dst = ((y + row) * atlas_w + x) * 4;
                atlas[dst..dst + p.w * 4]
                    .copy_from_slice(&p.rgba[row * p.w * 4..(row + 1) * p.w * 4]);
            }
        }
        let bank_rects =
            |ix: &mgc_formats::bundle::SpriteIndex, base: (usize, usize)| -> Vec<Option<Rect>> {
                ix.sprites
                    .iter()
                    .map(|s| {
                        let f = s.frames.first()?;
                        (s.width > 0 && s.height > 0).then_some((
                            (base.0 + f.x as usize) as f32,
                            (base.1 + f.y as usize) as f32,
                            s.width as f32,
                            s.height as f32,
                        ))
                    })
                    .collect()
            };
        let portraits = portrait_at
            .iter()
            .enumerate()
            .map(|(k, &pos)| {
                let (x, y) = at[4 + k];
                let p = &pieces[4 + k];
                Mask {
                    uv: (x as f32, y as f32, p.w as f32, p.h as f32),
                    at: pos,
                }
            })
            .collect();
        let mut masks = Vec::new();
        let mut next = 4 + portrait_at.len();
        for pos in &window_at {
            masks.push(pos.map(|pos| {
                let (x, y) = at[next];
                let p = &pieces[next];
                next += 1;
                Mask {
                    uv: (x as f32, y as f32, p.w as f32, p.h as f32),
                    at: pos,
                }
            }));
        }
        let windows = (0..8).map(|k| [masks[2 * k], masks[2 * k + 1]]).collect();
        Ok(Self {
            rects: bank_rects(&bank, at[1]),
            menu: bank_rects(&menu, at[2]),
            font: bank_rects(&font, at[3]),
            atlas,
            atlas_w: atlas_w as u32,
            atlas_h: atlas_h as u32,
            portraits,
            windows,
            view: None,
            shown: None,
            fade: 0.0,
            anim: 0.0,
            modal: None,
            actions: Vec::new(),
            sounds: Vec::new(),
            rules: Vec::new(),
            passphrase: None,
        })
    }

    /// The rules scroll's content: the session's rule rows, and (host)
    /// the passphrase.
    pub fn set_rules(&mut self, rows: Vec<RuleRow>, passphrase: Option<String>) {
        self.rules = rows;
        self.passphrase = passphrase;
    }

    /// The scroll's rows as drawn: the passphrase first (host only).
    fn rows(&self) -> Vec<RuleRow> {
        let mut rows = Vec::new();
        if let Some(p) = &self.passphrase {
            rows.push(RuleRow {
                label: "Passphrase".into(),
                value: p.clone(),
            });
        }
        rows.extend(self.rules.iter().cloned());
        rows
    }

    /// A host's value as drawn: between its step chevrons.
    fn shown_value(v: &str) -> String {
        format!("< {v} >")
    }

    /// Row `k`'s hit rect.
    fn row_rect(k: usize) -> [f32; 4] {
        let (x1, y1, w) = RULES;
        [x1 + 12.0, y1 + ROW_Y + ROW_H * k as f32, w - 24.0, ROW_H]
    }

    pub fn atlas(&self) -> (u32, u32, &[u8]) {
        (self.atlas_w, self.atlas_h, &self.atlas)
    }

    /// The lobby as it stands now (every frame, from the session).
    pub fn update(&mut self, view: &LobbyView) {
        if self.view.as_ref() != Some(view) {
            self.view = Some(view.clone());
        }
    }

    /// The portrait fades (retail +16/255 per 70 Hz frame).
    pub fn tick(&mut self, dt: f32) {
        self.anim += dt;
        let want = self
            .view
            .as_ref()
            .and_then(LobbyView::me)
            .map(|m| portrait_of_seat(m.wizard));
        let step = dt * 70.0 * 16.0 / 255.0;
        if want != self.shown {
            self.fade -= step;
            if self.fade <= 0.0 {
                self.fade = 0.0;
                self.shown = want;
            }
        } else {
            self.fade = (self.fade + step).min(1.0);
        }
    }

    pub fn take_actions(&mut self) -> Vec<LobbyAction> {
        std::mem::take(&mut self.actions)
    }

    pub fn take_sounds(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.sounds)
    }

    /// A text field is open (the passphrase).
    pub fn editing(&self) -> bool {
        matches!(self.modal, Some(Modal::Passphrase { .. }))
    }

    pub fn escape(&mut self) {
        if self.modal.take().is_some() {
            self.sounds.push(SND_CLICK);
        } else {
            self.actions.push(LobbyAction::Exit);
        }
    }

    /// The passphrase field: letters, digits and a few marks, max 24.
    pub fn key_char(&mut self, c: char) {
        if let Some(Modal::Passphrase { buf }) = &mut self.modal
            && buf.len() < 24
            && (c.is_ascii_alphanumeric() || "-_.!?".contains(c))
        {
            buf.push(c);
        }
    }

    pub fn key_backspace(&mut self) {
        if let Some(Modal::Passphrase { buf }) = &mut self.modal {
            buf.pop();
        }
    }

    pub fn key_enter(&mut self) {
        if self.modal.is_some() {
            self.press_ok();
        }
    }

    fn is_host(&self) -> bool {
        self.view.as_ref().is_some_and(LobbyView::is_host)
    }

    /// Does `btn` answer this member now?
    fn enabled(&self, btn: Btn) -> bool {
        let Some(v) = &self.view else { return false };
        match btn {
            Btn::PrevLevel | Btn::NextLevel => v.is_host(),
            Btn::Play => !v.is_host() || v.all_agreed(),
            Btn::Exit => true,
            Btn::PrevWizard | Btn::NextWizard => {
                v.me().is_some_and(|m| v.next_free(m.wizard, 1).is_some())
            }
        }
    }

    fn button_at(&self, mx: f32, my: f32) -> Option<Btn> {
        BUTTONS
            .iter()
            .find(|(_, r, _)| in_rect(mx, my, *r))
            .map(|&(b, ..)| b)
            .filter(|&b| self.enabled(b))
    }

    /// The member whose lit window is under the point (not ourselves).
    fn window_at(&self, mx: f32, my: f32) -> Option<(u32, u8)> {
        let v = self.view.as_ref()?;
        self.hovered_member(mx, my).filter(|(id, _)| *id != v.you)
    }

    /// The member whose lit window is under the point.
    fn hovered_member(&self, mx: f32, my: f32) -> Option<(u32, u8)> {
        let v = self.view.as_ref()?;
        v.members.iter().enumerate().take(7).find_map(|(k, m)| {
            let hit = self.windows[k]
                .iter()
                .flatten()
                .any(|w| in_rect(mx, my, [w.at.0, w.at.1, w.uv.2, w.uv.3]));
            hit.then_some((m.id, m.wizard))
        })
    }

    /// The parchment dialog's box (x1, y1, w, h).
    fn dialog_box(&self) -> (f32, f32, f32, f32) {
        (320.0 - 80.0, 150.0, 160.0, 80.0)
    }

    pub fn click(&mut self, size: (f32, f32), cursor: (f32, f32)) {
        let (mx, my) = crate::ui::unletterbox(cursor, size, W, H);
        if self.modal.is_some() {
            let (x1, y1, w, h) = self.dialog_box();
            if in_rect(mx, my, ok_rect(x1, y1, h)) {
                self.press_ok();
            } else if in_rect(mx, my, cancel_rect(x1, y1, w, h)) {
                self.modal = None;
                self.sounds.push(SND_CLICK);
            }
            return;
        }
        if let Some(b) = self.button_at(mx, my) {
            // Waiting for a game to finish: only Exit is live.
            if self.waiting() && b != Btn::Exit {
                return;
            }
            self.sounds.push(SND_CLICK);
            self.actions.push(match b {
                Btn::PrevLevel => LobbyAction::PrevLevel,
                Btn::NextLevel => LobbyAction::NextLevel,
                Btn::Exit => LobbyAction::Exit,
                Btn::Play => LobbyAction::Play,
                Btn::PrevWizard => LobbyAction::PrevWizard,
                Btn::NextWizard => LobbyAction::NextWizard,
            });
            return;
        }
        if !self.is_host() {
            return;
        }
        if let Some((id, _)) = self.window_at(mx, my) {
            self.sounds.push(SND_CLICK);
            let name = self.member_name(id);
            self.modal = Some(Modal::Kick { id, name });
        } else if in_rect(mx, my, LIMIT_BOX) {
            self.sounds.push(SND_CLICK);
            self.actions.push(LobbyAction::CycleCapacity);
        } else if let Some(k) = (0..self.rows().len()).find(|&k| in_rect(mx, my, Self::row_rect(k)))
        {
            self.sounds.push(SND_CLICK);
            // The step's side: the drawn `< value >`'s halves (the
            // label steps forward).
            let r = Self::row_rect(k);
            let w = self.text_width(&Self::shown_value(&self.rows()[k].value));
            let mid = r[0] + r[2] - w / 2.0;
            match (self.passphrase.is_some(), k) {
                (true, 0) => self.modal = Some(Modal::Passphrase { buf: String::new() }),
                (pass, k) => self.actions.push(LobbyAction::StepRule {
                    row: k - usize::from(pass),
                    forward: mx >= mid || mx < r[0] + r[2] - w,
                }),
            }
        }
    }

    /// A member's shown name: his own, else his wizard's.
    fn member_name(&self, id: u32) -> String {
        let Some(m) = self
            .view
            .as_ref()
            .and_then(|v| v.members.iter().find(|m| m.id == id))
        else {
            return String::new();
        };
        if m.name.is_empty() {
            mgc_sim::mc2::rivals::MC2_RIVAL_NAMES[usize::from(m.wizard.min(WIZARDS - 1))]
                .to_string()
        } else {
            m.name.clone()
        }
    }

    fn press_ok(&mut self) {
        match self.modal.take() {
            Some(Modal::Kick { id, .. }) => self.actions.push(LobbyAction::Kick(id)),
            Some(Modal::Passphrase { buf }) => self.actions.push(LobbyAction::SetPassphrase(buf)),
            None => return,
        }
        self.sounds.push(SND_CLICK);
    }

    /// A game is in progress without us: we left it, or joined during
    /// it — the lobby waits for it to finish (only Exit is live).
    fn waiting(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.playing)
    }

    /// The status line under the doorway.
    fn status(&self) -> String {
        let Some(v) = &self.view else {
            return String::new();
        };
        if v.playing {
            return "Waiting for the current game to finish".into();
        }
        let clients = v.members.len().saturating_sub(1);
        let agreed = v.members.iter().skip(1).filter(|m| m.agreed).count();
        if v.is_host() {
            if clients == 0 {
                "Waiting for players to join".into()
            } else if v.all_agreed() {
                "Everyone agreed - press Play".into()
            } else {
                format!("Waiting for agreement ({agreed} of {clients})")
            }
        } else if v.me().is_some_and(|m| m.agreed) {
            "You agreed - waiting for the host".into()
        } else {
            "Press Play to agree to the rules".into()
        }
    }

    fn sprite(
        &self,
        bank: &[Option<Rect>],
        id: usize,
        pos: (f32, f32),
        scale: f32,
    ) -> Option<UiQuad> {
        let (sx, sy, w, h) = bank.get(id).copied().flatten()?;
        Some(UiQuad {
            rect: [pos.0 * scale, pos.1 * scale, w * scale, h * scale],
            uv: [sx, sy, w, h],
            tint: [1.0; 4],
        })
    }

    fn text_width(&self, s: &str) -> f32 {
        s.chars()
            .map(|c| {
                self.font
                    .get((c as usize).wrapping_add(1))
                    .copied()
                    .flatten()
                    .map_or(4.0, |r| r.2)
            })
            .sum()
    }

    fn text(&self, s: &str, x: f32, y: f32, tint: [f32; 4], scale: f32, out: &mut Vec<UiQuad>) {
        let mut cx = x;
        for c in s.chars() {
            let Some((sx, sy, w, h)) = self
                .font
                .get((c as usize).wrapping_add(1))
                .copied()
                .flatten()
            else {
                cx += 4.0;
                continue;
            };
            if c != ' ' {
                out.push(UiQuad {
                    rect: [cx * scale, y * scale, w * scale, h * scale],
                    uv: [sx, sy, w, h],
                    tint,
                });
            }
            cx += w;
        }
    }

    fn mask(&self, m: &Mask, tint: [f32; 4], scale: f32) -> UiQuad {
        UiQuad {
            rect: [
                m.at.0 * scale,
                m.at.1 * scale,
                m.uv.2 * scale,
                m.uv.3 * scale,
            ],
            uv: [m.uv.0, m.uv.1, m.uv.2, m.uv.3],
            tint,
        }
    }

    /// Light window slot `k` in portrait `p`'s colours; `dim` before the
    /// member agreed.
    fn light(&self, k: usize, p: usize, dim: bool, scale: f32, out: &mut Vec<UiQuad>) {
        // Tints multiply in LINEAR space (the swapchain is sRGB):
        // decode, so a window shows retail's own value.
        let f = if dim { 0.4 } else { 1.0 };
        let (light, dark) = COLOURS[p];
        for (mask, c) in self.windows[k].iter().zip([light, dark]) {
            if let Some(m) = mask {
                let lin = |v: u8| crate::entities::srgb_lin((f32::from(v) * f) as u8);
                let [r, g, b] = rgb6(c);
                out.push(self.mask(m, [lin(r), lin(g), lin(b), 1.0], scale));
            }
        }
    }

    /// This frame's quads (letterboxed like the temple menu).
    pub fn quads(&self, size: (f32, f32), cursor: (f32, f32), level_name: &str) -> Vec<UiQuad> {
        let (scale, ox, oy) = crate::ui::letterbox(size, W, H);
        let (mx, my) = crate::ui::unletterbox(cursor, size, W, H);
        let mut q = vec![UiQuad {
            rect: [0.0, 0.0, W * scale, H * scale],
            uv: [0.0, 0.0, W, H],
            tint: [1.0; 4],
        }];
        if let Some(v) = &self.view {
            // The window row: every member in member order, yourself
            // included (retail left yours dark).
            for (k, m) in v.members.iter().enumerate().take(7) {
                self.light(k, portrait_of_seat(m.wizard), !m.agreed, scale, &mut q);
            }
            if let Some(me) = v.me() {
                self.light(7, portrait_of_seat(me.wizard), !me.agreed, scale, &mut q);
            }
        }
        // The portrait, faded.
        if let Some(p) = self.shown.and_then(|p| self.portraits.get(p)) {
            let f = self.fade;
            q.push(self.mask(p, [f, f, f, 1.0], scale));
        }
        // Hover art.
        if self.modal.is_none()
            && let Some(b) = self.button_at(mx, my)
            && let Some(&(_, r, spr)) = BUTTONS.iter().find(|(x, ..)| *x == b)
            && let Some(s) = self.sprite(&self.rects, spr, (r[0], r[1]), scale)
        {
            q.push(s);
        }
        // The level name, clipped to its field (x 246-355, y 14).
        let gold = [0.95, 0.85, 0.55, 1.0];
        let mut name = level_name.to_string();
        while self.text_width(&name) > 109.0 && name.pop().is_some() {}
        self.text(&name, 246.0, 14.0, gold, scale, &mut q);
        if let Some(v) = &self.view {
            for (b, n) in [(BOX, v.members.len()), (LIMIT_BOX, usize::from(v.capacity))] {
                let n = n.to_string();
                let nx = b[0] + (b[2] - self.text_width(&n)) / 2.0;
                self.text(&n, nx, b[1], gold, scale, &mut q);
            }
        }
        self.rules_scroll(mx, my, scale, &mut q);
        self.window_names(scale, &mut q);
        let status = match self.hovered_member(mx, my).filter(|_| self.modal.is_none()) {
            Some((id, _)) => self.member_name(id),
            None => self.status(),
        };
        let sx = (W - self.text_width(&status)) / 2.0;
        self.text(&status, sx, 452.0, [1.0; 4], scale, &mut q);
        // A dialog on top.
        if let Some(modal) = &self.modal {
            let (x1, y1, w, h) = self.dialog_box();
            self.parchment(x1, y1, w, h, scale, &mut q);
            let ink = [88.0 / 255.0, 64.0 / 255.0, 36.0 / 255.0, 1.0];
            let (title, body) = match modal {
                Modal::Kick { name, .. } => (format!("Kick {name}?"), String::new()),
                Modal::Passphrase { buf } => {
                    let caret = if (self.anim * 4.0) as u32 % 2 == 0 {
                        "_"
                    } else {
                        ""
                    };
                    ("New passphrase".to_string(), format!("{buf}{caret}"))
                }
            };
            let tx = x1 + (w - self.text_width(&title)) / 2.0;
            self.text(&title, tx, y1 + 14.0, ink, scale, &mut q);
            self.text(&body, x1 + 15.0, y1 + 34.0, [1.0; 4], scale, &mut q);
            let ok = ok_rect(x1, y1, h);
            let ca = cancel_rect(x1, y1, w, h);
            let ok_spr = if in_rect(mx, my, ok) { 71 } else { 70 };
            let ca_spr = if in_rect(mx, my, ca) { 69 } else { 68 };
            q.extend(self.sprite(&self.menu, ok_spr, (ok[0], ok[1]), scale));
            q.extend(self.sprite(&self.menu, ca_spr, (ca[0], ca[1]), scale));
        }
        // Cursor = lobby sprite 15 (MI:6108).
        q.extend(self.sprite(&self.rects, 15, (mx, my), scale));
        crate::ui::offset_quads(&mut q, ox, oy);
        q
    }

    /// The session rules on a parchment scroll at the right: label left,
    /// value right; the host's values in white with step chevrons, the
    /// hovered row's side lit.
    fn rules_scroll(&self, mx: f32, my: f32, scale: f32, out: &mut Vec<UiQuad>) {
        let rows = self.rows();
        if rows.is_empty() {
            return;
        }
        let (x1, y1, w) = RULES;
        let h = ROW_Y + ROW_H * rows.len() as f32 + 6.0;
        self.parchment(x1, y1, w, h, scale, out);
        let ink = [88.0 / 255.0, 64.0 / 255.0, 36.0 / 255.0, 1.0];
        let title = "Session rules";
        self.text(
            title,
            x1 + (w - self.text_width(title)) / 2.0,
            y1 + 14.0,
            ink,
            scale,
            out,
        );
        let host = self.is_host();
        for (k, row) in rows.iter().enumerate() {
            let r = Self::row_rect(k);
            self.text(&row.label, r[0], r[1], ink, scale, out);
            let hover = host && self.modal.is_none() && in_rect(mx, my, r);
            let value = if host {
                Self::shown_value(&row.value)
            } else {
                row.value.clone()
            };
            let vx = r[0] + r[2] - self.text_width(&value);
            let tint = if host { [1.0; 4] } else { ink };
            self.text(&value, vx, r[1], tint, scale, out);
            if hover {
                out.push(UiQuad {
                    rect: [
                        r[0] * scale,
                        (r[1] + ROW_H - 2.0) * scale,
                        r[2] * scale,
                        scale,
                    ],
                    uv: [0.0; 4],
                    tint: [1.0, 1.0, 1.0, 0.6],
                });
            }
        }
    }

    /// Each member's name on the lintel at his window, zigzagging —
    /// even windows below, odd above (player design 10-15) — centred on
    /// the window and cut to what fits between its neighbours. Yours in
    /// white.
    fn window_names(&self, scale: f32, out: &mut Vec<UiQuad>) {
        let Some(v) = &self.view else { return };
        let lh = self.font.iter().flatten().next().map_or(8.0, |r| r.3);
        for (k, m) in v.members.iter().enumerate().take(7) {
            let masks: Vec<&Mask> = self.windows[k].iter().flatten().collect();
            let Some(first) = masks.first() else { continue };
            let (mut x0, mut y0, mut x1, mut y1) = (first.at.0, first.at.1, 0.0f32, 0.0f32);
            for w in &masks {
                x0 = x0.min(w.at.0);
                y0 = y0.min(w.at.1);
                x1 = x1.max(w.at.0 + w.uv.2);
                y1 = y1.max(w.at.1 + w.uv.3);
            }
            // A name may reach halfway to the next same-side name: two
            // windows' pitch.
            let room = 2.0 * (x1 - x0 + 4.0) - 4.0;
            let mut name = self.member_name(m.id);
            while self.text_width(&name) > room && name.pop().is_some() {}
            let x = (x0 + x1) / 2.0 - self.text_width(&name) / 2.0;
            let y = if k % 2 == 0 { y1 + 2.0 } else { y0 - lh - 1.0 };
            let ink = if m.id == v.you {
                [1.0; 4]
            } else {
                [0.95, 0.85, 0.55, 1.0]
            };
            self.text(&name, x, y, ink, scale, out);
        }
    }

    /// The parchment scroll, stretched to `w` (the temple dialog's
    /// roller bar, sprite 72, and a solid fill between).
    fn parchment(&self, x1: f32, y1: f32, w: f32, h: f32, scale: f32, out: &mut Vec<UiQuad>) {
        let s = |r: [f32; 4]| [r[0] * scale, r[1] * scale, r[2] * scale, r[3] * scale];
        let parchment = [168.0 / 255.0, 144.0 / 255.0, 116.0 / 255.0, 1.0];
        let edge = [148.0 / 255.0, 124.0 / 255.0, 100.0 / 255.0, 1.0];
        let bar = self.menu.get(72).copied().flatten();
        let bar_h = bar.map_or(12.0, |b| b.3);
        let top = y1 + bar_h - 2.0;
        for (r, tint) in [
            ([x1 + 10.0, top, w - 22.0, h], parchment),
            ([x1 + 10.0, top, 1.0, h], edge),
            ([x1 + w - 12.0, top, 1.0, h], edge),
        ] {
            out.push(UiQuad {
                rect: s(r),
                uv: [0.0; 4],
                tint,
            });
        }
        if let Some((sx, sy, bw, bh)) = bar {
            for y in [y1, y1 + h] {
                out.push(UiQuad {
                    rect: s([x1, y, w.max(DIALOG_W), bh]),
                    uv: [sx, sy, bw, bh],
                    tint: [1.0; 4],
                });
            }
        }
    }
}

fn in_rect(mx: f32, my: f32, r: [f32; 4]) -> bool {
    mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
}

fn ok_rect(x1: f32, y1: f32, h: f32) -> [f32; 4] {
    [x1 + 15.0, y1 + h - 28.0, 42.0, 28.0]
}

fn cancel_rect(x1: f32, y1: f32, w: f32, h: f32) -> [f32; 4] {
    [x1 + w - 12.0 - 39.0, y1 + h - 30.0, 39.0, 30.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::Member;

    /// A screen with no art: geometry and state only.
    fn bare() -> LobbyScreen {
        LobbyScreen {
            atlas: Vec::new(),
            atlas_w: 1,
            atlas_h: 1,
            rects: Vec::new(),
            menu: Vec::new(),
            font: Vec::new(),
            portraits: Vec::new(),
            windows: (0..8)
                .map(|k| {
                    let m = Mask {
                        uv: (0.0, 0.0, 10.0, 10.0),
                        at: (200.0 + 20.0 * k as f32, 60.0),
                    };
                    [Some(m), None]
                })
                .collect(),
            view: None,
            shown: None,
            fade: 0.0,
            anim: 0.0,
            modal: None,
            actions: Vec::new(),
            sounds: Vec::new(),
            rules: Vec::new(),
            passphrase: None,
        }
    }

    fn view(you: u32, agreed: bool) -> LobbyView {
        LobbyView {
            you,
            rev: 0,
            level: "lvl".into(),
            rules: serde_json::json!({}),
            members: vec![
                Member {
                    id: 0,
                    wizard: 0,
                    name: String::new(),
                    models: serde_json::Value::Null,
                    agreed: true,
                },
                Member {
                    id: 1,
                    wizard: 2,
                    name: String::new(),
                    models: serde_json::Value::Null,
                    agreed,
                },
            ],
            capacity: 2,
            ai: 0,
            playing: false,
        }
    }

    const SIZE: (f32, f32) = (640.0, 480.0);

    fn click_btn(s: &mut LobbyScreen, b: Btn) {
        let r = BUTTONS.iter().find(|(x, ..)| *x == b).unwrap().1;
        s.click(SIZE, (r[0] + 1.0, r[1] + 1.0));
    }

    /// Ruling 7: only the host changes the level; his Play opens once
    /// everyone agreed. A client's Play is his agreement.
    #[test]
    fn the_buttons_answer_by_role() {
        let mut s = bare();
        s.update(&view(1, false));
        click_btn(&mut s, Btn::NextLevel);
        click_btn(&mut s, Btn::Play);
        assert_eq!(
            s.take_actions(),
            vec![LobbyAction::Play],
            "client: no level arrows"
        );
        s.update(&view(0, false));
        click_btn(&mut s, Btn::Play);
        click_btn(&mut s, Btn::PrevLevel);
        assert_eq!(
            s.take_actions(),
            vec![LobbyAction::PrevLevel],
            "host: Play waits"
        );
        s.update(&view(0, true));
        click_btn(&mut s, Btn::Play);
        assert_eq!(s.take_actions(), vec![LobbyAction::Play]);
    }

    /// Ruling 4: the host kicks by clicking a member's window, confirmed;
    /// a client's click on a window does nothing.
    #[test]
    fn the_host_kicks_through_a_confirmed_window_click() {
        let mut s = bare();
        s.update(&view(1, false));
        s.click(SIZE, (225.0, 65.0)); // window 1
        assert!(s.modal.is_none());
        s.update(&view(0, false));
        s.click(SIZE, (205.0, 65.0)); // window 0 = the host himself
        assert!(s.modal.is_none(), "not yourself");
        s.click(SIZE, (225.0, 65.0));
        assert!(matches!(&s.modal, Some(Modal::Kick { id: 1, name }) if name == "Rahn"));
        s.key_enter();
        assert_eq!(s.take_actions(), vec![LobbyAction::Kick(1)]);
    }

    /// The rules scroll: the host's first row is the passphrase (a
    /// dialog), the others step left/right; a client's clicks do nothing.
    #[test]
    fn the_host_works_the_rules_scroll() {
        let mut s = bare();
        let rows = vec![RuleRow {
            label: "Fog".into(),
            value: "40".into(),
        }];
        s.update(&view(1, false));
        s.set_rules(rows.clone(), None);
        let r = LobbyScreen::row_rect(0);
        s.click(SIZE, (r[0] + r[2] - 2.0, r[1] + 2.0));
        assert!(s.take_actions().is_empty(), "a client reads only");
        s.update(&view(0, false));
        s.set_rules(rows, Some("pw".into()));
        let r1 = LobbyScreen::row_rect(1);
        s.click(SIZE, (r1[0] + r1[2] - 2.0, r1[1] + 2.0));
        // The left chevron of the drawn `< 40 >` (no font here: 4 px a
        // character, 24 px in all) steps back.
        s.click(SIZE, (r1[0] + r1[2] - 22.0, r1[1] + 2.0));
        assert_eq!(
            s.take_actions(),
            vec![
                LobbyAction::StepRule {
                    row: 0,
                    forward: true
                },
                LobbyAction::StepRule {
                    row: 0,
                    forward: false
                },
            ]
        );
        s.click(SIZE, (r[0] + 2.0, r[1] + 2.0));
        assert!(s.editing());
        for c in "new pass:x".chars() {
            s.key_char(c);
        }
        s.key_enter();
        assert_eq!(
            s.take_actions(),
            vec![LobbyAction::SetPassphrase("newpassx".into())]
        );
        s.click(SIZE, (LIMIT_BOX[0] + 2.0, LIMIT_BOX[1] + 2.0));
        assert_eq!(s.take_actions(), vec![LobbyAction::CycleCapacity]);
    }

    /// The baked art: 8 portraits, every window slot's light AND dark
    /// mask found in the background, the scroll and cursor sprites.
    #[test]
    fn the_baked_lobby_loads() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../baked/assets/mc2-ui");
        if !dir.join("lobby-bg.bin").exists() {
            eprintln!("skipping: no baked lobby");
            return;
        }
        let s = LobbyScreen::load(&dir).unwrap();
        assert_eq!(s.portraits.len(), 8);
        for (k, w) in s.windows.iter().enumerate() {
            assert!(w[0].is_some() && w[1].is_some(), "window {k}");
        }
        assert!(s.rects.get(15).copied().flatten().is_some(), "cursor 15");
        assert!(s.menu.get(72).copied().flatten().is_some(), "scroll bar 72");
        let (w, h, px) = s.atlas();
        assert_eq!(px.len(), (w * h * 4) as usize);
    }

    #[test]
    fn portraits_map_to_seats_both_ways() {
        for p in 0..8 {
            assert_eq!(portrait_of_seat(PORTRAIT_SEAT[p]), p);
        }
        assert_eq!(PORTRAIT_SEAT[1], 2, "portrait 1 is Rahn");
        assert_eq!(mgc_sim::mc2::rivals::MC2_RIVAL_NAMES[2], "Rahn");
    }
}
