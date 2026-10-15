# MC1/HW multiplayer lobby + network frontend trace (2026-10-15)

Decompile recon for the in-game lobby (NETPLAY.md stage 3). Citations
`:NNNNN` = `reference/remc1/sub_main.cpp`; `hw:NNNNN` =
`reference/remc1hw/sub_main.cpp`. Binary claims are from the repo-root
shipped executables: `CARPET.EXE` file = VA + 0x187F8 (code) and data
VA − 0x90000 + 0x957F8; `HIDDEN.EXE` code = VA + 0x189F8, data
VA − 0x90000 + 0x959F8. HW code sits at MC1 VA + 0x340 (the second
address in a dual name, e.g. `sub_4D680_4D9C0`). Companion:
`mc1-campaign-save-menu.md` (main-menu conventions; its corrections are
listed at the end of this file).

## 0. Summary

- Retail MC1 has **ONE lobby screen** (frontend selector 4, `PMULTI`),
  reached from main-menu button 3. It has no host/join split and no
  address entry. Every peer opens the same screen, picks the same
  **channel name** (`CARPET0`..`CARPET9`) and the same **player count**
  (2-8), then presses START. NetBIOS name registration makes the first
  peer seat 0. The peers build a full NetBIOS session mesh and block
  until all N are connected. Seat 0's level choice is broadcast and
  everyone loads it.
- **The network is enabled only by the `-network` switch.** It loads
  the NetBIOS TSR and sets `byte_93958`. Without it the Multiplayer
  button gets no highlight, TAB skips it and a click does nothing. No
  message is shown.
- The other "network" switches (`-players`, `-session`, `-name`) are
  **dead or overwritten** in the shipped binaries (§1.2).
- MC1 and HW differ only in the map list: 20 arenas (levels 50-69)
  vs 10 (levels 50-59), with different names. They share the art and
  the code (§7).

## 1. Entry: main-menu button 3

### 1.1 The gate

- Button table `dword_4A12C_4A46C` (`:1788-1800`). In the binary
  (CARPET file 0x62924) each entry is 6 B: {fn, var}. Entry 3 = fn
  `sub_4FEB0` (`:60661`). HW: the decompile is a stub
  (`hw:1572 //fix it`); the binary table (HIDDEN file 0x62E64) has the
  same layout, with entry 3 = VA 0x501F0.
- Validity `sub_4C6A0_4C9E0` (`:58861-58881`): id 3 is valid only when
  `byte_93958 == 1` (`:58865-58868`). This one test serves three paths:
  - hover: the mask id under the mouse is zeroed when invalid
    (`:58536-58541`);
  - Esc re-probe (`:58550-58554`);
  - TAB cycling `sub_4C670` (`:58848`).
  ⇒ without a network, id 3 never highlights, never takes focus and
  never fires. There is no "no network" toast.
- Click handler `sub_4FEB0` (`:60661-60679`; HW binary VA 0x501F0
  identical: `cmpb $0,0x3958`, then `movb $4` to the selector), gated
  on `byte_93958`:
  - selector ← 4;
  - backs up the local wizard record: 2049 B from `+13323`
    (`str_13323[0]`) to `+9225`, the slot just below the `+11274`
    template;
  - saves the campaign level: `var_u16_19 = var_u16_17`;
  - sets `byte_12CBD5 |= 8`, so the dispatcher tears the menu down
    (`:58574-58578`).
  The backup lets Cancel restore the campaign (§6).

### 1.2 What "network" is: the command line

`ProcessCommandLine_34DD0_35190` (`:42052-42345`):

| switch | effect in the SHIPPED binary |
|---|---|
| `-network` | `InitNetwork_43270_435B0()` (`:42198-42201`). The local flag is then **cleared**: CARPET 0x4DAB2-0x4DAB9 `xor ebp,ebp; call; mov ebp,0x90(%esp)`, and HW 0x4E072-0x4E079 does the same. |
| `-players N` | `var_u16_10 = N` (default 2, `:42079`, `:42264`). Overwritten by the lobby at START (`:59545`). |
| `-session S` | `textBuffer_117 = S` (default "CARPET", `:42082`, `:42265`). Overwritten by frontend init: `sprintf("CARPET%d",0)` at `:57842`. |
| `-name S` | Parsed into a stack buffer (`:42148`; binary 0x40(%esp), written at 0x4D907) and **never read**. |

- ⚠ The block at `:42305-42316` looks like a startup auto-join. It
  calls `sub_43430` (session join) and sets `var_u8_0 |= 0x10` plus
  `var_u8_1 |= 1` (skip frontend). It is **DEAD in both shipped
  EXEs**, because the flag it tests was zeroed right after
  `InitNetwork`. So `-network` never skips the frontend.
- The decompile also mislabels the frontend-skip byte as
  `str_AE400->var_0.var_u8_1`. The binary writes `[AE408]+1`
  (CARPET 0x4DD33/0x4DDC5), which is the byte TopProcedure tests at
  `:41506`/`:41525` (binary 0x4C907, 0x4C98E).
- `InitNetwork_43270_435B0` (`:53605-53655`):
  - `system("netbios")` (`:53610`; string `aNetbios` `:12689`) loads
    the TSR;
  - allocates the main NCB (66 B), the 8 per-seat NCBs and 2 KB
    packet buffers;
  - `NetworkInit_42CF0` (`:53289-53302`) probes for a handler on INT
    5Ch (`dos_getvect(92)`), then issues NCB command 0x7F (invalid).
    Return code 3 ("invalid command") means NetBIOS is present;
  - success sets `byte_93958 = 1` (`:53630`).
  - On a failed probe it returns early (`:53616-53617`) **without
    freeing** the main NCB (retail leak), and `byte_93958` stays 0.
- The decompile's `SetNetbios_431A0_434E0` is stubbed (`:53570-53603`).
  The commented body shows the retail call: DPMI `int386x(0x31)`
  function 0x300 (simulate real-mode INT 0x5C).
- How the GOG release launches it:
  - `dosboxMC_server.conf` runs `IPXNET STARTSERVER` then
    `CARPET.EXE -network`; `dosboxMC_client.conf` runs
    `IPXNET CONNECT 127.0.0.1` then `CARPET.EXE -network`;
  - the root stub passes its arguments through (`DOS4GW CARPET %s` /
    `DOS4GW HIDDEN %s`, strings in the CD's `/CARPET.EXE`);
  - GOG added `CARPET/NETBIOS.EXE` (dated 2013 in `game.gog`), which
    is NetBIOS-over-IPX;
  - the 1995 manual says only "MULTIPLAYER MODE: 2-8 players,
    requires NETBIOS".

## 2. The lobby screen: selector 4 = `sub_4D680_4D9C0`

### 2.1 Lifecycle

- **Dispatch:** `sub_4AB20` case 4 (`:57899-57901`). The only caller is
  binary VA 0x4ABB5 (HW 0x4AEF5).
- **Setup:** `sub_4E0E0_4E420` (`:59754-59785`) runs when
  `byte_12CBD5 & 1 == 0` (`:59427-59431`). It:
  - fades to black;
  - **stops the music** (`sub_20E60` `:59757`). The lobby is SILENT:
    no song, and no `snds13` load of its own;
  - loads the `PMULTSPR` bank (`:59758-59762`);
  - binds the font object `str_9688C` to **SFONT1** (`sub_51480`
    `:59763`; binary pushes `0x9cb34` = `sFont1Tab_12CB34`);
  - loads `data\screens\pmulti.pal` and `pmulti.dat` (`:59767-59768`);
  - copies the background to the work screen and installs the
    hardware pointer (`SPTRS` frame [selector] = [4], `:59775-59776`);
  - fades in;
  - sets `byte_12CBD5 |= 1`;
  - sets the text colour to the palette entry nearest (26,18,13), a
    brown (`:59779-59780`).

  `sub_4E320_4E660` (`:59813-59840`) then resets every in-game seat's
  state to 0 and draws the first frame.
- **Per frame:** `sub_4DCC0_4E000` (`:59611-59752`) draws the 8 seat
  icons and the 5-row map list. The caller redraws the channel name.
  Drawing reuses the main-menu machinery:
  - 320×200 mode-13h background with a `.PAL`;
  - sprites blitted at 2×(x,y), i.e. 640-space, like MMSPR;
  - text through the same `DrawText_51560` / viewport
    `sub_51360(x,y,w,h)` pair as the save-slot labels.

  There is **no MMMASK-style hotspot bitmap and no palette-brighten
  highlight**. Every hit area is a hard-coded rectangle (below).
- **Teardown:** `sub_4E270_4E5B0` (`:59787-59810`) fades out, clears
  the screen, frees `PMULTSPR` and clears `byte_12CBD5 & 1`.

### 2.2 Layout (320×200 space, mouse >> 1)

| element | geometry | behaviour | cite |
|---|---|---|---|
| Channel name | text viewport (173,42,90,15), centred; click box x176-253 y46-58 | each click: `byte_12CBD0 = (byte_12CBD0+1) % 10`, `textBuffer_117 = "CARPET%d"` | `:59514-59521`, draw `:59595-59599`; binary 0x66206-0x66278 |
| Map list | viewport (173,72,90,61), 5 rows of `font_h − 2` px | hover x173-263 y72-133 → row `word_12CBC0`; click selects `word_12CBC2 = row + scroll`. Selected row white (63,63,63), others brown (26,18,13) | `:59438-59447`, draw `:59729-59750` |
| List up | x173-262 y57-71 | `--word_12CBBE` (scroll) if > 0 | `:59451-59464`; binary 0x65F29-0x65F69 |
| List down | x173-262 y134-148 | `++scroll` while `scroll + 5 < 20` (HW `< 10`) | `:59455-59458`; binary `cmp $0x14` 0x65FA4 / HW `cmp $0xa` 0x664E4 |
| Seat icon *i* (0-7) | at `word_4A7CC[2i], word_4A7CE[2i]`; hit box = sprite w/2 × h/2 | sets player count = max(*i*+1, 2): seats 0..max(*i*,1) on, others off (`byte_12CBD1` = count) | `:59480-59510`; binary 0x6627D-0x66341 |
| START | x284-303 y126-145 | join + launch (§5) | `:59528-59569`; binary 0x66011-0x661CA |
| EXIT | x236-256 y156-176 | leave the lobby (§6) | `:59524-59526`; binary 0x661CF-0x66201 |
| ABORT | x284-303 y48-67 (640-space x568-606 y96-134) | **live only while connecting** (`sub_428B0` `:53078-53094`). An idle click there is consumed with no effect (binary 0x65FC4-0x65FEC → 0x6634E) | |
| Esc | — | idle: same as EXIT (`:59432-59436`); connecting: same as ABORT (`lastPressedKey == 1`) | |

Seat-icon positions (binary CARPET file 0x62FC4 / HIDDEN 0x63504,
identical; the decompile has only `{51}` at `:2076`): (51,47) (75,45)
(100,46) (126,44) on the top row and (50,105) (77,101) (102,100)
(129,99) on the bottom row.

### 2.3 Seat icon states

Seat records are `byte_9689C[3*i]` = {in-game, flag, state}. Initial
data (CARPET file 0x9C094): seats 0-1 `{1,0,3}`, seats 2-7 `{0,0,3}`.
Draw logic at `:59656-59727`:

| record | sprite | meaning |
|---|---|---|
| not in game | `PMULTSPR[1+i]` | seat unused |
| in game, state 0 or 3 | `[9+i]` | idle, or failed |
| state 1 | `[17+i]`, **blinking** (frame bit 0x10) | trying: ADD NAME / CALL pending |
| state 2 | `[17+i]`, solid | connected / own name claimed |

The state is set by the net layer through three redraw hooks: 1
`sub_4E3D0` `:59842`, 2 `sub_4E470` `:59863`, 0 `sub_4E510` `:59884`.

- The middle byte `byte_9689D` draws `[17]` at (x+10, y−10) (binary
  0x665C4-0x665E4 confirms `tab+0x66`). It is **dead**: the only write
  stores 0 (binary 0x66339).
- `PMULTSPR[25]` (8×9) is referenced nowhere.
- While connecting, `byte_12CBD5 |= 0x10` animates the pointer through
  SPTRS frames 6..20, one step every 8 frames (`:59629-59636`): a busy
  cursor.

### 2.4 Lobby state and its persistence

- Frontend init `sub_4A980` (`:57815-57861`) sets:
  - `word_12CBC0` (hovered row) = `word_12CBC2` (selected map) =
    `word_12CBBE` (list scroll) = 0;
  - `byte_12CBD0` (channel) = 0;
  - `byte_12CBD1` (player count) = 2;
  - `textBuffer_117` = "CARPET0".
- `byte_12CBD0` and `byte_12CBD1` are **saved in the menu save** and
  folded into the encoded level (`:62061-62075`, read `:62031-62036`).
  So a loaded save restores the channel and player count.

## 3. Players and seats

- **Max 8, min 2.** There are 8 icons, 8 NetBIOS names `<chan>0..7`,
  and `connection_93990[8]`. The seat-click rule forces seats 0 and 1
  on (`:59497`).
- **The seat is not chosen.** It is the first index *i* whose NetBIOS
  name `<chan><i>` is free on the LAN (§5), so the first peer to press
  START is seat 0 = master. Wizard colour = seat (one portrait sprite
  per seat; which colour each depicts is not established here).
- **Every peer chooses the count locally** and the counts must agree.
  There is no negotiation and no count on the wire. A peer whose count
  is too high waits until it aborts (§5 step 5).
- **Name.** The lobby has no name field and shows no names. The name is
  the main menu's button 2 (`sub_50030`, `:60720-60760`; fields `:60745`, `:60749`). It edits two
  fields: `var_u8_29` (30 ch, prompt `ETEXT[34]` "Enter your name:") and
  `var_u8_61` (8 ch, `ETEXT[35]` "Enter your call-name:").
  - In a network game, each tick that a seat carries control command 1,
    the local seat's wizard name `str_14357` (12 B) is set to the
    **call-name** `var_u8_61` when the name field is filled. Otherwise
    it is set to the default `off_99B68[seat]` ("Zanzamar", "Vodor",
    "Gryshnak", "Mahmoud", ...) (`:48516-48531`, `:48568-48576`).
  - Names travel with the 2049-B wizard-record exchange (§5.2) and are
    shown in-game in the Network Matrix (manual).
  - The call-name is also a cheat password: control 30 compares
    `str_14357` with "chronicl…" (`:20925-20935`).
- **No AI fill.** `sub_3DD50` (`:49096-49277`) loops over all 8
  records. It sets the AI flag `+13332` only
  `if (!(var_u8_0 & 0x10) && seat != local)` (`:49152-49153`), so in a
  network game no seat is AI. Seats ≥ the node count never spawn
  (memory: MP step-1 research).
- **Equal arsenal.** A networked human reads the level's spell row 0
  (`v26 = 0` when networked, `:49201-49204`; grant test `:49233-49240`).
  The campaign carry `var_15318` is ignored.

## 4. Session options

| option | who picks | how it propagates |
|---|---|---|
| Arena | everyone picks; **seat 0's pick wins** | `var_u16_17 = row + 50` (`:59530-59531`), sent as `var_29715[seat][1]` (`:59561-59562`, binary `mov %dl,0x7414(%ecx)` 0x66179), then `var_u16_17 = (signed char) var_29715[0][1]` (`:59566`; binary `movsbw 0x7414(%eax)` = row 0, 0x6619E) |
| Channel | each peer, locally | implicit: a mismatched channel = a different session |
| Player count | each peer, locally | implicit, must agree |

- Clients never see the host's arena before START, and nothing in the
  lobby is synchronised. There is no game type, rule or option page.
- Arena names are drawn from `off_99A84[row]`. Binary, read from the
  shipped tables at data rel 0x9A84:
  - **MC1 (levels 50-69):** Bussorah, Bisnagar, Tartary, Akkania,
    Ryahn, Zhullor, Dombren, Xushtan, Wazzad, Phiria, Moussul, Aleppo,
    Schiraz, Ispahan, Coam, Rei, Buddoor, Jehoun, Nurrir, Comari.
  - **HW (levels 50-59):** Goyaan, Cyrecius, Esyphium, Barabban,
    Tervilar, Imnara, Kouppi, Fayaoud, Rama'Q, Halaj. Its table goes on
    with MC1's Moussul..Comari, which are unreachable because the list
    is capped at 10. `hw:4351` prints only 15 entries.

## 5. Protocol (NetBIOS, NCB-level)

### 5.1 Join: `sub_43430_43770(chan, count)` (`:53686-53858`), START only

NetBIOS name = `<chan><i>`, padded with spaces (`asc_A95F4` = " ") to
15 chars. ⚠ The prefix buffer `aTester` (data VA 0x939B8) is 7 B, and
"CARPET0" + NUL overruns it by one byte (`hw:3886`).

1. If already joined (`byte_93959`), return the stored seat
   immediately (`:53712-53713`). A second START does not reconnect.
2. Claim a name: for i = 0..7, ADD NAME `<chan><i>` (NCB 0xB0
   no-wait, `sub_42930` `:53100-53160`; loop `:53736-53752`). While it is pending, poll
   abort and blink seat i. Results:
   - success → that is my seat (`word_9395A`), drawn solid;
   - error 0x0D (duplicate in the local table) → DELETE NAME (0xB1,
     `sub_42D40` `:53305-53354`) and retry the same i;
   - any other error → next i.

   No free name, or an abort, returns −1.
3. Copy my name and LANA into all 8 NCBs (`:53760-53782`).
4. For every other seat < count, post LISTEN for `<chan><i>` (0x91,
   `sub_42E10` `:53371-53431`).
5. For every other seat whose LISTEN has not completed, CALL it (0x90,
   `sub_42A00` `:53162-53267`). It waits until the CALL or the LISTEN
   completes, CANCELs the loser (0x35, `sub_42C70` `:53269-53287`) and
   keeps the session in the seat's NCB. ⇒ a **full mesh of
   point-to-point sessions**, whichever side connects first.
6. Barrier loop until all `count` seats are connected (state 2) or
   aborted (`:53797-53837`). On abort: HANG UP (0x92, `sub_42DE0`) or
   DELETE NAME.
7. Success:
   - `sub_43150` marks the link table `byte_939B0[i]`: 2 = me,
     1 = connected peer, 0 = none;
   - master `word_9398A = 0`;
   - `byte_93959 = 1`;
   - return my seat.

On −1 the lobby redraws with all seat states reset to 0 and fades the
palette back in (`:59552-59556`). The player stays in the lobby.

### 5.2 Exchange primitive and hand-off

- `sub_43930_43C70(rows, size)` (`:53937-53961`) is a **star through
  the master**:
  - master: RECEIVE (0x95) each peer's row, then SEND (0x94) the whole
    `size × count` table to every peer;
  - peer: SEND own row to the master, then RECEIVE the table.
  - Chunks are 2 KB (`sub_430F0`/`sub_42FB0`); calls block on
    completion.
  - The lobby and every game tick both use this primitive.
- **START** (`:59528-59569`):
  1. `var_u16_17 = row + 50`; set the busy cursor; draw;
  2. `var_u16_10 = count`;
  3. join (§5.1). On success:
     - `var_u16_8 = seat` and `var_u8_0 |= 0x10` (network game);
     - send my level byte; the 10-B rows exchange;
     - `var_u16_17 = row0`;
     - `bool_96880 = true` → the frontend loop exits (`:41538-41542`).
- **Start barrier = the first tick.** `sub_3DD50` queues control 1
  ("present") in every record (`:49151`). Each tick, `sub_3C9D0`
  (`:48369`) in a network game:
  - exchanges the 10-B control rows (`:48504`);
  - if any row holds command 1, applies names (§3) and exchanges all
    **2049-B wizard records** (`:48543`; binary call sites 0x3CA0A /
    0x3CB05);
  - times the exchange into `var_u32_157` (`:48502-48506`).

  Because every call blocks, all peers leave tick 0 together. The level
  is the same file, and init derives the rest deterministically.

## 6. Exit paths

- **EXIT / Esc in the lobby, idle** (`:59591-59607`; binary
  0x66432-0x6648F):
  - teardown;
  - restore the 2049-B wizard record `+9225 → +13323`;
  - `var_u16_17 = var_u16_19`;
  - clear the network bit `var_u8_0 &= ~0x10`;
  - `var_u16_8 = 0`;
  - selector 2. Main-menu init `sub_4C7E0` (`:58936`, called `:58507-58508`)
    replays song 4 (`:58992`) because `byte_12CBD5 & 1` was cleared.

  ⚠ No NetBIOS hang-up happens here: a session joined earlier stays
  open (`byte_93959` stays 1).
- `sub_4E270` (`:59802-59803`) plays song 4 only "if selector == 2".
  Its single caller (binary 0x4DC58) runs before the selector is reset
  (binary 0x66450 call, 0x6647A store), so that arm never fires from the
  lobby. The music comes back via the menu's own init.
- **Abort while connecting** (button or Esc): fade to black, delete the
  name or hang up, return −1, stay in the lobby (`:53078-53094`,
  `:53753-53759`).
- **Leaving a networked level.** In-game control commands are processed
  per seat in `sub_3C9D0`:

  | command | trigger | status | cite |
  |---|---|---|---|
  | 27 | Space; needs status won | 10 | `:20080-20083`, `:20553`; gate `:20910-20915` |
  | 28 | Space while lost | 12 | `:20916-20921` |
  | 29 | Esc in the panel mode; message `ETEXT[62]` "has been eliminated from the realm." | 8 | `:20540-20541`, `:48812-48826` |
  | 2 | Alt+Q / Shift+Q; quit | if local: `+13327 = 1` and **TopProcedure exits the game** (`:41521-41522`, `:41617`) | `:20257-20258`, `:20475`; case `:48578-48586` |

  Every one of them calls `sub_43760_43AA0(seat)` (`:53860-53902`):
  - leaver = me: cancel and hang up every session, delete my name,
    `byte_93959 = 0`;
  - leaver = a peer: hang up that peer's session.

  Then the link table is rebuilt and the **master migrates** to the
  lowest still-linked seat (`:53887-53899`).
- **Back to the lobby after a level.** Nothing sets the selector after
  START (it stays 4), and the frontend loop re-enters with
  `byte_12CBD5 & 1` clear. So the **lobby reopens directly**. Measured
  against the dispatch, not playtested: there is no win/lose FMV and no
  PPERF score screen, which are selector 5 and only reached via New
  Game. A won level's `var_u16_17++` (`:41598-41601`) is irrelevant,
  because START re-picks. Since the local leave cleared `byte_93959`,
  the next START re-runs the full join.

## 7. MC1 vs HW

Same code and the same art (one `DATA/SCREENS` on the CD; HW loads the
same `pmulti.*` names, `hw:55861-55862`). HW functions sit at +0x340.

| item | MC1 | HW |
|---|---|---|
| Gate | `byte_93958` | same (`hw:54956`, binary 0x501F0) |
| Dead startup join | yes (`:42305`) | yes (`hw:38834`, binary 0x4E072-0x4E079) |
| Lobby list length | 20 (`:59421-59423`) | 10 (`hw:55515-55517`, binary `cmp $0xa`) |
| Arena levels | `LEVELS.DAT` 50-69 | `DDLEVELS.DAT` 50-59 (`+50`, `hw:55625`, binary 0x66585) |
| Arena names | Bussorah..Comari | Goyaan..Halaj |
| Seat positions, rects, NCB flow, names / call-name, exits | — | identical (`hw:44640-44720`, `hw:49124`, `hw:49732`) |

Port artefacts: HW's button table is a stub (`hw:1572`) and HW's
`sub_4FEB0` body is absent from the listing. The binary has both.

## 8. Assets

All of these are in the GOG ISO `gamedata/Magic Carpet Plus/CARPET.CD/game.gog`
under `/CARPET/DATA/SCREENS/`, shared by both EXEs.

| file | size | use | in gamedata | baked today |
|---|---|---|---|---|
| `PMULTI.DAT` | 64000 | 320×200 background | yes | **no** (no `mgc-import` reference) |
| `PMULTI.PAL` | 768 | palette | yes | **no** |
| `PMULTSPR.DAT` / `.TAB` | 29491 / 156 | 26 entries (6-B {ptr, w, h}, 640-space): [0] empty, [1-8] seat off, [9-16] seat idle, [17-24] seat connected, [25] 8×9 unused | yes | **no** |
| `SFONT1.DAT` / `.TAB` | 10819 / 1446 | lobby text (channel, map list) | yes | **yes**, in `baked/assets/mc1-movies` (FMV subtitle font; `bundle.rs:1467`). Not in `mc1-ui` |
| `SPTRS.DAT` / `.TAB` | 8408 / 198 | pointer [4], busy frames [6-20] | yes | **no** (`lib.rs:6123`: "not baked; the OS pointer stands in") |
| `ETEXT.DAT` [34]/[35]/[62] | — | name prompts, elimination message | yes | yes (mc1-movies / temperate / arctic) |
| Arena names | — | EXE tables (§4) | in the EXEs | **no**: `level-names.json` holds only the 50 campaign names per game |
| Arena levels 50-69 | — | — | yes | yes: `baked/mc1/level-050..069.mgcl`, `baked/mc1hw/level-050..069.mgcl` |

`PMULTSPR` has the same TAB layout as `MMSPR`, so `hspr::decode`
should handle it (not tried).

## 9. UNKNOWN / not recoverable

- What the PMULTI art depicts at each rect: labels, and which colour
  each seat portrait is. Answerable by looking at the file (stills can
  be rendered for the player on request).
- What happens when peers choose different player counts. The code
  implies the larger-count peer blocks until abort; this was not run.
- Whether any automatic level end exists for all peers when one seat
  wins. Only per-seat leave commands were traced.
- The 10-B control-row layout beyond byte 0 (command) and byte 1 (arg
  / level). See memory `retail-multiplayer-netbios-lockstep`.
- `byte_9689D`'s intended meaning (dead) and `PMULTSPR[25]` (unused).
- Exiting the lobby with a session still open (no hang-up): the effect
  on peers is not traced.
- The meaning of NetBIOS ADD NAME errors other than 0x0D. The code only
  distinguishes 0 / 0x0D / other.
- Real-world behaviour of GOG's IPX `NETBIOS.EXE` under DOSBox. No
  retail run was made.

## 10. Corrections to earlier notes

- `mc1-campaign-save-menu.md` recon 4 and `NETPLAY.md` stage 3 say
  "Multiplayer screen (:60004-06) plays song 1". **Wrong.** `:60004-06`
  is in `sub_4E5B0` = selector 5, the post-level PPERF score screen.
  The lobby stops the music and plays nothing (§2.1).
- "Returning to it replays 4 (:59803)": that arm is dead (§6). Song 4
  returns via main-menu init (`:58992`).
- The same document's "Key structs" reads `var_u8_61` as a "12-byte
  custom starting-spell loadout" with `off_99B68` as its default.
  **Wrong.** `var_u8_29` is the player's name (30 ch) and `var_u8_61`
  the 8-char **call-name** (ETEXT[34]/[35]). `off_99B68` holds the
  default wizard names. `:48521-48531` / `:48570-48575` copy a name
  into `str_14357`, not spells.
- "Multiplayer needs network" is true. The network comes **only** from
  `-network`. `-players`/`-session`/`-name` have no effect in the
  shipped EXEs (§1.2).

## 11. Mapping onto the remake

**Reusable as-is**
- The screen: background, palette, the 8 seat icons with their
  off/idle/blinking/connected states, the channel field slot, the
  5-row scrollable arena list in SFONT1 with white/brown selection,
  START, EXIT and ABORT rects (§2.2).
- Composition: CPU-composed 320×200 like `frontend_mc1.rs`, with
  sprites at 2× table positions.
- Silence on entry, and song 4 on return via the menu init the port
  already runs.
- Entry and exit: gate on "network available" (for us, always
  available, or "not in a session"); back up and restore the
  campaign record and level around the lobby; the lobby reopens after
  a networked level, with no FMV and no score screen.
- Arena list = levels 50-69 (MC1) / 50-59 (HW). Both are already
  baked; the names need a small bake addition (§8).
- Retail rules worth keeping: the host (seat 0) picks the arena; no AI
  fill; level spell row 0 for every human (matches NETPLAY's
  equal-arsenal direction); names = call-name or the default wizard
  name; master = lowest linked seat.

**Assumes NetBIOS machinery we do not have**
- The channel name `CARPET0-9`, which is a LAN rendezvous via NetBIOS
  name registration.
- "First free name = seat": seat order by arrival on the LAN.
- The full session mesh with LISTEN+CALL races.
- Blocking exchanges and abort polling inside them.

For us:
- the channel field becomes the typed host address plus recent hosts;
- seat order becomes host-assigned (seat 1 = host), as `net.rs` does;
- the mesh becomes the existing TCP star.

Retail's own data path is already a star through the master
(`sub_43930`), so our host-authority star matches retail's exchange.
Only connection setup differs.

**Gaps retail leaves (our parts)**
- Host / Join choice.
- Player names visible in the lobby.
- Live sync of the host's arena and count to clients.
- A count agreed by protocol instead of by convention.
- A ruleset page (NETPLAY `rules::PINS`).
- Ready states.
- Seat choice: retail has none; the PMULTSPR icons could become a seat
  picker.

**Open questions for the player**
1. Keep the 10 retail channel slots (the `CARPET%d` field) as a
   cosmetic "room" label? Or put the address entry there, in the same
   90×15 field?
2. Should the host's arena pick show live on the clients (retail: it
   does not)? Should the clients' list be read-only?
3. Player count: the host sets it (retail: everyone sets it, and it
   must match), or simply "whoever joined"?
4. Seat icons: keep retail's arrival order, or let a player click a
   free portrait to claim that seat (colour)?
5. After a networked level, return to the lobby as retail does, or
   offer the score screen first?
6. Show the call-name in the lobby (retail never shows names before
   play)? Should the profile's call-name (button 2) be the netplay
   name?
