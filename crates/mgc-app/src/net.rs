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

use mgc_formats::mgcr::PortInput;
use mgc_sim::Hx;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Bumped whenever [`Msg`] changes shape.
const PROTOCOL: u32 = 1;
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
    /// Client → host: who I am and which wizard seat I want (`None`:
    /// the host picks — seat 2, or seat 1 when the host flies another).
    Hello {
        protocol: u32,
        build: String,
        seat: Option<u8>,
    },
    /// Host → client: the session. `seats[k]` is human seat `Hx(k)`'s
    /// wizard colour (the host is `Hx(0)`); `config` is the host's
    /// sim-affecting configuration, adopted wholesale by the client.
    Welcome {
        seats: Vec<u8>,
        level: String,
        config: serde_json::Value,
    },
    Refuse {
        why: String,
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

/// A handshaken connection: the agreed session, before any world.
pub struct Link {
    stream: TcpStream,
    pub role: Role,
    /// Wizard colour per human seat, in seat order (`Hx(k)`).
    pub seats: Vec<u8>,
    /// This app's human seat.
    pub local: Hx,
    pub level: String,
    /// The host's sim-affecting config (see `Msg::Welcome`).
    pub config: serde_json::Value,
}

/// Host: wait on `addr` for one client, handshake, return the session.
/// A client that fails the handshake is refused and the next awaited.
pub fn host(
    addr: &str,
    seat: u8,
    level: &str,
    config: serde_json::Value,
    build: &str,
) -> Result<Link, String> {
    let listener = TcpListener::bind(addr).map_err(|e| format!("net: bind {addr}: {e}"))?;
    let at = listener
        .local_addr()
        .map_or_else(|_| addr.to_string(), |a| a.to_string());
    println!(
        "net: hosting on {at} as seat {} — waiting for --join",
        seat + 1
    );
    loop {
        let (mut stream, peer) = listener.accept().map_err(|e| format!("net: accept: {e}"))?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let refusal = match read_msg(&mut stream) {
            Ok(Msg::Hello {
                protocol,
                build: theirs,
                seat: s,
            }) => {
                let s = s.unwrap_or(u8::from(seat == 0));
                if protocol != PROTOCOL {
                    Some(format!("protocol {protocol}, host speaks {PROTOCOL}"))
                } else if theirs != build {
                    Some(format!("build {theirs}, host runs {build}"))
                } else if s > 7 || s == seat {
                    Some(format!("seat {} is not free", s + 1))
                } else {
                    let seats = vec![seat, s];
                    let welcome = Msg::Welcome {
                        seats: seats.clone(),
                        level: level.to_string(),
                        config: config.clone(),
                    };
                    if let Err(e) = write_msg(&mut stream, &welcome) {
                        eprintln!("net: {peer}: {e}");
                        continue;
                    }
                    let _ = stream.set_read_timeout(None);
                    println!("net: {peer} joined as seat {}", s + 1);
                    return Ok(Link {
                        stream,
                        role: Role::Host,
                        seats,
                        local: Hx::FIRST,
                        level: level.to_string(),
                        config,
                    });
                }
            }
            Ok(other) => Some(format!("expected hello, got {other:?}")),
            Err(e) => {
                eprintln!("net: {peer}: handshake: {e}");
                continue;
            }
        };
        if let Some(why) = refusal {
            eprintln!("net: refused {peer}: {why}");
            let _ = write_msg(&mut stream, &Msg::Refuse { why });
        }
    }
}

/// Client: connect to `addr` (retrying while the host comes up) and
/// ask for wizard seat `seat` (`None`: whichever the host assigns).
pub fn join(addr: &str, seat: Option<u8>, build: &str) -> Result<Link, String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut said = false;
    let mut stream = loop {
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
    let _ = stream.set_nodelay(true);
    let hello = Msg::Hello {
        protocol: PROTOCOL,
        build: build.to_string(),
        seat,
    };
    write_msg(&mut stream, &hello).map_err(|e| format!("net: {addr}: {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    match read_msg(&mut stream).map_err(|e| format!("net: {addr}: {e}"))? {
        Msg::Welcome {
            seats,
            level,
            config,
        } => {
            let _ = stream.set_read_timeout(None);
            // Two seats: the host is `Hx(0)`, we are the other.
            let local = 1;
            if seats.len() != 2 || seat.is_some_and(|s| s != seats[local]) {
                return Err(format!("net: the host seated us wrongly ({seats:?})"));
            }
            println!(
                "net: joined {addr} as seat {} — level {level}",
                seats[local] + 1
            );
            Ok(Link {
                stream,
                role: Role::Client,
                seats,
                local: Hx(local as u8),
                level,
                config,
            })
        }
        Msg::Refuse { why } => Err(format!("net: the host refused: {why}")),
        other => Err(format!("net: expected welcome, got {other:?}")),
    }
}

/// The running lockstep: turn assembly (host), turn intake (client),
/// the hash check. Non-blocking — the app polls it once per sim tick.
pub struct Lockstep {
    role: Role,
    local: Hx,
    seats: usize,
    /// The agreed sim-affecting config (`Link::config`).
    config: serde_json::Value,
    tx: TcpStream,
    rx: mpsc::Receiver<std::io::Result<Msg>>,
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
}

impl Lockstep {
    pub fn start(link: Link) -> Self {
        let (tx_msgs, rx) = mpsc::channel();
        let mut reader = link
            .stream
            .try_clone()
            .expect("a TCP stream clones its handle");
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
            role: link.role,
            local: link.local,
            seats: link.seats.len(),
            config: link.config,
            tx: link.stream,
            rx,
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
            closed: None,
        }
    }

    pub fn seats(&self) -> usize {
        self.seats
    }

    /// The session's agreed sim-affecting config.
    pub fn config(&self) -> &serde_json::Value {
        &self.config
    }

    /// Why the session ended, once it has (peer left, socket error).
    pub fn closed(&self) -> Option<&str> {
        self.closed.as_deref()
    }

    /// Is this seat's input for a further tick due? The first
    /// [`INPUT_DELAY`] ticks are lead-in (see [`Self::in_lead_in`]).
    pub fn wants_input(&self) -> bool {
        self.closed.is_none() && self.next_send < self.next_tick + INPUT_DELAY
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
    /// goes out to the client and onto the local ready queue.
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

    fn send(&mut self, m: &Msg) {
        if self.closed.is_some() {
            return;
        }
        if let Err(e) = write_msg(&mut self.tx, m) {
            self.closed = Some(format!("send: {e}"));
        }
    }

    /// Drain everything the peer has sent.
    pub fn poll(&mut self) {
        loop {
            let m = match self.rx.try_recv() {
                Ok(m) => m,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.closed.get_or_insert_with(|| "connection lost".into());
                    return;
                }
            };
            match (self.role, m) {
                (_, Err(e)) => {
                    self.closed
                        .get_or_insert_with(|| format!("connection lost: {e}"));
                    return;
                }
                (_, Ok(Msg::Bye { why })) => {
                    self.closed
                        .get_or_insert_with(|| format!("peer left ({why})"));
                    return;
                }
                (Role::Host, Ok(Msg::Input { tick, input })) => {
                    if tick < self.next_complete {
                        self.fail(format!(
                            "client input for tick {tick} arrived after its turn"
                        ));
                        return;
                    }
                    // Two seats: the client is the one seat that is not ours.
                    let ix = usize::from(self.local.0 == 0);
                    self.slot(tick)[ix] = Some(input);
                    self.complete();
                }
                (
                    Role::Client,
                    Ok(Msg::Turn {
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
                (_, Ok(other)) => {
                    self.fail(format!("unexpected {other:?}"));
                    return;
                }
            }
        }
    }

    fn fail(&mut self, why: String) {
        self.send(&Msg::Bye { why: why.clone() });
        self.closed.get_or_insert(why);
        let _ = self.tx.shutdown(Shutdown::Both);
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
}

impl Drop for Lockstep {
    fn drop(&mut self) {
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
        if self.closed.is_none() {
            let _ = write_msg(
                &mut self.tx,
                &Msg::Bye {
                    why: "closed".into(),
                },
            );
        }
        let _ = self.tx.shutdown(Shutdown::Both);
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
            host(&a, 0, "lvl", serde_json::json!({"k": 1}), &b).unwrap()
        });
        let client = join(&addr, Some(2), &build).unwrap();
        let host = host.join().unwrap();
        assert_eq!(client.seats, vec![0, 2]);
        assert_eq!(client.local, Hx(1));
        assert_eq!(client.level, "lvl");
        assert_eq!(client.config, serde_json::json!({"k": 1}));
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
