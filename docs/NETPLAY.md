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

- Reconciliation: the host's state wins (whole-state correction at tick N +
  re-simulate), then host handover on a crash.
- Live option toggles over the wire (the recorder's `set` channel per seat).
- Per-seat flight models (today the joiner flies the host's models).
- The sim's `Gen::local_seat()` (retail `playerIndex`) is `Hx::FIRST` on
  every peer — consistent, but retail evaluated those rules per peer.
- Level end / campaign play; more than two seats. Unverified by eye: how
  the other human (and the HUD/camera on the client's seat 2) look.
