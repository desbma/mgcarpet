//! The MC2 main menu — the retail "temple" frontend screen
//! (`MainMenu_76FA0`, MenusAndIntros.cpp:816; HSCREEN0 case-4 art;
//! docs/traces/mc2-campaign-save-menu.md "Main menu" recon).
//!
//! Retail law reproduced:
//! - 640×480 temple background with its own palette; button art is
//!   part of the background — hovering draws the lit sprite over it
//!   (the `byte_21` index, MI:2035-38), clicking plays sample 14.
//! - Idle dressing: two fires (sprites 1-8 / 9-16) and two incense
//!   burners (17-25 / 26-34) at ~25 fps (step every 4 ticks of the
//!   100 Hz clock); sprite 66 blitted permanently at (185,232)
//!   (MI:897).
//! - Cursor = case-4 sprite 39.
//! - Dialogs = the parchment scroll (strip sprite 72, OK 70/71,
//!   Cancel 68/69 — the case-4 `x_WORD_17DF06..0E` registry), same
//!   16-px-step opening as the map screen's.
//! - Name entry (`SetPlayerNameDialog_78E00` MI:4799): chars
//!   space/0-9/letters, uppercased, max 12, blinking `_` caret.
//! - Save/Load slot dialogs: 8 rows at 16-px pitch, "Empty" for
//!   vacant slots, Save rows editable (max 15).
//!
//! Buttons wired: New Game (enter the map — the campaign RESET lives
//! on the map's corner button, retail law), Set Name, Save, Load,
//! Exit. Multiplayer / Set Keys / Language / Joystick draw their
//! hover art but report as not-in-this-remake when clicked.

use std::path::Path;

use mgc_render::UiQuad;

/// One frontend action, drained by the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// New Game / the door — enter the world-map screen.
    EnterMap,
    /// Write the campaign to a slot. Carries NO label: slot names are
    /// not player-authored — see `MenuAction::SetName`.
    SaveTo {
        slot: usize,
    },
    LoadFrom(usize),
    SetName(String),
    /// Multiplayer → Host: open a lobby on `port`.
    Host {
        port: String,
        passphrase: String,
    },
    /// Multiplayer → Join: connect to `addr` on `port` (empty: the
    /// default; an `addr` that names its own `host:port` wins).
    Join {
        addr: String,
        port: String,
        passphrase: String,
    },
    Quit,
}

/// The frontend click sample (MI:5858).
const SND_CLICK: u8 = 14;

/// OK / Cancel positions+hit rects (retail DrawScrollDialog2 mode 3:
/// OK at x1+15, Cancel right-aligned to x1+barW-12, bottoms on
/// y1+height).
fn ok_rect(x1: f32, y1: f32, h: f32) -> (f32, f32, f32, f32) {
    (x1 + 15.0, y1 + h - 28.0, 42.0, 28.0)
}
fn cancel_rect(x1: f32, y1: f32, h: f32) -> (f32, f32, f32, f32) {
    (x1 + DIALOG_W - 12.0 - 39.0, y1 + h - 30.0, 39.0, 30.0)
}
/// The network scroll's clickable rows: Host, Join, up to three
/// fields ([`net_fields`]), two recent hosts.
fn net_rows(x1: f32, y1: f32) -> [(f32, f32, f32, f32); 7] {
    [
        (x1 + 40.0, y1 + 30.0, 50.0, 14.0),
        (x1 + 120.0, y1 + 30.0, 50.0, 14.0),
        (x1 + 15.0, y1 + 48.0, 190.0, 14.0),
        (x1 + 15.0, y1 + 63.0, 190.0, 14.0),
        (x1 + 15.0, y1 + 78.0, 190.0, 14.0),
        (x1 + 15.0, y1 + 98.0, 190.0, 14.0),
        (x1 + 15.0, y1 + 112.0, 190.0, 14.0),
    ]
}

/// A network scroll entry field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NetField {
    /// Join: the host's name or address.
    Addr,
    /// Host: the port to listen on.
    Port,
    /// Join: the host's port — its own field (player 2026-10-17: few
    /// know the `host:port` form).
    JoinPort,
    Pass,
}

/// The fields of each face, top to bottom (`focus` indexes these).
fn net_fields(host: bool) -> &'static [NetField] {
    if host {
        &[NetField::Port, NetField::Pass]
    } else {
        &[NetField::Addr, NetField::JoinPort, NetField::Pass]
    }
}

/// A recent host's (address, port) — stored as `host:port`; a
/// bracketed IPv6 keeps its brackets.
pub(crate) fn split_host_port(a: &str) -> (String, String) {
    match a.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
            (h.to_string(), p.to_string())
        }
        _ => (a.to_string(), String::new()),
    }
}

fn in_rect(mx: f32, my: f32, r: (f32, f32, f32, f32)) -> bool {
    mx >= r.0 && mx < r.0 + r.2 && my >= r.1 && my < r.1 + r.3
}

/// Screen dimensions (retail 640×480).
const W: f32 = 640.0;
const H: f32 = 480.0;
/// The parchment strip width (sprite 72).
const DIALOG_W: f32 = 114.0;

/// The nine temple buttons (`str_E1BAC`, MI:334-44): position,
/// hit-box, the hover sprite (`byte_21` — retail DrawMenuAnimations
/// draws this index for the hovered button), and what a click does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    NewGame,
    SetName,
    Multiplayer,
    Save,
    SetKeys,
    Load,
    Exit,
    Language,
    Joystick,
}

const BUTTONS: [(Button, (f32, f32), (f32, f32), usize); 9] = [
    (Button::NewGame, (206.0, 67.0), (80.0, 80.0), 51),
    (Button::SetName, (281.0, 65.0), (80.0, 80.0), 52),
    (Button::Multiplayer, (362.0, 72.0), (80.0, 80.0), 53),
    (Button::Save, (200.0, 157.0), (80.0, 80.0), 54),
    (Button::SetKeys, (405.0, 231.0), (60.0, 44.0), 106),
    (Button::Load, (391.0, 158.0), (80.0, 80.0), 55),
    (Button::Exit, (294.0, 25.0), (52.0, 44.0), 56),
    (Button::Language, (289.0, 155.0), (60.0, 44.0), 57),
    (Button::Joystick, (185.0, 232.0), (60.0, 44.0), 58),
];

/// The idle set dressing (MI:443-48): (x, y, first, last).
const ANIMS: [(f32, f32, usize, usize); 4] = [
    (17.0, 159.0, 1, 8),    // left fire
    (531.0, 156.0, 9, 16),  // right fire
    (154.0, 308.0, 17, 25), // left incense
    (482.0, 308.0, 26, 34), // right incense
];

enum Modal {
    /// Name entry ((356,112) h 80, title 420).
    Name { buf: String },
    /// Slot picker; `save` = editable rows (else Load).
    Slots {
        save: bool,
        open: f32,
        slots: Vec<(String, bool)>,
        selected: Option<usize>,
        edit: Option<String>,
    },
    /// Exit confirm ((352,26) h 80, title 407 "Exit").
    ExitConfirm,
    /// Multiplayer: retail's "Enter Network Session" scroll (title 412)
    /// widened for the port's entry — Host / Join, the address (the
    /// port when hosting), the passphrase, recent hosts.
    Network {
        host: bool,
        addr: String,
        /// The port to host on.
        port: String,
        /// The port to join on.
        join_port: String,
        pass: String,
        /// The field taking keystrokes: an index into [`net_fields`].
        focus: u8,
        recent: Vec<String>,
    },
    /// "Attempting Connection" (title 413): no buttons, the busy
    /// cursor (case-4 sprites 40-49) — retail `WaitToConnect_7C230`.
    Connecting,
    /// A one-line notice with OK (a refused join, a kick).
    Message { text: String },
}

/// The network scroll's box (x1, y1, w, h) — retail anchors the session
/// scroll at (430,78); the port's is wider and centred.
const NET_BOX: (f32, f32, f32, f32) = (210.0, 110.0, 220.0, 162.0);
/// The message scroll's box (x1, y1, w); its height follows the
/// wrapped text ([`MainMenu::message_layout`]).
const MSG_BOX: (f32, f32, f32) = (190.0, 150.0, 260.0);
/// The message scroll's line pitch.
const MSG_LH: f32 = 12.0;

pub struct MainMenu {
    atlas: Vec<u8>,
    atlas_w: u32,
    atlas_h: u32,
    rects: Vec<Option<(f32, f32, f32, f32)>>,
    font: Vec<Option<(f32, f32, f32, f32)>>,
    strings: Vec<String>,
    anim: f32,
    modal: Option<Modal>,
    pending: Option<MenuAction>,
    sounds: Vec<u8>,
}

impl MainMenu {
    /// Load the case-4 members out of the baked `assets/mc2-ui`
    /// bundle.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let read = |name: &str| -> Result<Vec<u8>, String> {
            std::fs::read(dir.join(name)).map_err(|e| {
                format!(
                    "{}: {e} (rebake — epoch 18 adds it)",
                    dir.join(name).display()
                )
            })
        };
        let bg = read("menu-bg.bin")?;
        if bg.len() != (W as usize) * (H as usize) {
            return Err(format!("menu-bg.bin: {} bytes", bg.len()));
        }
        let pal = read("menu-pal.bin")?;
        if pal.len() != 768 {
            return Err("menu-pal.bin: not 768 bytes".into());
        }
        let sprites_px = read("menu-sprites.bin")?;
        let index: mgc_formats::bundle::SpriteIndex =
            serde_json::from_slice(&read("menu-sprites.json")?)
                .map_err(|e| format!("menu-sprites.json: {e}"))?;
        let font_px = read("font.bin")?;
        let font_index: mgc_formats::bundle::SpriteIndex =
            serde_json::from_slice(&read("font.json")?).map_err(|e| format!("font.json: {e}"))?;
        let strings: Vec<String> = serde_json::from_slice(&read("strings.json")?)
            .map_err(|e| format!("strings.json: {e}"))?;

        let rgb =
            |i: usize| -> [u8; 3] { [pal[i * 3] << 2, pal[i * 3 + 1] << 2, pal[i * 3 + 2] << 2] };
        // Atlas: bg (640×480), sprite bank below, font masks below
        // that. Width = 640 (the packed banks are 512 wide).
        let atlas_w = W as u32;
        let sprites_y = H as usize;
        let font_y = sprites_y + index.atlas_height as usize;
        let atlas_h = font_y as u32 + font_index.atlas_height;
        let mut atlas = vec![0u8; (atlas_w * atlas_h * 4) as usize];
        for (i, &p) in bg.iter().enumerate() {
            let c = rgb(p as usize);
            let o = ((i / W as usize) * atlas_w as usize + i % W as usize) * 4;
            atlas[o..o + 3].copy_from_slice(&c);
            atlas[o + 3] = 255;
        }
        for y in 0..index.atlas_height as usize {
            for x in 0..index.atlas_width as usize {
                let p = sprites_px[y * index.atlas_width as usize + x];
                if p == 0 {
                    continue;
                }
                let c = rgb(p as usize);
                let o = ((sprites_y + y) * atlas_w as usize + x) * 4;
                atlas[o..o + 3].copy_from_slice(&c);
                atlas[o + 3] = 255;
            }
        }
        for y in 0..font_index.atlas_height as usize {
            for x in 0..font_index.atlas_width as usize {
                if font_px[y * font_index.atlas_width as usize + x] == 0 {
                    continue;
                }
                let o = ((font_y + y) * atlas_w as usize + x) * 4;
                atlas[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let rect_at = |s: &mgc_formats::bundle::SpriteEntry, base: usize| {
            let f = s.frames.first()?;
            (s.width > 0 && s.height > 0).then_some((
                f.x as f32,
                (f.y as usize + base) as f32,
                s.width as f32,
                s.height as f32,
            ))
        };
        let rects = index
            .sprites
            .iter()
            .map(|s| rect_at(s, sprites_y))
            .collect();
        let font = font_index
            .sprites
            .iter()
            .map(|s| rect_at(s, font_y))
            .collect();
        Ok(Self {
            atlas,
            atlas_w,
            atlas_h,
            rects,
            font,
            strings,
            anim: 0.0,
            modal: None,
            pending: None,
            sounds: Vec::new(),
        })
    }

    pub fn atlas(&self) -> (u32, u32, &[u8]) {
        (self.atlas_w, self.atlas_h, &self.atlas)
    }

    pub fn tick(&mut self, dt: f32) {
        self.anim += dt;
        if let Some(Modal::Slots { open, .. }) = &mut self.modal {
            *open = (*open + 16.0 * 70.0 * dt).min(200.0);
        }
    }

    pub fn take_action(&mut self) -> Option<MenuAction> {
        self.pending.take()
    }

    pub fn take_sounds(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.sounds)
    }

    /// A modal edit field is accepting keystrokes.
    pub fn editing(&self) -> bool {
        matches!(
            &self.modal,
            Some(Modal::Name { .. })
                | Some(Modal::Slots { edit: Some(_), .. })
                | Some(Modal::Network { .. })
        )
    }

    /// Esc: Cancel on an open dialog (the scroll widget's scancode-1
    /// arm, `DrawScrollDialog2_7B660` MI:5660-63), else select the
    /// Exit button exactly as a click would (MI:5842-43 — scancode 1
    /// sets `selected_8` on button id 11; retail plays no click
    /// sample in that arm, only in the mouse one).
    pub fn escape(&mut self) {
        if matches!(self.modal, Some(Modal::Connecting)) {
            return;
        }
        if self.modal.is_some() {
            self.press_cancel();
        } else {
            self.modal = Some(Modal::ExitConfirm);
        }
    }

    /// Keystrokes: the name filter is retail `sub_7C200` (space,
    /// 0-9, letters), uppercased, max 12 (MI:4848-73); slot labels
    /// max 15.
    pub fn key_char(&mut self, c: char) {
        if let Some(Modal::Network {
            host,
            addr,
            port,
            join_port,
            pass,
            focus,
            ..
        }) = &mut self.modal
        {
            let (field, ok, max) = match net_fields(*host)[usize::from(*focus)] {
                NetField::Port => (port, c.is_ascii_digit(), 5),
                NetField::JoinPort => (join_port, c.is_ascii_digit(), 5),
                NetField::Addr => (addr, c.is_ascii_alphanumeric() || ".:-[]".contains(c), 40),
                NetField::Pass => (pass, c.is_ascii_alphanumeric() || "-_.!?".contains(c), 24),
            };
            if ok && field.len() < max {
                field.push(c);
            }
            return;
        }
        if !(c == ' ' || c.is_ascii_alphanumeric()) {
            return;
        }
        match &mut self.modal {
            Some(Modal::Name { buf }) => {
                if buf.len() < 12 {
                    buf.push(c.to_ascii_uppercase());
                }
            }
            Some(Modal::Slots { edit: Some(e), .. }) if e.len() < 15 => {
                e.push(c);
            }
            _ => {}
        }
    }

    pub fn key_backspace(&mut self) {
        match &mut self.modal {
            Some(Modal::Network {
                host,
                addr,
                port,
                join_port,
                pass,
                focus,
                ..
            }) => {
                match net_fields(*host)[usize::from(*focus)] {
                    NetField::Port => port.pop(),
                    NetField::JoinPort => join_port.pop(),
                    NetField::Addr => addr.pop(),
                    NetField::Pass => pass.pop(),
                };
            }
            Some(Modal::Name { buf }) => {
                buf.pop();
            }
            Some(Modal::Slots { edit: Some(e), .. }) => {
                e.pop();
            }
            _ => {}
        }
    }

    /// Enter = the OK button (the scroll widget's scancode-28 arm,
    /// `DrawScrollDialog2_7B660` MI:5656-58, click sample included).
    /// An open slot-label edit field consumes it first, committing
    /// the label rather than the dialog.
    pub fn key_enter(&mut self) {
        match &mut self.modal {
            Some(Modal::Slots {
                edit: edit @ Some(_),
                selected,
                slots,
                ..
            }) => {
                if let (Some(e), Some(k)) = (edit.take(), *selected)
                    && let Some(s) = slots.get_mut(k)
                {
                    s.0 = e;
                }
            }
            // Enter steps to the next field; on the last it is OK.
            Some(Modal::Network { host, focus, .. })
                if usize::from(*focus) + 1 < net_fields(*host).len() =>
            {
                *focus += 1;
            }
            Some(_) => self.press_ok(),
            None => {}
        }
    }

    /// Tab: the network scroll's other field.
    pub fn key_tab(&mut self) {
        if let Some(Modal::Network { host, focus, .. }) = &mut self.modal {
            *focus = (*focus + 1) % net_fields(*host).len() as u8;
        }
    }

    /// The OK button's semantics for the open modal — one law for the
    /// mouse hit and the Enter key. A slot dialog with nothing valid
    /// selected clicks but stays up (retail: the sample plays, the
    /// action needs a selection).
    fn press_ok(&mut self) {
        let Some(modal) = &mut self.modal else { return };
        match modal {
            Modal::Name { buf } => {
                self.sounds.push(SND_CLICK);
                self.pending = Some(MenuAction::SetName(std::mem::take(buf)));
                self.modal = None;
            }
            Modal::ExitConfirm => {
                self.sounds.push(SND_CLICK);
                self.pending = Some(MenuAction::Quit);
                self.modal = None;
            }
            Modal::Network {
                host,
                addr,
                port,
                join_port,
                pass,
                ..
            } => {
                // The address (the port when hosting) is required; the
                // passphrase is optional — empty = an open session
                // (player ruling 2026-10-17).
                self.sounds.push(SND_CLICK);
                let target = if *host { &*port } else { &*addr };
                if target.is_empty() {
                    return;
                }
                self.pending = Some(if *host {
                    MenuAction::Host {
                        port: port.clone(),
                        passphrase: pass.clone(),
                    }
                } else {
                    MenuAction::Join {
                        addr: addr.clone(),
                        port: join_port.clone(),
                        passphrase: pass.clone(),
                    }
                });
                self.modal = None;
            }
            Modal::Connecting => {}
            Modal::Message { .. } => {
                self.sounds.push(SND_CLICK);
                self.modal = None;
            }
            Modal::Slots {
                save,
                open,
                slots,
                selected,
                ..
            } => {
                if *open < 200.0 {
                    return; // still unrolling — swallow, like clicks
                }
                self.sounds.push(SND_CLICK);
                let action = if *save {
                    selected.map(|k| MenuAction::SaveTo { slot: k })
                } else {
                    selected.filter(|&k| slots[k].1).map(MenuAction::LoadFrom)
                };
                if let Some(a) = action {
                    self.pending = Some(a);
                    self.modal = None;
                }
            }
        }
    }

    /// The Cancel button: close the dialog, keep the state (with the
    /// click sample — retail plays it on the keyboard arm too).
    fn press_cancel(&mut self) {
        if matches!(self.modal, Some(Modal::Connecting)) {
            return;
        }
        if self.modal.take().is_some() {
            self.sounds.push(SND_CLICK);
        }
    }

    /// Open the slot picker (the app scans SAVE%d.GAM labels).
    pub fn open_slots(&mut self, save: bool, slots: Vec<(String, bool)>) {
        self.modal = Some(Modal::Slots {
            save,
            open: 0.0,
            slots,
            selected: None,
            edit: None,
        });
    }

    /// Open the network scroll (Multiplayer), seeded with the last
    /// entries and the recent hosts.
    pub fn open_network(
        &mut self,
        recent: Vec<String>,
        port: String,
        join_port: String,
        pass: String,
    ) {
        let (addr, last_port) = recent
            .first()
            .map(|a| split_host_port(a))
            .unwrap_or_default();
        self.modal = Some(Modal::Network {
            host: recent.is_empty(),
            addr,
            port,
            join_port: if last_port.is_empty() {
                join_port
            } else {
                last_port
            },
            pass,
            focus: 0,
            recent,
        });
    }

    /// "Attempting Connection" while a join is under way.
    pub fn open_connecting(&mut self) {
        self.modal = Some(Modal::Connecting);
    }

    /// A notice with OK (replaces whatever dialog is up).
    pub fn open_message(&mut self, text: String) {
        self.modal = Some(Modal::Message { text });
    }

    /// The app closes the connecting scroll once the join resolved.
    pub fn close_connecting(&mut self) {
        if matches!(self.modal, Some(Modal::Connecting)) {
            self.modal = None;
        }
    }

    /// Open the name-entry dialog seeded with the current name.
    pub fn open_name(&mut self, current: &str) {
        self.modal = Some(Modal::Name {
            buf: current.to_string(),
        });
    }

    fn button_hit(mx: f32, my: f32) -> Option<Button> {
        BUTTONS
            .iter()
            .find(|(_, pos, size, _)| {
                mx >= pos.0 && mx < pos.0 + size.0 && my >= pos.1 && my < pos.1 + size.1
            })
            .map(|&(b, _, _, _)| b)
    }

    /// Left-click at a window position. Returns the button the app
    /// must service with a slot scan (Save/Load), if any.
    pub fn click(&mut self, size: (f32, f32), cursor: (f32, f32)) -> Option<&'static str> {
        let (mx, my) = crate::ui::unletterbox(cursor, size, W, H);
        if self.modal.is_some() {
            self.modal_click(mx, my);
            return None;
        }
        let btn = Self::button_hit(mx, my)?;
        self.sounds.push(SND_CLICK);
        match btn {
            Button::NewGame => self.pending = Some(MenuAction::EnterMap),
            Button::SetName => return Some("name"),
            Button::Save => return Some("save"),
            Button::Load => return Some("load"),
            Button::Exit => self.modal = Some(Modal::ExitConfirm),
            Button::Multiplayer => return Some("network"),
            Button::SetKeys | Button::Language | Button::Joystick => {
                println!("main menu: that button is not part of this remake (yet)");
            }
        }
        None
    }

    fn modal_click(&mut self, mx: f32, my: f32) {
        // The message scroll's height follows its wrapped text.
        let msg_h = match &self.modal {
            Some(Modal::Message { text }) => self.message_layout(text).1,
            _ => 0.0,
        };
        let Some(modal) = &mut self.modal else { return };
        // Each modal's dialog box, for the shared OK/Cancel rects.
        if let Modal::Network {
            host,
            addr,
            join_port,
            focus,
            recent,
            ..
        } = modal
        {
            let (x1, y1, w, h) = NET_BOX;
            let pass_ix = |host: bool| (net_fields(host).len() - 1) as u8;
            for (k, r) in net_rows(x1, y1).into_iter().enumerate() {
                if !in_rect(mx, my, r) {
                    continue;
                }
                match k {
                    0 | 1 => {
                        *host = k == 0;
                        *focus = 0;
                    }
                    2..=4 if k - 2 < net_fields(*host).len() => *focus = (k - 2) as u8,
                    5 | 6 => match recent.get(k - 5) {
                        Some(a) if !*host => {
                            (*addr, *join_port) = split_host_port(a);
                            *focus = pass_ix(false);
                        }
                        _ => return,
                    },
                    _ => return,
                }
                self.sounds.push(SND_CLICK);
                return;
            }
            if in_rect(mx, my, ok_rect(x1, y1, h)) {
                self.press_ok();
            } else if in_rect(mx, my, cancel_rect(x1 + w - DIALOG_W, y1, h)) {
                self.press_cancel();
            }
            return;
        }
        let (x1, y1, h) = match modal {
            Modal::Connecting => return,
            Modal::Message { .. } => {
                let (x1, y1, _) = MSG_BOX;
                if in_rect(mx, my, ok_rect(x1, y1, msg_h)) {
                    self.press_ok();
                }
                return;
            }
            Modal::Network { .. } => unreachable!("handled above"),
            Modal::Name { .. } => (356.0 - DIALOG_W / 2.0, 112.0, 80.0),
            Modal::ExitConfirm => (352.0 - DIALOG_W / 2.0, 26.0, 80.0),
            // Retail anchors: Save (78,160), Load (448,160).
            Modal::Slots { save, .. } => (if *save { 78.0 } else { 448.0 }, 160.0, 200.0),
        };
        if let Modal::Slots {
            save,
            open,
            slots,
            selected,
            edit,
        } = modal
        {
            if *open < 200.0 {
                return;
            }
            // Rows at y1+32+16k (retail y1+16*(k+1), 1-based).
            for k in 0..slots.len() {
                let ry = y1 + 32.0 + 16.0 * k as f32;
                if mx >= x1 + 10.0 && mx < x1 + 10.0 + 92.0 && my >= ry && my < ry + 16.0 {
                    if !*save && !slots[k].1 {
                        return;
                    }
                    *selected = Some(k);
                    if *save {
                        *edit = Some(if slots[k].1 {
                            slots[k].0.clone()
                        } else {
                            String::new()
                        });
                    }
                    self.sounds.push(SND_CLICK);
                    return;
                }
            }
        }
        if in_rect(mx, my, ok_rect(x1, y1, h)) {
            self.press_ok();
        } else if in_rect(mx, my, cancel_rect(x1, y1, h)) {
            self.press_cancel();
        }
    }

    fn sprite(&self, id: usize, pos: (f32, f32), scale: f32) -> Option<UiQuad> {
        let (sx, sy, w, h) = self.rects.get(id).copied().flatten()?;
        Some(UiQuad {
            rect: [pos.0 * scale, pos.1 * scale, w * scale, h * scale],
            uv: [sx, sy, w, h],
            tint: [1.0, 1.0, 1.0, 1.0],
        })
    }

    fn text(&self, s: &str, x: f32, y: f32, color: [f32; 4], scale: f32) -> Vec<UiQuad> {
        let mut out = Vec::new();
        let mut cx = x;
        for c in s.chars() {
            let id = (c as usize).wrapping_add(1);
            let Some((sx, sy, w, h)) = self.font.get(id).copied().flatten() else {
                cx += 4.0;
                continue;
            };
            if c != ' ' {
                out.push(UiQuad {
                    rect: [cx * scale, y * scale, w * scale, h * scale],
                    uv: [sx, sy, w, h],
                    tint: color,
                });
            }
            cx += w;
        }
        out
    }

    fn text_width(&self, s: &str) -> f32 {
        s.chars()
            .map(|c| {
                self.font
                    .get((c as usize).wrapping_add(1))
                    .copied()
                    .flatten()
                    .map_or(4.0, |(_, _, w, _)| w)
            })
            .sum()
    }

    /// A message wrapped at word bounds to the scroll's width, and the
    /// scroll's height: the lines plus room for the OK button below.
    fn message_layout(&self, text: &str) -> (Vec<String>, f32) {
        let max = MSG_BOX.2 - 30.0;
        let mut lines: Vec<String> = Vec::new();
        for word in text.split_whitespace() {
            match lines.last_mut() {
                Some(l) if self.text_width(&format!("{l} {word}")) <= max => {
                    l.push(' ');
                    l.push_str(word);
                }
                _ => lines.push(word.to_string()),
            }
        }
        let h = 22.0 + MSG_LH * lines.len() as f32 + 34.0;
        (lines, h.max(80.0))
    }

    /// The retail unrolled-scroll dialog panel at (x1,y1), open to
    /// `openh` (DrawScrollDialog2 law — the roller bar at the top
    /// AND the animated bottom edge, a solid parchment fill with
    /// vertical edge lines between them; ONE scroll, not a stack).
    fn parchment(&self, x1: f32, y1: f32, openh: f32, scale: f32, out: &mut Vec<UiQuad>) {
        self.parchment_w(x1, y1, DIALOG_W, openh, scale, out);
    }

    /// [`Self::parchment`] `bar_w` wide — the roller bar stretched (the
    /// port's wider dialogs; at `DIALOG_W` it is the retail scroll).
    fn parchment_w(
        &self,
        x1: f32,
        y1: f32,
        bar_w: f32,
        openh: f32,
        scale: f32,
        out: &mut Vec<UiQuad>,
    ) {
        let bar_h = self
            .rects
            .get(72)
            .copied()
            .flatten()
            .map_or(12.0, |(_, _, _, h)| h);
        let solid = |r: [f32; 4], tint: [f32; 4]| UiQuad {
            rect: [r[0] * scale, r[1] * scale, r[2] * scale, r[3] * scale],
            uv: [0.0; 4],
            tint,
        };
        if openh > 0.0 {
            let parchment = [168.0 / 255.0, 144.0 / 255.0, 116.0 / 255.0, 1.0];
            let edge = [148.0 / 255.0, 124.0 / 255.0, 100.0 / 255.0, 1.0];
            let top = y1 + bar_h - 2.0;
            out.push(solid([x1 + 10.0, top, bar_w - 22.0, openh], parchment));
            out.push(solid([x1 + 10.0, top, 1.0, openh], edge));
            out.push(solid([x1 + bar_w - 12.0, top, 1.0, openh], edge));
        }
        for y in [y1, y1 + openh] {
            if let Some(mut q) = self.sprite(72, (x1, y), scale) {
                q.rect[2] = bar_w * scale;
                out.push(q);
            }
        }
    }

    /// The title in dim ink under the top roller (retail 7FCB0 draw
    /// between x1+10 and x1+10+barW-22 at y1+barH+2).
    fn dialog_title(&self, idx: usize, x1: f32, y1: f32, scale: f32, out: &mut Vec<UiQuad>) {
        let ink = [88.0 / 255.0, 64.0 / 255.0, 36.0 / 255.0, 1.0];
        if let Some(t) = self.strings.get(idx) {
            let tx = x1 + 10.0 + (DIALOG_W - 22.0 - self.text_width(t)) / 2.0;
            out.extend(self.text(t, tx, y1 + 14.0, ink, scale));
        }
    }

    /// [`Self::dialog_title`] centred over a `w`-wide dialog.
    fn centred_title(
        &self,
        idx: usize,
        x1: f32,
        y1: f32,
        w: f32,
        scale: f32,
        out: &mut Vec<UiQuad>,
    ) {
        let ink = [88.0 / 255.0, 64.0 / 255.0, 36.0 / 255.0, 1.0];
        if let Some(t) = self.strings.get(idx) {
            let tx = x1 + (w - self.text_width(t)) / 2.0;
            out.extend(self.text(t, tx, y1 + 14.0, ink, scale));
        }
    }

    fn ok_cancel(
        &self,
        x1: f32,
        y1: f32,
        h: f32,
        cursor: (f32, f32),
        scale: f32,
        out: &mut Vec<UiQuad>,
    ) {
        let (mx, my) = cursor;
        let ok = ok_rect(x1, y1, h);
        let ca = cancel_rect(x1, y1, h);
        if let Some(q) = self.sprite(
            if in_rect(mx, my, ok) { 71 } else { 70 },
            (ok.0, ok.1),
            scale,
        ) {
            out.push(q);
        }
        if let Some(q) = self.sprite(
            if in_rect(mx, my, ca) { 69 } else { 68 },
            (ca.0, ca.1),
            scale,
        ) {
            out.push(q);
        }
    }

    /// This frame's quads (over black letterbox, like the map).
    pub fn quads(&self, size: (f32, f32), cursor: (f32, f32)) -> Vec<UiQuad> {
        // Everything below is authored in the temple screen's own
        // coordinates and scaled on the way out; the letterbox offset
        // is applied to the finished list, and the cursor makes the
        // same trip in reverse so hit tests stay in authored space.
        let (scale, ox, oy) = crate::ui::letterbox(size, W, H);
        let cursor = crate::ui::unletterbox(cursor, size, W, H);
        let mut quads = Vec::new();
        // Background.
        quads.push(UiQuad {
            rect: [0.0, 0.0, W * scale, H * scale],
            uv: [0.0, 0.0, W, H],
            tint: [1.0, 1.0, 1.0, 1.0],
        });
        // Idle dressing at ~25 fps.
        let frame = (self.anim * 25.0) as usize;
        for (x, y, first, last) in ANIMS {
            let id = first + frame % (last - first + 1);
            if let Some(q) = self.sprite(id, (x, y), scale) {
                quads.push(q);
            }
        }
        // The permanent (185,232) blit (MI:897).
        if let Some(q) = self.sprite(66, (185.0, 232.0), scale) {
            quads.push(q);
        }
        // Hover art (retail lights the hovered button only).
        let (mx, my) = cursor;
        if self.modal.is_none()
            && let Some(btn) = Self::button_hit(mx, my)
            && let Some(&(_, pos, _, spr)) = BUTTONS.iter().find(|(b, ..)| *b == btn)
        {
            if let Some(q) = self.sprite(spr, pos, scale) {
                quads.push(q);
            }
        }
        // Modal on top.
        let caret_on = (self.anim * 4.0) as u32 % 2 == 0;
        match &self.modal {
            None => {}
            Some(Modal::Name { buf }) => {
                let (x1, y1, h) = (356.0 - DIALOG_W / 2.0, 112.0, 80.0);
                self.parchment(x1, y1, h, scale, &mut quads);
                self.dialog_title(420, x1, y1, scale, &mut quads);
                let ink = [88.0 / 255.0, 64.0 / 255.0, 36.0 / 255.0, 1.0];
                let mut shown = buf.clone();
                if caret_on {
                    shown.push('_');
                }
                quads.extend(self.text(&shown, x1 + 15.0, y1 + 30.0, ink, scale));
                self.ok_cancel(x1, y1, h, (mx, my), scale, &mut quads);
            }
            Some(Modal::ExitConfirm) => {
                let (x1, y1, h) = (352.0 - DIALOG_W / 2.0, 26.0, 80.0);
                self.parchment(x1, y1, h, scale, &mut quads);
                self.dialog_title(407, x1, y1, scale, &mut quads);
                self.ok_cancel(x1, y1, h, (mx, my), scale, &mut quads);
            }
            Some(Modal::Network {
                host,
                addr,
                port,
                join_port,
                pass,
                focus,
                recent,
            }) => {
                let (x1, y1, w, h) = NET_BOX;
                let ink = [88.0 / 255.0, 64.0 / 255.0, 36.0 / 255.0, 1.0];
                let white = [1.0, 1.0, 1.0, 1.0];
                self.parchment_w(x1, y1, w, h, scale, &mut quads);
                self.centred_title(412, x1, y1, w, scale, &mut quads);
                let rows = net_rows(x1, y1);
                for (k, word) in [(0, "Host"), (1, "Join")] {
                    let on = (k == 0) == *host;
                    let r = rows[k];
                    quads.extend(self.text(word, r.0, r.1, if on { white } else { ink }, scale));
                }
                let field = |label: &str, value: &str, focused: bool| {
                    let mut v = value.to_string();
                    if focused && caret_on {
                        v.push('_');
                    }
                    (format!("{label}:"), v)
                };
                let lines: Vec<_> = net_fields(*host)
                    .iter()
                    .enumerate()
                    .map(|(k, f)| {
                        let (label, value) = match f {
                            NetField::Addr => ("Address", addr),
                            NetField::Port | NetField::JoinPort if *host => ("Port", port),
                            NetField::Port | NetField::JoinPort => ("Port", join_port),
                            NetField::Pass => ("Passphrase", pass),
                        };
                        field(label, value, usize::from(*focus) == k)
                    })
                    .collect();
                for (k, (label, value)) in lines.iter().enumerate() {
                    let r = rows[2 + k];
                    quads.extend(self.text(label, r.0, r.1, ink, scale));
                    let on = *focus as usize == k;
                    quads.extend(self.text(
                        value,
                        r.0 + 75.0,
                        r.1,
                        if on { white } else { ink },
                        scale,
                    ));
                }
                if !*host {
                    for (k, a) in recent.iter().take(2).enumerate() {
                        let r = rows[5 + k];
                        let hov = in_rect(mx, my, r);
                        quads.extend(self.text(
                            &format!("> {a}"),
                            r.0,
                            r.1,
                            if hov { white } else { ink },
                            scale,
                        ));
                    }
                }
                let (ok, ca) = (ok_rect(x1, y1, h), cancel_rect(x1 + w - DIALOG_W, y1, h));
                quads.extend(self.sprite(
                    if in_rect(mx, my, ok) { 71 } else { 70 },
                    (ok.0, ok.1),
                    scale,
                ));
                quads.extend(self.sprite(
                    if in_rect(mx, my, ca) { 69 } else { 68 },
                    (ca.0, ca.1),
                    scale,
                ));
            }
            Some(Modal::Connecting) => {
                let (x1, y1, h) = (430.0, 78.0, 80.0);
                self.parchment(x1, y1, h, scale, &mut quads);
                self.dialog_title(413, x1, y1, scale, &mut quads);
            }
            Some(Modal::Message { text }) => {
                let (x1, y1, w) = MSG_BOX;
                let (lines, h) = self.message_layout(text);
                let ink = [88.0 / 255.0, 64.0 / 255.0, 36.0 / 255.0, 1.0];
                self.parchment_w(x1, y1, w, h, scale, &mut quads);
                for (k, t) in lines.iter().enumerate() {
                    let tx = x1 + (w - self.text_width(t)) / 2.0;
                    let ty = y1 + 22.0 + MSG_LH * k as f32;
                    quads.extend(self.text(t, tx, ty, ink, scale));
                }
                let ok = ok_rect(x1, y1, h);
                quads.extend(self.sprite(
                    if in_rect(mx, my, ok) { 71 } else { 70 },
                    (ok.0, ok.1),
                    scale,
                ));
            }
            Some(Modal::Slots {
                save,
                open,
                slots,
                selected,
                edit,
            }) => {
                let (x1, y1, h) = (if *save { 78.0 } else { 448.0 }, 160.0, 200.0);
                self.parchment(x1, y1, *open, scale, &mut quads);
                if *open > 17.0 {
                    self.dialog_title(if *save { 422 } else { 421 }, x1, y1, scale, &mut quads);
                }
                if *open >= h {
                    let white = [1.0, 1.0, 1.0, 1.0];
                    let ink = [88.0 / 255.0, 64.0 / 255.0, 36.0 / 255.0, 1.0];
                    for (k, (label, _)) in slots.iter().enumerate() {
                        let ry = y1 + 32.0 + 16.0 * k as f32;
                        let sel = *selected == Some(k);
                        let shown = if sel && let Some(e) = edit {
                            let mut s = format!("{}. {e}", k + 1);
                            if caret_on {
                                s.push('_');
                            }
                            s
                        } else {
                            format!("{}. {label}", k + 1)
                        };
                        let color = if sel { white } else { ink };
                        quads.extend(self.text(&shown, x1 + 20.0, ry, color, scale));
                    }
                    self.ok_cancel(x1, y1, h, (mx, my), scale, &mut quads);
                }
            }
        }
        // Cursor (case-4 sprite 39). In authored space like every
        // other quad here — the letterbox translation below puts it
        // back under the real pointer.
        // While connecting: the busy cursor, sprites 40-49 stepping
        // every 4 ticks of the 100 Hz clock (retail WaitToConnect).
        let cursor_spr = if matches!(self.modal, Some(Modal::Connecting)) {
            40 + (self.anim * 25.0) as usize % 10
        } else {
            39
        };
        if let Some((sx, sy, w, h)) = self.rects.get(cursor_spr).copied().flatten() {
            quads.push(UiQuad {
                rect: [cursor.0 * scale, cursor.1 * scale, w * scale, h * scale],
                uv: [sx, sy, w, h],
                tint: [1.0, 1.0, 1.0, 1.0],
            });
        }
        crate::ui::offset_quads(&mut quads, ox, oy);
        quads
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare() -> MainMenu {
        MainMenu {
            atlas: Vec::new(),
            atlas_w: 1,
            atlas_h: 1,
            rects: Vec::new(),
            font: Vec::new(),
            strings: Vec::new(),
            anim: 0.0,
            modal: None,
            pending: None,
            sounds: Vec::new(),
        }
    }

    /// The retail key law end to end: Esc on the bare menu arms the
    /// exit confirm (MI:5842-43), Enter is its OK (MI:5656-58), a
    /// second Esc is its Cancel (MI:5660-63).
    #[test]
    fn escape_arms_exit_confirm_enter_confirms() {
        let mut m = bare();
        m.escape();
        assert!(matches!(m.modal, Some(Modal::ExitConfirm)));
        assert_eq!(m.take_action(), None, "arming must not quit by itself");
        m.key_enter();
        assert_eq!(m.take_action(), Some(MenuAction::Quit));
        assert!(m.modal.is_none());
    }

    /// The Join face has its own Port field (player 2026-10-17): the
    /// last host comes back split into address and port, Tab and Enter
    /// walk Address → Port → Passphrase, and OK joins with both; a
    /// recent host's click fills both fields.
    #[test]
    fn joining_takes_the_address_and_the_port_apart() {
        let mut m = bare();
        m.open_network(
            vec!["example.org:9000".into(), "[::1]:7000".into()],
            "7777".into(),
            "7777".into(),
            "pw".into(),
        );
        let Some(Modal::Network {
            host,
            addr,
            join_port,
            ..
        }) = &m.modal
        else {
            panic!("the network scroll")
        };
        assert!(!host, "a known host opens the Join face");
        assert_eq!((addr.as_str(), join_port.as_str()), ("example.org", "9000"));
        for _ in 0..20 {
            m.key_backspace();
        }
        "lan-box".chars().for_each(|c| m.key_char(c));
        m.key_tab();
        for _ in 0..5 {
            m.key_backspace();
        }
        "8123x".chars().for_each(|c| m.key_char(c));
        m.key_enter();
        assert_eq!(m.take_action(), None, "Enter steps to the passphrase");
        m.key_enter();
        assert_eq!(
            m.take_action(),
            Some(MenuAction::Join {
                addr: "lan-box".into(),
                port: "8123".into(),
                passphrase: "pw".into(),
            })
        );
        // A recent host's row fills both fields.
        m.open_network(
            vec!["a:1".into(), "[::1]:7000".into()],
            "7777".into(),
            "7777".into(),
            "pw".into(),
        );
        let r = net_rows(NET_BOX.0, NET_BOX.1)[6];
        m.modal_click(r.0 + 1.0, r.1 + 1.0);
        let Some(Modal::Network {
            addr, join_port, ..
        }) = &m.modal
        else {
            panic!("the network scroll")
        };
        assert_eq!((addr.as_str(), join_port.as_str()), ("[::1]", "7000"));
        assert_eq!(split_host_port("[::1]"), ("[::1]".into(), String::new()));
        assert_eq!(
            split_host_port("lan-box"),
            ("lan-box".into(), String::new())
        );
    }

    /// A long notice (a refused join) wraps at word bounds to the
    /// scroll's width, the scroll grows to hold it, and its OK still
    /// answers below the text.
    #[test]
    fn a_long_message_wraps_and_its_ok_still_works() {
        let mut m = bare();
        let text = "net: the host refused: a player named Zanzamar is already in this \
                    game - pick another name (Set Name)";
        let (lines, h) = m.message_layout(text);
        assert!(lines.len() > 1, "wrapped");
        assert!(lines.iter().all(|l| m.text_width(l) <= MSG_BOX.2 - 30.0));
        assert_eq!(
            lines.join(" "),
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        );
        m.open_message(text.into());
        let ok = ok_rect(MSG_BOX.0, MSG_BOX.1, h);
        m.modal_click(ok.0 + 2.0, ok.1 + 2.0);
        assert!(m.modal.is_none(), "OK closed it");
    }

    #[test]
    fn second_escape_declines_exit_confirm() {
        let mut m = bare();
        m.escape();
        m.escape();
        assert!(m.modal.is_none());
        assert_eq!(m.take_action(), None);
        // The decline clicked (the widget's keyboard arm plays the
        // sample); the arming press did not.
        assert_eq!(m.take_sounds(), vec![SND_CLICK]);
    }

    /// Enter presses OK on the slot dialogs too, honouring the same
    /// selection law as the mouse; an open label editor consumes it
    /// first.
    #[test]
    fn enter_presses_ok_on_slot_dialogs() {
        let mut m = bare();
        m.open_slots(false, vec![("GAME".into(), true), ("Empty".into(), false)]);
        // Unroll the parchment.
        for _ in 0..40 {
            m.tick(1.0 / 60.0);
        }
        // Nothing selected: the click sounds, the dialog stays.
        m.key_enter();
        assert!(m.modal.is_some());
        assert_eq!(m.take_action(), None);
        if let Some(Modal::Slots { selected, .. }) = &mut m.modal {
            *selected = Some(0);
        }
        m.key_enter();
        assert_eq!(m.take_action(), Some(MenuAction::LoadFrom(0)));
        assert!(m.modal.is_none());
    }

    #[test]
    fn enter_commits_an_open_label_edit_before_ok() {
        let mut m = bare();
        m.open_slots(true, vec![("OLD".into(), true)]);
        for _ in 0..40 {
            m.tick(1.0 / 60.0);
        }
        if let Some(Modal::Slots { selected, edit, .. }) = &mut m.modal {
            *selected = Some(0);
            *edit = Some("NEW".into());
        }
        m.key_enter();
        // First Enter closed the editor, not the dialog.
        assert!(m.modal.is_some());
        assert_eq!(m.take_action(), None);
        if let Some(Modal::Slots { slots, .. }) = &m.modal {
            assert_eq!(slots[0].0, "NEW");
        }
        m.key_enter();
        assert_eq!(m.take_action(), Some(MenuAction::SaveTo { slot: 0 }));
    }
}
