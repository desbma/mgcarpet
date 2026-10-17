//! ⭐ NETWORK TRACK — apps sharing one world by deterministic lockstep
//! over TCP (rulings 10-14: TCP only, star topology).
//!
//! The star's centre is a SERVER (player 2026-10-17): a headless
//! [`Server`] that holds the lobby, assembles every tick's turn and runs
//! the world itself — the session's neutral authority ([`Referee`]: the
//! hash every player is checked against, and the game's verdict). Every
//! player is a client ([`Guest`]); one of them is the ADMIN, a player
//! with the lobby's powers (level, rules, AI, limit, kick, passphrase,
//! Play — sent as requests the server accepts from the admin only).
//! The server runs two ways, one implementation:
//! - dedicated: `mgcarpet --serve` in the foreground — every player
//!   joins, the first one in is the admin;
//! - hosted: the UI's Host (and `--host`) starts it on a thread of the
//!   game ([`ServerHandle`]) and joins it over loopback as the admin
//!   (claimed with a key only that app knows). Closing that app stops
//!   the server; its player leaving a game only waits in the lobby.
//!
//! Only INPUTS cross the wire — retail's own model (NetBIOS lockstep,
//! `ReceiveSendAll_7438A`): every peer simulates the whole world from
//! the same per-tick input bundle. Each client sends its seat's input
//! for tick `t` to the server; once every seat's is in, the server
//! steps its own world and broadcasts the bundle as `Turn t`; nobody
//! steps a tick before its bundle exists. Each seat's input is
//! scheduled [`INPUT_DELAY`] ticks ahead, which hides the round trip
//! under that many ticks of control latency.
//!
//! Unlike retail (no desync detection at all), the server stamps its
//! pre-step `state_hash` every [`CHECK_EVERY`] ticks onto the turn and
//! every client compares. A client that disagrees asks for a RESYNC:
//! the server sends its world's snapshot ([`Referee::snapshot`], taken
//! before the next turn it steps) ahead of that turn, and the client
//! applies it right before stepping the same tick — the server's world
//! wins. The client reports where it had diverged (the sectioned state
//! hash, `Simulation::divergence_report`) and checks the restored hash;
//! a resync that does not heal stops the checks. Every resync is told
//! loudly: it hides a fork, it does not fix the bug behind it.
//!
//! A client leaving mid-game (Bye, its socket lost, or [`Msg::Leave`])
//! does not end the game: the server drops that seat at the first turn
//! not yet sent — the seat's input there is the suicide key, idle after
//! — and the turn says so ([`Msg::Turn::left`]). Every peer banishes the
//! seat on that tick (retail: an abandon banishes the wizard). A LEAVE
//! keeps the member: his lobby WAITS for the game to finish
//! ([`LobbyView::playing`]), and so does a newcomer admitted during it.
//!
//! The game ends when the server's world names a winner (one wizard
//! standing, [`Referee::verdict`] — announced on the next turn): every
//! client fades out and leaves, and the server reopens the lobby once
//! they all have (or [`GAME_OVER_GRACE`] passed) — or when nobody is
//! left playing.
//!
//! Wire: a little-endian `u32` length, then one JSON [`Msg`]. The input
//! payload is the recorder's [`PortInput`] (idle ticks encode as `{}`).

use crate::lobby::{LobbyCore, LobbyView, NetGame};
use mgc_formats::mgcr::PortInput;
use mgc_sim::Hx;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// Bumped whenever [`Msg`] changes shape.
const PROTOCOL: u32 = 7;
/// The most players a session holds — retail's 8 (one per wizard). The
/// admin lowers it in the lobby (`Guest::set_capacity`).
pub const MAX_PLAYERS: usize = 8;
/// Ticks between sampling a seat's input and stepping it (24 Hz: 3
/// ticks = 125 ms of control latency, ample on loopback/LAN/a tailnet).
pub const INPUT_DELAY: u64 = 3;
/// The server stamps its state hash on every this-many-th tick (1 s).
pub const CHECK_EVERY: u64 = 24;
/// After the verdict, the longest the server waits for the players'
/// fade-out before it reopens the lobby anyway.
pub const GAME_OVER_GRACE: Duration = Duration::from_secs(10);
/// A player the game has waited on this long — connected, but no input
/// (his app hung) — is dropped like a lost connection (player
/// 2026-10-17: slowdowns are accepted, a frozen player is not). Covers
/// the slowest machine's level load at the game's start.
pub const FROZEN_AFTER: Duration = Duration::from_secs(30);
/// A frame larger than this is a broken peer, not a message (a resync
/// snapshot is the largest: ~0.6 MiB of world, compressed).
const MAX_FRAME: usize = 16 << 20;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
enum Msg {
    /// Client → server: who I am (the campaign's player name, maybe
    /// empty), the game I play (none: a CLI player adopting the
    /// server's), the session's passphrase, and — the player a hosted
    /// server was started for — the admin key.
    Hello {
        protocol: u32,
        build: String,
        passphrase: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        game: Option<NetGame>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        admin_key: String,
    },
    Refuse {
        why: String,
    },
    /// Server → client: the lobby as that client sees it, on every
    /// change (and after a game: the lobby reopened).
    Lobby {
        view: LobbyView,
    },
    /// Server → client: something the lobby should hear (a Play the
    /// server could not start).
    Notice {
        text: String,
    },
    /// Client → server: take this wizard (first come, first served).
    Wizard {
        want: u8,
    },
    /// Client → server: I agree to rules revision `rev`.
    Agree {
        rev: u32,
    },
    /// Client → server: I take my agreement back.
    Withdraw,
    /// Client → server: my flight models (on joining, and on a change).
    Models {
        models: serde_json::Value,
    },
    /// Admin → server: the lobby's settings.
    SetLevel {
        level: String,
    },
    SetAi {
        ai: u8,
    },
    SetRules {
        rules: serde_json::Value,
    },
    SetCapacity {
        capacity: u8,
    },
    SetPassphrase {
        passphrase: String,
    },
    Kick {
        id: u32,
    },
    Play,
    /// Server → client: the game begins. `seats[k]` is human seat
    /// `Hx(k)`'s wizard, `local` the receiver's.
    Start {
        seats: Vec<u8>,
        names: Vec<String>,
        models: Vec<serde_json::Value>,
        ai: u8,
        local: u8,
        level: String,
        rules: serde_json::Value,
    },
    /// Client → server: I leave this game, but stay in the lobby (and
    /// wait for the game to finish). After the verdict: my fade-out is
    /// over.
    Leave,
    /// Client → server: the client seat's input for `tick`.
    Input {
        tick: u64,
        input: PortInput,
    },
    /// Server → client: every seat's input for `tick`, in seat order,
    /// plus (now and then) the server's pre-step hash of a tick.
    Turn {
        tick: u64,
        inputs: Vec<PortInput>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check: Option<(u64, u64)>,
        /// The human seats that left the game AT this tick (banished
        /// before it steps).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        left: Vec<u8>,
        /// The game is decided: the winner's announcement.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        over: Option<String>,
    },
    /// Client → server: my world disagreed with yours at `tick` —
    /// send me yours.
    Resync {
        tick: u64,
    },
    /// Server → client: its world before stepping `tick` (sent ahead of
    /// that turn): the net snapshot, zstd + base64, and its state hash.
    Snapshot {
        tick: u64,
        hash: u64,
        data: String,
    },
    Bye {
        why: String,
    },
}

fn write_msg(w: &mut impl Write, m: &Msg) -> std::io::Result<()> {
    let body = serde_json::to_vec(m).map_err(std::io::Error::other)?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    w.write_all(&frame)
}

fn read_msg(r: &mut impl Read) -> std::io::Result<Msg> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other(format!("{len}-byte frame")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}

/// `MGC_NET_TRACE=1`: print every hash check (server and clients).
fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MGC_NET_TRACE").is_some_and(|v| v != "0"))
}

/// This binary's identity: peers must run the SAME build (a different
/// build is a refusal at the handshake, never a desync mid-game).
pub fn build_id() -> String {
    let exe = std::env::current_exe()
        .and_then(std::fs::read)
        .unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in exe {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}+{h:016x}", env!("CARGO_PKG_VERSION"))
}

/// A fresh admin key: unguessable enough to keep a stranger from
/// claiming a hosted server's admin role (it never leaves the app).
pub fn admin_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    let a = h.finish();
    let b = std::collections::hash_map::RandomState::new().hash_one(a);
    format!("{a:016x}{b:016x}")
}

/// What one non-blocking receive found.
enum Recv {
    Msg(Msg),
    Empty,
    Lost(String),
}

/// One TCP peer: the write half, and a reader thread feeding a channel
/// — alive for the whole session, lobby and games alike. Dropping an
/// open one says goodbye.
struct Conn {
    tx: TcpStream,
    rx: mpsc::Receiver<std::io::Result<Msg>>,
    peer: String,
    since: Instant,
    open: bool,
}

impl Conn {
    fn new(stream: TcpStream, peer: String) -> Self {
        let _ = stream.set_nodelay(true);
        let (tx_msgs, rx) = mpsc::channel();
        let mut reader = stream.try_clone().expect("a TCP stream clones its handle");
        std::thread::Builder::new()
            .name("net-rx".into())
            .spawn(move || {
                loop {
                    let m = read_msg(&mut reader);
                    let stop = m.is_err();
                    if tx_msgs.send(m).is_err() || stop {
                        break;
                    }
                }
            })
            .expect("spawn the net reader");
        Self {
            tx: stream,
            rx,
            peer,
            since: Instant::now(),
            open: true,
        }
    }

    fn send(&mut self, m: &Msg) -> std::io::Result<()> {
        write_msg(&mut self.tx, m)
    }

    fn recv(&self) -> Recv {
        match self.rx.try_recv() {
            Ok(Ok(m)) => Recv::Msg(m),
            Ok(Err(e)) => Recv::Lost(e.to_string()),
            Err(mpsc::TryRecvError::Empty) => Recv::Empty,
            Err(mpsc::TryRecvError::Disconnected) => Recv::Lost("connection lost".into()),
        }
    }

    /// Say `why` and hang up.
    fn close(&mut self, why: &str) {
        if self.open {
            self.open = false;
            let _ = self.send(&Msg::Bye { why: why.into() });
            let _ = self.tx.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.close("left");
    }
}

/// How long a fresh connection may take to say hello.
const HELLO_WITHIN: Duration = Duration::from_secs(10);

/// One game as the lobby agreed it — what every peer builds its world
/// from (the server's [`Referee`] included).
#[derive(Clone, Debug)]
pub struct Game {
    /// Wizard seat per human seat, in seat order (`Hx(k)`).
    pub seats: Vec<u8>,
    /// Each human's campaign name, seat order (empty: none given).
    pub names: Vec<String>,
    /// Each human's flight models (`controls.models`), seat order.
    pub models: Vec<serde_json::Value>,
    /// The AI wizards to seat (the admin's rule).
    pub ai: u8,
    pub level: String,
    /// The session rules (`rules::collect` of a config).
    pub rules: serde_json::Value,
}

/// The server's own world — the session's authority. The app builds
/// it the way every player builds his (`load_level`), headless; the
/// socket side never sees a world.
pub trait Referee {
    /// Build the world for `game` — refused, the game does not start.
    fn begin(&mut self, game: &Game) -> Result<(), String>;
    /// The pre-step state hash (asked on check ticks only).
    fn hash(&self) -> u64;
    /// The pre-step world, for a player's resync
    /// (`Simulation::net_snapshot`).
    fn snapshot(&self) -> Vec<u8>;
    /// Banish the seats in `left`, then step the turn.
    fn step(&mut self, inputs: &[PortInput], left: &[u8]);
    /// The game is decided: the winner's announcement.
    fn verdict(&self) -> Option<String>;
    /// The game is over: drop the world.
    fn end(&mut self);
    /// The wizard seats `level` authors a start for (bit k = seat k) —
    /// the lobby holds its members to them ([`LobbyCore::set_open`]).
    fn open_seats(&self, _level: &str) -> u8 {
        u8::MAX
    }
}

/// What a hosted server's app wants to know about it (its close
/// warning).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerStatus {
    /// Members in the session.
    pub members: usize,
    /// Seats still playing a running game.
    pub playing: usize,
}

/// The server's settings at its start.
pub struct ServerOpts {
    /// The session's game, for the server's life: a player of another
    /// is refused at the door.
    pub game: NetGame,
    pub passphrase: String,
    pub build: String,
    /// The first level up — one of `game`'s (the admin changes it,
    /// within the game).
    pub level: String,
    /// The session's rules (the admin changes them).
    pub rules: serde_json::Value,
    /// A hosted server: the key its own player claims the admin role
    /// with. `None` (dedicated): the first member in is the admin.
    pub admin_key: Option<String>,
    /// How long the game waits on a silent player ([`FROZEN_AFTER`]).
    pub frozen_after: Duration,
}

/// A running game on the server: turn assembly, departures, verdict.
struct Match {
    /// Member id per human seat (`Hx` order).
    ids: Vec<u32>,
    /// The next turn to complete, and partial turns by tick.
    next_complete: u64,
    pending: BTreeMap<u64, Vec<Option<PortInput>>>,
    /// Per seat, the tick it left at, and whether it is banished there
    /// (its input from then on is the server's: the suicide key at that
    /// tick when banished, idle after).
    departed: Vec<Option<(u64, bool)>>,
    /// The seats banished at each tick (a turn's `left`).
    departures: BTreeMap<u64, Vec<u8>>,
    /// The verdict, once the world named a winner — and since when.
    over: Option<(String, Instant)>,
    /// The verdict still to ride a turn.
    announce: Option<String>,
    /// Hash checks stamped (the summary).
    checks: u64,
    /// Per seat, when its last input arrived (the game's start before
    /// the first).
    heard: Vec<Instant>,
    /// The seats that asked for a resync: the server's world goes out
    /// to them ahead of the next turn.
    resync: Vec<usize>,
    /// Resync snapshots sent (the summary).
    resyncs: u64,
}

impl Match {
    fn seat_of(&self, id: u32) -> Option<usize> {
        self.ids.iter().position(|&m| m == id)
    }

    /// Is seat `ix` still playing?
    fn active(&self, ix: usize) -> bool {
        self.departed.get(ix).is_some_and(Option::is_none)
    }

    fn playing(&self) -> usize {
        self.departed.iter().filter(|d| d.is_none()).count()
    }
}

/// The SERVER: listens, admits clients that pass the handshake
/// (protocol, build, passphrase), serves their lobby requests against
/// the [`LobbyCore`], runs the games. Non-blocking — [`Self::poll`] it
/// in a loop ([`Self::run`]).
pub struct Server {
    core: LobbyCore,
    /// The session's game.
    plays: NetGame,
    passphrase: String,
    build: String,
    admin_key: Option<String>,
    listener: TcpListener,
    /// Connected, not yet said hello.
    pending: Vec<Conn>,
    /// Admitted members, in join order.
    clients: Vec<(u32, Conn)>,
    referee: Box<dyn Referee>,
    game: Option<Match>,
    frozen_after: Duration,
    dirty: bool,
}

impl Server {
    /// Open a session on `listener` (a bound socket).
    pub fn new(listener: TcpListener, opts: ServerOpts, referee: Box<dyn Referee>) -> Self {
        let _ = listener.set_nonblocking(true);
        let mut core = LobbyCore::new(opts.game, opts.level.clone(), opts.rules, MAX_PLAYERS);
        core.set_open(referee.open_seats(&opts.level));
        Self {
            core,
            plays: opts.game,
            passphrase: opts.passphrase,
            build: opts.build,
            admin_key: opts.admin_key.filter(|k| !k.is_empty()),
            listener,
            pending: Vec::new(),
            clients: Vec::new(),
            referee,
            game: None,
            frozen_after: opts.frozen_after,
            dirty: false,
        }
    }

    /// Bind `addr` for a server.
    pub fn bind(addr: &str) -> Result<TcpListener, String> {
        TcpListener::bind(addr).map_err(|e| format!("net: bind {addr}: {e}"))
    }

    /// Poll until `stop`; then every member hears the server is gone.
    pub fn run(mut self, stop: &AtomicBool, status: Option<&Mutex<ServerStatus>>) {
        while !stop.load(Ordering::Relaxed) {
            self.poll();
            if let Some(s) = status
                && let Ok(mut s) = s.lock()
            {
                *s = self.status();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        for (_, c) in &mut self.clients {
            c.close("the server stopped");
        }
        println!("server: stopped");
    }

    pub fn status(&self) -> ServerStatus {
        ServerStatus {
            members: self.clients.len(),
            playing: self.game.as_ref().map_or(0, Match::playing),
        }
    }

    pub fn poll(&mut self) {
        self.accept();
        self.handshake();
        self.serve();
        self.drop_frozen();
        self.referee_turns();
        self.flush();
    }

    fn note(&self, what: impl AsRef<str>) {
        println!("server: {}", what.as_ref());
    }

    fn accept(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    let _ = stream.set_nonblocking(false);
                    self.pending.push(Conn::new(stream, peer.to_string()));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(e) => {
                    self.note(format!("accept: {e}"));
                    return;
                }
            }
        }
    }

    fn refuse(&self, mut conn: Conn, why: String) {
        self.note(format!("refused {}: {why}", conn.peer));
        let _ = conn.send(&Msg::Refuse { why: why.clone() });
        conn.close(&why);
    }

    fn handshake(&mut self) {
        for conn in std::mem::take(&mut self.pending) {
            match conn.recv() {
                Recv::Msg(Msg::Hello {
                    protocol,
                    build,
                    passphrase,
                    name,
                    game,
                    admin_key,
                }) => {
                    let refusal = if protocol != PROTOCOL {
                        Some(format!("protocol {protocol}, the server speaks {PROTOCOL}"))
                    } else if game.is_some_and(|g| g != self.plays) {
                        Some(format!("this server runs {}", self.plays.title()))
                    } else if build != self.build {
                        Some(format!("build {build}, the server runs {}", self.build))
                    } else if !self.passphrase.is_empty() && passphrase != self.passphrase {
                        // An empty passphrase = an open session: anyone
                        // gets in (player ruling 2026-10-17).
                        Some("wrong passphrase".into())
                    } else {
                        None
                    };
                    match refusal.map_or_else(|| self.core.join(name), Err) {
                        Ok(id) => {
                            if self.admin_key.as_deref() == Some(admin_key.as_str()) {
                                self.core.claim_admin(id);
                            }
                            let w = self.core.view_for(id).me().map_or(0, |m| m.wizard);
                            let admin = if self.core.is_admin(id) {
                                " (admin)"
                            } else {
                                ""
                            };
                            self.note(format!("{} joined as wizard {}{admin}", conn.peer, w + 1));
                            self.clients.push((id, conn));
                            self.dirty = true;
                        }
                        Err(why) => self.refuse(conn, why),
                    }
                }
                Recv::Msg(other) => self.refuse(conn, format!("expected hello, got {other:?}")),
                Recv::Empty if conn.since.elapsed() < HELLO_WITHIN => self.pending.push(conn),
                Recv::Empty => self.refuse(conn, "no hello".into()),
                Recv::Lost(_) => {}
            }
        }
    }

    fn serve(&mut self) {
        let mut events: Vec<(u32, Result<Msg, String>)> = Vec::new();
        for (id, conn) in &self.clients {
            loop {
                match conn.recv() {
                    Recv::Msg(m) => events.push((*id, Ok(m))),
                    Recv::Lost(e) => {
                        events.push((*id, Err(e)));
                        break;
                    }
                    Recv::Empty => break,
                }
            }
        }
        for (id, ev) in events {
            if self.clients.iter().any(|(c, _)| *c == id) {
                self.handle(id, ev);
            }
        }
    }

    fn handle(&mut self, id: u32, ev: Result<Msg, String>) {
        let admin = self.core.is_admin(id);
        match ev {
            Err(e) => self.drop_client(id, &e),
            Ok(Msg::Bye { why }) => self.drop_client(id, &why),
            Ok(Msg::Input { tick, input }) => self.input(id, tick, input),
            Ok(Msg::Resync { tick }) => self.resync(id, tick),
            Ok(Msg::Leave) => self.leave_game(id),
            Ok(Msg::Wizard { want }) => self.dirty |= self.core.pick_wizard(id, want),
            Ok(Msg::Agree { rev }) => self.dirty |= self.core.agree(id, rev),
            Ok(Msg::Withdraw) => self.dirty |= self.core.withdraw(id),
            Ok(Msg::Models { models }) => self.dirty |= self.core.set_models(id, models),
            // The admin's powers; anyone else's request is ignored.
            Ok(
                Msg::SetLevel { .. }
                | Msg::SetAi { .. }
                | Msg::SetRules { .. }
                | Msg::SetCapacity { .. }
                | Msg::SetPassphrase { .. }
                | Msg::Kick { .. }
                | Msg::Play,
            ) if !admin => {}
            Ok(Msg::SetLevel { level }) => {
                if NetGame::of_level(&level) != Some(self.plays) {
                    self.note(format!("refused level {level}: not {}", self.plays.title()));
                }
                let open = self.referee.open_seats(&level);
                if self.core.set_level(level) {
                    self.core.set_open(open);
                    self.dirty = true;
                }
            }
            Ok(Msg::SetAi { ai }) => self.dirty |= self.core.set_ai(ai),
            Ok(Msg::SetRules { rules }) => self.dirty |= self.core.set_rules(rules),
            Ok(Msg::SetCapacity { capacity }) => {
                self.dirty |= self.core.set_capacity(usize::from(capacity));
            }
            Ok(Msg::SetPassphrase { passphrase }) => {
                // Later joiners need the new one (a kicked player can't
                // come back); members already in stay.
                self.note("the passphrase changed");
                self.passphrase = passphrase;
                self.dirty = true;
            }
            Ok(Msg::Kick { id: who }) if who != id => {
                self.note(format!("member {who} kicked"));
                self.drop_client(who, "kicked by the admin");
            }
            Ok(Msg::Kick { .. }) => {}
            Ok(Msg::Play) => self.play(),
            Ok(other) => self.drop_client(id, &format!("sent {other:?}")),
        }
    }

    /// Member `id` is gone (hung up, lost, kicked): out of the lobby,
    /// and — in a game — banished like any departure.
    fn drop_client(&mut self, id: u32, why: &str) {
        if let Some(ix) = self.game.as_ref().and_then(|g| g.seat_of(id)) {
            self.depart(ix, why);
        }
        if let Some(i) = self.clients.iter().position(|(c, _)| *c == id) {
            let (_, mut conn) = self.clients.remove(i);
            conn.close(why);
            self.note(format!("member {id} left ({why})"));
        }
        self.dirty |= self.core.leave(id);
    }

    /// Member `id` leaves the game, not the session: he waits in the
    /// lobby for it to finish.
    fn leave_game(&mut self, id: u32) {
        if let Some(ix) = self.game.as_ref().and_then(|g| g.seat_of(id)) {
            self.depart(ix, "left the game");
            // He waits — he hears the lobby from now on.
            self.dirty = true;
        }
    }

    /// Seat `ix` left the game: it is out at the first turn not yet
    /// sent — whatever it had scheduled from there on is dropped, and
    /// its input becomes the server's. Before the verdict it is
    /// banished; after it, it only stops playing (his fade is over).
    fn depart(&mut self, ix: usize, why: &str) {
        let Some(g) = &mut self.game else { return };
        if !g.active(ix) {
            return;
        }
        let tick = g.next_complete;
        let banish = g.over.is_none();
        g.departed[ix] = Some((tick, banish));
        if banish {
            g.departures.entry(tick).or_default().push(ix as u8);
        }
        for turn in g.pending.range_mut(tick..).map(|(_, t)| t) {
            turn[ix] = None;
        }
        if banish {
            self.note(format!("seat {ix} left ({why}) — banished at tick {tick}"));
        }
        self.complete();
    }

    fn input(&mut self, id: u32, tick: u64, input: PortInput) {
        let Some(g) = &mut self.game else { return };
        let Some(ix) = g.seat_of(id).filter(|&ix| g.active(ix)) else {
            // A game's tail (inputs sent before he left it).
            return;
        };
        if tick < g.next_complete {
            let why = format!("input for tick {tick} arrived after its turn");
            self.drop_client(id, &why);
            return;
        }
        let n = g.ids.len();
        g.heard[ix] = Instant::now();
        g.pending.entry(tick).or_insert_with(|| vec![None; n])[ix] = Some(input);
        self.complete();
    }

    /// Member `id`'s world disagreed at `tick`: his seat gets the
    /// server's world ahead of the next turn.
    fn resync(&mut self, id: u32, tick: u64) {
        let Some(g) = &mut self.game else { return };
        let Some(ix) = g.seat_of(id).filter(|&ix| g.active(ix)) else {
            return;
        };
        if !g.resync.contains(&ix) {
            g.resync.push(ix);
        }
        self.note(format!("seat {ix} desynced at tick {tick} — resync asked"));
    }

    /// Every turn whose seats have all reported, in tick order: the
    /// server's world steps it, and it goes out to the players.
    fn complete(&mut self) {
        let Some(g) = &mut self.game else { return };
        while let Some(turn) = g.pending.get_mut(&g.next_complete) {
            let tick = g.next_complete;
            // A departed seat's input is the server's to write.
            for (ix, d) in g.departed.iter().enumerate() {
                if let Some((at, banish)) = *d
                    && tick >= at
                    && turn[ix].is_none()
                {
                    turn[ix] = Some(PortInput {
                        suicide: banish && tick == at,
                        ..PortInput::default()
                    });
                }
            }
            if turn.iter().any(Option::is_none) {
                break;
            }
            let inputs: Vec<PortInput> = g
                .pending
                .remove(&tick)
                .into_iter()
                .flatten()
                .flatten()
                .collect();
            g.next_complete += 1;
            let check = (tick % CHECK_EVERY == 0).then(|| (tick, self.referee.hash()));
            if let Some((t, h)) = check {
                g.checks += 1;
                if trace() {
                    println!("server: tick {t} hash {h:016x}");
                }
            }
            // A resync: the world before this turn, ahead of it.
            if !g.resync.is_empty() {
                let hash = self.referee.hash();
                let data = encode_snapshot(&self.referee.snapshot());
                let kib = data.len() / 1024;
                let snap = Msg::Snapshot { tick, hash, data };
                for ix in std::mem::take(&mut g.resync) {
                    let id = g.ids[ix];
                    if let Some((_, c)) = self.clients.iter_mut().find(|(c, _)| *c == id)
                        && let Err(e) = c.send(&snap)
                    {
                        eprintln!("server: send to {}: {e}", c.peer);
                        let _ = c.tx.shutdown(Shutdown::Both);
                    }
                    g.resyncs += 1;
                    println!("server: seat {ix} resynced at tick {tick} ({kib} KiB)");
                }
            }
            let left = g.departures.remove(&tick).unwrap_or_default();
            let over = g.announce.take();
            self.referee.step(&inputs, &left);
            if g.over.is_none()
                && let Some(v) = self.referee.verdict()
            {
                println!("server: the game is over — {v}");
                g.over = Some((v.clone(), Instant::now()));
                g.announce = Some(v);
            }
            let turn = Msg::Turn {
                tick,
                inputs,
                check,
                left,
                over,
            };
            for (id, c) in &mut self.clients {
                if g.seat_of(*id).is_some_and(|ix| g.active(ix))
                    && let Err(e) = c.send(&turn)
                {
                    // He leaves at the next poll (a Lost follows).
                    eprintln!("server: send to {}: {e}", c.peer);
                    let _ = c.tx.shutdown(Shutdown::Both);
                }
            }
        }
    }

    /// Drop every player the game has been waiting on for
    /// `frozen_after`: still playing, silent that long, and missing
    /// from the next turn (the others ran ahead and wait on him — they
    /// are silent too, but not missing).
    fn drop_frozen(&mut self) {
        let Some(g) = &self.game else { return };
        let next = g.pending.get(&g.next_complete);
        let frozen: Vec<u32> = (0..g.ids.len())
            .filter(|&ix| {
                g.active(ix)
                    && g.heard[ix].elapsed() > self.frozen_after
                    && next.is_none_or(|turn| turn[ix].is_none())
            })
            .map(|ix| g.ids[ix])
            .collect();
        let why = format!("no input for {} s", self.frozen_after.as_secs());
        for id in frozen {
            self.drop_client(id, &why);
        }
    }

    /// The running game's end: nobody plays any more, or the verdict's
    /// grace ran out — the lobby reopens for everyone.
    fn referee_turns(&mut self) {
        let Some(g) = &self.game else { return };
        let done = g.playing() == 0
            || g.over
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() > GAME_OVER_GRACE);
        if !done {
            return;
        }
        let g = self.game.take().expect("a game");
        self.note(format!(
            "game over after {} ticks, {} hash checks stamped, {} resyncs",
            g.next_complete, g.checks, g.resyncs
        ));
        self.referee.end();
        self.core.reopen();
        self.dirty = true;
    }

    /// Play (the admin's): the server builds the world, every member
    /// gets his seat and the agreed session, and the lobby waits.
    fn play(&mut self) {
        if self.game.is_some() || !self.core.can_play() {
            return;
        }
        let view = self.core.view_for(0);
        let game = Game {
            seats: view.seats(),
            names: view.names(),
            models: view.models(),
            ai: view.ai,
            level: view.level.clone(),
            rules: view.rules.clone(),
        };
        if let Err(e) = self.referee.begin(&game) {
            let text = format!("the server could not start the game: {e}");
            self.note(&text);
            for (_, c) in &mut self.clients {
                let _ = c.send(&Msg::Notice { text: text.clone() });
            }
            return;
        }
        let ids: Vec<u32> = view.members.iter().map(|m| m.id).collect();
        let mut lost = Vec::new();
        for (id, conn) in &mut self.clients {
            let Some(local) = ids.iter().position(|m| m == id) else {
                continue;
            };
            let start = Msg::Start {
                seats: game.seats.clone(),
                names: game.names.clone(),
                models: game.models.clone(),
                ai: game.ai,
                local: local as u8,
                level: game.level.clone(),
                rules: game.rules.clone(),
            };
            if let Err(e) = conn.send(&start) {
                lost.push((*id, format!("{}: {e}", conn.peer)));
            }
        }
        self.note(format!(
            "the game begins — {} players, level {}",
            ids.len(),
            game.level
        ));
        let n = ids.len();
        self.game = Some(Match {
            ids,
            next_complete: 0,
            pending: BTreeMap::new(),
            departed: vec![None; n],
            departures: BTreeMap::new(),
            over: None,
            announce: None,
            checks: 0,
            heard: vec![Instant::now(); n],
            resync: Vec::new(),
            resyncs: 0,
        });
        self.core.begin();
        // A player lost while the game starts leaves at its first tick,
        // like any departure; the rest play on.
        for (id, why) in lost {
            self.drop_client(id, &why);
        }
    }

    /// Send every member who is not in the running game the lobby (a
    /// lobby view ends a player's game).
    fn flush(&mut self) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let mut lost = Vec::new();
        for (id, conn) in &mut self.clients {
            if self
                .game
                .as_ref()
                .is_some_and(|g| g.seat_of(*id).is_some_and(|ix| g.active(ix)))
            {
                continue;
            }
            let mut view = self.core.view_for(*id);
            if view.is_admin() {
                view.passphrase = Some(self.passphrase.clone());
            }
            if let Err(e) = conn.send(&Msg::Lobby { view }) {
                lost.push((*id, e.to_string()));
            }
        }
        for (id, why) in lost {
            self.drop_client(id, &why);
        }
        if self.dirty {
            self.flush();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        for (_, c) in &mut self.clients {
            c.close("the server stopped");
        }
    }
}

/// A server on a thread of this app (the UI's Host, `--host`): it runs
/// as long as the handle lives — dropping it stops the server, and
/// every member hears so.
pub struct ServerHandle {
    addr: SocketAddr,
    admin_key: String,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<ServerStatus>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerHandle {
    /// Bind `addr` and start the server, its world built by the
    /// referee `make` returns (on the server's thread). `opts.admin_key`
    /// is set here: the handle's own player claims the admin role.
    pub fn spawn(
        addr: &str,
        mut opts: ServerOpts,
        make: impl FnOnce() -> Box<dyn Referee> + Send + 'static,
    ) -> Result<Self, String> {
        let listener = Server::bind(addr)?;
        let bound = listener
            .local_addr()
            .map_err(|e| format!("net: {addr}: {e}"))?;
        let key = opts.admin_key.get_or_insert_with(admin_key).clone();
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(ServerStatus::default()));
        let (s, st) = (stop.clone(), status.clone());
        let thread = std::thread::Builder::new()
            .name("net-server".into())
            .spawn(move || Server::new(listener, opts, make()).run(&s, Some(&st)))
            .map_err(|e| format!("net: server thread: {e}"))?;
        println!("server: listening on {bound}");
        Ok(Self {
            addr: bound,
            admin_key: key,
            stop,
            status,
            thread: Some(thread),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The address this app's own player joins by (loopback when the
    /// server listens everywhere).
    pub fn local_addr(&self) -> String {
        let ip = match self.addr.ip() {
            std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
            }
            std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
                std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
            }
            ip => ip,
        };
        SocketAddr::new(ip, self.addr.port()).to_string()
    }

    pub fn admin_key(&self) -> &str {
        &self.admin_key
    }

    pub fn status(&self) -> ServerStatus {
        self.status.lock().map(|s| *s).unwrap_or_default()
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A Start as received (seats, names, models, ai, local, level, rules).
type Start = (
    Vec<u8>,
    Vec<String>,
    Vec<serde_json::Value>,
    u8,
    u8,
    String,
    serde_json::Value,
);

/// A player's end of the session: the server's latest lobby view, our
/// requests (the admin's too), and the Start that turns it into a game.
pub struct Guest {
    conn: Conn,
    view: LobbyView,
    start: Option<Start>,
    /// The flight models last sent.
    models_sent: Option<serde_json::Value>,
    notices: Vec<String>,
    closed: Option<String>,
}

impl Guest {
    /// Connect to `addr` (retrying for up to `wait` while the server
    /// comes up) and pass the handshake — the first lobby view, or the
    /// server's refusal. `game`: the game this player plays — a server
    /// of another refuses him (`None`: adopt the server's, the CLI's
    /// way). `admin_key`: a hosted server's key (its own player); empty
    /// for everyone else.
    #[allow(clippy::too_many_arguments)]
    pub fn connect(
        addr: &str,
        passphrase: &str,
        name: &str,
        game: Option<NetGame>,
        build: &str,
        admin_key: &str,
        wait: Duration,
    ) -> Result<Self, String> {
        let deadline = Instant::now() + wait;
        let mut said = false;
        let stream = loop {
            match TcpStream::connect(addr) {
                Ok(s) => break s,
                Err(e) if Instant::now() < deadline => {
                    if !said {
                        println!("net: waiting for a server at {addr} ({e})");
                        said = true;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(e) => return Err(format!("net: connect {addr}: {e}")),
            }
        };
        let mut conn = Conn::new(stream, addr.to_string());
        conn.send(&Msg::Hello {
            protocol: PROTOCOL,
            build: build.to_string(),
            passphrase: passphrase.to_string(),
            name: name.to_string(),
            game,
            admin_key: admin_key.to_string(),
        })
        .map_err(|e| format!("net: {addr}: {e}"))?;
        let deadline = Instant::now() + HELLO_WITHIN;
        loop {
            match conn.recv() {
                Recv::Msg(Msg::Lobby { view }) => {
                    return Ok(Self {
                        conn,
                        view,
                        start: None,
                        models_sent: None,
                        notices: Vec::new(),
                        closed: None,
                    });
                }
                Recv::Msg(Msg::Refuse { why }) => {
                    conn.open = false;
                    return Err(format!("net: the server refused: {why}"));
                }
                Recv::Msg(other) => return Err(format!("net: expected the lobby, got {other:?}")),
                Recv::Lost(e) => return Err(format!("net: {addr}: {e}")),
                Recv::Empty if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Recv::Empty => return Err(format!("net: {addr}: no answer from the server")),
            }
        }
    }

    pub fn view(&self) -> &LobbyView {
        &self.view
    }

    /// Why the lobby ended for us (kicked, the server stopped, lost).
    pub fn closed(&self) -> Option<&str> {
        self.closed.as_deref()
    }

    pub fn started(&self) -> bool {
        self.start.is_some()
    }

    /// What the server told the lobby since the last call.
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    pub fn poll(&mut self) {
        while self.closed.is_none() && self.start.is_none() {
            match self.conn.recv() {
                Recv::Msg(Msg::Lobby { view }) => self.view = view,
                Recv::Msg(Msg::Notice { text }) => self.notices.push(text),
                Recv::Msg(Msg::Start {
                    seats,
                    names,
                    models,
                    ai,
                    local,
                    level,
                    rules,
                }) => self.start = Some((seats, names, models, ai, local, level, rules)),
                // A game's tail (turns sent before we left it).
                Recv::Msg(Msg::Turn { .. }) => {}
                Recv::Msg(Msg::Bye { why }) => {
                    self.conn.open = false;
                    self.closed = Some(why);
                }
                Recv::Msg(other) => {
                    let why = format!("the server sent {other:?}");
                    self.conn.close(&why);
                    self.closed = Some(why);
                }
                Recv::Lost(e) => self.closed = Some(e),
                Recv::Empty => return,
            }
        }
    }

    fn send(&mut self, m: &Msg) {
        if self.closed.is_none()
            && let Err(e) = self.conn.send(m)
        {
            self.closed = Some(e.to_string());
        }
    }

    /// Ask for `wizard` (the server decides; the next view tells).
    pub fn pick_wizard(&mut self, wizard: u8) {
        self.send(&Msg::Wizard { want: wizard });
    }

    /// Agree to the rules as currently shown.
    pub fn agree(&mut self) {
        let rev = self.view.rev;
        self.send(&Msg::Agree { rev });
    }

    pub fn withdraw(&mut self) {
        self.send(&Msg::Withdraw);
    }

    /// Tell the server our flight models (only when they changed).
    pub fn set_models(&mut self, models: serde_json::Value) {
        if self.models_sent.as_ref() != Some(&models) {
            self.send(&Msg::Models {
                models: models.clone(),
            });
            self.models_sent = Some(models);
        }
    }

    /// Admin: change the level — voids every agreement.
    pub fn set_level(&mut self, level: String) {
        self.send(&Msg::SetLevel { level });
    }

    /// Admin: change the AI wizard count — a rule.
    pub fn set_ai(&mut self, ai: u8) {
        self.send(&Msg::SetAi { ai });
    }

    /// Admin: change the rules — voids every agreement.
    pub fn set_rules(&mut self, rules: serde_json::Value) {
        self.send(&Msg::SetRules { rules });
    }

    /// Admin: the player limit (2 up to [`MAX_PLAYERS`], never below
    /// the members in).
    pub fn set_capacity(&mut self, capacity: usize) {
        let capacity = capacity.min(MAX_PLAYERS) as u8;
        self.send(&Msg::SetCapacity { capacity });
    }

    /// Admin: a new passphrase for later joiners.
    pub fn set_passphrase(&mut self, passphrase: String) {
        self.send(&Msg::SetPassphrase { passphrase });
    }

    /// Admin: show member `id` the door.
    pub fn kick(&mut self, id: u32) {
        self.send(&Msg::Kick { id });
    }

    /// Admin: start the game (the server refuses unless everyone
    /// agreed).
    pub fn play(&mut self) {
        self.send(&Msg::Play);
    }

    /// The game the server started (the guest handed back before
    /// [`Self::started`]).
    pub fn into_link(self) -> Result<Link, Box<Self>> {
        let Some((seats, names, models, ai, local, level, rules)) = self.start.clone() else {
            return Err(Box::new(self));
        };
        let mut guest = self;
        guest.start = None;
        Ok(Link {
            guest,
            game: Game {
                seats,
                names,
                models,
                ai,
                level,
                rules,
            },
            local: Hx(local),
        })
    }
}

/// A started game: the agreed session, before any world.
pub struct Link {
    guest: Guest,
    pub game: Game,
    /// This app's human seat.
    pub local: Hx,
}

/// CLI server (`--serve`): run in the foreground until killed.
pub fn serve(addr: &str, opts: ServerOpts, referee: Box<dyn Referee>) -> Result<(), String> {
    let listener = Server::bind(addr)?;
    let bound = listener
        .local_addr()
        .map_or_else(|_| addr.to_string(), |a| a.to_string());
    println!(
        "server: {} on {bound} — the first player in is the admin",
        opts.game.title()
    );
    let stop = AtomicBool::new(false);
    Server::new(listener, opts, referee).run(&stop, None);
    Ok(())
}

/// CLI player (`--join`, and `--host`'s own player): connect, take
/// `wizard` if asked (else the server's pick), agree to whatever the
/// admin sets — or, as the admin, press Play once everyone agreed —
/// and wait for the start.
#[allow(clippy::too_many_arguments)]
pub fn join(
    addr: &str,
    game: Option<NetGame>,
    wizard: Option<u8>,
    models: serde_json::Value,
    passphrase: &str,
    build: &str,
    admin_key: &str,
) -> Result<Link, String> {
    let mut g = Guest::connect(
        addr,
        passphrase,
        "",
        game,
        build,
        admin_key,
        Duration::from_secs(60),
    )?;
    g.set_models(models);
    if let Some(w) = wizard {
        if !g.view().is_free(w) && g.view().me().is_none_or(|m| m.wizard != w) {
            return Err(format!("net: seat {} is not free", w + 1));
        }
        g.pick_wizard(w);
    }
    let mut agreed = None;
    let mut pressed = None;
    loop {
        g.poll();
        for n in g.take_notices() {
            println!("net: {n}");
        }
        if let Some(why) = g.closed() {
            return Err(format!("net: the lobby closed: {why}"));
        }
        if g.started() {
            let link = g.into_link().unwrap_or_else(|_| unreachable!("started"));
            println!(
                "net: joined {addr} as seat {} — level {}",
                link.game.seats[usize::from(link.local.0)] + 1,
                link.game.level
            );
            return Ok(link);
        }
        let v = g.view();
        let mine = v.me().map(|m| m.wizard);
        let ready = wizard.is_none_or(|w| mine == Some(w));
        let agreement = (v.rev, v.agreement());
        if v.is_admin() {
            if ready && v.all_agreed() && pressed != Some(agreement) {
                pressed = Some(agreement);
                g.play();
            }
        } else if ready && agreed != Some(v.rev) {
            agreed = Some(v.rev);
            g.agree();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The server's world for a resync: applied before stepping `tick`,
/// its pre-step state hash `hash` (`Simulation::net_snapshot` bytes).
pub struct Snapshot {
    pub tick: u64,
    pub hash: u64,
    pub bytes: Vec<u8>,
}

/// A resync snapshot on the wire: zstd, then base64 (the frame is
/// JSON).
fn encode_snapshot(bytes: &[u8]) -> String {
    use base64::Engine as _;
    let packed = zstd::bulk::compress(bytes, 3).expect("zstd compresses in memory");
    base64::engine::general_purpose::STANDARD.encode(packed)
}

fn decode_snapshot(data: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let packed = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| e.to_string())?;
    zstd::stream::decode_all(&packed[..]).map_err(|e| e.to_string())
}

/// A player's running lockstep: input out, turns in, the hash check
/// against the server. Non-blocking — the app polls it once per sim
/// tick. Runs on the session's connection and hands it back at the
/// end ([`Self::finish`]): the lobby comes back with the same players.
pub struct Lockstep {
    seats: usize,
    /// The agreed session rules.
    config: serde_json::Value,
    /// `None` once handed back by [`Self::finish`].
    guest: Option<Guest>,
    /// The next tick to step.
    next_tick: u64,
    /// The next tick this seat's input is scheduled for.
    next_send: u64,
    /// Turns not yet stepped.
    ready: BTreeMap<u64, Vec<PortInput>>,
    /// Hashes awaiting their counterpart.
    ours: BTreeMap<u64, u64>,
    theirs: BTreeMap<u64, u64>,
    /// A desync or resync to tell (the toast).
    desync: Option<String>,
    /// Desyncs detected (the summary).
    desyncs: u64,
    /// A resync asked for (at the desynced tick), not yet applied: the
    /// checks wait for it.
    resync_asked: Option<u64>,
    /// The server's world, waiting for its tick.
    snapshot: Option<Snapshot>,
    /// Resyncs that healed.
    resyncs: u64,
    /// A resync that did not heal: the checks stop.
    unhealed: bool,
    /// Hash checks that agreed (the session summary).
    agreed: u64,
    closed: Option<String>,
    /// The server reopened the lobby (the game is over).
    ended: bool,
    /// The seats leaving at each tick (see [`Self::left_at`]).
    departures: BTreeMap<u64, Vec<u8>>,
    /// The verdict, by the tick whose turn carried it …
    overs: BTreeMap<u64, String>,
    /// … once stepped.
    verdict: Option<String>,
}

impl Lockstep {
    pub fn start(link: Link) -> Self {
        Self {
            seats: link.game.seats.len(),
            config: link.game.rules,
            guest: Some(link.guest),
            next_tick: 0,
            next_send: 0,
            ready: BTreeMap::new(),
            ours: BTreeMap::new(),
            theirs: BTreeMap::new(),
            desync: None,
            desyncs: 0,
            resync_asked: None,
            snapshot: None,
            resyncs: 0,
            unhealed: false,
            agreed: 0,
            closed: None,
            ended: false,
            departures: BTreeMap::new(),
            overs: BTreeMap::new(),
            verdict: None,
        }
    }

    pub fn seats(&self) -> usize {
        self.seats
    }

    /// The session's agreed rules.
    pub fn config(&self) -> &serde_json::Value {
        &self.config
    }

    /// Why the session ended, once it has (the server stopped, lost).
    pub fn closed(&self) -> Option<&str> {
        self.closed.as_deref()
    }

    /// The server ended the game and reopened the lobby.
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// Is this seat's input for a further tick due? The first
    /// [`INPUT_DELAY`] ticks are lead-in (see [`Self::in_lead_in`]).
    pub fn wants_input(&self) -> bool {
        self.closed.is_none() && !self.ended && self.next_send < self.next_tick + INPUT_DELAY
    }

    /// The tick about to be scheduled is lead-in: nobody has sampled
    /// a control for it — submit an idle input.
    pub fn in_lead_in(&self) -> bool {
        self.next_send < INPUT_DELAY
    }

    /// Schedule this seat's input for the next due tick.
    pub fn submit(&mut self, input: PortInput) {
        let tick = self.next_send;
        self.next_send += 1;
        self.send(&Msg::Input { tick, input });
    }

    fn send(&mut self, m: &Msg) {
        if self.closed.is_some() {
            return;
        }
        if let Some(g) = &mut self.guest
            && let Err(e) = g.conn.send(m)
        {
            self.closed = Some(format!("send: {e}"));
        }
    }

    /// The human seats that leave the game at `tick` — banish them
    /// before stepping it. Call once per stepped tick.
    pub fn left_at(&mut self, tick: u64) -> Vec<u8> {
        self.departures.remove(&tick).unwrap_or_default()
    }

    /// Drain everything the server has sent.
    pub fn poll(&mut self) {
        if self.closed.is_some() || self.ended {
            return;
        }
        let Some(g) = &mut self.guest else { return };
        let mut checked = false;
        loop {
            match g.conn.recv() {
                Recv::Empty => break,
                Recv::Lost(e) => {
                    self.closed = Some(format!("connection lost: {e}"));
                    break;
                }
                Recv::Msg(Msg::Bye { why }) => {
                    g.conn.open = false;
                    self.closed = Some(format!("the server left ({why})"));
                    break;
                }
                Recv::Msg(Msg::Turn {
                    tick,
                    inputs,
                    check,
                    left,
                    over,
                }) => {
                    if inputs.len() != self.seats {
                        let why = format!("turn {tick} carries {} seats", inputs.len());
                        g.conn.close(&why);
                        self.closed = Some(why);
                        break;
                    }
                    if !left.is_empty() {
                        self.departures.insert(tick, left);
                    }
                    if let Some(v) = over {
                        self.overs.insert(tick, v);
                    }
                    self.ready.insert(tick, inputs);
                    if let Some((t, h)) = check {
                        self.theirs.insert(t, h);
                        checked = true;
                    }
                }
                Recv::Msg(Msg::Snapshot { tick, hash, data }) => match decode_snapshot(&data) {
                    Ok(bytes) => self.snapshot = Some(Snapshot { tick, hash, bytes }),
                    Err(e) => {
                        let why = format!("resync snapshot: {e}");
                        g.conn.close(&why);
                        self.closed = Some(why);
                        break;
                    }
                },
                Recv::Msg(Msg::Lobby { view }) => {
                    g.view = view;
                    self.ended = true;
                    break;
                }
                Recv::Msg(Msg::Notice { text }) => g.notices.push(text),
                Recv::Msg(other) => {
                    let why = format!("unexpected {other:?}");
                    g.conn.close(&why);
                    self.closed = Some(why);
                    break;
                }
            }
        }
        if checked {
            self.compare();
        }
    }

    /// The next tick's bundle (every seat's input, seat order), once
    /// it exists. The returned tick is the lockstep's own counter.
    pub fn take_turn(&mut self) -> Option<(u64, Vec<PortInput>)> {
        let inputs = self.ready.remove(&self.next_tick)?;
        let tick = self.next_tick;
        self.next_tick += 1;
        if let Some(v) = self.overs.remove(&tick) {
            self.verdict = Some(v);
        }
        Some((tick, inputs))
    }

    /// The server's verdict (the winner's announcement), once, when the
    /// turn that carried it has been taken.
    pub fn take_verdict(&mut self) -> Option<String> {
        self.verdict.take()
    }

    /// The server's world to apply before stepping `tick` (a resync),
    /// once it has arrived — then [`Self::resynced`].
    pub fn take_snapshot(&mut self, tick: u64) -> Option<Snapshot> {
        self.snapshot.take_if(|s| s.tick == tick)
    }

    /// The resync snapshot of `tick` is applied: `Ok` — the world is the
    /// server's again (its hash checked), the report says where it had
    /// diverged; `Err` — it did not heal (why): the checks stop.
    pub fn resynced(&mut self, tick: u64, outcome: Result<&[String], String>) {
        self.resync_asked = None;
        self.ours.retain(|&k, _| k >= tick);
        self.theirs.retain(|&k, _| k >= tick);
        let msg = match outcome {
            Ok(report) => {
                self.resyncs += 1;
                let head: Vec<&str> = report
                    .iter()
                    .take(3)
                    .map(|l| l.split(':').next().unwrap_or(l))
                    .collect();
                let more = report.len().saturating_sub(head.len());
                let more = if more > 0 {
                    format!(" +{more}")
                } else {
                    String::new()
                };
                format!(
                    "resynced from the server at tick {tick} ({}{more})",
                    head.join(", ")
                )
            }
            Err(why) => {
                self.unhealed = true;
                format!("RESYNC FAILED at tick {tick}: {why} — hash checks stop")
            }
        };
        eprintln!("net: {msg}");
        self.desync = Some(msg);
    }

    /// The pre-step state of lockstep tick `tick` — hashed (lazily)
    /// only on check ticks, and not while a resync is on its way.
    pub fn note_hash(&mut self, tick: u64, hash: impl FnOnce() -> u64) {
        if tick % CHECK_EVERY != 0 || self.unhealed || self.resync_asked.is_some() {
            return;
        }
        let h = hash();
        if trace() {
            println!("net: tick {tick} hash {h:016x}");
        }
        self.ours.insert(tick, h);
        self.compare();
    }

    fn compare(&mut self) {
        let common: Vec<u64> = self
            .ours
            .keys()
            .filter(|t| self.theirs.contains_key(t))
            .copied()
            .collect();
        for t in common {
            let (ours, theirs) = (self.ours[&t], self.theirs[&t]);
            if ours == theirs {
                self.agreed += 1;
            } else if self.resync_asked.is_none() && !self.unhealed {
                self.desyncs += 1;
                self.resync_asked = Some(t);
                let msg = format!(
                    "DESYNC at tick {t}: server {theirs:016x}, here {ours:016x} — resyncing"
                );
                eprintln!("net: {msg}");
                self.desync = Some(msg);
                self.send(&Msg::Resync { tick: t });
            }
            // Everything up to a compared tick is settled.
            self.ours.retain(|&k, _| k > t);
            self.theirs.retain(|&k, _| k > t);
        }
    }

    /// A desync or a resync to tell, once (for the toast).
    pub fn take_desync(&mut self) -> Option<String> {
        self.desync.take()
    }

    /// Desyncs detected, resyncs that healed.
    #[cfg(test)]
    pub fn desyncs(&self) -> (u64, u64) {
        (self.desyncs, self.resyncs)
    }

    fn summary(&self) {
        let mut fate = String::new();
        if self.desyncs > 0 {
            fate += &format!(" — {} DESYNCS, {} resynced", self.desyncs, self.resyncs);
        }
        if self.unhealed {
            fate += " — UNHEALED";
        }
        if let Some(why) = &self.closed {
            fate += &format!(" — ended: {why}");
        }
        println!(
            "net: {} ticks stepped, {} hash checks agreed{fate}",
            self.next_tick, self.agreed
        );
    }

    /// Leave the game but not the session: the server banishes this
    /// seat (after the verdict: only stops waiting for it), and the
    /// lobby comes back WAITING for the game to finish — unless the
    /// server already reopened it. `None` when the session is lost.
    pub fn finish(mut self) -> Option<Guest> {
        self.summary();
        if self.closed.is_some() {
            self.guest = None;
            return None;
        }
        if !self.ended {
            self.send(&Msg::Leave);
        }
        let mut guest = self.guest.take()?;
        if !self.ended {
            guest.view.playing = true;
        }
        Some(guest)
    }
}

impl Drop for Lockstep {
    fn drop(&mut self) {
        if self.guest.is_some() {
            self.summary();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A referee whose world is the tick counter: hash `t * 7` — what
    /// the tests' peers hash too — and a verdict once a seat has left
    /// and `verdict_after` more ticks stepped (or never).
    struct Counter {
        tick: u64,
        verdict_at: Option<u64>,
        begun: Arc<Mutex<Vec<Game>>>,
    }

    impl Referee for Counter {
        fn begin(&mut self, game: &Game) -> Result<(), String> {
            if game.level.ends_with("broken.mgcl") {
                return Err("no such level".into());
            }
            self.tick = 0;
            self.begun.lock().unwrap().push(game.clone());
            Ok(())
        }
        fn hash(&self) -> u64 {
            self.tick * 7
        }
        fn snapshot(&self) -> Vec<u8> {
            self.tick.to_le_bytes().to_vec()
        }
        fn step(&mut self, _: &[PortInput], _: &[u8]) {
            self.tick += 1;
        }
        fn verdict(&self) -> Option<String> {
            self.verdict_at
                .filter(|&t| self.tick >= t)
                .map(|_| "P0 is victorious!".into())
        }
        fn end(&mut self) {}
    }

    struct Rig {
        server: ServerHandle,
        begun: Arc<Mutex<Vec<Game>>>,
    }

    /// A hosted-style server on a free loopback port (no admin key: the
    /// first member in is the admin, like a dedicated one).
    fn server(pass: &str, verdict_at: Option<u64>) -> Rig {
        frozen_server(pass, verdict_at, FROZEN_AFTER)
    }

    /// [`server`] dropping a player silent for `frozen`.
    fn frozen_server(pass: &str, verdict_at: Option<u64>, frozen: Duration) -> Rig {
        let begun = Arc::new(Mutex::new(Vec::new()));
        let b = begun.clone();
        let server = ServerHandle::spawn(
            "127.0.0.1:0",
            ServerOpts {
                game: NetGame::Mc2,
                passphrase: pass.into(),
                build: "test".into(),
                level: "baked/mc2/level-050.mgcl".into(),
                rules: serde_json::json!({"fog": 40}),
                admin_key: Some(String::new()),
                frozen_after: FROZEN_AFTER.min(frozen),
            },
            move || {
                Box::new(Counter {
                    tick: 0,
                    verdict_at,
                    begun: b,
                })
            },
        )
        .unwrap();
        Rig { server, begun }
    }

    fn guest(r: &Rig, name: &str) -> Guest {
        connect(r, name, "pw", "test").unwrap()
    }

    fn connect(r: &Rig, name: &str, pass: &str, build: &str) -> Result<Guest, String> {
        Guest::connect(
            &r.server.local_addr(),
            pass,
            name,
            Some(NetGame::Mc2),
            build,
            "",
            Duration::from_secs(5),
        )
    }

    /// Poll `guests` until `done` holds.
    fn settle(guests: &mut [&mut Guest], done: impl Fn(&[&mut Guest]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(guests) {
            assert!(Instant::now() < deadline, "the lobby never settled");
            guests.iter_mut().for_each(|g| g.poll());
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The admin (`gs[0]`) starts a game everyone agrees to: the links,
    /// in member order.
    fn start(gs: Vec<Guest>) -> Vec<Link> {
        let mut gs = gs;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !gs[0].view().all_agreed() {
            assert!(Instant::now() < deadline, "never agreed");
            for g in &mut gs[1..] {
                g.poll();
                if g.view().me().is_some_and(|m| !m.agreed) {
                    g.agree();
                }
            }
            gs[0].poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        gs[0].play();
        let mut refs: Vec<&mut Guest> = gs.iter_mut().collect();
        settle(&mut refs, |gs| gs.iter().all(|g| g.started()));
        gs.into_iter()
            .map(|g| g.into_link().ok().unwrap())
            .collect()
    }

    fn input(stick_x: i16) -> PortInput {
        PortInput {
            stick_x,
            ..PortInput::default()
        }
    }

    /// Pump `peers` until each has stepped `ticks` more turns (seat k's
    /// input `stick_x = k`); returns per peer the turns' stick_x rows
    /// and the seats seen leaving. A `None` peer sits out.
    fn pump(peers: &mut [Option<&mut Lockstep>], ticks: u64) -> Vec<(Vec<Vec<i16>>, Vec<u8>)> {
        let mut out = vec![(Vec::new(), Vec::new()); peers.len()];
        let deadline = Instant::now() + Duration::from_secs(10);
        while peers
            .iter()
            .zip(&out)
            .any(|(p, o)| p.is_some() && (o.0.len() as u64) < ticks)
        {
            assert!(Instant::now() < deadline, "lockstep stalled");
            for (k, p) in peers.iter_mut().enumerate() {
                let Some(p) = p else { continue };
                p.poll();
                assert!(p.closed().is_none(), "{:?}", p.closed());
                while p.wants_input() {
                    let x = if p.in_lead_in() { 0 } else { k as i16 + 1 };
                    p.submit(input(x));
                }
                if (out[k].0.len() as u64) < ticks
                    && let Some((t, inputs)) = p.take_turn()
                {
                    p.note_hash(t, || t * 7);
                    out[k].1.extend(p.left_at(t));
                    out[k].0.push(inputs.iter().map(|i| i.stick_x).collect());
                }
            }
            std::thread::yield_now();
        }
        out
    }

    #[test]
    fn every_player_steps_the_servers_bundles() {
        let r = server("pw", None);
        let links = start(vec![guest(&r, "A"), guest(&r, "B")]);
        assert_eq!(links[1].local, Hx(1));
        assert_eq!(links[1].game.level, "baked/mc2/level-050.mgcl");
        assert_eq!(links[1].game.rules, serde_json::json!({"fog": 40}));
        assert_eq!(
            r.begun.lock().unwrap()[0].seats,
            vec![0, 1],
            "the server built it too"
        );
        let mut ls: Vec<Lockstep> = links.into_iter().map(Lockstep::start).collect();
        let (a, b) = ls.split_at_mut(1);
        let out = pump(&mut [Some(&mut a[0]), Some(&mut b[0])], 100);
        assert_eq!(out[0].0, out[1].0);
        for (t, turn) in out[0].0.iter().enumerate() {
            if (t as u64) < INPUT_DELAY {
                assert_eq!(turn, &vec![0, 0]);
            } else {
                assert_eq!(turn, &vec![1, 2], "tick {t}");
            }
        }
        for l in &mut ls {
            assert!(l.take_desync().is_none());
            assert!(l.agreed >= 4, "checked against the server");
        }
    }

    /// A player whose world forks from the SERVER's is told — each
    /// player is checked, the hosting one too.
    #[test]
    fn a_fork_from_the_server_is_resynced() {
        let r = server("pw", None);
        let links = start(vec![guest(&r, "A"), guest(&r, "B")]);
        let mut ls: Vec<Lockstep> = links.into_iter().map(Lockstep::start).collect();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stepped = [0u64; 2];
        // Each player's "world" is the server's counter; player 0's
        // forks by 1000 at tick 30.
        let mut worlds = [0u64; 2];
        let mut told = Vec::new();
        while stepped.iter().any(|&n| n < 120) {
            assert!(Instant::now() < deadline, "stalled");
            for (k, p) in ls.iter_mut().enumerate() {
                p.poll();
                while p.wants_input() {
                    p.submit(input(0));
                }
                if let Some((t, _)) = p.take_turn() {
                    if let Some(snap) = p.take_snapshot(t) {
                        assert_eq!(k, 0, "only the forked player is resynced");
                        worlds[k] = u64::from_le_bytes(snap.bytes[..].try_into().unwrap());
                        assert_eq!(worlds[k] * 7, snap.hash);
                        p.resynced(t, Ok(&["the counter".into()]));
                    }
                    let w = worlds[k];
                    p.note_hash(t, || w * 7);
                    worlds[k] += 1 + if k == 0 && t == 30 { 1000 } else { 0 };
                    stepped[k] += 1;
                    told.extend(p.take_desync().map(|d| (k, d)));
                }
            }
        }
        assert_eq!(told.len(), 2, "{told:?}");
        assert!(told[0].1.contains("DESYNC at tick 48"), "{told:?}");
        assert!(told[1].1.contains("resynced from the server"), "{told:?}");
        assert!(told[1].1.contains("the counter"), "{told:?}");
        assert_eq!(ls[0].desyncs(), (1, 1));
        // (One peer may have stepped a tick further.)
        assert_eq!(worlds[0] - stepped[0], worlds[1] - stepped[1], "healed");
        assert_eq!(ls[1].desyncs(), (0, 0));
        assert!(ls[0].agreed >= 3, "the checks go on after the resync");
    }

    #[test]
    fn the_handshake_checks_passphrase_and_build() {
        let r = server("secret", None);
        let e = connect(&r, "A", "wrong", "test").err().unwrap();
        assert!(e.contains("wrong passphrase"), "{e}");
        let e = connect(&r, "A", "secret", "other").err().unwrap();
        assert!(e.contains("build other"), "{e}");
        let a = connect(&r, "A", "secret", "test").unwrap();
        assert!(a.view().is_admin(), "the first one in");
        assert_eq!(
            a.view().passphrase.as_deref(),
            Some("secret"),
            "the admin's view"
        );
        let b = connect(&r, "B", "secret", "test").unwrap();
        assert_eq!(b.view().members.len(), 2);
        assert_eq!(b.view().me().unwrap().wizard, 1, "the first free wizard");
        assert!(!b.view().is_admin());
        assert_eq!(b.view().passphrase, None, "the admin's only");
    }

    /// An empty passphrase is an open session: anyone gets in, whatever
    /// he sends (player ruling 2026-10-17) — until the admin sets one.
    #[test]
    fn an_empty_passphrase_opens_the_session() {
        let r = server("", None);
        let a = connect(&r, "A", "", "test").unwrap();
        connect(&r, "B", "anything", "test").unwrap();
        let mut a = a;
        a.set_passphrase("now".into());
        let deadline = Instant::now() + Duration::from_secs(5);
        while a.view().passphrase.as_deref() != Some("now") {
            assert!(Instant::now() < deadline);
            a.poll();
        }
        let e = connect(&r, "C", "", "test").err().unwrap();
        assert!(e.contains("wrong passphrase"), "{e}");
    }

    /// A session plays ONE game, the server's: a player of another is
    /// refused at the door, and the admin can't switch the level to
    /// another game's (player 2026-10-17).
    #[test]
    fn the_session_plays_the_servers_game_only() {
        let r = server("pw", None);
        let addr = r.server.local_addr();
        let wait = Duration::from_secs(5);
        let e = Guest::connect(&addr, "pw", "M1", Some(NetGame::Mc1), "test", "", wait)
            .err()
            .unwrap();
        assert!(e.contains("this server runs Magic Carpet 2"), "{e}");
        let mut a = guest(&r, "A");
        assert_eq!(a.view().game, NetGame::Mc2);
        let mut cli = Guest::connect(&addr, "pw", "CLI", None, "test", "", wait).unwrap();
        a.set_level("baked/mc1/level-051.mgcl".into());
        a.set_level("baked/mc2/level-052.mgcl".into());
        settle(&mut [&mut a, &mut cli], |g| {
            g[1].view().level == "baked/mc2/level-052.mgcl"
        });
        assert_eq!(a.view().rev, 1, "the MC1 level never landed");
    }

    /// The hosted server's own player claims the admin role with its
    /// key, whoever came first; a dedicated server's goes to the first.
    #[test]
    fn the_admin_key_claims_the_role() {
        let begun = Arc::new(Mutex::new(Vec::new()));
        let h = ServerHandle::spawn(
            "127.0.0.1:0",
            ServerOpts {
                game: NetGame::Mc2,
                passphrase: "pw".into(),
                build: "test".into(),
                level: "baked/mc2/level-050.mgcl".into(),
                rules: serde_json::json!({}),
                admin_key: None,
                frozen_after: FROZEN_AFTER,
            },
            move || {
                Box::new(Counter {
                    tick: 0,
                    verdict_at: None,
                    begun,
                })
            },
        )
        .unwrap();
        let addr = h.local_addr();
        let mut early = Guest::connect(
            &addr,
            "pw",
            "EARLY",
            None,
            "test",
            "",
            Duration::from_secs(5),
        )
        .unwrap();
        let fake = Guest::connect(
            &addr,
            "pw",
            "FAKE",
            None,
            "test",
            "nope",
            Duration::from_secs(5),
        )
        .unwrap();
        let host = Guest::connect(
            &addr,
            "pw",
            "HOST",
            None,
            "test",
            h.admin_key(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(host.view().is_admin());
        assert!(!fake.view().is_admin());
        settle(&mut [&mut early], |g| !g[0].view().is_admin());
    }

    /// Rulings 2, 3, 7: Play waits for the agreement; an admin change
    /// voids it; a non-admin's settings are ignored.
    #[test]
    fn play_waits_for_agreement_and_a_change_voids_it() {
        let r = server("pw", None);
        let (mut a, mut b) = (guest(&r, "A"), guest(&r, "B"));
        b.set_rules(serde_json::json!({"fog": 10}));
        b.play();
        b.agree();
        settle(&mut [&mut a, &mut b], |g| g[0].view().all_agreed());
        assert_eq!(
            a.view().rules,
            serde_json::json!({"fog": 40}),
            "not B's to set"
        );
        a.set_rules(serde_json::json!({"fog": 90}));
        settle(&mut [&mut a, &mut b], |g| g[1].view().rev == 1);
        assert!(!a.view().all_agreed() && !b.view().all_agreed());
        assert_eq!(b.view().rules, serde_json::json!({"fog": 90}));
        // Play before the agreement — a round trip on A's own
        // connection (a limit change) proves the server took it first.
        a.play();
        a.set_capacity(3);
        settle(&mut [&mut a], |g| g[0].view().capacity == 3);
        b.agree();
        settle(&mut [&mut a, &mut b], |g| g[0].view().all_agreed());
        assert!(!b.started(), "the refused Play never started");
        assert!(r.begun.lock().unwrap().is_empty());
    }

    /// A level the server cannot build: no game, the lobby is told.
    #[test]
    fn a_game_the_server_cannot_build_never_starts() {
        let r = server("pw", None);
        let (mut a, mut b) = (guest(&r, "A"), guest(&r, "B"));
        a.set_level("baked/mc2/broken.mgcl".into());
        settle(&mut [&mut a, &mut b], |g| {
            g[1].view().level == "baked/mc2/broken.mgcl"
        });
        b.agree();
        settle(&mut [&mut a, &mut b], |g| g[0].view().all_agreed());
        a.play();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut told = Vec::new();
        while told.is_empty() {
            assert!(Instant::now() < deadline);
            b.poll();
            told = b.take_notices();
        }
        assert!(told[0].contains("no such level"), "{told:?}");
        assert!(!a.started() && !b.started());
    }

    /// Rulings 4 and 6: a kicked member is out, and a new passphrase
    /// keeps him out.
    #[test]
    fn a_kicked_member_cannot_return_after_a_passphrase_change() {
        let r = server("pw", None);
        let (mut a, mut b) = (guest(&r, "A"), guest(&r, "B"));
        let id = b.view().you;
        a.kick(id);
        settle(&mut [&mut a, &mut b], |g| g[1].closed().is_some());
        assert!(b.closed().unwrap().contains("kicked"));
        settle(&mut [&mut a], |g| g[0].view().members.len() == 1);
        a.set_passphrase("new".into());
        settle(&mut [&mut a], |g| {
            g[0].view().passphrase.as_deref() == Some("new")
        });
        assert!(connect(&r, "B", "pw", "test").is_err());
        assert!(connect(&r, "B", "new", "test").is_ok());
    }

    #[test]
    fn a_wizard_request_is_first_come_first_served() {
        let r = server("pw", None);
        let (mut a, mut b) = (guest(&r, "A"), guest(&r, "B"));
        b.pick_wizard(0);
        b.pick_wizard(5);
        settle(&mut [&mut a, &mut b], |g| {
            g[1].view().me().unwrap().wizard == 5 && g[0].view().seats() == vec![0, 5]
        });
        a.pick_wizard(5);
        std::thread::sleep(Duration::from_millis(50));
        a.poll();
        assert_eq!(a.view().seats(), vec![0, 5], "A can't take B's 5");
    }

    /// The admin's leaving the session hands the role on.
    #[test]
    fn the_admin_role_passes_on() {
        let r = server("pw", None);
        let (a, mut b) = (guest(&r, "A"), guest(&r, "B"));
        drop(a);
        settle(&mut [&mut b], |g| g[0].view().is_admin());
    }

    /// Ruling 10: a won game ends back in the lobby with the same
    /// players; Play needs a fresh agreement; a newcomer during the
    /// game waits for its end.
    #[test]
    fn after_the_verdict_the_lobby_reopens_for_the_next_game() {
        let r = server("pw", Some(40));
        let (mut a, mut b) = (guest(&r, "A"), guest(&r, "B"));
        for round in 0..2 {
            let links = start(vec![a, b]);
            assert_eq!(links[0].game.names, vec!["A", "B"], "names travel");
            let mut ls: Vec<Lockstep> = links.into_iter().map(Lockstep::start).collect();
            let late = (round == 0).then(|| guest(&r, "LATE"));
            assert!(late.as_ref().is_none_or(|l| l.view().playing), "he waits");
            {
                let (x, y) = ls.split_at_mut(1);
                pump(&mut [Some(&mut x[0]), Some(&mut y[0])], 45);
            }
            for l in &mut ls {
                assert_eq!(l.take_verdict().as_deref(), Some("P0 is victorious!"));
            }
            // Each fades out and leaves; the lobby reopens once both
            // have.
            let mut back: Vec<Guest> = ls.into_iter().filter_map(Lockstep::finish).collect();
            assert!(back.iter().all(|g| g.view().playing));
            {
                let mut refs: Vec<&mut Guest> = back.iter_mut().collect();
                settle(&mut refs, |gs| gs.iter().all(|g| !g.view().playing));
            }
            let members = if round == 0 { 3 } else { 2 };
            assert_eq!(back[0].view().members.len(), members, "same players");
            assert!(!back[0].view().all_agreed(), "everyone approves again");
            drop(late);
            b = back.pop().unwrap();
            a = back.pop().unwrap();
            settle(&mut [&mut a], |g| g[0].view().members.len() == 2);
        }
        assert_eq!(r.begun.lock().unwrap().len(), 2);
    }

    /// A player LEAVING the game (player 2026-10-17) stays a member: the
    /// game goes on for the rest (his seat banished on one tick for
    /// all), his lobby waits for it to finish, and it reopens with him
    /// in it — the admin (the UI's host) alike.
    #[test]
    fn a_leaver_waits_in_the_lobby_for_the_game_to_finish() {
        let r = server("pw", None);
        let links = start(vec![guest(&r, "ONE"), guest(&r, "TWO"), guest(&r, "THREE")]);
        let mut ls: Vec<Option<Lockstep>> = links
            .into_iter()
            .map(|l| Some(Lockstep::start(l)))
            .collect();
        {
            let mut refs: Vec<Option<&mut Lockstep>> = ls.iter_mut().map(Option::as_mut).collect();
            pump(&mut refs, 20);
        }
        // The admin leaves.
        let mut waiting = ls[0].take().unwrap().finish().expect("his session");
        assert!(waiting.view().playing, "his lobby waits");
        let out = {
            let mut refs: Vec<Option<&mut Lockstep>> = ls.iter_mut().map(Option::as_mut).collect();
            pump(&mut refs, 40)
        };
        assert_eq!(
            (&out[1].1, &out[2].1),
            (&vec![0], &vec![0]),
            "seat 0 banished for both"
        );
        waiting.poll();
        assert!(waiting.closed().is_none() && waiting.view().playing);
        assert!(waiting.view().is_admin(), "still the admin");
        assert_eq!(r.server.status().playing, 2);
        // The other two leave too: nobody plays, the lobby reopens.
        let mut back: Vec<Guest> = ls
            .into_iter()
            .flatten()
            .filter_map(Lockstep::finish)
            .collect();
        back.push(waiting);
        let mut refs: Vec<&mut Guest> = back.iter_mut().collect();
        settle(&mut refs, |gs| gs.iter().all(|g| !g.view().playing));
        assert_eq!(back[0].view().members.len(), 3, "everyone still a member");
    }

    /// A player hanging up mid-game is banished; the rest play on.
    #[test]
    fn a_lost_player_is_banished_not_fatal() {
        let r = server("pw", None);
        let links = start(vec![guest(&r, "ONE"), guest(&r, "TWO")]);
        let mut ls: Vec<Lockstep> = links.into_iter().map(Lockstep::start).collect();
        let two = ls.pop().unwrap();
        drop(two);
        let out = pump(&mut [Some(&mut ls[0])], 40);
        assert_eq!(out[0].1, vec![1], "seat 1 leaves");
        assert!(ls[0].closed().is_none());
    }

    /// A player connected but silent (his app hung) holds the game only
    /// so long: he is dropped and banished, the rest play on. The
    /// others, waiting on him, are silent too — but not dropped.
    #[test]
    fn a_frozen_player_is_dropped_after_the_timeout() {
        let r = frozen_server("pw", None, Duration::from_millis(400));
        let links = start(vec![guest(&r, "ONE"), guest(&r, "TWO")]);
        let mut ls: Vec<Lockstep> = links.into_iter().map(Lockstep::start).collect();
        {
            let (a, b) = ls.split_at_mut(1);
            pump(&mut [Some(&mut a[0]), Some(&mut b[0])], 20);
        }
        // TWO freezes: polls nothing, sends nothing.
        let mut two = ls.pop().unwrap();
        let start = Instant::now();
        let out = pump(&mut [Some(&mut ls[0])], 20);
        assert!(start.elapsed() >= Duration::from_millis(300), "ONE waited");
        assert_eq!(out[0].1, vec![1], "TWO banished");
        assert!(ls[0].closed().is_none(), "ONE plays on");
        let deadline = Instant::now() + Duration::from_secs(5);
        while two.closed().is_none() {
            assert!(Instant::now() < deadline);
            two.poll();
        }
        assert!(
            two.closed().unwrap().contains("no input"),
            "{:?}",
            two.closed()
        );
    }

    /// The server stopping (the hosting app closed) ends the session for
    /// every player.
    #[test]
    fn a_stopped_server_closes_the_session() {
        let r = server("pw", None);
        let links = start(vec![guest(&r, "A"), guest(&r, "B")]);
        let mut ls: Vec<Lockstep> = links.into_iter().map(Lockstep::start).collect();
        drop(r);
        let deadline = Instant::now() + Duration::from_secs(5);
        while ls.iter().any(|l| l.closed().is_none()) {
            assert!(Instant::now() < deadline);
            ls.iter_mut().for_each(Lockstep::poll);
            std::thread::yield_now();
        }
        assert!(ls[0].closed().unwrap().contains("server stopped"));
    }
}
