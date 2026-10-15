//! The netplay lobby's state — HOST-AUTHORITATIVE, socket-free (the
//! wire lives in `net.rs`). The player's rulings (2026-10-15):
//!
//! 1. Only the host changes parameters — the level and the session
//!    rules ([`crate::rules`]).
//! 2. Every client must AGREE to them before play (a consensus) …
//! 3. … and any host change voids EVERY agreement (no last-second
//!    switcheroo). Agreements name the rules revision they accept, so a
//!    stale one can never land.
//! 4. The host can kick anyone (`net::Host::kick`).
//! 7. Only the host presses Play, and only once everyone has agreed.
//! 8. Wizards are first come, first served: one a member holds can't be
//!    taken by anyone else.
//! 10. After a game the lobby reopens with the same players and
//!     settings; everyone approves Play again.
//!
//! A wizard IS a seat here: the port's seat decides the colour, the
//! castle and the carpet art, so the lobby's wizard choice (retail's
//! totems) picks the seat. Retail kept a separate join-order seat.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wizard seats a level can hold (retail: 8 portraits / 8 colours).
pub const WIZARDS: u8 = 8;

/// One lobby member as everyone sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// Stable for the session (the host is 0); never reused.
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
    /// Has agreed to the current rules revision. The host's own
    /// agreement is his Play press, so his row is always agreed.
    pub agreed: bool,
}

/// The lobby as one member sees it — what the host broadcasts on
/// every change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LobbyView {
    /// The receiving member's id.
    pub you: u32,
    /// The rules revision: bumped by every host change to the level
    /// or the rules.
    pub rev: u32,
    /// The level (a baked level path; every peer runs the same build).
    pub level: String,
    /// The session rules (`rules::collect` of the host's config).
    pub rules: Value,
    /// In join order; `members[0]` is the host.
    pub members: Vec<Member>,
    /// The player limit (the host sets it; the lintel's left box).
    pub capacity: u8,
    /// AI wizards to seat besides the humans (the host's rule; retail's
    /// network game seats none — the default). Capped at the free seats
    /// when the game starts.
    #[serde(default)]
    pub ai: u8,
}

impl LobbyView {
    pub fn me(&self) -> Option<&Member> {
        self.members.iter().find(|m| m.id == self.you)
    }

    pub fn is_host(&self) -> bool {
        self.members.first().is_some_and(|m| m.id == self.you)
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

    /// Play is open: at least one client, and every client agreed.
    pub fn all_agreed(&self) -> bool {
        self.members.len() > 1 && self.members.iter().all(|m| m.agreed)
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

/// The host's lobby model.
pub struct LobbyCore {
    rev: u32,
    level: String,
    rules: Value,
    members: Vec<Member>,
    next_id: u32,
    capacity: usize,
    /// The most the host may set ([`Self::set_capacity`]).
    max: usize,
    ai: u8,
}

impl LobbyCore {
    /// A lobby holding the host (`name`) alone, flying `host_wizard`,
    /// with room for `capacity` members in all.
    pub fn new(
        host_wizard: u8,
        name: String,
        level: String,
        rules: Value,
        capacity: usize,
    ) -> Self {
        let max = capacity.clamp(1, usize::from(WIZARDS));
        Self {
            rev: 0,
            level,
            rules,
            members: vec![Member {
                id: 0,
                wizard: host_wizard.min(WIZARDS - 1),
                name,
                models: Value::Null,
                agreed: true,
            }],
            next_id: 1,
            capacity: max,
            max,
            ai: 0,
        }
    }

    pub fn view_for(&self, you: u32) -> LobbyView {
        LobbyView {
            you,
            rev: self.rev,
            level: self.level.clone(),
            rules: self.rules.clone(),
            members: self.members.clone(),
            capacity: self.capacity as u8,
            ai: self.ai,
        }
    }

    #[cfg(test)]
    fn rev(&self) -> u32 {
        self.rev
    }

    /// Admit a new member (`name`) on the first free wizard; refused
    /// when full, or when another member already goes by that name
    /// (case-blind — two ZANZAMARs in one game is no game; player
    /// ruling 10-15).
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
                Ok(id)
            }
            _ => Err(format!("the lobby is full ({} players)", self.capacity)),
        }
    }

    /// A member left or was kicked: his wizard is free again. The host
    /// can't leave his own lobby (closing it is dropping it).
    pub fn leave(&mut self, id: u32) -> bool {
        let before = self.members.len();
        self.members.retain(|m| m.id == 0 || m.id != id);
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

    /// Member `id` agrees to revision `rev`; a stale revision (the host
    /// changed something meanwhile) is ignored.
    pub fn agree(&mut self, id: u32, rev: u32) -> bool {
        match self.members.iter_mut().find(|m| m.id == id && m.id != 0) {
            Some(m) if rev == self.rev && !m.agreed => {
                m.agreed = true;
                true
            }
            _ => false,
        }
    }

    /// Member `id` takes his agreement back.
    pub fn withdraw(&mut self, id: u32) -> bool {
        match self.members.iter_mut().find(|m| m.id == id && m.id != 0) {
            Some(m) if m.agreed => {
                m.agreed = false;
                true
            }
            _ => false,
        }
    }

    /// The host sets the level; a change voids every agreement.
    pub fn set_level(&mut self, level: String) -> bool {
        if level == self.level {
            return false;
        }
        self.level = level;
        self.void();
        true
    }

    /// The host sets the AI wizard count (0 up to the seats the humans
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

    /// The host sets the rules; a change voids every agreement.
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
        for m in self.members.iter_mut().filter(|m| m.id != 0) {
            m.agreed = false;
        }
    }

    /// After a game: same players and settings, every client approves
    /// Play again.
    pub fn reopen(&mut self) {
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

    /// The host sets the player limit: 2 up to the most the session
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

    fn lobby() -> LobbyCore {
        LobbyCore::new(0, String::new(), "lvl".into(), json!({"fog": 40}), 4)
    }

    #[test]
    fn joiners_take_the_first_free_wizard_until_full() {
        let mut l = LobbyCore::new(1, String::new(), "lvl".into(), json!({}), 3);
        let a = l.join(String::new()).unwrap();
        let b = l.join(String::new()).unwrap();
        assert_eq!(l.view_for(a).seats(), vec![1, 0, 2]);
        assert!(l.join(String::new()).is_err(), "capacity 3");
        assert!(l.leave(b));
        assert_eq!(
            l.join(String::new()).map(|_| l.view_for(0).seats()),
            Ok(vec![1, 0, 2])
        );
    }

    #[test]
    fn a_held_wizard_cannot_be_taken() {
        let mut l = lobby();
        let a = l.join(String::new()).unwrap();
        let b = l.join(String::new()).unwrap();
        assert!(!l.pick_wizard(b, 1), "a's wizard");
        assert!(!l.pick_wizard(b, 0), "the host's wizard");
        assert!(!l.pick_wizard(b, WIZARDS), "no such wizard");
        assert!(l.pick_wizard(b, 5));
        assert!(l.leave(a));
        assert!(l.pick_wizard(b, 1), "freed by the leaver");
    }

    #[test]
    fn the_totems_skip_held_wizards_and_wrap() {
        let mut l = lobby();
        let a = l.join(String::new()).unwrap();
        l.pick_wizard(a, 2);
        let v = l.view_for(a);
        assert_eq!(v.next_free(2, 1), Some(3));
        assert_eq!(v.next_free(1, -1), Some(7), "0 is the host's — wraps");
        let mut full = LobbyCore::new(0, String::new(), "l".into(), json!({}), 8);
        while full.join(String::new()).is_ok() {}
        assert_eq!(full.view_for(0).next_free(0, 1), None);
    }

    /// Rulings 2, 3 and 7: Play needs every client's agreement to the
    /// CURRENT revision, and any host change voids them all.
    #[test]
    fn a_host_change_voids_every_agreement() {
        let mut l = lobby();
        assert!(!l.can_play(), "alone");
        let a = l.join(String::new()).unwrap();
        let b = l.join(String::new()).unwrap();
        assert!(l.agree(a, 0));
        assert!(!l.can_play(), "b has not agreed");
        assert!(l.agree(b, 0));
        assert!(l.can_play());
        assert!(l.pick_wizard(b, 6), "a wizard is not a rule");
        assert!(l.can_play());
        assert!(!l.set_level("lvl".into()), "no change, no void");
        assert!(l.set_level("other".into()));
        assert!(!l.can_play());
        assert!(!l.agree(a, 0), "a stale revision never lands");
        assert!(l.agree(a, 1) && l.agree(b, 1));
        assert!(l.set_rules(json!({"fog": 90})));
        assert_eq!(l.rev(), 2);
        assert!(!l.can_play());
    }

    #[test]
    fn the_host_cannot_leave_or_withdraw() {
        let mut l = lobby();
        assert!(!l.leave(0));
        assert!(!l.withdraw(0));
        assert!(!l.agree(0, 0));
        assert!(l.view_for(0).is_host());
    }

    #[test]
    fn names_travel_and_the_limit_holds_the_members_in() {
        let mut l = LobbyCore::new(0, "ARIEL".into(), "l".into(), json!({}), 4);
        let a = l.join("BRAN".into()).unwrap();
        assert_eq!(l.view_for(a).names(), vec!["ARIEL", "BRAN"]);
        assert_eq!(l.view_for(a).capacity, 4);
        l.join(String::new()).unwrap();
        assert!(l.set_capacity(2));
        assert_eq!(l.view_for(0).capacity, 3, "never below the members in");
        l.set_capacity(9);
        assert_eq!(l.view_for(0).capacity, 4, "nor above the session's most");
    }

    #[test]
    fn the_ai_count_is_a_rule_capped_by_the_free_seats() {
        let mut l = lobby();
        let a = l.join(String::new()).unwrap();
        l.agree(a, 0);
        assert!(l.set_ai(3));
        assert!(!l.can_play(), "a rule change voids");
        assert!(l.set_ai(9));
        assert_eq!(l.view_for(0).ai, 6, "8 seats - 2 humans");
    }

    #[test]
    fn a_taken_name_is_refused() {
        let mut l = LobbyCore::new(0, "Zanzamar".into(), "l".into(), json!({}), 4);
        assert!(l.join("ZANZAMAR".into()).is_err(), "case-blind");
        assert!(l.join("Rahn".into()).is_ok());
        assert!(l.join(String::new()).is_ok(), "no name, no clash");
        assert!(l.join(String::new()).is_ok());
    }

    #[test]
    fn reopening_after_a_game_asks_again() {
        let mut l = lobby();
        let a = l.join(String::new()).unwrap();
        l.agree(a, 0);
        assert!(l.can_play());
        l.reopen();
        assert!(!l.can_play());
        assert_eq!(l.rev(), 0, "same settings");
        assert!(l.agree(a, 0));
    }
}
