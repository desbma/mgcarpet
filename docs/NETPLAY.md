# Netplay (MVP, 2026-10-15)

Two app instances, one world, **deterministic lockstep over TCP**, star
topology with the host as the authority. Code: `crates/mgc-app/src/net.rs`
(wire + lockstep), `App::net_turn` (the per-tick gate), `net_handshake`
(launch folding). Rulings behind it: TCP only (10-14), star + host
authority, retail's own model of shipping inputs only.

## Running it

**In the game (MC2):** `mgcarpet --campaign mc2` → the temple's
Multiplayer button → the "Enter Network Session" scroll: **Host** (a port,
default 7777) or **Join** (the host's address; recent hosts listed), plus
the passphrase → the lobby (retail HSCREEN0 case 7, `lobby_mc2.rs`):
- totems = previous/next free wizard (your seat); the doorway shows your
  portrait, the lintel windows every member incl. you (bright = agreed,
  dim = not; hover one for the player's name); the lintel's left box (by
  the figure) = the player limit (the host cycles it), right box (by the
  ring) = players in;
- the session rules on a parchment scroll at the right (`LOBBY_RULES`):
  the host steps each row left/right (its first row is the passphrase),
  clients read them;
- client: the Play plaque agrees / takes the agreement back;
- host: the level arrows (arenas 50-59), Play once everyone agreed, click a
  lit window to kick (confirmed); the host's options (the scroll or the P
  menu) ARE the rules — an edit reaches everyone and voids the agreements;
- names: each member carries his campaign's player name (MC2 Set Name /
  MC1's save name) — the in-game tags, map reveals and roster show it;
  empty = the wizard's own name;
- Exit (or Esc) leaves (the host: closes the lobby).
In the game: the host's Esc → exit ends it for everyone (back to the
lobby); a client's leaves the session. A won level returns to the lobby.
The campaign steps aside for a lobby game (no campaign book — every seat
gets the level's own — and no campaign progress or saves). Entries
persist in `saves/netplay.json` (recent hosts, port, passphrase).

**From the CLI:**

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
- `--passphrase WORD`: the session's passphrase (host sets, joiner must
  match; empty when omitted). Not a secret — just keeps strangers and kicked
  players out.
- Under the hood the CLI goes through the LOBBY (`lobby.rs` +
  `net::Host`/`net::Guest`, protocol 2): `--join` agrees to the host's rules
  by itself and `--host` presses Play once it has. The in-game lobby UI
  drives the same calls.
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

- **Sync** (the worlds fork without it): `sim` (incl. game speed),
  `gameplay.patches`, `gameplay.cheat`, `dev.plausible_spellbook`.
- **Fair** (shows one seat more than another): fog distance, rival tags,
  owned buildings + jar spells on the map, map extent fog, beyond-sight
  areas, creature health bars, autoaim hints, trigger-area overlay.
- ⚖ **The session's values** (player, 10-15): a multiplayer game runs
  every pin at its DEFAULT — no cheats, the port's fixes (retail gameplay
  may be unplayable without them), the aids and debug overlays as shipped
  (MC2: rival tags and houses on the map on), the dev sim parameters off —
  save `rules::HOST_CHOSEN` (the fog), the host's pick on the lobby's
  scroll. `rules::session` builds it; the HOST plays by it too.
- **Per seat**: the flight models (thrust / altitude) are each player's
  own (designed near-equivalent): each member's travels with Start and the
  sim flies every carpet with its own (`App::apply_net_models`); frozen in
  a game (`rules::locked_in_game`). The one world-level arm keyed on the
  altitude model, `mc2_leviathan_high_lunge`, is on when ANY seat flies
  the enhanced lift (`App::world_patches_now`).
- Everything else is the player's own (bindings, mouse, audio, the spell
  selector, autofire, look-only render options). ⚖ Autofire and the fly
  assistant are preferences (player, 10-15); the table gets fine-tuned
  over time.
- In play, a pinned option snaps back with a toast (keys), and its menu row
  is locked "fixed by the host" — a preset row too when it would set a
  pinned member (Controls: the flight models; Visuals: fog, tags, map aids).
- A pinned value is never written to the player's `mgcarpet.json` (it is
  the host's, not his).
- A network game never pauses (player ruling 2026-10-16; retail refused
  pause in multiplayer): P opens the netplay panel — Options and Leave, no
  Save/Load — over the RUNNING world, and the carpet holds an idle hover
  while it is up (no movement read, a full stop every tick, no fire;
  equips and spell picks still pass). Leave = the abandon confirm. No
  host-only pause either (ruled out). The sim-side guard stays: were a
  peer ever paused, it would burn no draws (a one-sided draw would fork
  the worlds).
- A castle-less death's level restart ends the tick burst on its tick, so
  both peers rebuild before the same next tick.
- A CLIENT leaving mid-game (Leave, or its connection lost) is BANISHED
  and the game goes on for the rest (player 2026-10-17; retail: an abandon
  banishes the wizard). The host drops the seat at the first turn not yet
  sent — its input there is the suicide key, idle after — and that turn
  carries `left` (protocol 4); every peer calls `World::depart` before
  stepping it. A send failure to one client departs that client only. The
  usual game over follows (one contender left → "<name> is victorious!",
  back to the lobby). The HOST leaving still ends netplay: a client whose
  host is gone goes on locally, the other carpets idle (host handover is
  a later stage).

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
   (a limited arsenal for every seat — sim-side withholding). ⚖ MC1 ONLY
   (player, 10-15): MC2's arenas make their own spell selection by which
   jars they place; MC1 gives everyone everything.
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
- 2026-10-15 #5 (lobby core, no UI yet): `lobby::LobbyCore` (host model:
  members, first-come wizards, rules revision, agreements, Play gate,
  reopen) + `net::Host` / `net::Guest` (passphrase + build handshake, kick,
  snapshot broadcast per change, Start) + `Lockstep` on the session's
  connections with `end()` → back to the lobby (ruling 10); late joiners
  refused mid-game. Lobby capacity `MAX_PLAYERS` = 2 (the lockstep is
  N-seat; >2 humans in one world unproven). Tests: 6 lobby + 5 socket
  tests (passphrase/build refusal, agreement void, kick + passphrase
  change, first-come wizard, game → lobby → second game + late-join
  refusal); kill-switch proved on the passphrase check and the stale-rev
  guard. Xvfb two-app run via the CLI lobby: wrong passphrase refused,
  right one in, 21/21 hash traces identical.
- 2026-10-15 #6 (MC2 lobby UI): bake epoch 28 (`assets/mc2-ui/lobby-*`,
  HSCREEN0 case 7 + 8 portraits); `lobby_mc2::LobbyScreen` (retail art,
  palette windows as tinted masks — tints decoded to linear via
  `entities::srgb_lin`, the swapchain is sRGB); the temple's Multiplayer
  scroll (Host/Join/address/port/passphrase/recents, "Attempting
  Connection" with the busy cursor, a message scroll for refusals); app
  flow (`NetLobby`, `start_net_game`, `net_game_over`). Fixed on the way:
  `load_level` refused seat 2 on the five arenas whose header says 1 player
  (51/52/55/56/57) — a multi-seat launch now checks the start markers only
  (test `every_mc2_arena_seats_two_humans`, kill-switch proved). Xvfb +
  xdotool two-app run through the real UI: host, join (a doubled
  passphrase was refused), agree, Play, 187 ticks / 8 of 8 hash checks
  agreed, the host's exit → both back in the lobby with the agreement
  cleared. ⏭ playtest.
- 2026-10-15 #7 (playtest round on the lobby): (1) campaign names travel
  (Hello/Start, protocol 3) and replace the table names on tags/roster
  (`name_humans`; `RivalView::name` is a `String` now); (2) your own lintel
  window lights too; (3) the lintel boxes = limit (left, host cycles,
  `LobbyCore::set_capacity`) / players in (right); the passphrase moved to
  the rules scroll; (4) the rules scroll (`LOBBY_RULES`, 17 pins +
  passphrase; `settings::step` steps any Spec one notch; "Fixes" = port /
  retail / custom). Tests: names travel, limit holds, scroll rows are all
  pins and read, step, name override. Xvfb shots: names on hover, both
  scrolls. ⏭ playtest.
- 2026-10-15 #8 (playtest round 2 on the lobby): (1) the fog's left
  chevron steps down (the click split the row, both chevrons sat in its
  right half); (2) the limit cycles 2..8 — `MAX_PLAYERS` 8 after
  `four_mc2_peers_stay_identical` / `three_mc1_peers_stay_identical` (the
  real lobby, mixed per-seat models, 1,200 ticks identical); (3) the
  session's values: defaults but the host's fog (`rules::session`), the
  scroll shows passphrase + fog only; (4) per-seat flight models. Xvfb:
  limit 8→2→3→4, fog 90→70→50. ⏭ playtest.
- 2026-10-15 #9 (playtest, two windows on one desktop): after Play the
  BACKGROUND window grabbed the pointer (an X11 grab is global) while the
  focused one kept the keyboard — Esc went to the window not holding the
  pointer, so neither could be freed. Fixed in `set_grab`: an unfocused
  window never grabs; it arms the existing focus-gated wish (`boot_grab`),
  completed when the window is focused (click / alt-tab). The map's
  confinement got the same gate. Xvfb: the client logs "the grab waits for
  this window's focus", the focused host grabs. ⏭ playtest.
- 2026-10-15 #10 (playtest round 3): (1) a spell pick's toast and chime
  reached every human — notifications now carry an optional seat
  (`set_seat_notification*`, `World::notifications_for`; the MC2 pick and
  level-up are seat-scoped, deaths stay global) and the app plays only its
  own seat's PLAYER sounds (test `a_spell_pick_toasts_and_chimes_for_its_
  own_player_only`, kill-switch proved); (2) names on the lintel, zigzag
  at each window (player design: even below, odd above, centred); (3) a
  taken name is refused at the join, case-blind (`LobbyCore::join`) — a
  fresh MC2 campaign is "Zanzamar", so two fresh players collide by
  design. ⏭ open: banishment in MP (interim until the ghost) + the game's
  end + the AI-wizards rule.
- 2026-10-15 #11 (banishment, the game's end, AI wizards — player-agreed
  plan): (1) a castle-less death in a multi-human world BANISHES the seat
  (`World::banish_instead` at the head of both respawn paths: "<name> has
  been banished from the realm.", never respawns, no level restart; the
  flag is `Player::banished`, hashed only once set, NOT in the snapshot —
  lobby games are never saved); the banished player stays in the lockstep
  watching ("Banished - waiting for the game to end"), the interim until
  the spectator ghost; (2) `App::net_standings`: when one wizard alone
  stands (humans not banished + rivals not eliminated ≤ 1) or no human
  does, "<winner> is victorious!" and the fade takes everyone back to the
  lobby; (3) "AI wizards" 0..(8 − players) on the rules scroll
  (`LobbyCore::set_ai`, a rule — voids agreements; default 0 = retail),
  seated by `World::seat_mc*_net` in the first free authored colours — all
  eight colours seat humans now (a "1-player" arena header no longer
  drops the second human's record). Given names replace table names in
  event lines. Tests: banish (MC1+MC2, kill-switch proved), AI count (MC2
  "1-player" arena 51 + MC1 50), lobby AI cap; 633/633 fixtures, 1,621
  tests. The CLI `--host/--join` keeps the authored roster. ⏭ playtest.
- 2026-10-15 #12: (1) the temple's message scroll wraps at word bounds and
  grows to the text (`MainMenu::message_layout`) — the duplicate-name
  refusal no longer runs off the scroll; (2) the session's fog now reaches
  the CLIENT's renderer: adopting rules set the config only, and the fog
  lives in the renderer once applied — `App::adopt_rules` re-applies every
  pin the session changed (and again when the player's own values come
  back). Xvfb: host fog 20 / client config 90 → the client draws 20.
  ⏭ OPEN for the next session — **PAUSE IN NETPLAY** (player, 10-15): a
  pause freezes the game for EVERY peer until the pauser unpauses (the
  lockstep stalls on his inputs), and pause is today the only way into the
  in-game menu (options, exit). Needs a design discussion: a menu that
  does not pause in netplay, a host-only pause, a vote, a pause timeout…
  (retail refused pause in multiplayer altogether, MC2 trace
  "MULTIPLAYER_MODE gates").
- 2026-10-16 #13: the lobby's rules no longer leak into the host's own
  game (player-reported: a lobby fog persisted into single-player). The
  scroll's chevrons stepped the HOST's config — applied to his renderer and
  written to his `mgcarpet.json`. Now the rules belong to the lobby
  (`LobbyCore`): seeded once from `rules::session(cfg)` when the lobby
  opens, stepped on a scratch config (`step_lobby_rule`), never applied or
  persisted; the per-frame push of the host's config into the rules is
  gone. In a game everyone adopts the session and gets his own back at the
  end (unchanged). Test `a_lobby_rule_step_leaves_the_players_config_alone`.
- 2026-10-16 #14: **PAUSE IN NETPLAY — SOLVED** (player ruling: a menu
  without pausing, idle hover, no host pause). `App::toggle_menu` opens
  `MiniMenu::netplay()` (Options, Leave) without setting `paused`;
  `menu_up()` (paused OR the panel) keeps the pointer free; `net_menu()`
  folds into `tick_input`'s movement-suspend (`book`) and sets `full_stop`.
  Xvfb, 2 peers: B's menu open 5 s, then both menus 4 s — both peers kept
  one hash check per second (no stall, no desync); B held W into the menu
  and its position froze (Y 9149 over 3 s); Leave → Enter left the game
  (14 checks agreed). Noted: when a CLIENT leaves, the host's game goes on
  with the leaver's carpet idling (no game over — it still counts as a
  contender).
- 2026-10-17 #15: **A LEAVER IS BANISHED** (player-confirmed next fix): a
  client leaving no longer closes the host's lockstep (which also starved
  every OTHER client in a 3+ game — the host stopped sending). See "Pinned
  for the session" for the mechanism. Also: MC2's human "has died." (lang
  374, verbatim, no name — retail writes it into the dying wizard's OWN
  slot) is now seat-scoped; the others saw a nameless "has died.". Test
  `a_leaving_client_is_banished_and_the_game_goes_on` (3 peers, MC2 + MC1:
  the stayers stay identical, banish on one tick, the wizard is down, the
  stayers' lines hold the banishment and not "has died."); kill-switch
  proved three ways (no suicide input / the old close-on-Bye / the
  broadcast death line). 1,624 workspace tests, 633/633 fixtures, clippy
  clean. Xvfb: B leaves → host "BRAN has been banished from the realm.",
  "ARIEL is victorious!", fade to the lobby (Ariel alone, rules kept).
- 2026-10-17 #16: **CROSSED BALLOON HUD** (player-reported: a player saw the
  other's balloon damage/cargo, swapping after a redeploy). `World::
  player_balloons` took the first balloons owned by ANY human
  (`is_human_tag`) — pool order decided whose. Now seat-scoped
  (`hx_of_tag(id) == Some(hx)`); the stats screen's temporary-sphere share
  (`temporary_sphere_mana`) had the same any-human read and is scoped too.
  Test `each_seat_sees_only_its_own_balloons` (kill-switch proved); 1,625
  tests, 633/633 fixtures, clippy clean.
- 2026-10-17 #17: **THE WAITING LOBBY** (player: a client leaving landed in
  the temple and had to rejoin; the host sat alone in the lobby). A
  client's in-game Leave now sends `Msg::Leave` (protocol 4) instead of
  hanging up: the host banishes the seat as before (#15) but keeps the
  member and his connection; `Lockstep::leave` hands the client his
  lobby with `LobbyView::playing` set — status "Waiting for the current
  game to finish", only Exit live (Exit = hang up, back to the temple).
  A NEWCOMER connecting during a game is admitted into the same waiting
  lobby (it used to be refused "a game is in progress"). `LobbyCore::
  begin`/`reopen` set/clear `playing`; the in-game host lockstep runs the
  handshake, sends lobby views ONLY to waiting members (a view ends an
  in-game client's game), drops a waiting member who hangs up at the end,
  and keeps his flight-model changes. At the game's end the reopened lobby
  holds everyone. Tests `a_leaver_waits_in_the_lobby_for_the_game_to_finish`
  (3 peers; kill-switch proved: a hang-up instead of Leave fails it) and
  the reworked newcomer half of `after_a_game_the_lobby_reopens_for_the_next`.
  Xvfb: B leaves → B's waiting lobby → A wins → both in the reopened lobby.
