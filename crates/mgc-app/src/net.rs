//! ⭐ NETWORK TRACK, MVP — two apps, one world, deterministic lockstep
//! over TCP (rulings 10-14: TCP only; star topology, the host is the
//! authority).
//!
//! Only INPUTS cross the wire — retail's own model (NetBIOS lockstep,
//! `ReceiveSendAll_7438A`): every peer simulates the whole world from
//! the same per-tick input bundle. The star: the CLIENT sends its seat's
//! input for tick `t` to the HOST; the host pairs it with its own and
//! broadcasts the bundle as `Turn t`; nobody steps a tick before its
//! bundle exists (the host steps from its own completed bundles). Each
//! seat's input is scheduled [`INPUT_DELAY`] ticks ahead, which hides
//! the round trip under that many ticks of control latency.
//!
//! Unlike retail (no desync detection at all), the host stamps its
//! pre-step `state_hash` every [`CHECK_EVERY`] ticks onto a turn and the
//! client compares — DETECTION only for now; reconciliation (the host's
//! state wins) and host handover are later stages.
//!
//! Wire: a little-endian `u32` length, then one JSON [`Msg`]. The input
//! payload is the recorder's [`PortInput`] (idle ticks encode as `{}`).

use crate::lobby::{LobbyCore, LobbyView};
use mgc_formats::mgcr::PortInput;
use mgc_sim::Hx;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Bumped whenever [`Msg`] changes shape.
const PROTOCOL: u32 = 3;
/// The most players a session holds — retail's 8 (one per wizard). The
/// host lowers it in the lobby (`Host::set_capacity`).
pub const MAX_PLAYERS: usize = 8;
/// Ticks between sampling a seat's input and stepping it (24 Hz: 3
/// ticks = 125 ms of control latency, ample on loopback/LAN/a tailnet).
pub const INPUT_DELAY: u64 = 3;
/// The host stamps its state hash on every this-many-th tick (1 s).
pub const CHECK_EVERY: u64 = 24;
/// A frame larger than this is a broken peer, not a message.
const MAX_FRAME: usize = 1 << 20;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
enum Msg {
    /// Client → host: who I am (the campaign's player name, maybe
    /// empty), and the session's passphrase.
    Hello {
        protocol: u32,
        build: String,
        passphrase: String,
        name: String,
    },
    Refuse {
        why: String,
    },
    /// Host → client: the lobby as that client sees it, on every
    /// change (and after a game: the lobby reopened).
    Lobby {
        view: LobbyView,
    },
    /// Client → host: take this wizard (first come, first served).
    Wizard {
        want: u8,
    },
    /// Client → host: I agree to rules revision `rev`.
    Agree {
        rev: u32,
    },
    /// Client → host: I take my agreement back.
    Withdraw,
    /// Client → host: my flight models (on joining, and on a change).
    Models {
        models: serde_json::Value,
    },
    /// Host → client: the game begins. `seats[k]` is human seat
    /// `Hx(k)`'s wizard (the host is `Hx(0)`), `local` the receiver's.
    Start {
        seats: Vec<u8>,
        names: Vec<String>,
        models: Vec<serde_json::Value>,
        ai: u8,
        local: u8,
        level: String,
        rules: serde_json::Value,
    },
    /// Client → host: the client seat's input for `tick`.
    Input {
        tick: u64,
        input: PortInput,
    },
    /// Host → client: every seat's input for `tick`, in seat order,
    /// plus (now and then) the host's pre-step hash of an earlier tick.
    Turn {
        tick: u64,
        inputs: Vec<PortInput>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check: Option<(u64, u64)>,
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

/// `MGC_NET_TRACE=1`: print every hash check (both peers).
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Host,
    Client,
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

/// The HOST's lobby: listens, admits clients that pass the handshake
/// (protocol, build, passphrase), serves their requests against the
/// [`LobbyCore`] and broadcasts every change. Non-blocking — poll it
/// once per frame.
pub struct Host {
    core: LobbyCore,
    passphrase: String,
    build: String,
    listener: TcpListener,
    /// Connected, not yet said hello.
    pending: Vec<Conn>,
    /// Admitted clients, in member (join) order.
    clients: Vec<(u32, Conn)>,
    notes: Vec<String>,
    dirty: bool,
}

impl Host {
    /// Open a lobby on `addr` with the host flying `wizard`.
    pub fn open(
        addr: &str,
        wizard: u8,
        name: String,
        level: String,
        rules: serde_json::Value,
        passphrase: String,
        build: String,
    ) -> Result<Self, String> {
        let listener = TcpListener::bind(addr).map_err(|e| format!("net: bind {addr}: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("net: {addr}: {e}"))?;
        Ok(Self {
            core: LobbyCore::new(wizard, name, level, rules, MAX_PLAYERS),
            passphrase,
            build,
            listener,
            pending: Vec::new(),
            clients: Vec::new(),
            notes: Vec::new(),
            dirty: false,
        })
    }

    /// The address actually bound (a `:0` port resolved).
    pub fn addr(&self) -> String {
        self.listener
            .local_addr()
            .map_or_else(|_| "?".into(), |a| a.to_string())
    }

    pub fn view(&self) -> LobbyView {
        self.core.view_for(0)
    }

    /// What happened since the last call ("joined", "left", refusals).
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    pub fn poll(&mut self) {
        self.accept();
        self.handshake();
        self.serve();
        self.flush();
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
                    self.notes.push(format!("accept: {e}"));
                    return;
                }
            }
        }
    }

    fn refuse(&mut self, mut conn: Conn, why: String) {
        self.notes.push(format!("refused {}: {why}", conn.peer));
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
                }) => {
                    let refusal = if protocol != PROTOCOL {
                        Some(format!("protocol {protocol}, the host speaks {PROTOCOL}"))
                    } else if build != self.build {
                        Some(format!("build {build}, the host runs {}", self.build))
                    } else if passphrase != self.passphrase {
                        Some("wrong passphrase".into())
                    } else {
                        None
                    };
                    match refusal.map_or_else(|| self.core.join(name), Err) {
                        Ok(id) => {
                            let w = self.core.view_for(id).me().map_or(0, |m| m.wizard);
                            self.notes
                                .push(format!("{} joined as wizard {}", conn.peer, w + 1));
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
        let mut gone: Vec<(u32, String)> = Vec::new();
        for (id, conn) in &self.clients {
            loop {
                match conn.recv() {
                    Recv::Msg(Msg::Wizard { want }) => {
                        self.dirty |= self.core.pick_wizard(*id, want);
                    }
                    Recv::Msg(Msg::Agree { rev }) => self.dirty |= self.core.agree(*id, rev),
                    Recv::Msg(Msg::Withdraw) => self.dirty |= self.core.withdraw(*id),
                    Recv::Msg(Msg::Models { models }) => {
                        self.dirty |= self.core.set_models(*id, models);
                    }
                    // A game's tail (inputs sent before its end landed).
                    Recv::Msg(Msg::Input { .. }) => {}
                    Recv::Msg(Msg::Bye { why }) => {
                        gone.push((*id, why));
                        break;
                    }
                    Recv::Msg(other) => {
                        gone.push((*id, format!("sent {other:?}")));
                        break;
                    }
                    Recv::Lost(e) => {
                        gone.push((*id, e));
                        break;
                    }
                    Recv::Empty => break,
                }
            }
        }
        for (id, why) in gone {
            self.drop_client(id, &why);
            self.notes.push(format!("member {id} left ({why})"));
        }
    }

    fn drop_client(&mut self, id: u32, why: &str) {
        if let Some(i) = self.clients.iter().position(|(c, _)| *c == id) {
            let (_, mut conn) = self.clients.remove(i);
            conn.close(why);
        }
        self.dirty |= self.core.leave(id);
    }

    fn flush(&mut self) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let mut lost = Vec::new();
        for (id, conn) in &mut self.clients {
            let view = self.core.view_for(*id);
            if let Err(e) = conn.send(&Msg::Lobby { view }) {
                lost.push((*id, e.to_string()));
            }
        }
        for (id, why) in lost {
            self.drop_client(id, &why);
            self.notes.push(format!("member {id} lost ({why})"));
        }
        if self.dirty {
            self.flush();
        }
    }

    /// The host's own wizard (first come, first served applies to him
    /// too).
    pub fn pick_wizard(&mut self, wizard: u8) -> bool {
        let ok = self.core.pick_wizard(0, wizard);
        self.dirty |= ok;
        self.flush();
        ok
    }

    /// Change the level — voids every agreement.
    pub fn set_level(&mut self, level: String) {
        self.dirty |= self.core.set_level(level);
        self.flush();
    }

    /// Change the AI wizard count — a rule: voids every agreement.
    pub fn set_ai(&mut self, ai: u8) {
        self.dirty |= self.core.set_ai(ai);
        self.flush();
    }

    /// Change the rules — voids every agreement.
    pub fn set_rules(&mut self, rules: serde_json::Value) {
        self.dirty |= self.core.set_rules(rules);
        self.flush();
    }

    /// Change the passphrase: later joiners need the new one (a kicked
    /// player can't come back); members already in stay.
    pub fn set_passphrase(&mut self, passphrase: String) {
        self.passphrase = passphrase;
    }

    /// The host's own flight models.
    pub fn set_models(&mut self, models: serde_json::Value) {
        self.dirty |= self.core.set_models(0, models);
        self.flush();
    }

    pub fn passphrase(&self) -> &str {
        &self.passphrase
    }

    /// The player limit (2 up to [`MAX_PLAYERS`], never below the
    /// members in).
    pub fn set_capacity(&mut self, capacity: usize) {
        self.dirty |= self.core.set_capacity(capacity);
        self.flush();
    }

    /// Show member `id` the door.
    pub fn kick(&mut self, id: u32) {
        if id != 0 {
            self.drop_client(id, "kicked by the host");
            self.notes.push(format!("member {id} kicked"));
            self.flush();
        }
    }

    pub fn can_play(&self) -> bool {
        self.core.can_play()
    }

    /// Play: every client gets its seat and the agreed session, and the
    /// lobby becomes a game. Refused (the lobby handed back) unless
    /// every client agreed.
    pub fn play(mut self) -> Result<Link, Box<Self>> {
        self.poll();
        if !self.can_play() {
            return Err(Box::new(self));
        }
        let view = self.view();
        let seats = view.seats();
        let mut lost = None;
        for (id, conn) in &mut self.clients {
            let local = view.members.iter().position(|m| m.id == *id);
            let start = Msg::Start {
                seats: seats.clone(),
                names: view.names(),
                models: view.models(),
                ai: view.ai,
                local: local.unwrap_or(0) as u8,
                level: view.level.clone(),
                rules: view.rules.clone(),
            };
            if let Err(e) = conn.send(&start) {
                lost.get_or_insert(format!("{}: {e}", conn.peer));
            }
        }
        Ok(Link {
            session: Session::Host(self),
            role: Role::Host,
            names: view.names(),
            models: view.models(),
            ai: view.ai,
            seats,
            local: Hx::FIRST,
            level: view.level,
            config: view.rules,
            lost,
        })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        for (_, c) in &mut self.clients {
            c.close("the host closed the lobby");
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

/// A CLIENT's lobby: the host's latest view, our requests, and the
/// Start that turns it into a game.
pub struct Guest {
    conn: Conn,
    view: LobbyView,
    start: Option<Start>,
    /// The flight models last sent.
    models_sent: Option<serde_json::Value>,
    closed: Option<String>,
}

impl Guest {
    /// Connect to `addr` (retrying for up to `wait` while the host
    /// comes up) and pass the handshake — the first lobby view, or the
    /// host's refusal.
    pub fn connect(
        addr: &str,
        passphrase: &str,
        name: &str,
        build: &str,
        wait: Duration,
    ) -> Result<Self, String> {
        let deadline = Instant::now() + wait;
        let mut said = false;
        let stream = loop {
            match TcpStream::connect(addr) {
                Ok(s) => break s,
                Err(e) if Instant::now() < deadline => {
                    if !said {
                        println!("net: waiting for a host at {addr} ({e})");
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
                        closed: None,
                    });
                }
                Recv::Msg(Msg::Refuse { why }) => {
                    conn.open = false;
                    return Err(format!("net: the host refused: {why}"));
                }
                Recv::Msg(other) => return Err(format!("net: expected the lobby, got {other:?}")),
                Recv::Lost(e) => return Err(format!("net: {addr}: {e}")),
                Recv::Empty if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Recv::Empty => return Err(format!("net: {addr}: no answer from the host")),
            }
        }
    }

    pub fn view(&self) -> &LobbyView {
        &self.view
    }

    /// Why the lobby ended for us (kicked, the host closed it, lost).
    pub fn closed(&self) -> Option<&str> {
        self.closed.as_deref()
    }

    pub fn started(&self) -> bool {
        self.start.is_some()
    }

    pub fn poll(&mut self) {
        while self.closed.is_none() && self.start.is_none() {
            match self.conn.recv() {
                Recv::Msg(Msg::Lobby { view }) => self.view = view,
                Recv::Msg(Msg::Start {
                    seats,
                    names,
                    models,
                    ai,
                    local,
                    level,
                    rules,
                }) => self.start = Some((seats, names, models, ai, local, level, rules)),
                // A game's tail (turns sent before its end landed).
                Recv::Msg(Msg::Turn { .. }) => {}
                Recv::Msg(Msg::Bye { why }) => {
                    self.conn.open = false;
                    self.closed = Some(why);
                }
                Recv::Msg(other) => {
                    let why = format!("the host sent {other:?}");
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

    /// Ask for `wizard` (the host decides; the next view tells).
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

    /// Tell the host our flight models (only when they changed).
    pub fn set_models(&mut self, models: serde_json::Value) {
        if self.models_sent.as_ref() != Some(&models) {
            self.send(&Msg::Models {
                models: models.clone(),
            });
            self.models_sent = Some(models);
        }
    }

    /// The game the host started (the guest handed back before
    /// [`Self::started`]).
    pub fn into_link(self) -> Result<Link, Box<Self>> {
        let Some((seats, names, models, ai, local, level, rules)) = self.start.clone() else {
            return Err(Box::new(self));
        };
        let mut guest = self;
        guest.start = None;
        Ok(Link {
            session: Session::Guest(guest),
            role: Role::Client,
            seats,
            names,
            models,
            ai,
            local: Hx(local),
            level,
            config: rules,
            lost: None,
        })
    }
}

/// Either end of a session, between games.
pub enum Session {
    Host(Host),
    Guest(Guest),
}

/// A started game: the agreed session, before any world.
pub struct Link {
    session: Session,
    pub role: Role,
    /// Wizard seat per human seat, in seat order (`Hx(k)`).
    pub seats: Vec<u8>,
    /// Each human's campaign name, seat order (empty: none given).
    pub names: Vec<String>,
    /// Each human's flight models (`controls.models`), seat order.
    pub models: Vec<serde_json::Value>,
    /// The AI wizards to seat (the host's rule).
    pub ai: u8,
    /// This app's human seat.
    pub local: Hx,
    pub level: String,
    /// The session rules (`rules::collect` of the host's config).
    pub config: serde_json::Value,
    /// Host: a client lost while starting (the lockstep closes at once).
    lost: Option<String>,
}

/// CLI host (`--host`): open the lobby, wait for the clients to agree
/// (`--join` agrees by itself) and start.
pub fn host(
    addr: &str,
    wizard: u8,
    level: &str,
    rules: serde_json::Value,
    models: serde_json::Value,
    passphrase: &str,
    build: &str,
) -> Result<Link, String> {
    let mut h = Host::open(
        addr,
        wizard,
        String::new(),
        level.to_string(),
        rules,
        passphrase.to_string(),
        build.to_string(),
    )?;
    h.set_models(models);
    println!(
        "net: hosting on {} as seat {} — waiting for --join",
        h.addr(),
        wizard + 1
    );
    loop {
        h.poll();
        for n in h.take_notes() {
            println!("net: {n}");
        }
        if h.can_play() {
            match h.play() {
                Ok(link) => return Ok(link),
                Err(back) => h = *back,
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// CLI client (`--join`): connect, take `wizard` if asked (else the
/// host's pick), agree to whatever the host sets, wait for the start.
pub fn join(
    addr: &str,
    wizard: Option<u8>,
    models: serde_json::Value,
    passphrase: &str,
    build: &str,
) -> Result<Link, String> {
    let mut g = Guest::connect(addr, passphrase, "", build, Duration::from_secs(60))?;
    g.set_models(models);
    if let Some(w) = wizard {
        if !g.view().is_free(w) && g.view().me().is_none_or(|m| m.wizard != w) {
            return Err(format!("net: seat {} is not free", w + 1));
        }
        g.pick_wizard(w);
    }
    let mut agreed = None;
    loop {
        g.poll();
        if let Some(why) = g.closed() {
            return Err(format!("net: the lobby closed: {why}"));
        }
        if g.started() {
            let link = g.into_link().unwrap_or_else(|_| unreachable!("started"));
            println!(
                "net: joined {addr} as seat {} — level {}",
                link.seats[usize::from(link.local.0)] + 1,
                link.level
            );
            return Ok(link);
        }
        let mine = g.view().me().map(|m| m.wizard);
        if wizard.is_none_or(|w| mine == Some(w)) && agreed != Some(g.view().rev) {
            agreed = Some(g.view().rev);
            g.agree();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The running lockstep: turn assembly (host), turn intake (client),
/// the hash check. Non-blocking — the app polls it once per sim tick.
/// Runs on the session's connections and hands them back at the end
/// ([`Self::end`]): the lobby reopens with the same players.
pub struct Lockstep {
    role: Role,
    local: Hx,
    seats: usize,
    /// The agreed session rules (`Link::config`).
    config: serde_json::Value,
    /// `None` once handed back by [`Self::end`].
    session: Option<Session>,
    /// The next tick to step.
    next_tick: u64,
    /// The next tick this seat's input is scheduled for.
    next_send: u64,
    /// Host: the next turn to complete, and partial turns by tick.
    next_complete: u64,
    pending: BTreeMap<u64, Vec<Option<PortInput>>>,
    /// Complete turns not yet stepped.
    ready: BTreeMap<u64, Vec<PortInput>>,
    /// Host: the hash stamp riding the next turn sent.
    stamp: Option<(u64, u64)>,
    /// Client: hashes awaiting their counterpart.
    ours: BTreeMap<u64, u64>,
    theirs: BTreeMap<u64, u64>,
    desync: Option<String>,
    desync_seen: bool,
    /// Client: hash checks that agreed (the session summary).
    agreed: u64,
    closed: Option<String>,
    /// Client: the host reopened the lobby (the game is over).
    ended: bool,
    /// Host: member ids of the clients that left mid-game.
    gone: Vec<u32>,
}

impl Lockstep {
    pub fn start(link: Link) -> Self {
        Self {
            role: link.role,
            local: link.local,
            seats: link.seats.len(),
            config: link.config,
            session: Some(link.session),
            next_tick: 0,
            next_send: 0,
            next_complete: 0,
            pending: BTreeMap::new(),
            ready: BTreeMap::new(),
            stamp: None,
            ours: BTreeMap::new(),
            theirs: BTreeMap::new(),
            desync: None,
            desync_seen: false,
            agreed: 0,
            closed: link.lost,
            ended: false,
            gone: Vec::new(),
        }
    }

    pub fn seats(&self) -> usize {
        self.seats
    }

    /// The session's agreed rules.
    pub fn config(&self) -> &serde_json::Value {
        &self.config
    }

    /// Why the session ended, once it has (peer left, socket error).
    pub fn closed(&self) -> Option<&str> {
        self.closed.as_deref()
    }

    pub fn is_host(&self) -> bool {
        self.role == Role::Host
    }

    /// Client: the host ended the game and reopened the lobby.
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
        match self.role {
            Role::Host => {
                let ix = self.local.0 as usize;
                self.slot(tick)[ix] = Some(input);
                self.complete();
            }
            Role::Client => self.send(&Msg::Input { tick, input }),
        }
    }

    fn slot(&mut self, tick: u64) -> &mut Vec<Option<PortInput>> {
        let n = self.seats;
        self.pending.entry(tick).or_insert_with(|| vec![None; n])
    }

    /// Host: every turn whose seats have all reported, in tick order,
    /// goes out to the clients and onto the local ready queue.
    fn complete(&mut self) {
        while let Some(turn) = self.pending.get(&self.next_complete) {
            if turn.iter().any(Option::is_none) {
                break;
            }
            let tick = self.next_complete;
            let inputs: Vec<PortInput> = self
                .pending
                .remove(&tick)
                .into_iter()
                .flatten()
                .flatten()
                .collect();
            self.next_complete += 1;
            let check = self.stamp.take();
            self.send(&Msg::Turn {
                tick,
                inputs: inputs.clone(),
                check,
            });
            self.ready.insert(tick, inputs);
        }
    }

    /// Host: to every client; client: to the host.
    fn send(&mut self, m: &Msg) {
        if self.closed.is_some() {
            return;
        }
        let err = match &mut self.session {
            Some(Session::Host(h)) => h
                .clients
                .iter_mut()
                .find_map(|(_, c)| c.send(m).err().map(|e| format!("{}: {e}", c.peer))),
            Some(Session::Guest(g)) => g.conn.send(m).err().map(|e| e.to_string()),
            None => None,
        };
        if let Some(e) = err {
            self.closed = Some(format!("send: {e}"));
        }
    }

    /// Drain everything the peers have sent.
    pub fn poll(&mut self) {
        if self.closed.is_some() || self.ended {
            return;
        }
        let mut got: Vec<(usize, Recv)> = Vec::new();
        let mut ids: Vec<u32> = Vec::new();
        match &mut self.session {
            Some(Session::Host(h)) => {
                // Nobody joins a game in progress.
                h.accept();
                for mut conn in std::mem::take(&mut h.pending) {
                    let why = "a game is in progress";
                    let _ = conn.send(&Msg::Refuse { why: why.into() });
                    conn.close(why);
                }
                // Client k is human seat Hx(k + 1) (member order).
                ids = h.clients.iter().map(|(id, _)| *id).collect();
                for (k, (_, conn)) in h.clients.iter().enumerate() {
                    loop {
                        let r = conn.recv();
                        let stop = !matches!(r, Recv::Msg(Msg::Input { .. }));
                        got.push((k + 1, r));
                        if stop {
                            break;
                        }
                    }
                }
            }
            Some(Session::Guest(g)) => loop {
                let r = g.conn.recv();
                let stop = matches!(r, Recv::Empty | Recv::Lost(_));
                got.push((0, r));
                if stop {
                    break;
                }
            },
            None => return,
        }
        for (ix, r) in got {
            match (self.role, r) {
                (_, Recv::Empty) => {}
                (_, Recv::Lost(e)) => {
                    self.gone.extend(ix.checked_sub(1).and_then(|k| ids.get(k)));
                    self.closed
                        .get_or_insert_with(|| format!("connection lost: {e}"));
                    return;
                }
                (_, Recv::Msg(Msg::Bye { why })) => {
                    self.gone.extend(ix.checked_sub(1).and_then(|k| ids.get(k)));
                    self.closed
                        .get_or_insert_with(|| format!("peer left ({why})"));
                    return;
                }
                (Role::Host, Recv::Msg(Msg::Input { tick, input })) => {
                    if tick < self.next_complete {
                        self.fail(format!(
                            "client input for tick {tick} arrived after its turn"
                        ));
                        return;
                    }
                    self.slot(tick)[ix] = Some(input);
                    self.complete();
                }
                (
                    Role::Client,
                    Recv::Msg(Msg::Turn {
                        tick,
                        inputs,
                        check,
                    }),
                ) => {
                    if inputs.len() != self.seats {
                        self.fail(format!("turn {tick} carries {} seats", inputs.len()));
                        return;
                    }
                    self.ready.insert(tick, inputs);
                    if let Some((t, h)) = check {
                        self.theirs.insert(t, h);
                        self.compare();
                    }
                }
                (Role::Client, Recv::Msg(Msg::Lobby { view })) => {
                    if let Some(Session::Guest(g)) = &mut self.session {
                        g.view = view;
                    }
                    self.ended = true;
                    return;
                }
                (_, Recv::Msg(other)) => {
                    self.fail(format!("unexpected {other:?}"));
                    return;
                }
            }
        }
    }

    fn fail(&mut self, why: String) {
        match &mut self.session {
            Some(Session::Host(h)) => h.clients.iter_mut().for_each(|(_, c)| c.close(&why)),
            Some(Session::Guest(g)) => g.conn.close(&why),
            None => {}
        }
        self.closed.get_or_insert(why);
    }

    /// The next tick's bundle (every seat's input, seat order), once
    /// it exists. The returned tick is the lockstep's own counter.
    pub fn take_turn(&mut self) -> Option<(u64, Vec<PortInput>)> {
        let inputs = self.ready.remove(&self.next_tick)?;
        let tick = self.next_tick;
        self.next_tick += 1;
        Some((tick, inputs))
    }

    /// The pre-step state of lockstep tick `tick` — hashed (lazily)
    /// only on check ticks.
    pub fn note_hash(&mut self, tick: u64, hash: impl FnOnce() -> u64) {
        if tick % CHECK_EVERY != 0 || self.desync_seen {
            return;
        }
        let h = hash();
        if trace() {
            println!("net: tick {tick} hash {h:016x}");
        }
        match self.role {
            Role::Host => self.stamp = Some((tick, h)),
            Role::Client => {
                self.ours.insert(tick, h);
                self.compare();
            }
        }
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
            } else if !self.desync_seen {
                self.desync_seen = true;
                let msg = format!("DESYNC at tick {t}: host {theirs:016x}, here {ours:016x}");
                eprintln!("net: {msg}");
                self.desync = Some(msg);
            }
            // Everything up to a compared tick is settled.
            self.ours.retain(|&k, _| k > t);
            self.theirs.retain(|&k, _| k > t);
        }
    }

    /// A first desync, once (for the toast).
    pub fn take_desync(&mut self) -> Option<String> {
        self.desync.take()
    }

    fn summary(&self) {
        let checks = match self.role {
            Role::Host => String::new(),
            Role::Client => format!(", {} hash checks agreed", self.agreed),
        };
        let fate = match (&self.desync_seen, &self.closed) {
            (true, _) => " — DESYNCED".to_string(),
            (false, Some(why)) => format!(" — ended: {why}"),
            (false, None) => String::new(),
        };
        println!("net: {} ticks stepped{checks}{fate}", self.next_tick);
    }

    /// The game is over: hand the session back. The host reopens the
    /// lobby — same players and settings, every client approves Play
    /// again (ruling 10) — and tells the clients; a client waits for
    /// that in its lobby. The host keeps his lobby when a client left
    /// mid-game (without him); a client whose session closed gets
    /// `None`.
    pub fn end(mut self) -> Option<Session> {
        self.summary();
        let mut session = self.session.take()?;
        match &mut session {
            Session::Host(h) => {
                for id in std::mem::take(&mut self.gone) {
                    h.drop_client(id, "left");
                }
                h.core.reopen();
                h.dirty = true;
                h.flush();
            }
            Session::Guest(_) if self.closed.is_some() => return None,
            Session::Guest(_) => {}
        }
        Some(session)
    }
}

impl Drop for Lockstep {
    fn drop(&mut self) {
        if self.session.is_some() {
            self.summary();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Lockstep, Lockstep) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        drop(listener);
        let build = "test".to_string();
        let a = addr.clone();
        let b = build.clone();
        let host = std::thread::spawn(move || {
            host(
                &a,
                0,
                "lvl",
                serde_json::json!({"k": 1}),
                serde_json::json!({"t": 0}),
                "pw",
                &b,
            )
            .unwrap()
        });
        let client = join(&addr, Some(2), serde_json::json!({"t": 1}), "pw", &build).unwrap();
        let host = host.join().unwrap();
        assert_eq!(client.seats, vec![0, 2]);
        assert_eq!(client.local, Hx(1));
        assert_eq!(client.level, "lvl");
        assert_eq!(client.config, serde_json::json!({"k": 1}));
        assert_eq!(
            client.models,
            vec![serde_json::json!({"t": 0}), serde_json::json!({"t": 1})],
            "each seat's models travel"
        );
        (Lockstep::start(host), Lockstep::start(client))
    }

    fn input(stick_x: i16) -> PortInput {
        PortInput {
            stick_x,
            ..PortInput::default()
        }
    }

    /// Pump both peers until each has stepped `ticks` turns; seat k's
    /// input at tick t is `stick_x = 100*k + t` (lead-in idle).
    fn run(h: &mut Lockstep, c: &mut Lockstep, ticks: u64) -> Vec<Vec<Vec<i16>>> {
        let mut got = vec![Vec::new(), Vec::new()];
        let deadline = Instant::now() + Duration::from_secs(10);
        while got[0].len() < ticks as usize || got[1].len() < ticks as usize {
            assert!(Instant::now() < deadline, "lockstep stalled");
            for (k, p) in [&mut *h, &mut *c].into_iter().enumerate() {
                p.poll();
                assert!(p.closed().is_none(), "{:?}", p.closed());
                while p.wants_input() {
                    let t = p.next_send;
                    let x = if p.in_lead_in() {
                        0
                    } else {
                        (100 * k as u64 + t) as i16
                    };
                    p.submit(input(x));
                }
                if got[k].len() < ticks as usize
                    && let Some((t, inputs)) = p.take_turn()
                {
                    assert_eq!(t, got[k].len() as u64);
                    p.note_hash(t, || t * 7);
                    got[k].push(inputs.iter().map(|i| i.stick_x).collect());
                }
            }
            std::thread::yield_now();
        }
        got
    }

    #[test]
    fn both_peers_step_the_same_bundles() {
        let (mut h, mut c) = pair();
        let got = run(&mut h, &mut c, 100);
        assert_eq!(got[0], got[1]);
        for (t, turn) in got[0].iter().enumerate() {
            let t = t as i16;
            if (t as u64) < INPUT_DELAY {
                assert_eq!(turn, &vec![0, 0]);
            } else {
                assert_eq!(turn, &vec![t, 100 + t], "tick {t}");
            }
        }
        assert!(c.take_desync().is_none());
    }

    #[test]
    fn a_hash_mismatch_is_reported_once() {
        let (mut h, mut c) = pair();
        run(&mut h, &mut c, 30);
        assert!(c.take_desync().is_none());
        c.ours.insert(48, 1);
        c.theirs.insert(48, 2);
        c.compare();
        assert!(c.take_desync().unwrap().contains("tick 48"));
        c.ours.insert(72, 1);
        c.theirs.insert(72, 2);
        c.compare();
        assert!(c.take_desync().is_none());
    }

    /// A lobby host on a free loopback port.
    fn open_lobby(pass: &str) -> Host {
        Host::open(
            "127.0.0.1:0",
            0,
            "HOST".into(),
            "lvl".into(),
            serde_json::json!({"fog": 40}),
            pass.into(),
            "test".into(),
        )
        .unwrap()
    }

    /// Connect a guest while pumping the host (the host is polled, so
    /// the handshake needs both sides turning).
    fn guest(h: &mut Host, pass: &str, build: &str) -> Result<Guest, String> {
        let addr = h.addr();
        let (pass, build) = (pass.to_string(), build.to_string());
        let t = std::thread::spawn(move || {
            Guest::connect(&addr, &pass, "GUEST", &build, Duration::from_secs(5))
        });
        while !t.is_finished() {
            h.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        t.join().unwrap()
    }

    /// Pump both ends until `done` holds.
    fn settle(h: &mut Host, g: &mut Guest, done: impl Fn(&Host, &Guest) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(h, g) {
            assert!(Instant::now() < deadline, "lobby never settled");
            h.poll();
            g.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_handshake_checks_passphrase_and_build() {
        let mut h = open_lobby("secret");
        let e = guest(&mut h, "wrong", "test").err().unwrap();
        assert!(e.contains("wrong passphrase"), "{e}");
        let e = guest(&mut h, "secret", "other").err().unwrap();
        assert!(e.contains("build other"), "{e}");
        let g = guest(&mut h, "secret", "test").unwrap();
        assert_eq!(g.view().members.len(), 2);
        assert_eq!(g.view().me().unwrap().wizard, 1, "the first free wizard");
        assert!(!g.view().is_host());
    }

    /// Rulings 2, 3, 7: Play waits for the agreement; a host change
    /// voids it.
    #[test]
    fn play_waits_for_agreement_and_a_change_voids_it() {
        let mut h = open_lobby("pw");
        let mut g = guest(&mut h, "pw", "test").unwrap();
        assert!(!h.can_play());
        g.agree();
        settle(&mut h, &mut g, |h, _| h.can_play());
        h.set_rules(serde_json::json!({"fog": 90}));
        settle(&mut h, &mut g, |_, g| g.view().rev == 1);
        assert!(!h.can_play() && !g.view().all_agreed());
        assert_eq!(g.view().rules, serde_json::json!({"fog": 90}));
        g.agree();
        settle(&mut h, &mut g, |h, _| h.can_play());
    }

    /// Rulings 4 and 6: a kicked member is out, and a new passphrase
    /// keeps him out.
    #[test]
    fn a_kicked_member_cannot_return_after_a_passphrase_change() {
        let mut h = open_lobby("pw");
        let mut g = guest(&mut h, "pw", "test").unwrap();
        let id = g.view().you;
        h.kick(id);
        settle(&mut h, &mut g, |_, g| g.closed().is_some());
        assert!(g.closed().unwrap().contains("kicked"));
        assert_eq!(h.view().members.len(), 1);
        h.set_passphrase("new".into());
        assert!(guest(&mut h, "pw", "test").is_err());
        assert!(guest(&mut h, "new", "test").is_ok());
    }

    #[test]
    fn a_wizard_request_is_first_come_first_served() {
        let mut h = open_lobby("pw");
        let mut g = guest(&mut h, "pw", "test").unwrap();
        g.pick_wizard(0);
        g.pick_wizard(5);
        settle(&mut h, &mut g, |_, g| g.view().me().unwrap().wizard == 5);
        assert_eq!(h.view().seats(), vec![0, 5], "the host's 0 was refused");
        assert!(!h.pick_wizard(5));
    }

    /// Ruling 10: a game ends back in the lobby with the same players;
    /// Play needs a fresh agreement, and the second game runs; nobody
    /// joins while one is on.
    #[test]
    fn after_a_game_the_lobby_reopens_for_the_next() {
        let mut h = open_lobby("pw");
        let mut g = guest(&mut h, "pw", "test").unwrap();
        for round in 0..2 {
            g.agree();
            settle(&mut h, &mut g, |h, _| h.can_play());
            let hl = h.play().ok().unwrap();
            settle_start(&mut g);
            let gl = g.into_link().ok().unwrap();
            assert_eq!((hl.seats.clone(), gl.local), (vec![0, 1], Hx(1)));
            assert_eq!(gl.names, vec!["HOST", "GUEST"], "names travel");
            let (mut hs, mut cs) = (Lockstep::start(hl), Lockstep::start(gl));
            let got = run(&mut hs, &mut cs, 30);
            assert_eq!(got[0], got[1], "round {round}");
            if round == 0 {
                let addr = match &hs.session {
                    Some(Session::Host(h)) => h.addr(),
                    _ => unreachable!(),
                };
                let late = std::thread::spawn(move || {
                    Guest::connect(&addr, "pw", "", "test", Duration::from_secs(5))
                });
                while !late.is_finished() {
                    hs.poll();
                    std::thread::sleep(Duration::from_millis(2));
                }
                let e = late.join().unwrap().err().unwrap();
                assert!(e.contains("in progress"), "{e}");
            }
            let Some(Session::Host(back)) = hs.end() else {
                panic!("the host's lobby")
            };
            h = back;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !cs.ended() {
                assert!(Instant::now() < deadline);
                cs.poll();
            }
            let Some(Session::Guest(back)) = cs.end() else {
                panic!("the guest's lobby")
            };
            g = back;
            assert_eq!(h.view().members.len(), 2, "same players");
            assert!(!h.can_play(), "everyone approves again");
        }
    }

    fn settle_start(g: &mut Guest) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !g.started() {
            assert!(Instant::now() < deadline);
            g.poll();
        }
    }

    #[test]
    fn a_departing_peer_closes_the_session() {
        let (h, mut c) = pair();
        drop(h);
        let deadline = Instant::now() + Duration::from_secs(5);
        while c.closed().is_none() {
            assert!(Instant::now() < deadline);
            c.poll();
            std::thread::yield_now();
        }
        assert!(c.closed().unwrap().contains("peer left"));
    }
}
