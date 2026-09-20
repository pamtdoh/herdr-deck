//! Host connections. Every host pushes its own agent list; the panel shows them all, one after another.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use pixbar_proto::{decode_to_device, encode, Action, AgentState, FromDevice, LineBuffer, ToDevice, BEACON_PORT, BEACON_PREFIX, PROTO};
use pixbar_render::World;

/// Hosts resend their state every couple of seconds; silence this long means the link is dead.
const HOST_TIMEOUT: Duration = Duration::from_secs(10);

struct Host {
    stream: TcpStream,
    lines: LineBuffer,
    name: String,
    /// Came in through adbd's forward, i.e. over the cable.
    wired: bool,
    agents: Vec<AgentState>,
    focused: Option<String>,
    /// Bumped whenever this host's focus moves, so the most recently used machine wins.
    focus_stamp: u64,
    last_heard: Instant,
}

pub struct Hosts {
    listener: TcpListener,
    hosts: Vec<Host>,
    clock: u64,
    beacon: Option<UdpSocket>,
    last_beacon: Instant,
    port: u16,
    refresh_ms: u32,
    /// FNV-1a over our own executable, the same the bridge computes over the copy it carries.
    build: String,
    mac: String,
    boot: String,
}

impl Hosts {
    pub fn listen(port: u16) -> io::Result<Hosts> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        listener.set_nonblocking(true)?;
        let beacon = UdpSocket::bind(("0.0.0.0", 0)).and_then(|s| s.set_broadcast(true).map(|_| s)).ok();
        let exe = std::fs::read("/proc/self/exe").unwrap_or_default();
        let build = format!("{:016x}", exe.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)));
        let mac = std::fs::read_to_string("/sys/class/net/wlan0/address").unwrap_or_default();
        let mac = mac.chars().filter(char::is_ascii_hexdigit).collect::<String>().to_lowercase();
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default().trim().to_string();
        Ok(Hosts { listener, hosts: Vec::new(), clock: 0, beacon, last_beacon: Instant::now(), port, refresh_ms: 1000, build, mac, boot })
    }

    /// The refresh setting lives on the panel but is the hosts' to carry out.
    pub fn set_refresh(&mut self, refresh_ms: u32) {
        self.refresh_ms = refresh_ms;
        let line = encode(&FromDevice::Config { refresh_ms });
        for host in &mut self.hosts {
            let _ = send(&mut host.stream, &line);
        }
    }

    /// The panel goes back to Ulanzi's app because its user said so; hosts must not undo that.
    pub fn announce_stock(&mut self) {
        let line = encode(&FromDevice::Stock);
        for host in &mut self.hosts {
            let _ = send(&mut host.stream, &line);
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.hosts.iter().map(|h| format!("{} {}", h.name, if h.wired { "USB" } else { "WIFI" })).collect()
    }

    pub fn connected(&self) -> usize {
        self.hosts.len()
    }

    /// Accepts, reads and expires connections. Returns true when the merged world may have changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((stream, addr)) = self.listener.accept() {
            if stream.set_nonblocking(true).is_err() {
                continue;
            }
            let _ = stream.set_nodelay(true);
            eprintln!("host connected: {addr}");
            let mut host = Host {
                stream,
                lines: LineBuffer::default(),
                name: addr.to_string(),
                wired: addr.ip().is_loopback(),
                agents: Vec::new(),
                focused: None,
                focus_stamp: 0,
                last_heard: Instant::now(),
            };
            let hello = FromDevice::Hello { version: env!("CARGO_PKG_VERSION").into(), build: self.build.clone(), mac: self.mac.clone(), boot: self.boot.clone(), proto: PROTO };
            let config = FromDevice::Config { refresh_ms: self.refresh_ms };
            if send(&mut host.stream, &(encode(&hello) + &encode(&config))).is_ok() {
                self.hosts.push(host);
                changed = true;
            }
        }

        let mut buf = [0u8; 8192];
        let mut dead = Vec::new();
        for (i, host) in self.hosts.iter_mut().enumerate() {
            loop {
                match host.stream.read(&mut buf) {
                    Ok(0) => {
                        dead.push(i);
                        break;
                    }
                    Ok(n) => {
                        host.lines.push(&buf[..n]);
                        host.last_heard = Instant::now();
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        dead.push(i);
                        break;
                    }
                }
            }
            while let Some(line) = host.lines.next_line() {
                match decode_to_device(&line) {
                    Ok(ToDevice::Hello { host: name, proto }) => {
                        if proto != PROTO {
                            eprintln!("{name} speaks protocol {proto}, this program {PROTO}: what one does not know of the other is left out");
                        }
                        host.name = name;
                        changed = true;
                    }
                    Ok(ToDevice::State { agents, focused }) => {
                        if focused != host.focused {
                            self.clock += 1;
                            host.focus_stamp = self.clock;
                        }
                        changed |= agents != host.agents || focused != host.focused;
                        host.agents = agents;
                        host.focused = focused;
                    }
                    Err(e) => eprintln!("bad line from {}: {e}", host.name),
                }
            }
            if host.last_heard.elapsed() > HOST_TIMEOUT && !dead.contains(&i) {
                dead.push(i);
            }
        }
        for i in dead.into_iter().rev() {
            eprintln!("host gone: {}", self.hosts[i].name);
            self.hosts.remove(i);
            changed = true;
        }

        if self.last_beacon.elapsed() >= Duration::from_secs(1) {
            self.last_beacon = Instant::now();
            if let Some(sock) = &self.beacon {
                let _ = sock.send_to(format!("{BEACON_PREFIX} {} {}", self.port, self.mac).as_bytes(), ("255.255.255.255", BEACON_PORT));
            }
        }
        changed
    }

    /// All hosts' agents in connection order, and for each list position who owns it.
    pub fn world(&self) -> (World, Vec<(usize, String)>) {
        let mut world = World::default();
        let mut owners = Vec::new();
        let mut best = 0;
        for (h, host) in self.hosts.iter().enumerate() {
            for a in &host.agents {
                if host.focused.as_deref() == Some(a.id.as_str()) && host.focus_stamp >= best {
                    best = host.focus_stamp;
                    world.focused = world.agents.len();
                }
                world.agents.push(a.agent.clone());
                owners.push((h, a.id.clone()));
            }
        }
        (world, owners)
    }

    pub fn send_intent(&mut self, owner: &(usize, String), action: Action) {
        if let Some(host) = self.hosts.get_mut(owner.0) {
            let msg = FromDevice::Intent { id: owner.1.clone(), action };
            if let Err(e) = send(&mut host.stream, &encode(&msg)) {
                eprintln!("send to {} failed: {e}", host.name);
            }
        }
    }
}

/// The socket is non-blocking for reads; lines are tiny, so a full send buffer is only ever momentary.
fn send(stream: &mut TcpStream, line: &str) -> io::Result<()> {
    let mut bytes = line.as_bytes();
    let deadline = Instant::now() + Duration::from_millis(200);
    while !bytes.is_empty() {
        match stream.write(bytes) {
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
