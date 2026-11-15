//! Port forwarding on the router (#651): while a session is hosted, its ports are forwarded to this computer by the
//! home router itself, over UPnP's Internet Gateway Device protocol (`igd`), so someone outside the network can join
//! without anyone opening the router's settings.
//!
//! A [`PortForward`] asks on a thread of its own (finding the router takes up to a few seconds), keeps the forwarding
//! alive by renewing its lease, and takes it back when dropped. What came of it ([`Forwarding`]) says where others
//! join from outside, or why they cannot and what to do instead.

mod igd;

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::transport::Waker;
use igd::Gateway;

/// How long a forwarding is asked for, in seconds; it is renewed at half of it. Not forever, so a Studio that quits
/// without taking it back leaves the port open an hour at most.
const LEASE: u32 = 3600;

/// How long finding the router may take.
const SEARCH: Duration = Duration::from_secs(4);

/// How long dropping a [`PortForward`] waits for the router to take the forwarding back.
const RELEASE_WAIT: Duration = Duration::from_secs(2);

/// What came of asking the router to forward the ports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Forwarding {
    /// Still asking.
    Asking,
    /// Forwarded: people outside the network join at `public`.
    Forwarded {
        /// The router's address on the internet.
        public: IpAddr,
    },
    /// Forwarded, but the router's own outside address is not a public one: the internet provider (carrier-grade
    /// NAT) or a second router stands between, so people outside still cannot reach this computer.
    Shared {
        /// The router's outside address.
        router: IpAddr,
    },
    /// Not forwarded, and why, in a few words.
    Failed(String),
}

/// The session's ports forwarded on the router while this lives (see the module documentation).
pub struct PortForward {
    state: Arc<Mutex<Forwarding>>,
    stop: Arc<AtomicBool>,
    released: mpsc::Receiver<()>,
}

impl PortForward {
    /// Starts asking the router on this network to forward the TCP `ports` (each to the same port here), labelled
    /// `description` in the router's list; `waker` is called when the answer comes.
    pub fn start(ports: Vec<u16>, description: String, waker: Waker) -> Result<Self, String> {
        Self::start_with(move || igd::search(SEARCH), ports, description, waker)
    }

    /// [`PortForward::start`] with the router found by `find`.
    fn start_with(
        find: impl FnOnce() -> Result<Gateway, String> + Send + 'static,
        ports: Vec<u16>,
        description: String,
        waker: Waker,
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(Forwarding::Asking));
        let stop = Arc::new(AtomicBool::new(false));
        let (done, released) = mpsc::channel();
        let (thread_state, thread_stop) = (Arc::clone(&state), Arc::clone(&stop));
        thread::Builder::new()
            .name("metallic-port-forward".to_owned())
            .spawn(move || {
                run(find, &ports, &description, &thread_state, &thread_stop, &waker);
                let _ = done.send(());
            })
            .map_err(|error| format!("could not start the port forwarding thread: {error}"))?;
        Ok(Self { state, stop, released })
    }

    /// What has come of it so far.
    pub fn state(&self) -> Forwarding {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Drop for PortForward {
    /// Takes the forwarding back: waits a moment for the router when there is one to take back, so a Studio that
    /// quits right after does not leave the ports open.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if matches!(self.state(), Forwarding::Forwarded { .. } | Forwarding::Shared { .. }) {
            let _ = self.released.recv_timeout(RELEASE_WAIT);
        }
    }
}

fn set(state: &Mutex<Forwarding>, value: Forwarding, waker: &Waker) {
    *state.lock().unwrap_or_else(PoisonError::into_inner) = value;
    waker();
}

/// The thread: find the router, forward, renew until told to stop, take it back.
fn run(
    find: impl FnOnce() -> Result<Gateway, String>,
    ports: &[u16],
    description: &str,
    state: &Mutex<Forwarding>,
    stop: &AtomicBool,
    waker: &Waker,
) {
    let gateway = match find() {
        Ok(gateway) => gateway,
        Err(why) => {
            log::info!("team create: no UPnP router: {why}");
            return set(state, Forwarding::Failed(why), waker);
        }
    };
    if stop.load(Ordering::Relaxed) {
        return;
    }
    let outcome = local_address(gateway.addr).and_then(|local| {
        let public = gateway
            .external_ip()
            .map_err(|error| format!("the router did not say its outside address ({error})"))?;
        let lease = forward_all(&gateway, ports, local, description)?;
        Ok((public, local, lease))
    });
    let (public, local, lease) = match outcome {
        Ok(found) => found,
        Err(why) => {
            log::warn!("team create: port forwarding failed: {why}");
            return set(state, Forwarding::Failed(why), waker);
        }
    };
    log::info!("team create: the router forwards ports {ports:?} to {local}; its outside address is {public}");
    let answer = if is_public(public) {
        Forwarding::Forwarded { public }
    } else {
        Forwarding::Shared { router: public }
    };
    set(state, answer, waker);
    // Renewed at half the lease (a permanent one needs none), until told to stop.
    let mut renewed = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(100));
        if lease > 0 && renewed.elapsed() >= Duration::from_secs(u64::from(lease / 2)) {
            if let Err(why) = forward_all(&gateway, ports, local, description) {
                log::warn!("team create: renewing the port forwarding failed: {why}");
            }
            renewed = Instant::now();
        }
    }
    for &port in ports {
        if let Err(error) = gateway.remove_port(port) {
            log::info!("team create: taking back the forwarding of port {port}: {error}");
        }
    }
}

/// This computer's address on the network the router is on: the one a packet to it would leave from.
fn local_address(router: SocketAddr) -> Result<IpAddr, String> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|error| format!("no network address ({error})"))?;
    socket
        .connect(router)
        .and_then(|()| socket.local_addr())
        .map(|addr| addr.ip())
        .map_err(|error| format!("no route to the router ({error})"))
}

/// Forwards each of `ports` to the same port at `local`; returns the lease the router took (0: permanent).
fn forward_all(gateway: &Gateway, ports: &[u16], local: IpAddr, description: &str) -> Result<u32, String> {
    let mut lease = LEASE;
    for &port in ports {
        let mut result = gateway.add_port(port, local, lease, description);
        if result == Err(igd::Error::Upnp(igd::ONLY_PERMANENT)) {
            lease = 0;
            result = gateway.add_port(port, local, lease, description);
        }
        result.map_err(|error| match error {
            igd::Error::Upnp(igd::CONFLICT) => format!("the router already forwards port {port} to another computer"),
            igd::Error::Upnp(igd::NOT_AUTHORIZED) => "the router does not let programs forward ports".to_owned(),
            other => format!("the router refused to forward port {port} ({other})"),
        })?;
    }
    Ok(lease)
}

/// Whether `address` is reachable from the internet at large: not private, shared (carrier-grade NAT), loopback,
/// link-local or unspecified.
pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            let shared = a == 100 && (64..128).contains(&b);
            !(v4.is_private() || shared || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified())
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            let unique_local = first & 0xfe00 == 0xfc00;
            let link_local = first & 0xffc0 == 0xfe80;
            !(v6.is_loopback() || v6.is_unspecified() || unique_local || link_local)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::igd::tests::FakeRouter;
    use super::*;
    use crate::transport::no_waker;

    #[test]
    fn only_addresses_the_internet_can_reach_are_public() {
        for public in ["203.0.113.5", "8.8.8.8", "100.128.0.1", "2001:db8::1"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
        for shared in [
            "10.0.0.1",
            "172.16.4.2",
            "192.168.1.1",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "fd00::1",
            "fe80::1",
            "::1",
        ] {
            assert!(!is_public(shared.parse().unwrap()), "{shared}");
        }
    }

    fn settled(forward: &PortForward) -> Forwarding {
        let start = Instant::now();
        while forward.state() == Forwarding::Asking {
            assert!(start.elapsed() < Duration::from_secs(10), "no answer");
            thread::sleep(Duration::from_millis(5));
        }
        forward.state()
    }

    #[test]
    fn the_ports_are_forwarded_while_it_lives_and_taken_back_when_dropped() {
        let router = FakeRouter::start("203.0.113.5", None);
        let gateway = router.gateway();
        let forward =
            PortForward::start_with(move || Ok(gateway), vec![7350, 7351], "Metallic".into(), no_waker()).unwrap();
        assert_eq!(
            settled(&forward),
            Forwarding::Forwarded {
                public: "203.0.113.5".parse().unwrap()
            }
        );
        let ports: Vec<u16> = router
            .forwarded
            .lock()
            .unwrap()
            .iter()
            .map(|(port, _, _)| *port)
            .collect();
        assert_eq!(ports, [7350, 7351]);
        drop(forward);
        assert!(router.forwarded.lock().unwrap().is_empty(), "taken back");
    }

    #[test]
    fn a_shared_outside_address_and_a_refusal_are_told_apart() {
        let router = FakeRouter::start("100.72.1.9", None);
        let gateway = router.gateway();
        let forward = PortForward::start_with(move || Ok(gateway), vec![7350], "Metallic".into(), no_waker()).unwrap();
        assert_eq!(
            settled(&forward),
            Forwarding::Shared {
                router: "100.72.1.9".parse().unwrap()
            }
        );

        let refusing = FakeRouter::start("203.0.113.5", Some(igd::CONFLICT));
        let gateway = refusing.gateway();
        let forward = PortForward::start_with(move || Ok(gateway), vec![7350], "Metallic".into(), no_waker()).unwrap();
        assert_eq!(
            settled(&forward),
            Forwarding::Failed("the router already forwards port 7350 to another computer".to_owned())
        );

        let forward = PortForward::start_with(
            || Err("no router answered".into()),
            vec![7350],
            "Metallic".into(),
            no_waker(),
        )
        .unwrap();
        assert_eq!(settled(&forward), Forwarding::Failed("no router answered".to_owned()));
    }
}
