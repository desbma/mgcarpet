# Netplay (MVP, 2026-10-15)

Two app instances, one world, **deterministic lockstep over TCP**, star
topology with the host as the authority. Code: `crates/mgc-app/src/net.rs`
(wire + lockstep), `App::net_turn` (the per-tick gate), `net_handshake`
(launch folding). Rulings behind it: TCP only (10-14), star + host
authority, retail's own model of shipping inputs only.

## Running it

```
mgcarpet --level baked/mc1/level-002.mgcl --host            # seat 1, port 7777, all interfaces
mgcarpet --join 127.0.0.1                                    # seat 2 (the host assigns it)
```

- `--host [ADDR]`: `host:port`, a bare port, or nothing (`0.0.0.0:7777`).
  Needs a `--level`; `--seat N` picks the host's wizard seat.
- `--join ADDR`: `host:port` or a bare host (port 7777). Retries for 60 s,
  so either side may start first. `--seat N` asks for a seat; without it
  the host assigns seat 2 (seat 1 if the host flies another).
- The joiner **adopts** the host's level and the session rules (below),
  and prints which of his own options they override. Both must run the
  **same binary** (the handshake compares a hash of the executable); a
  mismatch is a refusal.
- Not with `--campaign`, `--replay`, `--record`, `--film`,
  `--set-spellbook` or the headless modes.

## How it works

- Every peer builds the SAME two-human world (`load_level(.., seats)`:
  `seats[0]` is the host's wizard, the rest `World::add_human_seat`);
  the host flies `Hx(0)`, the client `Hx(1)` (`local_hx()`).
- Each seat's input is sampled `INPUT_DELAY` = 3 ticks (125 ms) ahead
  and sent to the host; the host pairs it with its own and broadcasts
  `Turn t` (every seat's `PortInput`, seat order). A tick steps
  (`Simulation::step_seats`) only once its turn exists — a starved peer
  waits ("waiting for the other player…").
- Wire: little-endian `u32` length + one JSON message. Idle input is `{}`.
- **Desync detection**: the host stamps its pre-step `state_hash` every
  24 ticks onto a turn; the client compares and toasts/logs the first
  `DESYNC at tick N`. Detection only — no reconciliation yet.
  `MGC_NET_TRACE=1` prints every check on both peers (diff the logs).
- On exit each peer prints `net: N ticks stepped, M hash checks agreed`.

## Pinned for the session

The session rules are one table, `rules::PINS` (`crates/mgc-app/src/rules.rs`):
the host's values for those config paths travel in the Welcome and every
peer plays by them.

- **Sync** (the worlds fork without it): `sim` (incl. game speed — a faster
  peer would only stall on turns), `gameplay.patches`, `gameplay.cheat`,
  `controls.models`, `dev.plausible_spellbook`.
- **Fair** (shows one seat more than another): fog distance, rival tags,
  owned buildings + jar spells on the map, map extent fog, beyond-sight
  areas, creature health bars, autoaim hints, trigger-area overlay.
- Everything else is the player's own (bindings, mouse, audio, the spell
  selector, autofire, look-only render options). ⚖ Autofire and the fly
  assistant are preferences (player, 10-15); the table gets fine-tuned
  over time.
- In play, a pinned option snaps back with a toast (keys), and its menu row
  is locked "fixed by the host" — a preset row too when it would set a
  pinned member (Controls: the flight models; Visuals: fog, tags, map aids).
- A pinned value is never written to the player's `mgcarpet.json` (it is
  the host's, not his).
- Pause (P) stalls BOTH peers and burns no draws (a one-sided draw would
  fork the worlds — the netplay-only exception to the paused-draw law).
- A castle-less death's level restart ends the tick burst on its tick, so
  both peers rebuild before the same next tick.
- The peer leaving ends netplay: the game goes on locally, the other
  carpet idles.

## Evidence

- `net::tests` (3): bundles identical on both peers, lead-in idle, a hash
  mismatch reported once, a departing peer closes the session.
- `net_lockstep_tests` (2): two worlds built through the app's own
  `load_level`, scripted weaving flight + casts per seat, 2400 ticks
  through the real TCP lockstep — hash-identical every tick (MC1 level 2,
  MC2 level 1), the client's carpet flies. Negative control: one forked
  client input at tick 300 → the test pins tick 300, the network check
  reports at tick 312.
- Two windowed apps under Xvfb (idle input, ~28 s): 673 ticks, 29/29
  checks identical.

## Open (next stages)

Roadmap agreed with the player 2026-10-15 (recommended order):

1. **Session rules from the host.** ✅ 1a (10-15): `rules::PINS` — sync +
   fairness pins, adopted at the handshake, held in play, menu-locked,
   never persisted (see "Pinned for the session"). Today the host's own
   config IS the ruleset; the lobby (step 3) becomes its editor, sending
   each change before Start. ⏭ 1b: the **available spell set** as a rule
   (a limited arsenal for every seat — sim-side withholding, MC2's
   `BlockedSpells` is the precedent; MC1 has none).
2. **AI opponents on/off** (a port option: retail NEVER fills network
   seats with AI in either game — MC1 recon §, MC2 EF:38449). Needs the
   win condition re-read with no rivals (last castle standing).
3. **In-game connect + lobby UI — its own development (player, 10-15).**
   Use the RETAIL lobby: both games' main menus have a Multiplayer button,
   inactive today (and in retail without a network run). Wire it to the
   retail lobby screens, tweaked for us. The player's campaign profile
   carries into a netplay session — PREFERENCES only (user tag/name, spell
   hotkeys, selector quick-select), never the collected spells: everyone
   gets the same arsenal, and the available spell set is itself a fairness
   option (a deliberately limited arsenal). Leads: MC1 button 3 → menu
   sel 4, the PMULTI screen `sub_4D680` (:59382; no music), validity gate
   :58861, `frontend_mc1.rs` `hot_id` (id 3); MC2 `Button::Multiplayer`
   (362,72) in `frontend.rs`, remc2 `MenusAndIntros.cpp` / `Network.cpp`.
   Retail recon: `docs/traces/mc1-multiplayer-lobby.md`, `mc2-multiplayer-lobby.md`
   (10-15). Also needed: Host / Join (address entry, recent
   hosts), the seat chooser, the ruleset page, Start. ⚖ No LAN
   autodiscovery (player, 10-15): it would not carry past the LAN anyway —
   connect by typed address + recent hosts; finding games later is a
   service (rendezvous / lobby server), its own track.
   **⚖ Lobby rulings (player, 10-15) — the first lobby is MC2's:**
   1. Only the host (server) changes parameters.
   2. Every client must AGREE to the rules — a consensus.
   3. Any host change to an option voids EVERY agreement (no last-second
      switcheroo).
   4. The host can KICK any client (misbehaving, refusing to agree).
   5. No chat / in-game messaging (the retail "I" key) — players talk
      over voice (Discord) these days.
   6. A session needs a PASSPHRASE to enter: the host hands out hostname +
      passphrase, and can change it (a kicked player can't come back).
      Maybe optional later; required for now.
   7. Only the host sets the level and presses Play. The level is part of
      the rules everyone agrees to; Play is possible only once everyone
      has agreed.
   8. Wizards (the totems) are first come, first served: a wizard someone
      holds can't be chosen by anyone else.
   9. The rules get shown in the lobby somehow — design open (the player
      wants the lobby art in front of him first). The first working
      version doesn't show them.
   10. After a game: back to the lobby, same players and settings; everyone
      approves Play again (or the host changes things).
   Lobby art renders: `sprite-export/mc2-lobby/` (bg, 8 wizard portraits,
   sprite sheet).

4. **Desync recovery.** Detection exists (24-tick hash). Next: a
   divergence report (which part of the state differs), then the host's
   snapshot wins (save-format world snapshot over the wire, client
   reloads and re-simulates the turns it already holds).
5. Later: 🏦 **spectator ghosts** (player idea, 10-15, banked — "a cherry
   on top"): a BANISHED player rises from his last corpse as a visible
   in-game ghost (mostly translucent, greyscale or another filter), flies
   freely and watches the rest of the battle, but can neither touch nor be
   touched. Starting points: the `ghost`/`inert` cheats (unseen,
   no triggers) and the greyscale death look. Open: other players see the
   ghost, so its flight must stay in the lockstep as a seat that cannot
   interact, not a local-only camera.
   Also later: host handover; per-seat flight models; `Gen::local_seat()`
   per peer; level end / campaign; more than two seats.

## Playtest log

- 10-15 (MC1, seats 1 + 2 = Zanzamar + Vodor): works; seat, castle and HUD
  colours right. BUG: both humans drew Zanzamar's carpet sprite — the MC1
  human record kept the constructor's row 44 and never took retail's
  per-seat art (`sub_44D30` :54927-55, run for every wizard incl. the
  human). Fixed: `mc1_wizard_sprite_row(seat)` on the human record's class
  flip (seat 0 unchanged = 44, so single-player is byte-identical; MC2
  already used `carpet_sprite_row(seat)`). Test
  `each_human_wears_his_seats_art` (kill-switch proved).
- 2026-10-15 #3: the other human had no rival tag and was missing from the
  map roster (ALT / the strip below the map); both read only the AI rivals.
  Fixed: `World::human_views(local)` presents the other seats in the
  rival-view shape (seat's retail table name — no entered names travel
  yet), feeding the tag, the Beyond-Sight map reveal and the roster. Test
  `each_human_sees_the_other_on_tag_and_roster` (kill-switch proved).
  Playtest passed.
- 2026-10-15 #4 (step 1a, session rules): `rules::PINS` replaces the
  ad-hoc `net_config`. Fixed on the way: a joiner touching a pinned option
  used to snap it back AND save the host's value into his own
  `mgcarpet.json`; `gameplay.enhancement` (spell selector, wheel) was
  adopted from the host but is a preference. Tests: `rules::tests` (5, the
  fog pin kill-switch-proved), `menu::tests::session_pins_lock_their_rows`;
  Xvfb two-app smoke: the joiner's `--fog-distance 70` overridden by the
  host's 40 (reported), 19/19 hash traces identical. Playtest deferred to the lobby (player, 10-15).
