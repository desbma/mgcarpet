# MC2 multiplayer lobby / network-game frontend — retail recon (2026-10-15)

Read-only recon of what retail Magic Carpet 2 (`NETHERW.EXE`) does from the
main menu's **Multiplayer** button up to the level start, and back. It
exists to inform the remake's in-game lobby (docs/NETPLAY.md, roadmap
step 3). Companion: `mc2-campaign-save-menu.md` (main menu, scroll dialog,
Set Name).

## Citation conventions and source hygiene

- `MI:` = `reference/remc2/remc2/engine/MenusAndIntros.cpp`, `EF:` =
  `…/EventsFunctions.cpp`, `NW:` = `…/Network.cpp` (current tree).
- ⚠ **The current remc2 tree has its own TCP netplay grafted onto these
  functions** (commits `08a22f94a` "network rework", `abefa7806`,
  `c0fab6d57`, …). Everything mentioning `Iam_server`, `UpdateNetInfo`,
  `AutoTest`, `g_matchResultPending`/`ShowScores`, `NetworkLeaveSession`,
  `MatchScore*`, `DestroyPlayerCastle`, `debug_net_printf`, server hand-over
  "stability" logic, and the hard-coded `v23 = 1; //Skip Game Number entry`
  (MI:5544-45) is **remc2, not retail**. For the transport and in-game
  halves this doc cites the last pre-rework listing instead:
  `NWold:` = `git -C reference/remc2 show a88a8eecc:remc2/engine/Network.cpp`
  (2022-09-01, when the net code was first split out), and
  `SMold:` = `git -C reference/remc2 show a88a8eecc:remc2/sub_main.cpp`.
  Even `NWold` has remc2 shims (`ReceiveServerAddName`, "WAITING FOR
  SERVER", `maxSizeOfPacket = 2048*30`); those are flagged where they matter.
- `EXE:` = disassembly of the shipped `NETHERW.EXE` (repo root, LE code
  object: **file offset = VA + 0x24800**; the data object: **file offset =
  data-operand + 0xE5800**, i.e. a remc2 `x_…_Exxxx` global lives at
  operand `0x1xxxx`, a `…_17Dxxx` at `0xADxxx`; jump-table operands carry a
  `-0x10000` bias). Every structural claim below marked ✔EXE was checked
  against the shipped bytes; the shipped exe outranks the listing.
- Language strings: `LANGUAGE/L2.TXT` as baked in
  `baked/assets/mc2-ui/strings.json` (index = `langindexbuffer[n]`).

---

## 1. Entry — what the Multiplayer button needs

### 1.1 Network detection = NetBIOS at boot, no switch

- At boot, unconditionally, `NetworkAllocation2_5C430` (remc2
  `sub_main.cpp:524-527`; ✔EXE caller at 0x5BBEA, no condition) stores
  `isNetwork_216w = NetworkAllocation_74556()` (NWold:607-686).
- `NetworkAllocation_74556` allocates the 9 NCBs + 10 packet buffers and
  calls `NetworkInit_74A11` (NWold:758-775; ✔EXE 0x74A11): `getvect(0x5C)`
  must be non-null (the NetBIOS interrupt vector), then a NetBIOS call with
  command `0x7F` (an invalid command — the classic "is NetBIOS installed"
  probe) must come back with `retcode == 3`. Any failure → `x_BYTE_E1274 =
  0`, `isNetwork_216w = 0`.
- **No command-line switch enables networking.** The retail argument parser
  does accept `network` (string at file 0xE6154; EF:39415), but in the
  shipped exe the match jumps straight to `argnumber++` (✔EXE 0x562BF →
  0x56576): it is a parsed no-op (vestigial). The other retail switches in
  the same table: `level`, `harddrive`, `skipscreens`, `nocd`,
  `showversion`, `showversion2`, `detectoff`, `langcheck`, `vio`, `extern`,
  `VFX1`, `cc`, `spellsedit`, `music2` (string pool at file 0xE6154+).
  (`client`/`server`/`otherserverport` and `editor`/`reglevel` are remc2.)

### 1.2 The button is inert without a network

- `sub_7BEC0` (MI:2189-2210; ✔EXE 0x7BEC0, called from
  `MenusAndIntros_76930` at 0x7699C) sets, for the main-menu button whose
  `byte_22 == 7` (Multiplayer), `canSelect_23 = (isNetwork_216w != 0)`
  (struct offset 0xD8 = 216 in the shipped code).
- `DrawAndServe_7B250` only hovers/clicks buttons with `canSelect_23`
  (MI:6186-6201). So **a player without NetBIOS sees the Multiplayer
  button as plain background art: no lit hover sprite, no click sound, no
  dialog**. The hover sprite is drawn only for `canSelect_23 &&
  gold_color_24` (MI:2059), so even the F1 help cycler, which still shows
  the Multiplayer tooltip (417) in turn and sets `gold_color_24` on it
  (MI:6223-6229), does not light it. There is no "no network" message
  anywhere.
- Main-menu table row (MI:341, ✔EXE data): Multiplayer at (362,72) 80×80,
  sprites `byte_20/21 = 61/53`, `byte_22 = 7`, `word_18 = 409` ("Play
  MultiPlayer Game"); its dialog `str_26 = {x1 430, y1 78, open-height 80,
  title 412, mode 3}`; handler VA 0x25EE80 → `MultiplayerMenu_7DE80`
  (MI:4937-4940). F1 tooltip row `textBoxStr_E25DC[7]` = string 417 "Play
  MultiPlayer Game" (MI:422).

### 1.3 Click → "Enter Network Session" scroll dialog (session digit)

`MultiplayerMenu_7DE80` (MI:5479-5595; ✔EXE 0x7DE80-0x7E0CD):

- The ordinary parchment scroll dialog (`DrawScrollDialog_7BF20`, the same
  one the main menu uses for Exit/Name, see the companion doc's
  "DrawScrollDialog2 composition") at (430,78), title **412 "Enter Network
  Session"**, mode 3 = OK + Cancel; it opens over the temple screen, which
  keeps animating.
- Once the scroll is more than half open, a **single digit 0-9** is edited
  in the middle of it: the last typed ASCII char in `'0'..'9'` replaces
  `x_BYTE_E29DF` (Backspace 0x0E, Del 0x53 and Tab 0x0F are ignored; the
  edit is additionally gated by `x_WORD_17DEF0 < 12` — the *name-entry*
  length counter, a leftover). Drawn with `"%d"` in FONT1, black, plus a
  blinking `_` caret every 12 clock ticks (✔EXE 0x7DF34-0x7E022).
- ⚠ remc2 forces `v23 = 1` (MI:5544-45) so the dialog auto-confirms; the
  shipped code uses the dialog's real result (✔EXE 0x7DF10 stores it,
  0x7E028 tests it).
- **OK** (or Enter): `networkSession_17DEFA = digit`, `x_WORD_E131A = 0`,
  cursor → empty sprite, call the lobby `sub_77680` (✔EXE 0x7E087 — the
  shipped call pushes a dummy argument 0). Its result goes into the
  button's `dword_4`, which makes `DrawAndServe` reload the temple screen
  (HSCREEN case 4) and the main-menu cursor (MI:6156-6165) — *unless* the
  lobby started a game (then `m_ExitMenuLoop_E29DC` is already set and the
  menu loop ends).
- **Cancel** (or Esc): the digit reverts to the last confirmed
  `networkSession_17DEFA`; back to the menu.
- The digit **is** the game identity: there is no session list, no browser,
  no host/join choice. Everyone who types the same digit lands in the same
  game (§5.1).

---

## 2. The screens

### 2.1 "Attempting Connection" (only while NetBIOS is busy)

`WaitToConnect_7C230` (NWold:1054-1087; ✔EXE 0x7C230) is the idle callback
of every blocking NetBIOS wait during connection (ADD NAME, CALL; ✔EXE
callers 0x72F9E, 0x73013, 0x747E8). Each call redraws:

- the temple background + permanent sprite 66 at (185,232) + the fire /
  incense animations (same as the main menu);
- a scroll dialog `str_WORD_E1F70 = {430, 78, 0, 0, 80, 0, 413, 0, 0}`
  (✔EXE data 0x11F70): same place as the session dialog, title **413
  "Attempting Connection"**, **mode 0 = no buttons** (nothing to click; the
  "closed" flag `x_WORD_E131A` it would set is write-only — ✔EXE: only two
  writes, 0x7C2BC and 0x7E055, no reads);
- an animated "busy" cursor: case-4 menu sprites **40..49** (24×28 each),
  stepping every 4 clock ticks (`timeCounter_E2094` between
  `minTimeCounter_E2096 = 40` and `maxTimeCounter_E2097 = 49`, ✔EXE data
  0x12094-0x12097), drawn at the mouse position.

There is no cancel: the wait ends when NetBIOS completes or the CALL times
out (§5.1).

### 2.2 The lobby screen (HSCREEN0 case 7)

`sub_77680` (MI:5903-6125; ✔EXE 0x77680-0x77976, retail loop body =
0x777F8-0x7794B) loads `sub_7A110_load_hscreen(.., 7)` (EF:46496-46523).
The case-7 chunks are **contiguous** in `DATA/SCREENS/HSCREEN0.DAT`
(verified against the pristine install, all RNC as noted):

| chunk | file offset | packed | unpacked | content |
|---|---|---|---|---|
| palette | 0x42471 | 768 raw | — | the lobby's own 6-bit palette |
| background | 0x42771 | 82 233 RNC | 307 200 | 640×480 8bpp |
| sprite pool | 0x568AA | 21 216 RNC | 42 428 | lobby sprite bank |
| sprite TAB | 0x5BB8A | 148 RNC | 198 | 33 records |
| wizard portraits ×8 | 0x5BC1E…0x76BBD | 11.9-17.1 K RNC each | w×h | raw rect images (§2.2.3) |
| wizard palettes ×8 | 0x76BBD + 0x300·k | 768 raw | — | per-portrait palette |
| "4b" font pool/TAB | 0x13ACCA / 0x13B194 | 1226 / 548 RNC | 6613 / 1632 | small help font (shared with case 4) |

**What it looks like** (decoded for this recon): a dark stone temple
doorway seen from inside a cave. Above the door a carved lintel with a
slot bar along the very top, a row of **seven small windows** (y≈54-69) and
one **oval window** below them (286-319, 81-95); two carved glyph plaques
left and right of the lintel; two **carved totem pillars** flanking the
doorway; the doorway itself is black — the wizard portrait appears in it.

#### 2.2.1 Buttons (`str_E1BAC_0x1b8`, MI:437-444, ✔EXE data)

| # | handler (MI) | rect (x,y,w,h) | lit sprite (`byte_21`) / unlit (`byte_20`) | what it is |
|---|---|---|---|---|
| 0 | `MultiplayerPreviousLevel_7CCF0` (:5419) | 284,2,30,10 | 4 / 11 | small arrow, top bar |
| 1 | `MultiplayerNextLevel_7CD30` (:5430) | 282,34,30,10 | 5 / 12 | small arrow under the level name |
| 2 | `MultiplayerExit_7CCA0` (:5442) | 111,101,40,40 | 1 / 8 | left plaque |
| 3 | `MultiplayerPlayLevel_7CDA0` (:5450) | 449,100,40,40 | 2 / 9 | right plaque |
| 4 | `MultiplayerPreviousWizard_7CBF0` (:5457) | 154,259,80,150 | 6 / 13 | left totem |
| 5 | `MultiplayerNextWizard_7CC40` (:5468) | 399,258,80,150 | 7 / 14 | right totem |

- Like the main menu, the button art is part of the background; on hover
  the frame draws the **`byte_21`** sprite at the rect origin
  (MI:6356-6361); a click plays **sample 14** (vol 127) and fires the
  handler (`sub_7CB10`, MI:6413-6460). Sprites 8-14 (`byte_20`, darker
  copies) are never drawn by this code.
- Lobby bank census (TAB = 33 records): 0 empty; 1-7 lit plaques/arrows/
  totems; 8-14 unlit twins; 3/10 an 18×22 plaque no button references;
  **15 = cursor** (19×21; `SetCursor(spritestr[15])` after the fade-in,
  MI:6108 / ✔EXE 0x7790E: `+0x5A` = 15×6); 16-18 116-px bars and 19-21
  small bits (probably the help-textbox frame — not traced); 22-32 eleven
  11×9 frames (use not traced).
- **Keyboard: only F1** (scancode 59, help toggle) is read in the lobby
  loop (✔EXE 0x77800). There is no Esc, no Enter.

#### 2.2.2 Text fields

- **Level name** (`DrawNetworkLevelName_7D380`, MI:6544-6551; ✔EXE
  0x7D380): clears background pixels ≥ 0x9F in (246,14) 109×14, then FONT1
  `"%d. %s"` with `level − 49` and `LevelsNames_D9204[level]`
  (EF:2305-2366), clipped between x 246 and 355, y 14. Multiplayer levels
  are **50-59**: 1. Thrull, 2. Keevur, 3. Braak, 4. Trapox, 5. Hibren Zhor,
  6. Jinople, 7. Dethrem, 8. Canquin, 9. Zephulum, 10. Verune (strings in
  the exe at file 0xE688C-0xE68E2; the remake already has the table as
  `MC2_LEVEL_NAMES`, `crates/mgc-app/src/worldmap.rs:117`).
- **Session digit**: `sub_7C140_draw_text_background(382, 18, 16, 16, 0)`
  then `"%d"` of `networkSession_17DEFA` centred in x 382-398, y 18
  (MI:6363-6365; ✔EXE 0x7C65F-0x7C6A5) — the little box at the top right of
  the lintel. Font = whatever is current (not traced; FONT1 is the only one
  selected in this frame).
- **No player names** anywhere in the lobby (no name is exchanged before
  the level; §3.1).

#### 2.2.3 The player row and the portrait (palette tricks)

- The seven lintel windows and the oval are drawn in background palette
  indices **134-149**, one light/dark pair per slot: slot *k* ↔ entries
  `134+2k`, `135+2k` (window k for k=0..6; slot 7's pair 148/149 is the
  oval). They are **black** in the shipped palette (measured).
- `SetMultiplayerColors_7D310` (NWold:1186-1198; NW:1610) recolours, every
  frame, the pair of **every connected slot except your own** from
  `str_BYTE_E1711` (NWold:33-53; ✔EXE data 0x11711): wizard colour index
  0 grey (36,36,36)/(22,22,22), 1 green, 2 red, 3 violet, 4 pink, 5 brown,
  6 blue, 7 orange (6-bit light/dark). So the windows light up in the
  other players' colours, in **slot (join) order**.
- Your own wizard is the **portrait** in the doorway: `str_E1BAC_0x2ec`
  (MI:229-238; ✔EXE data), 8 entries = {image offset, packed length,
  palette offset, 768, x, y, w, h}; e.g. #0 = 136×348 at (244,133). The
  portrait is a raw rect (0 and 0xFF transparent) composited into the
  background buffer (`sub_7C720`, MI:6392-6406 / `sub_85BF5`) and uses
  palette entries **159-254 plus 148/149** from its own wizard palette —
  148/149 of each wizard palette is that wizard's colour pair, so **the
  oval window shows your own colour** (measured: e.g. wizard 1 → green
  (00,26,00)/(00,16,00)). Note the clash: a remote player in slot 7
  recolours 148/149 too.
- Portrait change animation, state `x_WORD_17DEF6` (MI:6301-6350):
  entry state 5 (clear the portrait rect (208,111) 200×369 of pixels ≥ 0x9F
  and copy the master's level, §4) → 1 (load image + palette) → 3 (fade
  the 98 portrait colours **in**, +16/255 per frame, `PaletteCopy_7C800`)
  → 4 (idle); prev/next → 2 (fade **out**, `PaletteMulti_7C9D0`) → 1 → 3.
  The displayed portrait is `x_WORD_17DEF4 = master.colors[mySlot]`, so it
  changes only after the master has answered (§4).
- Lobby wizard index → in-game identity (`GetTrueWizardNumber_61790`,
  Basic.cpp:3005-3036, ✔EXE jump table 0x61768): 0 Zanzamar, 1 Rahn,
  2 Nyphur, 3 Jark, 4 Elyssia, 5 Yragore, 6 Belix, 7 Prish (names
  `WizardsNames_D93A0`, GameUI.h:39-41).

#### 2.2.4 Help mode (F1)

With help on, the lobby cycles one tooltip every ~2 s from
`textBoxStr_E2570` (MI:406-412; loop MI:6080-6096 / ✔EXE 0x7782D-0x778D3,
`DrawTextBoxWithLine_7E840(tb, 21, 27)`): 457 "Exit mulitplayer session"
[sic], 459 "Select previous character", 458 "Select next character",
456 "Start multiplayer game", 460 "Select multiplayer level".

#### 2.2.5 Sound / music / timing

- No music call in the lobby or the session dialog (✔EXE call list of
  0x77680 / 0x7DE80): **main-menu track 4 keeps playing** (started by
  `MainMenu_76FA0`, MI:841 / ✔EXE 0x7702B).
- Only sample 14 (button click) is used.
- Palette fade-in of the whole screen on the first frame
  (`sub_90B27(.., 0x20, 0)`, ✔EXE 0x77909); fade-to-black on leaving
  (`FadeClearBlit_7B5D0`, ✔EXE 0x77950).
- There is no frame limiter: each frame blocks on the lobby exchange (§5.2),
  so the lobby runs in lock-step with the slowest peer.

### 2.3 Chat

None in the lobby. In-game, retail has keystroke chat carried in the input
stream: input action **0x11** appends `byte1` as a character (8 =
backspace) to the sender's message buffer `names_81[...]`, max 43 chars
(SMold:51415-51443). How a message is opened/sent/shown was not traced.

---

## 3. Players and seats

### 3.1 Identity / name

- Set Name (`SetPlayerNameDialog_78E00`, companion doc) writes
  `player_name_57ar` (max 12, uppercased). It is **not used in the lobby**.
- In the level, every peer's first input carries action 1 (set at level
  init, EF:38446). On a networked turn where any action is 1, each node
  copies **its own** `player_name_57ar` (if non-empty) into its wizard's
  name and then exchanges the whole player-struct array `array_0x2BDE`
  (`ReceiveSendAll` #2, SMold:51270-51302; ✔EXE second call at 0x51C6B).
  That is how names reach the other players. Without a name the wizard
  keeps the default `WizardsNames_D93A0[GetTrueWizardNumber(slot)]`
  (EF:38485-38489).

### 3.2 Seat = NetBIOS slot (join order); wizard = lobby choice

- **Seat**: your NetBIOS index (0-7, first free name, §5.1) becomes
  `serverIndex_17DEFC` and, at Start, `D41A0_0.LevelIndex_0xc`
  (NWold:1149-1150; ✔EXE 0x7D144-0x7D150). You cannot choose it.
- **Wizard / colour**: one of 8 portraits, assigned and arbitrated by the
  master (§4): on joining, the master picks for slot *i* the first wizard
  **not used by another connected slot**, searching *down* from `i−1`
  (slot 0 starts from 0) with wrap 0→7 (`sub_7D230`, NWold:1201-1249;
  action 2, NWold:1120-1129; ✔EXE 0x7CFC4-0x7D07F). The do/while always
  steps at least once, so slot 0 starts at wizard 7 and slot 1 at 7 (or 6
  if taken). Prev/next totems step down/up to the next free one.
- **The choice is frozen at 7+ players**: actions 3/4 are honoured only
  while `connected < 7` (✔EXE 0x7D098 / 0x7D0F9).
- At level init the chosen index is stored per seat:
  `str_611.byte_0x1C0_448 = x_BYTE_E131C[seat]` (EF:38481-38483,
  ✔EXE read at 0x532C8), and drives the wizard name
  (`GetTrueWizardNumber_61790`) and the in-game colour
  (`TransformPlayerColorIndex_616D0`, GameUI.cpp:869-900; remc2 changed
  index 7 there: "fix wizard index, in original is 7").
- **Maximum players: 8** (`NetworkInitConnection_7308F(name, 8)`,
  MI:5916 / ✔EXE 0x776E0).
- **No AI fill**: in multiplayer no seat gets `IsAiPlayer` (EF:38449 —
  the flag is set only when `!MULTIPLAYER_MODE`), and `NumberOfPlayers_0xe`
  = the connected count (✔EXE 0x7D176-0x7D182). (This matches the
  project's 10-10 ruling; docs/NETPLAY.md step 2's "retail always fills"
  does not hold for MC2 MP — see Open questions.)

---

## 4. Session options

Only two things can be set in retail, and there is no "host" role in the
UI — anyone can press anything:

| option | who | mechanism |
|---|---|---|
| **level** (50-59) | **any player** | prev/next arrows change your own `selectedLevel_10` and raise action 6; every node copies the sender's level (action 6, NWold:1159-1161; ✔EXE 0x7CF21). On joining you adopt the master's level (state 5, MI:6343-6349). Initial value = last `levelnumber_43w` if ≥ 50, else 50 (MI:5934-5937; ✔EXE 0x777A6-0x777DD) — so the lobby remembers the last MP level. Bounds: > 50 for prev, next stops at an empty or `"0"` name (index 60) (✔EXE 0x7CCF0-0x7CD9C). |
| **wizard** | each player for himself, arbitrated by the master | §3.2 |

- **Start**: the Play plaque sets action 5 **for any player, ungated**
  (✔EXE 0x7CDA0: one store, no test).
- No game type, no spell set, no options, no team/alliance setting, no
  ready flags, no kick. The level always plays with its own authored
  rules (the remake's memory: MC2 MP = battle royale, no objectives).
- Clients see exactly the same screen: level name, session digit, the
  window row, their own portrait.

---

## 5. Protocol

### 5.1 Transport and joining (NetBIOS)

- Node name: `"NETH%d"` with `session + 20` (MI:5913; exe string "NETH%d"
  at file 0xE725A) → `NETH20`…`NETH29`; per slot `"%s%d"` → `NETH200` …
  `NETH207`, space-padded to 15 chars (NWold:82-93, 689-702).
- `NetworkInitConnection_7308F` (NWold:174-274; ✔EXE 0x7308F-0x73360,
  minus remc2's "WAITING FOR SERVER" and test hooks): try ADD NAME for
  slot 0, 1, …; the first that succeeds is your slot (a code-13 result is
  retried after DELETE NAME — NetBIOS "duplicate name in local table").
  - **Slot 0 = the master** ("collector", `IndexInNetwork2_E12A8`): it posts
    LISTEN on every other slot (`NetworkListenAll_7302E`) and enters the
    lobby alone, immediately.
  - Other slots CALL slot 0 and wait (`NetworkTestCall_72FBB`, ✔EXE
    0x72FBB: timeout `0x78` = 120 ticks of the clock at 0x17DB54 — rate
    not established, remc2 comments "original only 120"); no answer →
    DELETE NAME, return −1 → **silently back to the main menu** with the
    normal cursor (✔EXE 0x776F9-0x77713). Same silent return if NetBIOS is
    absent or a session is already open (`x_BYTE_E1275`).
- Topology: **star**. Clients only talk to the master. A second, mesh-style
  connect routine exists (0x73361, `sub_72E70` CALLs every slot) but has
  no direct caller in the shipped exe (dead or reached via a pointer — not
  established).

### 5.2 The lobby exchange (every lobby frame)

`SetMultiplayerColors_7CE50` (NWold:1090-1170; ✔EXE 0x7CE50-0x7D1E3):

1. `array[mySlot].connected_0 = 1`; refresh connection states
   (`NetworkUpdateConnections_74F76`, NWold:974-983).
2. `ReceiveSendAll_7438A(array, 11)` (NWold:529-574): the **master**
   RECEIVEs 11 bytes from each connected client into that client's row,
   then SENDs the whole array (11 × 8 = 88 bytes — `countConnected` is
   forced to 8, NWold:982) to each client; a **client** SENDs its own row
   and RECEIVEs the 88-byte array. All calls block until done → the lobby
   is itself lock-step.
3. Everyone copies the master's `colors[8]` into every row and into
   `x_BYTE_E131C` (the seat→wizard table used at level init).
4. For each connected row, act on its `action_9` (jump table ✔EXE
   0x7CE2C):

| action | raised by | effect on every node |
|---|---|---|
| 0 | — | nothing |
| 1 | Exit | blank palette 134-147 (the 7 windows; re-lit next frame for those still in), clear that row's `connected_0`, `NetworkSomeChange_73AA1(slot)` (hang up that slot; master hand-off logic, NWold:401-472); if it is my slot → leave the lobby |
| 2 | lobby entry (MI:5959) | master assigns that slot a wizard (§3.2) |
| 3 | prev totem | master: next free wizard downward (if < 7 players) |
| 4 | next totem | master: next free wizard upward (if < 7 players) |
| 5 | Play | **start** (§5.3) |
| 6 | level arrows | copy the sender's `selectedLevel_10` into mine |

5. Clear my own `action_9` (actions are one-shot), set `x_WORD_17DEF4 =
   master.colors[mySlot]`, recount connected rows (`x_WORD_17DEFE`).

**Lobby record** (`type_BYTE_17DE68x`, 11 bytes packed, Basic.h:48-54):

| offset | field | meaning |
|---|---|---|
| +0 | `makeUpdate_0` / `connected_0` | row in use |
| +1..+8 | `playerIndex_1[8]` / `arrayColors_1` | wizard per slot — authoritative only in the master's row |
| +9 | `action_9` | one-shot command (table above) |
| +10 | `selectedLevel_10` | signed byte, 50-59 |

### 5.3 Hand-off into the level

Action 5, on every node in the same exchange round (NWold:1149-1157;
✔EXE 0x7D144-0x7D197):

- `LevelIndex_0xc = mySlot`; `levelnumber_43w = master.selectedLevel`;
  `setting_byte1_22 |= MULTIPLAYER_MODE (0x10)`;
  `NumberOfPlayers_0xe = connected count`;
- `NetworkCancelAll_7449C` (the master cancels its pending LISTENs → **no
  late joining**);
- `m_ExitMenuLoop_E29DC = 1` → the menu loop ends, the main loop loads the
  level (`sub_46830_main_loop`, SMold:38582-38778).
- Same-tick start is implicit: everyone left the lobby on the same lock-step
  round, level init is deterministic, and the first in-game
  `ReceiveSendAll` (inputs, 10 bytes per seat, SMold:51275) blocks until
  every peer is in.
- ⚠ Observation, not tested: `NumberOfPlayers` is a *count* while seats are
  *slot numbers*; if a middle slot left the lobby, the highest seat can be
  ≥ the count.

### 5.4 In-game (for completeness)

Inputs only (memory: retail-multiplayer-netbios-lockstep); the master
reflects them. Peer departures go through `NetworkEvent_7373D`
(NWold:301-372, ✔EXE callers inside `GameEvents_51BB0`): if the leaver
is the master, it first sends every client the connected list; the
lowest connected slot becomes the new master, LISTENs, and the rest CALL
it (**master migration exists in-game**, not in the lobby).

---

## 6. Exit paths

- **Leave the lobby**: the Exit plaque. A client may always leave; the
  **master may leave only when alone** (`connected == 1`, ✔EXE
  0x7CCA0-0x7CCDF) — otherwise the plaque does nothing. Leaving fades out
  and returns to the temple main menu (HSCREEN 4 reload, MI:6156-6165).
- **A peer leaving cleanly (Exit)**: its window goes dark, others stay.
- **A peer vanishing in the lobby** (no action 1): not handled by the lobby
  code — its row keeps `connected_0 = 1` and the exchange's
  RECEIVE/SEND to it fail or block. Outcome **not established** without
  running retail (see UNKNOWN).
- **In the level**: a player who dies with no castle is **banished**
  (action 0xF → the banished path in MP, SMold:51387-51395, 51509-51526):
  "has been banished from the realm." (string 283), his wizard is removed
  (`sub_5E660`), `NetworkEvent_7373D(slot)` drops his connection, and on
  his own machine `byte[2] = 8` ends the level. The same happens on
  abandon (Esc → confirm → actions 29/27, companion doc).
- **After a network game**: `sub_46830_main_loop` sets `x_WORD_E29D8 = 4`
  (SMold:38777) → `MainMenu_76FA0`, which, seeing `MULTIPLAYER_MODE`,
  skips the world map, **clears the flag and shows the temple main menu**
  (MI:856-858; ✔EXE 0x77070-0x77079). No return to the lobby, no score
  screen (remc2's `ShowScores`, MI:845-853, is not retail). To play again:
  Multiplayer → digit → lobby (it remembers the last level, §4).
- In-game multiplayer refuses pause and save/load (companion doc,
  "Gating and write order"); actions 0x19/0x1A end the session for all
  active players (SMold:51485-51497 — what raises them not traced).

---

## 7. Assets

| asset | used for | in `gamedata/`? | baked? |
|---|---|---|---|
| HSCREEN0 case-7 palette + 640×480 bg | lobby screen | ✅ `DATA/SCREENS/HSCREEN0.DAT` (CD image) | ❌ — `crates/mgc-import/src/hscreen.rs` decodes only case 4 (`mainmenu`), case 6 (`worldmap`), the FMV font, the border |
| case-7 sprite bank (33) | buttons, cursor 15 | ✅ same file | ❌ |
| 8 wizard portraits + 8 palettes | own-wizard portrait, own colour | ✅ same file (0x5BC1E-0x783BD) | ❌ |
| colour table `str_BYTE_E1711` | window colours | exe data only (0x11711) | ❌ (16 RGB triplets, transcribe) |
| MP level names 50-59 | level field | exe strings | ✅ as code: `MC2_LEVEL_NAMES` (worldmap.rs:117) |
| case-4 temple bg/palette/sprites | session dialog backdrop, scroll parts 68-72, busy cursor 40-49, cursor 39 | ✅ | ✅ `baked/assets/mc2-ui/menu-{bg,pal,sprites}` |
| FONT1 (`DATA/FONT1.DAT/.TAB`) | session digit, level name | ✅ | ✅ `mc2-ui/font.bin` |
| "4b" small font (HSCREEN0 0x13ACCA/0x13B194) | F1 help tooltips (lobby and main menu) | ✅ | ❌ (main-menu F1 help isn't ported either) |
| strings 412, 413, 417, 456-460, 283 | dialog titles, tooltips, banish | ✅ `LANGUAGE/L2.TXT` | ✅ `mc2-ui/strings.json` |
| sample 14 | clicks | ✅ | ✅ (the ported main menu plays it) |
| music track 4 | continues from the main menu | ✅ | ✅ (main menu) |

All case-7 chunks decode cleanly with the project's own `rnc` +
`hspr::decode` (checked with a throw-away program outside the repo): bg
307 200 bytes, bank 33 records, portraits exactly w×h of their table rects
(e.g. #0 136×348 = 47 328).

---

## 8. UNKNOWN / not recoverable here

- What a peer **vanishing** in the lobby does (hang, timeout, stale window)
  — needs a retail run with real NetBIOS.
- The clock behind the 120-tick CALL timeout (0x17DB54) → the real join
  timeout in seconds.
- Whether the master's Play needs anything else in practice (the code lets
  anyone start, even alone — a one-player MP level is allowed by the code).
- Use of lobby sprites 3/10, 16-21, 22-32 (probably help-box frame and an
  unused/animated element).
- The font of the session digit in the lobby (no explicit `GetFont`).
- How in-game chat (action 0x11) is opened, sent and displayed.
- What raises in-game actions 0x19/0x1A (end for all) and how a match is
  "won" (last wizard standing?) — in-level, outside this recon.
- Who calls the mesh connect routine at 0x73361 (no `e8` caller).
- Whether the slot-7 / oval palette clash is visible in practice.

---

## 9. Mapping onto the remake

**Reusable as-is (art + layout + behaviour):**

- The temple's Multiplayer button: just make it selectable (retail gates
  it on NetBIOS; ours would always enable it).
- The parchment scroll dialog (already ported) for the entry step and for
  "Attempting Connection" (title 413, no buttons, busy cursor 40-49 — all
  baked).
- The whole lobby screen: background, palette, the six buttons and their
  hit rects, hover sprites, sample 14, cursor 15, level name field and
  arrows (levels 50-59, names already in the code), the portrait swap with
  its palette fades, the seven lit windows + own-colour oval, F1 tooltips.
  Needs a new bake: case-7 bg/palette/bank + 8 portraits + 8 palettes
  (+ the "4b" font if F1 help is wanted), plus the colour table.
- The lobby record and action set (join / leave / prev / next wizard /
  level / start) map cleanly onto typed messages over our TCP star with
  the host as the master; the host-arbitrated wizard choice ("first free"
  search) and "everyone adopts the master's level" are directly portable.
- Hand-off: seat = join order, wizard identity = lobby choice, level =
  master's choice, everyone starts on the same round, no late join, no AI
  seats — consistent with the existing `--host/--join` flow and the 10-10
  rulings.
- After the game: back to the temple main menu (retail) — or back to the
  lobby, if the player prefers (not retail).

**Assumes NetBIOS machinery we don't have (needs replacing):**

- The session **digit** = NetBIOS name `NETH2n`; "same digit" is LAN
  discovery by name. Ours needs: Host / Join choice, a typed address
  (+ recent hosts), port — the digit dialog could become that entry
  scroll, and the digit box on the lintel could show something else
  (seat number? a short session code?).
- Slot = first free NetBIOS name; ours: the host assigns seats on join.
- Join failure is silent in retail; ours should say why (refused, version
  mismatch — the exe-hash handshake already exists, timeout).
- The per-frame blocking lobby exchange is a NetBIOS artefact; an
  event-driven lobby over TCP (messages only on change) is the natural fit.
- No in-lobby handling of a vanished peer, and the master cannot leave
  while others are in — both need our own answers (host leaves = lobby
  closes, or host migration later — NETPLAY step 5).

**Open questions for the player:**

1. Entry step: keep a scroll dialog (repurposed from "Enter Network
   Session") with Host / Join / address + recent hosts, or a new screen?
   Where does the session rules page (`rules::PINS`, spell set) live —
   new screen, or scrolls over the lobby?
2. Who may press Play and the level arrows? Retail lets **anyone** start
   and change the level; the remake's model has a host authority.
3. Seat vs wizard: retail seat = join order and the lobby picks only the
   wizard identity/colour. Keep that, or let the totems pick the seat
   (`--seat`) as the CLI does now?
4. Show player names in the lobby (retail doesn't; names arrive only
   in-level via Set Name)? The ten-character `player_name` would fit the
   seven-window row only as tooltips/labels — new art decision.
5. Allow more than 7 changes / the 7-player freeze, and the slot-7 oval
   clash — irrelevant at 2 seats today; decide when > 2 seats arrive.
6. After a match: retail returns to the temple menu; return to the lobby
   instead (same peers, quick rematch)?
7. AI seats: docs/NETPLAY.md step 2 says "retail always fills", but MC2
   retail MP never sets `IsAiPlayer` (EF:38449). Is AI-on a deliberate
   port option (fine), or should the doc be corrected?
8. MC1 has its own lobby (NETPLAY leads: remc1 :60004) — same treatment
   per game, or one shared remake lobby skinned per game?
