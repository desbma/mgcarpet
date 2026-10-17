//! The netplay lobby's state — SERVER-AUTHORITATIVE, socket-free (the
//! wire lives in `net.rs`). The player's rulings (2026-10-15):
//!
//! 1. Only the admin changes parameters — the level and the session
//!    rules ([`crate::rules`]).
//! 2. Every other member must AGREE to them before play (a consensus) …
//! 3. … and any admin change voids EVERY agreement (no last-second
//!    switcheroo). Agreements name the rules revision they accept, so a
//!    stale one can never land.
//! 4. The admin can kick anyone (`net::Guest::kick`).
//! 7. Only the admin presses Play, and only once everyone has agreed.
//! 8. Wizards are first come, first served: one a member holds can't be
//!    taken by anyone else.
//! 10. After a game the lobby reopens with the same players and
//!     settings; everyone approves Play again.
//!
//! The ADMIN (player 2026-10-17) is a member like any other, with the
//! powers above: the dedicated server plays no seat. The first member
//! in is the admin — unless the server was started for one (the UI's
//! Host: its player claims it, [`LobbyCore::claim_admin`]); when the
//! admin leaves the session, the longest-standing member takes over.
//!
//! A wizard IS a seat here: the port's seat decides the colour, the
//! castle and the carpet art, so the lobby's wizard choice (retail's
//! totems) picks the seat. Retail kept a separate join-order seat.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wizard seats a level can hold (retail: 8 portraits / 8 colours).
pub const WIZARDS: u8 = 8;

/// The game a session plays — fixed for the server's life, chosen by
/// whoever runs it (`--serve --game`, the hosting player's game), never
/// by the first player in (player 2026-10-17: players of different
/// games in one session would be a mess). Keys = the baked tree's game
/// directories and `--level game:index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetGame {
    Mc1,
    Mc1hw,
    Mc2,
}

impl NetGame {
    pub fn parse(key: &str) -> Result<Self, String> {
        match key {
            "mc1" => Ok(Self::Mc1),
            "mc1hw" => Ok(Self::Mc1hw),
            "mc2" => Ok(Self::Mc2),
            _ => Err(format!("unknown game {key:?} (mc1, mc1hw or mc2)")),
        }
    }

    /// The baked tree's directory (and `--level` key).
    pub fn key(self) -> &'static str {
        match self {
            Self::Mc1 => "mc1",
            Self::Mc1hw => "mc1hw",
            Self::Mc2 => "mc2",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Mc1 => "Magic Carpet",
            Self::Mc1hw => "Magic Carpet: Hidden Worlds",
            Self::Mc2 => "Magic Carpet 2",
        }
    }

    /// The game's multiplayer arenas (retail: MC1 `LEVELS.DAT` 50-69,
    /// HW `DDLEVELS.DAT` 50-59, MC2 50-59).
    pub fn arenas(self) -> std::ops::RangeInclusive<u32> {
        match self {
            Self::Mc1 => 50..=69,
            Self::Mc1hw | Self::Mc2 => 50..=59,
        }
    }

    /// The game a baked level belongs to (`…/<game>/level-NNN.mgcl`).
    pub fn of_level(path: &str) -> Option<Self> {
        let dir = std::path::Path::new(path).parent()?.file_name()?.to_str()?;
        Self::parse(dir).ok()
    }
}

/// One lobby member as everyone sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// Stable for the session; never reused.
    pub id: u32,
    /// The wizard seat held (0-based).
    pub wizard: u8,
    /// The player's campaign name (empty: the wizard's own name).
    #[serde(default)]
    pub name: String,
    /// The player's own flight models (`controls.models`) — per seat,
    /// carried into the game.
    #[serde(default)]
    pub models: Value,
    /// Has agreed to the current rules revision. The admin's own
    /// agreement is his Play press, so his row is always agreed.
    pub agreed: bool,
}

/// The lobby as one member sees it — what the server sends on every
/// change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LobbyView {
    /// The receiving member's id.
    pub you: u32,
    /// The rules revision: bumped by every admin change to the level
    /// or the rules.
    pub rev: u32,
    /// The session's game (fixed for the server's life).
    pub game: NetGame,
    /// The level (a baked level path; every peer runs the same build).
    pub level: String,
    /// The session rules (`rules::collect` of a config).
    pub rules: Value,
    /// In join order.
    pub members: Vec<Member>,
    /// The admin's member id (`None`: nobody in yet).
    #[serde(default)]
    pub admin: Option<u32>,
    /// The session's passphrase — the admin's view only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
    /// The player limit (the admin sets it; the lintel's left box).
    pub capacity: u8,
    /// AI wizards to seat besides the humans (the admin's rule; retail's
    /// network game seats none — the default). Capped at the free seats
    /// when the game starts.
    #[serde(default)]
    pub ai: u8,
    /// A game is in progress: a member who sees this WAITS for it to
    /// finish (he left it, or joined during it); the lobby reopens
    /// when it ends.
    #[serde(default)]
    pub playing: bool,
}

impl LobbyView {
    pub fn me(&self) -> Option<&Member> {
        self.members.iter().find(|m| m.id == self.you)
    }

    pub fn is_admin(&self) -> bool {
        self.admin == Some(self.you)
    }

    /// Nobody holds `wizard`.
    pub fn is_free(&self, wizard: u8) -> bool {
        wizard < WIZARDS && self.members.iter().all(|m| m.wizard != wizard)
    }

    /// The next free wizard stepping from `from` in direction `dir`
    /// (±1, wrapping) — the totems. `None` when every other is held.
    pub fn next_free(&self, from: u8, dir: i8) -> Option<u8> {
        let n = i16::from(WIZARDS);
        (1..n)
            .map(|k| (i16::from(from) + k * i16::from(dir.signum())).rem_euclid(n) as u8)
            .find(|&w| self.is_free(w))
    }

    /// Play is open: two members at least, and every one agreed (the
    /// admin's row always reads agreed).
    pub fn all_agreed(&self) -> bool {
        self.members.len() > 1 && self.members.iter().all(|m| m.agreed)
    }

    /// The members besides the admin, and how many of them agreed.
    pub fn agreement(&self) -> (usize, usize) {
        let others = self.members.iter().filter(|m| Some(m.id) != self.admin);
        let (n, agreed) = others.fold((0, 0), |(n, a), m| (n + 1, a + usize::from(m.agreed)));
        (n, agreed)
    }

    /// The wizard seat per human, in member order (`Hx(k)` = member k).
    pub fn seats(&self) -> Vec<u8> {
        self.members.iter().map(|m| m.wizard).collect()
    }

    /// Each human's flight models, in member order.
    pub fn models(&self) -> Vec<Value> {
        self.members.iter().map(|m| m.models.clone()).collect()
    }

    /// Each human's name, in member order (empty: none given).
    pub fn names(&self) -> Vec<String> {
        self.members.iter().map(|m| m.name.clone()).collect()
    }
}

/// The server's lobby model.
pub struct LobbyCore {
    rev: u32,
    game: NetGame,
    level: String,
    rules: Value,
    members: Vec<Member>,
    admin: Option<u32>,
    next_id: u32,
    capacity: usize,
    /// The most the admin may set ([`Self::set_capacity`]).
    max: usize,
    ai: u8,
    /// A game is in progress ([`LobbyView::playing`]).
    playing: bool,
}

impl LobbyCore {
    /// An empty lobby of `game` with room for `capacity` members.
    pub fn new(game: NetGame, level: String, rules: Value, capacity: usize) -> Self {
        let max = capacity.clamp(2, usize::from(WIZARDS));
        Self {
            rev: 0,
            game,
            level,
            rules,
            members: Vec::new(),
            admin: None,
            next_id: 1,
            capacity: max,
            max,
            ai: 0,
            playing: false,
        }
    }

    pub fn view_for(&self, you: u32) -> LobbyView {
        let mut members = self.members.clone();
        for m in &mut members {
            m.agreed |= Some(m.id) == self.admin;
        }
        LobbyView {
            you,
            rev: self.rev,
            game: self.game,
            level: self.level.clone(),
            rules: self.rules.clone(),
            members,
            admin: self.admin,
            passphrase: None,
            capacity: self.capacity as u8,
            ai: self.ai,
            playing: self.playing,
        }
    }

    pub fn is_admin(&self, id: u32) -> bool {
        self.admin == Some(id)
    }

    #[cfg(test)]
    fn admin(&self) -> Option<u32> {
        self.admin
    }

    /// The game begins: members from now on wait for it.
    pub fn begin(&mut self) {
        self.playing = true;
    }

    #[cfg(test)]
    fn rev(&self) -> u32 {
        self.rev
    }

    /// Admit a new member (`name`) on the first free wizard; refused
    /// when full, or when another member already goes by that name
    /// (case-blind — two ZANZAMARs in one game is no game; player
    /// ruling 10-15). The first member in is the admin.
    pub fn join(&mut self, name: String) -> Result<u32, String> {
        if !name.is_empty()
            && self
                .members
                .iter()
                .any(|m| m.name.eq_ignore_ascii_case(&name))
        {
            return Err(format!(
                "a player named {name} is already in this game - pick another name (Set Name)"
            ));
        }
        let view = self.view_for(0);
        let wizard = (0..WIZARDS).find(|&w| view.is_free(w));
        match wizard {
            Some(wizard) if self.members.len() < self.capacity => {
                let id = self.next_id;
                self.next_id += 1;
                self.members.push(Member {
                    id,
                    wizard,
                    name,
                    models: Value::Null,
                    agreed: false,
                });
                self.admin.get_or_insert(id);
                Ok(id)
            }
            _ => Err(format!("the lobby is full ({} players)", self.capacity)),
        }
    }

    /// Member `id` becomes the admin (the player the server was started
    /// for — the UI's Host).
    pub fn claim_admin(&mut self, id: u32) -> bool {
        if self.admin == Some(id) || self.members.iter().all(|m| m.id != id) {
            return false;
        }
        self.admin = Some(id);
        if let Some(m) = self.members.iter_mut().find(|m| m.id == id) {
            m.agreed = false;
        }
        true
    }

    /// A member left or was kicked: his wizard is free again. An
    /// admin's leaving hands the role to the longest-standing member.
    pub fn leave(&mut self, id: u32) -> bool {
        let before = self.members.len();
        self.members.retain(|m| m.id != id);
        if self.admin == Some(id) {
            self.admin = self.members.first().map(|m| m.id);
            if let Some(m) = self.members.first_mut() {
                m.agreed = false;
            }
        }
        self.members.len() != before
    }

    /// Take `wizard` for member `id` — only a free one (first come,
    /// first served). A wizard is not a rule: nobody's agreement moves.
    pub fn pick_wizard(&mut self, id: u32, wizard: u8) -> bool {
        if !self.view_for(id).is_free(wizard) {
            return false;
        }
        match self.members.iter_mut().find(|m| m.id == id) {
            Some(m) => {
                m.wizard = wizard;
                true
            }
            None => false,
        }
    }

    /// Member `id` agrees to revision `rev`; a stale revision (the admin
    /// changed something meanwhile) is ignored. The admin's agreement
    /// is his Play press.
    pub fn agree(&mut self, id: u32, rev: u32) -> bool {
        let admin = self.admin;
        match self
            .members
            .iter_mut()
            .find(|m| m.id == id && Some(m.id) != admin)
        {
            Some(m) if rev == self.rev && !m.agreed => {
                m.agreed = true;
                true
            }
            _ => false,
        }
    }

    /// Member `id` takes his agreement back.
    pub fn withdraw(&mut self, id: u32) -> bool {
        let admin = self.admin;
        match self
            .members
            .iter_mut()
            .find(|m| m.id == id && Some(m.id) != admin)
        {
            Some(m) if m.agreed => {
                m.agreed = false;
                true
            }
            _ => false,
        }
    }

    /// The admin sets the level — one of the session's game; a change
    /// voids every agreement.
    pub fn set_level(&mut self, level: String) -> bool {
        if level == self.level || NetGame::of_level(&level) != Some(self.game) {
            return false;
        }
        self.level = level;
        self.void();
        true
    }

    /// The admin sets the AI wizard count (0 up to the seats the humans
    /// leave); a RULE — a change voids every agreement.
    pub fn set_ai(&mut self, ai: u8) -> bool {
        let ai = ai.min(WIZARDS.saturating_sub(self.members.len() as u8));
        if ai == self.ai {
            return false;
        }
        self.ai = ai;
        self.void();
        true
    }

    /// The admin sets the rules; a change voids every agreement.
    pub fn set_rules(&mut self, rules: Value) -> bool {
        if rules == self.rules {
            return false;
        }
        self.rules = rules;
        self.void();
        true
    }

    fn void(&mut self) {
        self.rev += 1;
        self.clear_agreements();
    }

    fn clear_agreements(&mut self) {
        for m in &mut self.members {
            m.agreed = false;
        }
    }

    /// After a game: same players and settings, everyone approves Play
    /// again.
    pub fn reopen(&mut self) {
        self.playing = false;
        self.clear_agreements();
    }

    /// Member `id`'s flight models (his own choice — no agreement moves).
    pub fn set_models(&mut self, id: u32, models: Value) -> bool {
        match self.members.iter_mut().find(|m| m.id == id) {
            Some(m) if m.models != models => {
                m.models = models;
                true
            }
            _ => false,
        }
    }

    /// The admin sets the player limit: 2 up to the most the session
    /// allows, never below the members already in. Not a rule — no
    /// agreement moves.
    pub fn set_capacity(&mut self, capacity: usize) -> bool {
        let c = capacity.clamp(self.members.len().max(2), self.max.max(2));
        let changed = c != self.capacity;
        self.capacity = c;
        changed
    }

    pub fn can_play(&self) -> bool {
        self.view_for(0).all_agreed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A lobby of `n` members (the first is the admin), room for `cap`.
    fn lobby_of(n: usize, cap: usize) -> (LobbyCore, Vec<u32>) {
        let mut l = LobbyCore::new(
            NetGame::Mc2,
            "baked/mc2/level-050.mgcl".into(),
            json!({"fog": 40}),
            cap,
        );
        let ids = (0..n).map(|_| l.join(String::new()).unwrap()).collect();
        (l, ids)
    }

    #[test]
    fn joiners_take_the_first_free_wizard_until_full() {
        let (mut l, ids) = lobby_of(1, 3);
        l.pick_wizard(ids[0], 1);
        let a = l.join(String::new()).unwrap();
        let b = l.join(String::new()).unwrap();
        assert_eq!(l.view_for(a).seats(), vec![1, 0, 2]);
        assert!(l.join(String::new()).is_err(), "capacity 3");
        assert!(l.leave(b));
        assert_eq!(
            l.join(String::new()).map(|_| l.view_for(a).seats()),
            Ok(vec![1, 0, 2])
        );
    }

    #[test]
    fn a_held_wizard_cannot_be_taken() {
        let (mut l, ids) = lobby_of(3, 4);
        let (a, b) = (ids[1], ids[2]);
        assert!(!l.pick_wizard(b, 1), "a's wizard");
        assert!(!l.pick_wizard(b, 0), "the admin's wizard");
        assert!(!l.pick_wizard(b, WIZARDS), "no such wizard");
        assert!(l.pick_wizard(b, 5));
        assert!(l.leave(a));
        assert!(l.pick_wizard(b, 1), "freed by the leaver");
    }

    #[test]
    fn the_totems_skip_held_wizards_and_wrap() {
        let (mut l, ids) = lobby_of(2, 4);
        let a = ids[1];
        l.pick_wizard(a, 2);
        let v = l.view_for(a);
        assert_eq!(v.next_free(2, 1), Some(3));
        assert_eq!(v.next_free(1, -1), Some(7), "0 is the admin's — wraps");
        let mut full = LobbyCore::new(
            NetGame::Mc2,
            "baked/mc2/level-050.mgcl".into(),
            json!({}),
            8,
        );
        while full.join(String::new()).is_ok() {}
        assert_eq!(full.view_for(1).next_free(0, 1), None);
    }

    /// Rulings 2, 3 and 7: Play needs every other member's agreement
    /// to the CURRENT revision, and any admin change voids them all.
    #[test]
    fn an_admin_change_voids_every_agreement() {
        let (mut l, ids) = lobby_of(1, 4);
        assert!(!l.can_play(), "alone");
        let a = l.join(String::new()).unwrap();
        let b = l.join(String::new()).unwrap();
        assert!(l.agree(a, 0));
        assert!(!l.can_play(), "b has not agreed");
        assert!(l.agree(b, 0));
        assert!(l.can_play());
        assert!(l.pick_wizard(b, 6), "a wizard is not a rule");
        assert!(l.can_play());
        assert!(
            !l.set_level("baked/mc2/level-050.mgcl".into()),
            "no change, no void"
        );
        assert!(
            !l.set_level("baked/mc1/level-051.mgcl".into()),
            "another game's"
        );
        assert!(l.set_level("baked/mc2/level-051.mgcl".into()));
        assert!(!l.can_play());
        assert!(!l.agree(a, 0), "a stale revision never lands");
        assert!(l.agree(a, 1) && l.agree(b, 1));
        assert!(l.set_rules(json!({"fog": 90})));
        assert_eq!(l.rev(), 2);
        assert!(!l.can_play());
        assert!(l.view_for(ids[0]).is_admin());
    }

    #[test]
    fn the_admin_cannot_agree_or_withdraw_and_always_reads_agreed() {
        let (mut l, ids) = lobby_of(2, 4);
        assert!(!l.withdraw(ids[0]));
        assert!(!l.agree(ids[0], 0));
        let v = l.view_for(ids[1]);
        assert!(v.members[0].agreed && !v.members[1].agreed);
        assert_eq!(v.agreement(), (1, 0));
    }

    /// The first member in is the admin; a claim (the UI's Host) takes
    /// the role over; the admin's leaving hands it on.
    #[test]
    fn the_admin_is_the_first_in_a_claim_or_the_next_in_line() {
        let (mut l, ids) = lobby_of(3, 4);
        assert_eq!(l.admin(), Some(ids[0]));
        assert!(l.claim_admin(ids[2]));
        assert!(l.view_for(ids[2]).is_admin() && !l.view_for(ids[0]).is_admin());
        assert!(!l.claim_admin(99), "not a member");
        assert!(l.leave(ids[2]));
        assert_eq!(l.admin(), Some(ids[0]), "the longest-standing member");
        assert!(l.leave(ids[0]) && l.leave(ids[1]));
        assert_eq!(l.admin(), None);
        let c = l.join(String::new()).unwrap();
        assert_eq!(l.admin(), Some(c), "the next one in");
    }

    #[test]
    fn names_travel_and_the_limit_holds_the_members_in() {
        let mut l = LobbyCore::new(
            NetGame::Mc2,
            "baked/mc2/level-050.mgcl".into(),
            json!({}),
            4,
        );
        l.join("ARIEL".into()).unwrap();
        let a = l.join("BRAN".into()).unwrap();
        assert_eq!(l.view_for(a).names(), vec!["ARIEL", "BRAN"]);
        assert_eq!(l.view_for(a).capacity, 4);
        l.join(String::new()).unwrap();
        assert!(l.set_capacity(2));
        assert_eq!(l.view_for(a).capacity, 3, "never below the members in");
        l.set_capacity(9);
        assert_eq!(l.view_for(a).capacity, 4, "nor above the session's most");
    }

    #[test]
    fn the_ai_count_is_a_rule_capped_by_the_free_seats() {
        let (mut l, ids) = lobby_of(2, 4);
        l.agree(ids[1], 0);
        assert!(l.set_ai(3));
        assert!(!l.can_play(), "a rule change voids");
        assert!(l.set_ai(9));
        assert_eq!(l.view_for(ids[0]).ai, 6, "8 seats - 2 humans");
    }

    #[test]
    fn a_taken_name_is_refused() {
        let mut l = LobbyCore::new(
            NetGame::Mc2,
            "baked/mc2/level-050.mgcl".into(),
            json!({}),
            4,
        );
        l.join("Zanzamar".into()).unwrap();
        assert!(l.join("ZANZAMAR".into()).is_err(), "case-blind");
        assert!(l.join("Rahn".into()).is_ok());
        assert!(l.join(String::new()).is_ok(), "no name, no clash");
        assert!(l.join(String::new()).is_ok());
    }

    #[test]
    fn a_level_names_its_game() {
        assert_eq!(
            NetGame::of_level("baked/mc1hw/level-003.mgcl"),
            Some(NetGame::Mc1hw)
        );
        assert_eq!(
            NetGame::of_level("/x/baked/mc2/level-050.mgcl"),
            Some(NetGame::Mc2)
        );
        assert_eq!(NetGame::of_level("lvl"), None);
        assert!(NetGame::parse("mc3").is_err());
    }

    #[test]
    fn reopening_after_a_game_asks_again() {
        let (mut l, ids) = lobby_of(2, 4);
        l.agree(ids[1], 0);
        assert!(l.can_play());
        l.reopen();
        assert!(!l.can_play());
        assert_eq!(l.rev(), 0, "same settings");
        assert!(l.agree(ids[1], 0));
    }
}
