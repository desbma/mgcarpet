//! The MC1/HW multiplayer lobby screen — retail PMULTI (frontend
//! selector 4, `sub_4D680_4D9C0`; recon docs/traces/
//! mc1-multiplayer-lobby.md), a VIEW over the server-authoritative
//! lobby (`lobby.rs`, `net::Server`/`net::Guest`), like the MC2 one
//! (`lobby_mc2.rs`). Shared by both games (one `DATA/SCREENS` set);
//! only the arena list differs (MC1 levels 50-69, HW 50-59).
//!
//! Retail law reproduced: PMULTI.DAT over PMULTI.PAL, composed on the
//! CPU at 320×200 like the main menu; hard-coded hit rectangles (no
//! hotspot bitmap, no highlight); the 5-row map list in SFONT1 centred
//! in its viewport (173,72,90,61), the selected arena white (63,63,63),
//! the others brown (26,18,13), black outline; the list's up / down
//! strips scroll it; the eight seat icons at their shipped positions,
//! PMULTSPR [1+i] unused / [9+i] idle / [17+i] connected (blinking on
//! frame bit 0x10 while "trying"); START, EXIT, Esc = EXIT. The screen
//! is silent (retail stops the music, `sub_20E60`).
//!
//! The port's mapping onto its lobby (a first pass for the player to
//! assess, 2026-10-17 — the MC2 experience on retail's art):
//! - icon i = wizard seat i (MC1's colour IS the seat). A member's seat
//!   lights ("connected"), blinking until he agrees (the admin's always
//!   steady); a free seat is idle while the session has room, off once
//!   it is full. Clicking an idle seat takes it; the admin's click on an
//!   off seat opens one more place; his click on another member's seat
//!   kicks him (confirmed).
//! - the channel box (retail `CARPET0`..`9`) reads "Rules" and opens the
//!   rules page: the SCROLL.DAT parchment (MC1's dialog idiom) with the
//!   passphrase (the admin's), the player limit and the session rules —
//!   the admin steps them, the others read.
//! - the map list: only the admin picks (ruling 1); everyone sees the
//!   session's arena selected.
//! - START: the admin's starts once everyone agreed, everyone else's
//!   toggles his agreement (rulings 2, 7).
//! - a status line under the seats says what the lobby waits for, or
//!   whose seat the pointer is on.
//! - ABORT (retail: only while connecting) has nothing to abort here —
//!   the join happens before the lobby, on the main menu.

use std::path::Path;

use mgc_render::UiQuad;

use crate::frontend_mc1::Bank;
use crate::lobby::{LobbyView, WIZARDS};
use crate::lobby_mc2::{LobbyAction, RuleRow};

const W: usize = 320;
const H: usize = 200;

/// The seat icons' positions (binary CARPET file 0x62FC4 / HIDDEN
/// 0x63504; the decompile has only `{51}`, :2076).
const SEATS: [(i32, i32); 8] = [
    (51, 47),
    (75, 45),
    (100, 46),
    (126, 44),
    (50, 105),
    (77, 101),
    (102, 100),
    (129, 99),
];
/// The channel box: text viewport and click box (:59514-21, :59595-99).
const CHANNEL_VIEW: [i32; 4] = [173, 42, 90, 15];
const CHANNEL_HIT: [f32; 4] = [176.0, 46.0, 78.0, 13.0];
/// The map list's viewport (hover box x173-263 y72-133, :59438-47).
const LIST_VIEW: [i32; 4] = [173, 72, 90, 61];
const LIST_ROWS: usize = 5;
const LIST_UP: [f32; 4] = [173.0, 57.0, 90.0, 15.0];
const LIST_DOWN: [f32; 4] = [173.0, 134.0, 90.0, 15.0];
const START: [f32; 4] = [284.0, 126.0, 20.0, 20.0];
const EXIT: [f32; 4] = [236.0, 156.0, 21.0, 21.0];
/// The players' names (FONT1, 7 px): the top row's on two zig-zag
/// lines above it, the bottom row's on two below it (player design
/// 2026-10-17, MC2's lintel names on PMULTI's two rows).
const NAMES_ABOVE: [i32; 2] = [26, 35];
const NAMES_BELOW: [i32; 2] = [148, 157];
/// A name may reach halfway to the next name on its line (two icons on).
const NAME_ROOM: i32 = 48;
/// The status line: below the names, left of EXIT.
const STATUS_Y: i32 = 170;
const STATUS_X: (i32, i32) = (20, 232);

/// The rules page (the SCROLL.DAT parchment; the main menu's dialog
/// viewport (65,75,189,44) and OK / Cancel spots, :59141-66).
const PAGE_X: (i32, i32) = (86, 236);
const PAGE_TITLE_Y: i32 = 70;
const PAGE_ROW_Y: i32 = 80;
const PAGE_ROW_H: i32 = 9;

/// What the parchment page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Page {
    Rules,
    Kick { id: u32, name: String },
    Passphrase { buf: String },
}

pub struct Mc1LobbyScreen {
    bg: Vec<u8>,
    pal: [u8; 768],
    seats: Bank,
    sfont: Bank,
    /// The rules page: the SCROLL.DAT parchment under MAINMENU.PAL,
    /// FONT1 and the MMSPR OK / Cancel art (None on an older bake:
    /// a dark panel stands in).
    scroll_bg: Option<Vec<u8>>,
    menu_pal: [u8; 768],
    menu_font: Bank,
    menu_sprites: Bank,
    /// POINTERS [7], the menu's golden hand (MAINMENU.PAL).
    pointers: Option<Bank>,
    /// Arena names, levels 50.. (the EXE's `off_99A84`).
    arenas: Vec<String>,
    first_arena: u32,
    /// Inks: (white, brown, black) in PMULTI.PAL; (white, black) in
    /// MAINMENU.PAL.
    ink: (u8, u8, u8),
    /// Other players' names: gold in PMULTI.PAL (MC2's lintel ink).
    gold: u8,
    menu_ink: (u8, u8),
    view: Option<LobbyView>,
    /// The list's first shown row (retail `word_12CBBE`).
    scroll: usize,
    /// The arena last brought into view (a change scrolls to it).
    followed: Option<u32>,
    page: Option<Page>,
    rules: Vec<RuleRow>,
    passphrase: Option<String>,
    actions: Vec<LobbyAction>,
    alert: Option<(String, f32)>,
    anim: f32,
    screen: Vec<u8>,
}

fn nearest(pal: &[u8], r: u8, g: u8, b: u8) -> u8 {
    (0..256)
        .min_by_key(|&i| {
            let d = |k: usize, v: u8| (i32::from(pal[i * 3 + k]) - i32::from(v)).pow(2);
            d(0, r) + d(1, g) + d(2, b)
        })
        .unwrap_or(0) as u8
}

fn in_rect(mx: f32, my: f32, r: [f32; 4]) -> bool {
    mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
}

/// The level number of an arena path (`…/level-NNN.mgcl`).
fn level_of(path: &str) -> Option<u32> {
    let stem = Path::new(path).file_stem()?.to_str()?;
    stem.strip_prefix("level-")?.parse().ok()
}

impl Mc1LobbyScreen {
    /// Load PMULTI (+ the menu's parchment, FONT1, MMSPR and pointer)
    /// out of the baked `assets/mc1-ui` bundle; `hidden` picks HW's
    /// arena names.
    pub fn load(dir: &Path, hidden: bool) -> Result<Self, String> {
        let read = |name: &str| -> Result<Vec<u8>, String> {
            std::fs::read(dir.join(name)).map_err(|e| {
                format!(
                    "{}: {e} (rebake — epoch 29 adds it)",
                    dir.join(name).display()
                )
            })
        };
        let bg = read("pmulti-bg.bin")?;
        let pal: [u8; 768] = read("pmulti-pal.bin")?
            .try_into()
            .map_err(|_| "pmulti-pal.bin: wrong size")?;
        if bg.len() != W * H {
            return Err("pmulti-bg.bin: wrong size".into());
        }
        let menu_pal: [u8; 768] = read("menu-pal.bin")?
            .try_into()
            .map_err(|_| "menu-pal.bin: wrong size")?;
        let scroll_bg = std::fs::read(dir.join("scroll-bg.bin"))
            .ok()
            .filter(|b| b.len() == W * H);
        let key = if hidden { "mc1hw" } else { "mc1" };
        let arenas: Vec<String> = std::fs::read(dir.join("arena-names.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| serde_json::from_value(v.get(key)?.clone()).ok())
            .unwrap_or_default();
        let count = if hidden { 10 } else { 20 };
        let arenas = (0..count)
            .map(|k| {
                arenas
                    .get(k)
                    .cloned()
                    .unwrap_or_else(|| format!("Arena {}", k + 1))
            })
            .collect();
        Ok(Self {
            seats: Bank::load(dir, "pmulti-sprites.bin", "pmulti-sprites.json")?,
            sfont: Bank::load(dir, "sfont1.bin", "sfont1.json")?,
            menu_font: Bank::load(dir, "font.bin", "font.json")?,
            menu_sprites: Bank::load(dir, "menu-sprites.bin", "menu-sprites.json")?,
            pointers: Bank::load(dir, "pointer-sprites.bin", "pointer-sprites.json").ok(),
            ink: (
                nearest(&pal, 63, 63, 63),
                nearest(&pal, 26, 18, 13),
                nearest(&pal, 0, 0, 0),
            ),
            menu_ink: (nearest(&menu_pal, 63, 63, 63), nearest(&menu_pal, 0, 0, 0)),
            gold: nearest(&pal, 60, 54, 35),
            bg,
            pal,
            scroll_bg,
            menu_pal,
            arenas,
            first_arena: 50,
            view: None,
            scroll: 0,
            followed: None,
            page: None,
            rules: Vec::new(),
            passphrase: None,
            actions: Vec::new(),
            alert: None,
            anim: 0.0,
            screen: vec![0; W * H],
        })
    }

    /// The lobby as it stands now (every frame, from the session). A new
    /// session arena scrolls the list to it.
    pub fn update(&mut self, view: &LobbyView) {
        if self.view.as_ref() != Some(view) {
            self.view = Some(view.clone());
        }
        let level = level_of(&view.level);
        if level != self.followed {
            self.followed = level;
            if let Some(row) = self.selected_row() {
                if row < self.scroll {
                    self.scroll = row;
                } else if row >= self.scroll + LIST_ROWS {
                    self.scroll = row + 1 - LIST_ROWS;
                }
            }
        }
    }

    /// The rules page's content: the session's rule rows (the player
    /// limit first), and (admin) the passphrase.
    pub fn set_rules(&mut self, rows: Vec<RuleRow>, passphrase: Option<String>) {
        self.rules = rows;
        self.passphrase = passphrase;
    }

    pub fn tick(&mut self, dt: f32) {
        self.anim += dt;
        if let Some((_, t)) = &mut self.alert {
            *t -= dt;
            if *t <= 0.0 {
                self.alert = None;
            }
        }
    }

    pub fn take_actions(&mut self) -> Vec<LobbyAction> {
        std::mem::take(&mut self.actions)
    }

    /// Show `text` over the status line for a few seconds.
    pub fn alert(&mut self, text: String) {
        self.alert = Some((text, 5.0));
    }

    /// A text field is open (the passphrase).
    pub fn editing(&self) -> bool {
        matches!(self.page, Some(Page::Passphrase { .. }))
    }

    /// Esc: close the page, else EXIT (retail `:59432-36`).
    pub fn escape(&mut self) {
        if self.page.take().is_none() {
            self.actions.push(LobbyAction::Exit);
        }
    }

    pub fn key_char(&mut self, c: char) {
        if let Some(Page::Passphrase { buf }) = &mut self.page
            && buf.len() < 24
            && (c.is_ascii_alphanumeric() || "-_.!?".contains(c))
        {
            buf.push(c);
        }
    }

    pub fn key_backspace(&mut self) {
        if let Some(Page::Passphrase { buf }) = &mut self.page {
            buf.pop();
        }
    }

    pub fn key_enter(&mut self) {
        self.press_ok();
    }

    fn is_admin(&self) -> bool {
        self.view.as_ref().is_some_and(LobbyView::is_admin)
    }

    /// A game runs without us: only EXIT is live.
    fn waiting(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.playing)
    }

    /// The session arena's list row.
    fn selected_row(&self) -> Option<usize> {
        let n = self.followed?;
        let row = n.checked_sub(self.first_arena)? as usize;
        (row < self.arenas.len()).then_some(row)
    }

    /// The seat icon under the point.
    fn seat_at(&self, mx: f32, my: f32) -> Option<u8> {
        (0..WIZARDS).find(|&i| {
            let (x, y) = SEATS[usize::from(i)];
            let (w, h) = self.seats.size_of(9 + usize::from(i)).unwrap_or((24, 44));
            in_rect(mx, my, [x as f32, y as f32, w as f32, h as f32])
        })
    }

    /// Who holds seat `i` (member id, agreed).
    fn holder(&self, i: u8) -> Option<(u32, bool)> {
        self.view
            .as_ref()?
            .members
            .iter()
            .find(|m| m.wizard == i)
            .map(|m| (m.id, m.agreed))
    }

    /// The session has room for another player.
    fn has_room(&self) -> bool {
        self.view
            .as_ref()
            .is_some_and(|v| v.members.len() < usize::from(v.capacity))
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
            mgc_sim::mc1::rivals::RIVAL_NAMES[usize::from(m.wizard.min(WIZARDS - 1))].to_string()
        } else {
            m.name.clone()
        }
    }

    /// The rules page's rows as drawn: the passphrase (admin only), the
    /// player limit, then the session rules.
    fn rows(&self) -> Vec<RuleRow> {
        let mut rows = Vec::new();
        if let Some(p) = &self.passphrase {
            rows.push(RuleRow {
                label: "Passphrase".into(),
                // Empty = an open session (anyone gets in).
                value: if p.is_empty() {
                    "none".into()
                } else {
                    p.clone()
                },
            });
        }
        if let Some(v) = &self.view {
            rows.push(RuleRow {
                label: "Players".into(),
                value: format!("{} of {}", v.members.len(), v.capacity),
            });
        }
        rows.extend(self.rules.iter().cloned());
        rows
    }

    fn row_rect(k: usize) -> [f32; 4] {
        [
            PAGE_X.0 as f32,
            (PAGE_ROW_Y + PAGE_ROW_H * k as i32) as f32,
            (PAGE_X.1 - PAGE_X.0) as f32,
            PAGE_ROW_H as f32,
        ]
    }

    /// OK on the page: commit the kick / passphrase, close the rules.
    fn press_ok(&mut self) {
        match self.page.take() {
            Some(Page::Kick { id, .. }) => self.actions.push(LobbyAction::Kick(id)),
            Some(Page::Passphrase { buf }) => self.actions.push(LobbyAction::SetPassphrase(buf)),
            Some(Page::Rules) | None => {}
        }
    }

    pub fn click(&mut self, size: (f32, f32), cursor: (f32, f32)) {
        let (mx, my) = crate::ui::unletterbox(cursor, size, W as f32, H as f32);
        if let Some(page) = &self.page {
            let ok = (68.0..=81.0).contains(&mx) && (106.0..=116.0).contains(&my);
            let cancel = (240.0..=250.0).contains(&mx) && (105.0..=115.0).contains(&my);
            if ok {
                self.press_ok();
            } else if cancel {
                self.page = None;
            } else if *page == Page::Rules && self.is_admin() {
                self.rules_click(mx, my);
            }
            return;
        }
        if in_rect(mx, my, EXIT) {
            self.actions.push(LobbyAction::Exit);
            return;
        }
        if self.waiting() {
            return;
        }
        if in_rect(mx, my, CHANNEL_HIT) {
            self.page = Some(Page::Rules);
        } else if in_rect(mx, my, START) {
            let Some(v) = &self.view else { return };
            if v.is_admin() && !v.all_agreed() {
                self.alert(self.status());
            } else {
                self.actions.push(LobbyAction::Play);
            }
        } else if in_rect(mx, my, LIST_UP) {
            self.scroll = self.scroll.saturating_sub(1);
        } else if in_rect(mx, my, LIST_DOWN) {
            if self.scroll + LIST_ROWS < self.arenas.len() {
                self.scroll += 1;
            }
        } else if in_rect(mx, my, LIST_VIEW.map(|v| v as f32)) {
            let row = ((my - LIST_VIEW[1] as f32) / self.row_pitch() as f32) as usize;
            let n = self.scroll + row.min(LIST_ROWS - 1);
            if self.is_admin() && n < self.arenas.len() {
                self.actions
                    .push(LobbyAction::SetArena(self.first_arena + n as u32));
            }
        } else if let Some(i) = self.seat_at(mx, my) {
            self.seat_click(i);
        }
    }

    fn seat_click(&mut self, i: u8) {
        let Some(v) = &self.view else { return };
        let admin = v.is_admin();
        match self.holder(i) {
            Some((id, _)) if admin && id != v.you => {
                let name = self.member_name(id);
                self.page = Some(Page::Kick { id, name });
            }
            Some(_) => {}
            // A seat the arena has no start for.
            None if !v.is_open(i) => {}
            // An off seat (the session is full): the admin opens one
            // more place (retail's seat click sets the player count).
            None if admin && !self.has_room() => self
                .actions
                .push(LobbyAction::SetCapacity(usize::from(v.capacity) + 1)),
            // A free seat: take it.
            None => self.actions.push(LobbyAction::PickWizard(i)),
        }
    }

    /// The admin's click on the rules page: the passphrase row opens its
    /// field, every other row steps (left half back, right half on).
    fn rules_click(&mut self, mx: f32, my: f32) {
        let rows = self.rows();
        let Some(k) = (0..rows.len()).find(|&k| in_rect(mx, my, Self::row_rect(k))) else {
            return;
        };
        let pass = usize::from(self.passphrase.is_some());
        if pass == 1 && k == 0 {
            self.page = Some(Page::Passphrase { buf: String::new() });
            return;
        }
        let r = Self::row_rect(k);
        let w = self.menu_text_width(&format!("< {} >", rows[k].value)) as f32;
        let forward = mx >= r[0] + r[2] - w / 2.0;
        if k == pass {
            let Some(v) = &self.view else { return };
            let cap = usize::from(v.capacity);
            let next = if forward {
                cap + 1
            } else {
                cap.saturating_sub(1)
            };
            self.actions.push(LobbyAction::SetCapacity(next));
        } else {
            self.actions.push(LobbyAction::StepRule {
                row: k - pass - 1,
                forward,
            });
        }
    }

    /// The status line: what the lobby waits for.
    fn status(&self) -> String {
        let Some(v) = &self.view else {
            return String::new();
        };
        if v.playing {
            return "Waiting for the game to finish".into();
        }
        if !v.fits() {
            return "Too many players for this arena".into();
        }
        let (others, agreed) = v.agreement();
        if v.is_admin() {
            if others == 0 {
                "Waiting for players to join".into()
            } else if v.all_agreed() {
                "Everyone agreed - press START".into()
            } else {
                format!("Waiting for agreement ({agreed} of {others})")
            }
        } else if v.me().is_some_and(|m| m.agreed) {
            "Agreed - waiting for the host".into()
        } else {
            "Press START to agree".into()
        }
    }

    /// SFONT1's row pitch in the list (`var8_5 - 2`, :59750).
    fn row_pitch(&self) -> i32 {
        self.sfont
            .size_of('A' as usize - 31)
            .map_or(12, |(_, h)| h as i32 - 2)
    }

    /// SFONT1 pen width (retail advances width − 1, `DrawText_51560`).
    fn text_width(&self, s: &str) -> i32 {
        s.chars()
            .map(|c| {
                self.sfont
                    .size_of((c as usize).wrapping_sub(31))
                    .map_or(4, |(w, _)| w as i32 - 1)
            })
            .sum()
    }

    fn text(&self, s: &str, x: i32, y: i32, body: u8, clip: [i32; 4], buf: &mut [u8]) {
        let clip = (clip[0], clip[1], clip[0] + clip[2], clip[1] + clip[3]);
        let mut cx = x;
        for c in s.chars() {
            let id = (c as usize).wrapping_sub(31);
            if c != ' ' {
                self.sfont
                    .blit_inks(id, cx, y, &[body, self.ink.2], clip, buf);
            }
            cx += self.sfont.size_of(id).map_or(4, |(w, _)| w as i32 - 1);
        }
    }

    /// `s` cut to `room` px.
    fn fit(&self, s: &str, room: i32) -> String {
        let mut s = s.to_string();
        while self.text_width(&s) > room && s.pop().is_some() {}
        s
    }

    fn menu_text_width(&self, s: &str) -> usize {
        s.chars()
            .map(|c| self.menu_font.width_of((c as usize).wrapping_add(1)))
            .sum()
    }

    fn menu_text(&self, s: &str, x: i32, y: i32, color: u8, buf: &mut [u8]) {
        let mut cx = x;
        for c in s.chars() {
            let id = (c as usize).wrapping_add(1);
            if c != ' ' {
                self.menu_font.blit_mask(id, cx, y, color, buf);
            }
            cx += self.menu_font.width_of(id) as i32;
        }
    }

    /// The lobby screen into `buf` (PMULTI.PAL indices).
    fn compose_lobby(&self, mx: f32, my: f32, buf: &mut [u8]) {
        buf.copy_from_slice(&self.bg);
        let blink = (self.anim * 70.0) as u32 & 0x10 != 0;
        let full = !self.has_room();
        for i in 0..WIZARDS {
            let (x, y) = SEATS[usize::from(i)];
            let k = usize::from(i);
            let spr = match self.holder(i) {
                Some((id, agreed)) => {
                    let steady = agreed || self.view.as_ref().and_then(|v| v.admin) == Some(id);
                    if steady || blink { 17 + k } else { 9 + k }
                }
                None if full || !self.view.as_ref().is_some_and(|v| v.is_open(i)) => 1 + k,
                None => 9 + k,
            };
            self.seats.blit(spr, x, y, buf);
        }
        self.seat_names(buf);
        // The channel box: the rules page's door.
        let label = "Rules";
        let [cx, cy, cw, _] = CHANNEL_VIEW;
        let hot = in_rect(mx, my, CHANNEL_HIT) && !self.waiting();
        let ink = if hot { self.ink.0 } else { self.ink.1 };
        self.text(
            label,
            cx + (cw - self.text_width(label)) / 2,
            cy,
            ink,
            CHANNEL_VIEW,
            buf,
        );
        // The map list (:59729-50).
        let selected = self.selected_row();
        let pitch = self.row_pitch();
        for (k, name) in self
            .arenas
            .iter()
            .enumerate()
            .skip(self.scroll)
            .take(LIST_ROWS)
        {
            let y = LIST_VIEW[1] + pitch * (k - self.scroll) as i32;
            let ink = if Some(k) == selected {
                self.ink.0
            } else {
                self.ink.1
            };
            let x = LIST_VIEW[0] + (LIST_VIEW[2] - self.text_width(name)) / 2;
            self.text(name, x, y, ink, LIST_VIEW, buf);
        }
        // The status line (or the hovered seat's holder).
        let hovered = self
            .seat_at(mx, my)
            .and_then(|i| self.holder(i).map(|(id, agreed)| (i, id, agreed)));
        let text = match (&self.alert, hovered) {
            (Some((t, _)), _) => t.clone(),
            (None, Some((_, id, agreed))) => {
                let admin = self.view.as_ref().and_then(|v| v.admin) == Some(id);
                let tag = if admin {
                    " (host)"
                } else if agreed {
                    " (agreed)"
                } else {
                    ""
                };
                format!("{}{tag}", self.member_name(id))
            }
            (None, None) => self.status(),
        };
        let room = STATUS_X.1 - STATUS_X.0;
        let text = self.fit(&text, room);
        let x = STATUS_X.0 + (room - self.text_width(&text)) / 2;
        let clip = [STATUS_X.0, STATUS_Y, room, H as i32 - STATUS_Y];
        self.text(&text, x, STATUS_Y, self.ink.0, clip, buf);
    }

    /// Each held seat's player name, centred on its icon: top-row seats
    /// on the two lines above (even columns the upper), bottom-row seats
    /// on the two below (even columns the nearer); cut to fit; yours in
    /// white, outlined in black.
    fn seat_names(&self, buf: &mut [u8]) {
        let Some(v) = &self.view else { return };
        for m in &v.members {
            let i = usize::from(m.wizard.min(WIZARDS - 1));
            let (x, _) = SEATS[i];
            let (w, _) = self.seats.size_of(9 + i).unwrap_or((24, 44));
            let col = i % 4;
            let y = if i < 4 {
                NAMES_ABOVE[col % 2]
            } else {
                NAMES_BELOW[col % 2]
            };
            let mut name = self.member_name(m.id);
            while self.menu_text_width(&name) as i32 > NAME_ROOM && name.pop().is_some() {}
            let nx = x + w as i32 / 2 - self.menu_text_width(&name) as i32 / 2;
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                self.menu_text(&name, nx + dx, y + dy, self.ink.2, buf);
            }
            let ink = if m.id == v.you { self.ink.0 } else { self.gold };
            self.menu_text(&name, nx, y, ink, buf);
        }
    }

    /// The parchment page into `buf` (MAINMENU.PAL indices).
    fn compose_page(&self, page: &Page, buf: &mut [u8]) {
        match &self.scroll_bg {
            Some(s) => buf.copy_from_slice(s),
            None => buf.fill(0),
        }
        let (white, _) = self.menu_ink;
        let centred = |s: &str| 65 + (189 - self.menu_text_width(s) as i32) / 2;
        match page {
            Page::Rules => {
                let title = "Session rules";
                self.menu_text(title, centred(title), PAGE_TITLE_Y, white, buf);
                let admin = self.is_admin();
                for (k, row) in self.rows().iter().enumerate() {
                    let r = Self::row_rect(k);
                    let y = r[1] as i32;
                    self.menu_text(&row.label, PAGE_X.0, y, white, buf);
                    let value = if admin {
                        format!("< {} >", row.value)
                    } else {
                        row.value.clone()
                    };
                    let x = PAGE_X.1 - self.menu_text_width(&value) as i32;
                    self.menu_text(&value, x, y, white, buf);
                }
            }
            Page::Kick { name, .. } => {
                let title = format!("Kick {name}?");
                self.menu_text(&title, centred(&title), 92, white, buf);
            }
            Page::Passphrase { buf: edit } => {
                let title = "New passphrase";
                self.menu_text(title, centred(title), 78, white, buf);
                let mut shown = edit.clone();
                if (self.anim * 4.0) as u32 % 2 == 0 {
                    shown.push('_');
                }
                self.menu_text(&shown, 112, 90, white, buf);
            }
        }
        // OK (MMSPR 5); Cancel (6) on the dialogs that can decline.
        self.menu_sprites.blit(5, 66, 104, buf);
        if *page != Page::Rules {
            self.menu_sprites.blit(6, 238, 104, buf);
        }
    }

    /// Compose + resolve this frame: the RGBA screen (re-uploaded as the
    /// UI atlas) and its letterboxed quad.
    pub fn frame(&mut self, size: (f32, f32), cursor: (f32, f32)) -> (Vec<u8>, Vec<UiQuad>) {
        let (scale, ox, oy) = crate::ui::letterbox(size, W as f32, H as f32);
        let (mx, my) = crate::ui::unletterbox(cursor, size, W as f32, H as f32);
        let mut buf = std::mem::take(&mut self.screen);
        let pal = match &self.page {
            Some(p) => {
                self.compose_page(p, &mut buf);
                self.menu_pal
            }
            None => {
                self.compose_lobby(mx, my, &mut buf);
                self.pal
            }
        };
        let mut rgba = vec![0u8; W * H * 4];
        for (i, &idx) in buf.iter().enumerate() {
            let idx = usize::from(idx);
            rgba[i * 4..i * 4 + 4].copy_from_slice(&[
                pal[idx * 3] << 2,
                pal[idx * 3 + 1] << 2,
                pal[idx * 3 + 2] << 2,
                255,
            ]);
        }
        // The pointer: the menu's golden hand, through its own palette
        // (retail's PMULTI pointer, SPTRS [4], is not baked).
        if let Some(p) = &self.pointers {
            let mut hand = vec![0u8; W * H];
            p.blit(7, mx as i32, my as i32, &mut hand);
            for (i, &idx) in hand.iter().enumerate().filter(|(_, v)| **v != 0) {
                let idx = usize::from(idx);
                rgba[i * 4..i * 4 + 3].copy_from_slice(&[
                    self.menu_pal[idx * 3] << 2,
                    self.menu_pal[idx * 3 + 1] << 2,
                    self.menu_pal[idx * 3 + 2] << 2,
                ]);
            }
        }
        self.screen = buf;
        let quads = vec![UiQuad {
            rect: [ox, oy, W as f32 * scale, H as f32 * scale],
            uv: [0.0, 0.0, W as f32, H as f32],
            tint: [1.0; 4],
        }];
        (rgba, quads)
    }

    /// The pointer is composed into the frame (no OS pointer needed).
    pub fn has_pointer(&self) -> bool {
        self.pointers.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::Member;

    fn screen() -> Option<Mc1LobbyScreen> {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../baked/assets/mc1-ui");
        if !dir.join("pmulti-bg.bin").exists() {
            eprintln!("skipping: no baked PMULTI");
            return None;
        }
        Some(Mc1LobbyScreen::load(&dir, false).unwrap())
    }

    fn member(id: u32, wizard: u8, agreed: bool) -> Member {
        Member {
            id,
            wizard,
            name: String::new(),
            models: serde_json::Value::Null,
            agreed,
        }
    }

    fn view(you: u32, capacity: u8) -> LobbyView {
        LobbyView {
            you,
            rev: 0,
            game: crate::lobby::NetGame::Mc1,
            level: "baked/mc1/level-052.mgcl".into(),
            rules: serde_json::json!({}),
            members: vec![member(0, 0, true), member(1, 3, false)],
            admin: Some(0),
            passphrase: None,
            capacity,
            ai: 0,
            playing: false,
            open: u8::MAX,
        }
    }

    const SIZE: (f32, f32) = (320.0, 200.0);

    fn click_seat(s: &mut Mc1LobbyScreen, i: usize) {
        let (x, y) = SEATS[i];
        s.click(SIZE, (x as f32 + 2.0, y as f32 + 2.0));
    }

    /// Stills for the player's review (`sprite-export/mc1-lobby/`,
    /// 2× nearest): the lobby as a client sees a 5-of-6 session (names
    /// zig-zag above and below the rows), and the host's rules page. Run with
    /// `cargo test -p mgc-app --lib -- --ignored lobby_mc1_stills`.
    #[test]
    #[ignore]
    fn lobby_mc1_stills() {
        let Some(mut s) = screen() else { return };
        let out =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sprite-export/mc1-lobby");
        std::fs::create_dir_all(&out).unwrap();
        let mut v = view(1, 6);
        v.members.push(member(2, 6, true));
        v.members.push(member(3, 2, false));
        v.members.push(member(4, 5, true));
        v.members[0].name = "RAIN".into();
        v.members[4].name = "SCHEHERAZADE".into();
        s.update(&v);
        s.set_rules(
            vec![
                RuleRow {
                    label: "Fog".into(),
                    value: "90".into(),
                },
                RuleRow {
                    label: "AI wizards".into(),
                    value: "0".into(),
                },
            ],
            None,
        );
        let save = |name: &str, rgba: &[u8]| {
            let mut big = vec![0u8; W * H * 16];
            for y in 0..H * 2 {
                for x in 0..W * 2 {
                    let src = ((y / 2) * W + x / 2) * 4;
                    big[(y * W * 2 + x) * 4..(y * W * 2 + x) * 4 + 4]
                        .copy_from_slice(&rgba[src..src + 4]);
                }
            }
            crate::write_png(&out.join(name), (W * 2) as u32, (H * 2) as u32, &big).unwrap();
        };
        // The pointer over the session's first seat (its name shows).
        let (rgba, _) = s.frame(SIZE, (SEATS[0].0 as f32 + 4.0, SEATS[0].1 as f32 + 8.0));
        save("lobby-client.png", &rgba);
        v.you = 0;
        s.update(&v);
        s.set_rules(s.rules.clone(), Some("letmein".into()));
        s.page = Some(Page::Rules);
        let (rgba, _) = s.frame(SIZE, (300.0, 190.0));
        save("rules-host.png", &rgba);
    }

    /// The baked art, both arena lists and the inks.
    #[test]
    fn the_baked_lobby_loads() {
        let Some(s) = screen() else { return };
        assert_eq!(s.arenas.len(), 20);
        assert_eq!(s.arenas[0], "Bussorah");
        assert_eq!(s.arenas[19], "Comari");
        for k in 1..=24 {
            assert!(s.seats.size_of(k).is_some(), "seat sprite {k}");
        }
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../baked/assets/mc1-ui");
        let hw = Mc1LobbyScreen::load(&dir, true).unwrap();
        assert_eq!(hw.arenas.len(), 10);
        assert_eq!(hw.arenas[8], "Rama'Q");
        assert_eq!(s.row_pitch(), 12, "SFONT1 14 px − 2 (:59750)");
    }

    /// Seats: a client takes a free seat; the admin kicks through a
    /// confirmed click on another's seat, and opens a place by clicking
    /// an off seat once the session is full (a free seat takes him
    /// there while it has room).
    #[test]
    fn the_seats_answer_by_role() {
        let Some(mut s) = screen() else { return };
        s.update(&view(1, 4));
        click_seat(&mut s, 5);
        click_seat(&mut s, 0);
        assert_eq!(s.take_actions(), vec![LobbyAction::PickWizard(5)]);
        assert!(s.page.is_none(), "a client kicks no one");
        s.update(&view(0, 2));
        click_seat(&mut s, 3);
        assert!(matches!(&s.page, Some(Page::Kick { id: 1, name }) if name == "Mahmoud"));
        s.key_enter();
        assert_eq!(s.take_actions(), vec![LobbyAction::Kick(1)]);
        click_seat(&mut s, 6);
        assert_eq!(s.take_actions(), vec![LobbyAction::SetCapacity(3)]);
    }

    /// The list: the admin picks an arena; a client's click only
    /// scrolls; a new session arena scrolls into view.
    #[test]
    fn the_map_list_is_the_admins() {
        let Some(mut s) = screen() else { return };
        s.update(&view(1, 2));
        let row_y = |k: i32| (LIST_VIEW[1] + 12 * k + 3) as f32;
        s.click(SIZE, (200.0, row_y(1)));
        assert!(s.take_actions().is_empty());
        s.update(&view(0, 2));
        s.click(SIZE, (200.0, row_y(4)));
        assert_eq!(s.take_actions(), vec![LobbyAction::SetArena(54)]);
        let mut far = view(1, 2);
        far.level = "baked/mc1/level-068.mgcl".into();
        s.update(&far);
        assert_eq!(s.selected_row(), Some(18));
        assert!((s.scroll..s.scroll + LIST_ROWS).contains(&18));
    }

    /// START: the admin's waits for everyone; a client's agrees. The
    /// rules page: the admin steps rows, a client reads.
    #[test]
    fn start_and_the_rules_page() {
        let Some(mut s) = screen() else { return };
        s.update(&view(0, 2));
        s.click(SIZE, (START[0] + 2.0, START[1] + 2.0));
        assert!(s.take_actions().is_empty(), "not everyone agreed");
        s.update(&view(1, 2));
        s.click(SIZE, (START[0] + 2.0, START[1] + 2.0));
        assert_eq!(s.take_actions(), vec![LobbyAction::Play]);
        let rows = vec![RuleRow {
            label: "Fog".into(),
            value: "40".into(),
        }];
        s.set_rules(rows.clone(), None);
        s.click(SIZE, (CHANNEL_HIT[0] + 2.0, CHANNEL_HIT[1] + 2.0));
        assert_eq!(s.page, Some(Page::Rules));
        let r = Mc1LobbyScreen::row_rect(0);
        s.click(SIZE, (r[0] + r[2] - 1.0, r[1] + 2.0));
        assert!(s.take_actions().is_empty(), "a client reads only");
        s.escape();
        assert!(s.page.is_none() && s.take_actions().is_empty());
        s.update(&view(0, 2));
        s.set_rules(rows, Some("pw".into()));
        s.page = Some(Page::Rules);
        let r1 = Mc1LobbyScreen::row_rect(1);
        let r2 = Mc1LobbyScreen::row_rect(2);
        s.click(SIZE, (r1[0] + r1[2] - 1.0, r1[1] + 2.0));
        s.click(SIZE, (r2[0] + r2[2] - 1.0, r2[1] + 2.0));
        assert_eq!(
            s.take_actions(),
            vec![
                LobbyAction::SetCapacity(3),
                LobbyAction::StepRule {
                    row: 0,
                    forward: true
                }
            ]
        );
        s.click(SIZE, (r[0] + 2.0, r[1] + 2.0));
        assert!(s.editing());
        for c in "new:x".chars() {
            s.key_char(c);
        }
        s.key_enter();
        assert_eq!(
            s.take_actions(),
            vec![LobbyAction::SetPassphrase("newx".into())]
        );
        s.escape();
        assert_eq!(s.take_actions(), vec![LobbyAction::Exit]);
    }
}
