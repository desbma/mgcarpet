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
- The joiner **adopts** the host's level and sim-affecting options
  (`sim`, `gameplay` incl. patches + cheats, `controls.models`,
  `dev.plausible_spellbook`). Both must run the **same binary** (the
  handshake compares a hash of the executable); a mismatch is a refusal.
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

- Sim-affecting options (menu or keys) snap back to the agreed set with a
  toast; game speed may change (it only paces).
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

1. **Session rules from the host.** A lobby phase before the level starts:
   the host edits one ruleset, clients receive every change, Start freezes
   it. Two kinds of pin: *determinism* (`sim`, `gameplay.patches`,
   `gameplay.cheat`, `controls.models` — already adopted at the handshake
   and re-adopted on a local toggle) and *fairness* (render-side: fog
   distance, map extent fog, beyond-sight areas, trigger overlays, rival
   tags/selector surfaces, sight mist; the available spell set). Pinned options show locked in the
   client's menu.
2. **AI opponents on/off** (a port option; retail always fills). Needs the
   win condition re-read with no rivals (last castle standing).
3. **In-game connect + lobby UI — its own development (player, 10-15).**
   Use the RETAIL lobby: both games' main menus have a Multiplayer button,
   inactive today (and in retail without a network run). Wire it to the
   retail lobby screens, tweaked for us. The player's campaign profile
   carries into a netplay session — PREFERENCES only (user tag/name, spell
   hotkeys, selector quick-select), never the collected spells: everyone
   gets the same arsenal, and the available spell set is itself a fairness
   option (a deliberately limited arsenal). Leads: MC1 button 3 → menu
   sel 4, multiplayer screen at remc1 :60004 (song 1), validity gate
   :58861, `frontend_mc1.rs` `hot_id` (id 3); MC2 `Button::Multiplayer`
   (362,72) in `frontend.rs`, remc2 `MenusAndIntros.cpp` / `Network.cpp`.
   Also needed: Host / Join (address entry, recent
   hosts), the seat chooser, the ruleset page, Start. ⚖ No LAN
   autodiscovery (player, 10-15): it would not carry past the LAN anyway —
   connect by typed address + recent hosts; finding games later is a
   service (rendezvous / lobby server), its own track.
4. **Desync recovery.** Detection exists (24-tick hash). Next: a
   divergence report (which part of the state differs), then the host's
   snapshot wins (save-format world snapshot over the wire, client
   reloads and re-simulates the turns it already holds).
5. Later: host handover; per-seat flight models; `Gen::local_seat()`
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
