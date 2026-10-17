# Netplay (MVP, 2026-10-15)

App instances sharing one world, **deterministic lockstep over TCP**, star
topology with a **server** at the centre (2026-10-17): it holds the lobby,
assembles the turns and runs the world itself, headless — the authority
every player's hash is checked against, and the one that names the winner.
Every player is a client; one is the **admin** (the lobby's powers). The
server runs dedicated (`--serve`) or as a thread of the hosting player's
app (the UI's Host, `--host`) — one implementation. Code:
`crates/mgc-app/src/net.rs` (wire, `Server`, `Guest`, `Lockstep`),
`ServerWorld` in `lib.rs` (the server's world), `App::net_turn` (the
per-tick gate), `net_handshake` (launch folding). Rulings behind it: TCP
only (10-14), star, the server ticks (10-17), retail's own model of
shipping inputs only.

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
- Exit (or Esc) leaves (the hosting player: stops his server — the
  session ends for everyone in it).
In the game: Esc → exit leaves the game for everyone alike, the hosting
player too — banished, back to the lobby, waiting for the game to finish.
The server's verdict (one wizard standing) returns everyone to the lobby.
Closing the hosting app — or its player's Exit from the lobby — with
other players on its server warns first ("N players still in this
session - close/exit again…"); the second try within 5 s stops the
server.
The campaign steps aside for a lobby game (no campaign book — every seat
gets the level's own — and no campaign progress or saves). Entries
persist in `saves/netplay.json` (recent hosts, port, passphrase).

**From the CLI:**

```
mgcarpet --serve 7777 --game mc2 --passphrase pw             # dedicated server, headless
mgcarpet --level baked/mc1/level-002.mgcl --host              # a server in this app + its admin player
mgcarpet --join 127.0.0.1                                     # a player (the server assigns the seat)
```

- `--serve [ADDR] --game mc1|mc1hw|mc2`: the DEDICATED server — no
  window, runs until killed; `host:port`, a bare port, or nothing
  (`0.0.0.0:7777`). `--game` is REQUIRED: a server plays one game for its
  whole life (⚖ player 2026-10-17 — never the first player's; players of
  different games in one session would be a mess). The first player in is
  the admin (the next in line when he leaves the session). The lobby opens
  on `--level` (one of that game's; default: its first arena, level 50)
  under the default session rules; the admin changes them in the lobby,
  levels within the game only.
- `--host [ADDR]`: a server on a thread of this app, joined over loopback
  as the admin (a key only this app knows claims the role). Needs a
  `--level`; `--seat N` picks the admin's wizard seat.
- The game travels in the handshake (protocol 6): a player of another
  game is refused — "this server runs Magic Carpet 2". The in-game lobby
  sends its campaign's game; `--host` the game of its `--level`; `--join`
  sends `--game` when given, else adopts the server's.
- `--join ADDR`: `host:port` or a bare host (port 7777). Retries for 60 s,
  so either side may start first. `--seat N` asks for a seat; without it
  the server assigns the first free one.
- Every player **adopts** the server's level and the session rules
  (below), and prints which of his own options they override. All must run
  the **same binary** (the handshake compares a hash of the executable); a
  mismatch is a refusal.
- `--passphrase WORD`: the session's passphrase (the server's; players
  must match; empty when omitted). Not a secret — just keeps strangers and
  kicked players out.
- Under the hood the CLI goes through the LOBBY (`lobby.rs` +
  `net::Server`/`net::Guest`, protocol 5): a `--join` that is not the
  admin agrees to whatever is set by itself; the admin (`--host`'s player,
  or a dedicated server's first `--join`) presses Play once everyone has.
  The in-game lobby UI drives the same calls.
- The CLI seats the session's AI count (0 by default — retail's network
  rule), like the lobby; it used to keep the level's authored roster.
- Not with `--campaign`, `--replay`, `--record`, `--film`,
  `--set-spellbook` or the headless modes.

## How it works

- Every peer — and the server — builds the SAME world (`net_level` +
  `session_sim`, one function for all: seats in member order, `Hx(k)` =
  member k; `World::add_human_seat` for the others); each app flies its
  own `Hx` (`local_hx()`).
- Each seat's input is sampled `INPUT_DELAY` = 3 ticks (125 ms) ahead
  and sent to the server; once every seat's is in, the server steps its
  world and broadcasts `Turn t` (every seat's `PortInput`, seat order). A
  tick steps (`Simulation::step_seats`) only once its turn exists — a
  starved peer waits ("waiting for the other player…"). The server keeps
  no clock: the players' clocks pace the game.
- The server's world (`ServerWorld`, a `net::Referee`) is built from the
  Start's data over a DEFAULT config + the session's rules — so every
  sim-affecting option must be a pin (or reach the world through the
  inputs). It also drains each step's sound requests, as every app does
  (`drain_sounds`: the pending vec is hashed).
- Wire: little-endian `u32` length + one JSON message. Idle input is `{}`.
- **Desync detection**: the server stamps its own world's pre-step
  `state_hash` (the FULL sim: world + every seat's flight tier) every 24
  ticks onto that tick's turn; every player — the hosting one too —
  compares. `MGC_NET_TRACE=1` prints every check on the server and the
  players (diff the logs).
- **Reconciliation (resync)**: a player whose hash disagrees toasts
  `DESYNC at tick N … — resyncing` and sends `Resync`. The server sends
  its world's NET SNAPSHOT (`Simulation::net_snapshot`: the save's
  snapshot + a net-only tail — each seat's banishment and the pool's
  ranked recycle victims, which a retail LOAD empties but a clone must
  keep), zstd + base64, ~130-240 KiB, taken BEFORE the next turn it
  steps and sent AHEAD of that turn. The player applies it right before
  stepping that same tick (`Lockstep::take_snapshot` → `net_resync`) —
  nothing to re-simulate: TCP order guarantees he has not stepped it
  yet. The server's world wins.
- **The divergence report**: before applying, the player's own world is
  compared with the server's section by section
  (`Simulation::divergence_report` over `state_sections`: the flight
  tier per seat, every `Gen` field, EVERY POOL SLOT — with class, model,
  position, life — and the world's lanes per seat). The sections come
  from the very walk that makes `state_hash` (`HashSink`: a plain FNV
  ignores the marks, `Sectioned` digests each section too), so the
  report cannot drift from the hash. Printed in full to stderr
  (`net:   g.ent[123]: here class 5 model 24 … / there …`); the toast
  names the first three (`resynced from the server at tick N
  (player[1], g.ent[812] +4)`).
- **The heal is verified**: the restored hash must equal the server's
  (`Snapshot::hash`) — a miss is `RESYNC FAILED … the snapshot misses
  state`, and the checks stop (no resync loop). Every resync is told
  loudly and counted: it hides a fork, it does not fix the bug behind
  it — the report is the lead. Hash-SILENT state is outside both the
  check and the report (by definition); a snapshot that dropped some
  would show as a recurring desync.
- **The game's end**: the server's world decides (`net_verdict`: one
  wizard standing, or no human); the announcement rides the next turn
  (`Turn::over`), every player shows "<name> is victorious!" and fades
  out, then leaves (`Msg::Leave` — after the verdict it banishes nobody).
  The server reopens the lobby when nobody plays any more, or 10 s after
  the verdict (`GAME_OVER_GRACE`). A player whose session is lost judges
  his own world.
- On exit each player prints `net: N ticks stepped, M hash checks agreed`
  (+ `— D DESYNCS, R resynced`, `— UNHEALED`); the server `game over after
  N ticks, M hash checks stamped, R resyncs` (and `seat k resynced at
  tick N (… KiB)` as it happens).

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
  is locked "fixed for this session" — a preset row too when it would set a
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
  back to the lobby). The hosting player leaving is no different (his
  server plays on). Only the SERVER stopping (`--serve` killed, the
  hosting app closed) ends netplay: a player whose server is gone goes on
  locally, the other carpets idle.
- A FROZEN player (connected, silent: his app hung) is dropped once the
  game has waited on him `FROZEN_AFTER` = 30 s — banished like a lost
  connection, the rest play on (player 2026-10-17: slowdowns are
  accepted, a frozen player is not). Only a player missing from the next
  turn counts: the others, waiting on him, are silent too.
- Game speed is a session pin (`sim`): everyone runs Normal, F3 snaps
  back (a speed-up cannot outrun the lockstep; a slow-down would slow
  every player).

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

4. ✅ **Desync recovery** (2026-10-17, log #28): the sectioned
   divergence report + the server's snapshot wins (no re-simulation: it
   is applied before the very tick it was taken at). Still open:
   PREVENTION — `libm` for the sim's ~7 transcendental calls
   (`sin_cos`/`sin` in the enhanced mover, `hypot` in world.rs) + a
   guard against new platform-math calls; cross-platform FP is the one
   known source of an honest fork.
5. ✅ **Spectator ghosts** LANDED 2026-10-17 (log #29). Was: (player idea, 10-15, banked — "a cherry
   on top"): a BANISHED player rises from his last corpse as a visible
   in-game ghost (mostly translucent, greyscale or another filter), flies
   freely and watches the rest of the battle, but can neither touch nor be
   touched. Starting points: the `ghost`/`inert` cheats (unseen,
   no triggers) and the greyscale death look. Open: other players see the
   ghost, so its flight must stay in the lockstep as a seat that cannot
   interact, not a local-only camera.
   Also later: level end / campaign. (Per-seat flight models ✅, more than
   two seats ✅.)
6. ✅ **Dedicated server + admin role** (player 2026-10-17; LANDED the
   same day, playtest log #25) — REPLACES host handover, which cannot work behind a port
   forward (clients reach only the server). A headless server process; the
   admin is the first member, a player with more privileges, who may quit
   while the rest play on. Open: does the server run the sim (authority,
   hash + snapshot source) or only relay? Desync prevention (`libm` for the
   sim's ~7 transcendental calls), a sectioned hash report, then snapshot
   recovery follow in the same session (item 4 depends on it).
   ⚖ (player 2026-10-17) The server AUGMENTS hosting, it does not replace
   it: `mgcarpet` from the command line can run a bare server (every player
   a client, one of them admin), AND the UI's Host still works — it starts
   the same server in the background and joins it as the admin. Most
   players will host for friends (VPN), so the one-click path stays.
   ⚖ The UI's server runs as a THREAD in the game process; whoever needs
   process isolation runs the command-line server instead.
   ⚖ A UI host leaving the MATCH goes back to the waiting lobby like any
   client while the rest play on; only closing the APP ends the session
   (as if the server were stopped), after a "N players still in game" warning.

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
- 2026-10-17 #18: **JOIN HAS A PORT FIELD** (player: few know `host:port`).
  The network scroll's Join face is Address / Port / Passphrase (Host
  keeps Port / Passphrase); Tab and Enter walk the fields, Enter on the
  last is OK. The join port is its own, remembered in `saves/netplay.json`
  (`join_port`); a recent host (stored `host:port`) opens and clicks back
  split into both fields; an address typed as `host:port` still wins.
  Test `joining_takes_the_address_and_the_port_apart`.
- 2026-10-17 #19: a client lost WHILE THE GAME STARTS (its Start send
  failed) is a departure like any other — `Link::lost` carries the seats,
  `Lockstep::start` queues them as failed sends, the first poll banishes
  them at tick 0; it used to close the host's lockstep for everyone. Test
  `a_client_lost_at_the_start_is_banished_not_fatal` (kill-switch: the
  host stalls). Playtests #13-#18 all passed (player 10-17).
- 2026-10-17 #20: **SEAT AUDIT** (3 parallel report-only agents: app
  presentation, sim view API, sim seat-0 bias). FIXED, each with a test and
  a kill-switch proof; single player unchanged (633/633 fixtures):
  - `no_spell_loss` death/respawn used EVERY human's spell tokens — one
    player's death ended the others' running spells, dropped jars of THEIR
    spells at his corpse (duplicable) and relinked their tokens to his
    castle. Now `seat_spell_tokens(hx)` (test
    `no_spell_loss_death_touches_only_the_dying_seats_book`).
  - The mana-ball owner contest weighed seat 0's mana ceiling for ANY
    human (the walkers' `ctx` is seat 0's) — `human_ceiling_of(hx_of_tag)`
    (test `a_ball_contest_weighs_its_own_seats_ceiling`).
  - MC1 mana spheres of a second human drew in the host's colour —
    owner's wizard colour when several humans play (one human: retail's 0).
  - Global Death's violet flash washed every peer's view — `PalFlash`
    carries the caster's seat (hash-silent; not in the snapshot) (test
    `a_global_death_flash_is_its_casters_alone`).
  - A banished/departed human stayed on the Alt roster (others' and his
    own row) — `eliminated |= banished` (asserted in the leave test).
  - MC2 map: own mines drew in seat 0's team colour — the owner's.
  OPEN — need rulings or only matter outside the lobby's arenas:
  - `World::won()` is world-global: one human's win (MC1 mana goal +
    Space, MC2 end sequence) ends every peer's game as "completed" with
    no winner named. Rec: in netplay a seat's win = game over naming that
    seat (`net_standings`). Matters for the MC1 lobby (mana goal).
  - Campaign maps in MP (`--host --level` only): the MC2 doomsday pyramid
    fights only the host (rec: nearest live human, re-elected), its devour
    cancels the HOST's Castle window for anyone's shot, doom summons home
    on the host, kill-all credits the host; the MC2 STAGE ENGINE runs for
    seat 0 only (co-op board vs per-seat boards = ruling) with its chimes
    and level-end segment; cave drip spawns around the host only; the
    castle fireball star only for the host's castle.
  - Stats ledger rows (deaths by you, enhanced accuracy, MC2 spells found)
    pool all humans — only matters if MP ever shows a stats screen.
  - MC1 human deaths are never announced to others (MC2 announces
    banishment only) — ruling. The level-start chime reaches seat 0 only
    (cosmetic). `dev_minted` is global (cheat-only, cheats are off).
- 2026-10-17 #21: **EFFECTS + COLOURS AUDIT** (2 report-only agents, on the
  player's hint: monster effects and owner-coloured markers).
  - Status effects: CLEAN. Webs (paralyze/slow — the manticore's spit is
    the same (10,65) slow), knock/hurl/spin/whirl, flood/quake pull,
    deflect debits, rebound, wraith steal, drain-life, duel tethers: each
    resolves its victim by `hx_of_tag`, stores per seat, and the app reads
    `local_hx()`. The `MobCtx` seat-0 trap is closed on these paths (it
    holds only `phx` + flags; "the player" reads go through
    `seat_at/seat_yaw/seat_dead/seat_purse(ctx, hx)`).
  - Map dots/stamps: CLEAN (every colour from the owner via `p.team`;
    volcano/fire/firestorm/lightning/meteor/quake entities stamp the
    casting seat's tag).
  - FIXED: the MC2 castle's team flag (world sprite) and the MC2 mana
    worm's colour band (+ its coil spacing row) mapped EVERY human to
    colour 0 — `Gen::human_colour_slot` (one human stays player 0); test
    `a_second_humans_art_wears_his_own_colour`. The third-person / filmed
    MC2 exit carpet drew colour-0 art — `World::human_carpet_row(local)`.
  - OPEN (minor): a spread forest fire past its first hop names the
    seeding fire, not the wizard, so it shows the environmental magenta
    (same in single player — a design choice, not a seat leak; the
    owner would need the fire-chain walk `stats_killer_of` does); two
    Global Deaths at once share one
    flash slot (cosmetic); apocalypse-rain spell XP goes to seat 0; the
    doomsday pyramid (#20).
- 2026-10-17 #22: ⚖ **THE STAR FIREBALL IS EVERY HUMAN'S** (player: MC2's
  star-vs-sphere split is silly; take the simplest rule). Every
  human-owned fireball source throws star sprite 42, on every screen —
  the cast and the fire orb already did; the castle's shot now does too
  (`is_human_tag(own)`, was the host's castle only). One human: retail.
- 2026-10-17 #23: ⚖ the MC2 DOOMSDAY PYRAMID and the APOCALYPSE RAIN are
  never in multiplayer (player): their seat-0 behaviour there is
  undefined by ruling — no fix, no test; struck from the #20/#21 open
  lists.
- 2026-10-17 #24: ⚖ rulings closing the audit lists (player): a spread
  forest fire's environmental colour is FINE (same in single player); MC2
  multiplayer arenas carry NO objectives (retail `-level` on an arena
  plays every narration one by one — the board is empty), so the
  seat-0 stage engine is moot there; two Global Death flashes coalescing
  is EXPECTED. The seat audit is closed.
- 2026-10-17 #25: **THE SERVER SPLIT** (player rulings: the server ticks;
  it augments hosting — `--serve` dedicated, the UI's Host = the same
  server on a thread + its player as admin; leaving a game = the waiting
  lobby for everyone; closing the hosting app warns, then stops it).
  `net::Server` (lobby + turn assembly + `Referee`), `ServerHandle`
  (thread, admin key, status), every player a `Guest` (admin requests:
  level / AI / rules / limit / passphrase / kick / Play; non-admin ones
  ignored), a client-only `Lockstep` (`finish`), `LobbyCore` with no host
  member (`admin`: first in / key claim / next in line), protocol 5.
  App: `ServerWorld` + `session_sim` (the one world build, shared with
  `install_level`), the verdict from the server (`Turn::over`), the close
  warning. FOUND by the server: the world's pending SOUND vec is hashed
  and every app drains it per tick — the headless server did not (DESYNC
  at tick 24 in the first three-process run); and an app with sound AND
  music off had no mixer and never drained (a latent fork between
  players). Both drain now (`drain_sounds`). Tests: `net::tests` (14,
  incl. admin key/role passing, a fork from the SERVER reported, the
  verdict → lobby, a level the server can't build), `lobby::tests` (10),
  `net_lockstep_tests` against the real `ServerWorld` (2-4 players, MC1 +
  MC2, AI rivals 0/3, leaving players incl. the admin, the verdict);
  kill-switch proved twice (the server skipping banishment / skipping the
  drain → "forked from the server"). 1,637 workspace tests, clippy clean.
  Xvfb: `--serve` + two `--join` 28/29 checks, no desync, the verdict
  reached the last player; `--host` + `--join` 22/22. ⏭ playtest (UI
  Host → lobby → game → leave → waiting lobby; the close warning).
- 2026-10-17 #26: ⚖ **THE SESSION'S GAME IS EXPLICIT** (player): it was
  implicit in the server's level path and unchecked at the door.
  `lobby::NetGame` (mc1 / mc1hw / mc2 — the baked tree's keys; `arenas()`,
  `of_level`), `ServerOpts::game`, `LobbyView::game`, `Hello::game`
  (protocol 6): a player of another game is refused, an admin's level of
  another game ignored (`LobbyCore::set_level`). `--serve` REQUIRES
  `--game` (its `--level` must be of it; default level 50); `--host` takes
  it from its level; `--join --game` optional. Tests
  `the_session_plays_the_servers_game_only` (kill-switch proved),
  `a_level_names_its_game`; 1,639 workspace tests, clippy clean. CLI: no
  `--game` → exit 2; MC2 level under `--game mc1` → refused; an MC1 joiner
  at an MC2 server → "this server runs Magic Carpet 2".
- 2026-10-17 #27: (1) the hosting player's lobby EXIT warns like the close
  (`App::hosted_end_confirmed`, shared). (2) A FROZEN player is dropped
  after 30 s (`Server::drop_frozen`, `ServerOpts::frozen_after`); test
  `a_frozen_player_is_dropped_after_the_timeout` (kill-switch proved: no
  drop → stall). Its "missing from the next turn" clause is a guard the
  test cannot tell apart (a waiting player always spoke after the one he
  waits on) — kept, unproven. (3) Game speed was already pinned (F3 snaps
  back); the messages no longer name a host ("fixed for this session").
  Fixed a race in `play_waits_for_agreement…`. 1,640 workspace tests,
  clippy clean.
- 2026-10-17 #28: ✅ **RECONCILIATION** — full state-hash verification
  with a sectioned report, and resync from the server's world.
  (1) `HashSink` / `Sectioned` (world.rs): ONE walk for `state_hash` and
  `state_sections` (`World::hash_walk`, `Gen::hash_walk` field by field
  + slot by slot, `Simulation::hash_walk`); every golden unmoved (the
  main stream is byte-identical; `the_sectioned_walk_is_the_derived_hash`
  pins `Gen`'s walk against its derive). (2) `Simulation::net_snapshot` /
  `net_restore` (snapshot.rs): the save + a net tail (banishment, the
  recycle stack) — NO `SNAPSHOT_VERSION` bump, saves untouched. Found by
  the tests: `banished` (offline, the report said `player[2]`) and the
  recycle stack (`g.mc2_recycle` on MC1 — retail's load empties it).
  (3) Protocol 7: `Resync` / `Snapshot`; `Referee::snapshot`;
  `Lockstep::take_snapshot` / `resynced`; app `net_resync` (shared with
  the tests) + `Session::reseed_after_restore` (shared with a save's
  resume). Tests: `net::tests::a_fork_from_the_server_is_resynced`;
  `{mc1,mc2}_a_snapshot_heals_a_fork` (3 humans + 3 AI, a forked world
  and a fresh one restored from the authority's — which banished a seat
  the fork never saw leave — then 1500 ticks identical, the fresh one
  re-restored every 100 ticks; also passed ad hoc on MC1 2/9/30 and MC2
  1/22); `{mc1,mc2}_a_forked_player_is_resynced` (real TCP + the hosted
  `ServerWorld`: a player banishes his own seat alone at tick 300 →
  DESYNC at 312 → resynced at 315, report `player[1]`, identical after).
  ⏭ playtest (nothing should change: a healthy session never resyncs).
- 2026-10-17 #29: ✅ **SPECTATOR GHOSTS** — ⚖ rulings (player): others
  see the ghost TRANSLUCENT IN ITS SEAT'S OWN COLOURS; its own view is
  FULL COLOUR, NO HUD, the line "Spectating - waiting for the game to
  end"; it flies THROUGH WALLS and FASTER than a carpet; it rises on
  SPACE at the corpse. ⚖ libm DEFERRED until a maths fork is actually
  seen (the resync heals it, the report names it).
  Design: the ghost is NOT in the world. `Player::risen` (hashed
  tag-only 0x6E, resync tail) marks the seat; the world goes on turning
  its corpse exactly as any banished one (no input reaches it — a
  control corpse settles identically), so nothing can see or touch the
  ghost by construction. The flight tier flies the seat's flyer with
  `Simulation::move_ghost` (either control scheme; 2× the faithful
  cruise; ground clearance to 12 tiles; no wall gate at all) and never
  derives it from the corpse (`SeatPre::ghost`). ONE Space: the rise is
  checked after the turn, so the press that banishes also raises
  (`World::rise`). Drawing: `push_spectators` (both draw arms) mints
  each OTHER risen seat's carpet art at its lerped flyer
  (`Session::prev_flyers`), `entities::spectator_billboard`, retail
  blend mode 2 (⅓ opaque — the knob is `SPECTATOR_BLEND`); the local
  ghost is the camera. App: a risen seat is not `player_down` (full
  controls; the sim drops its casts), greyscale off, HUD still cleared
  (its corpse is Dead), toast line as above. Test
  `a_banished_player_rises_as_a_ghost` (MC1 + MC2: one Space → banished
  + risen; 10 s forward flight > 60 tiles; corpse x/y unmoved and Dead;
  not a contender; two builds hash-identical every tick; a resync at
  t=120 carries the ghost). 1,646 workspace tests, clippy clean.
  ⏭ playtest (feel: speed/ease/ceiling knobs `GHOST_*` in mgc-sim
  lib.rs; the ⅓ alpha at distance).
- 2026-10-17 #30 (ghost playtest, MC2, enhanced thrust + altitude): the
  ghost banked to the clamp while turning on the spot. The first cut's
  mover had its own laws — a speed-blind bank, an instant turn, pitch
  always climbing. `move_ghost` now flies by the ENHANCED CARPET'S laws:
  chase-the-pointer steering (`aim_lead`, `CHASE_GAIN`, the turn-rate
  cap), `enhanced_bank` on the forward speed scaled to the carpet's
  range (a full-speed ghost banks like a carpet at full cruise; none
  standing still), and the altitude split (faithful: pitch climbs while
  moving; ExtendedLift: pitch is pure look, Q/E own the vertical at
  `GHOST_LIFT` × speed). The ghost test turns hard on the spot (|roll| <
  0.01) and caps the flying bank; kill-switch proved (the old law:
  "banks -0.375 standing still"). 1,646 workspace tests, clippy clean.
  ⏭ re-playtest.
