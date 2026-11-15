//! A small UPnP Internet Gateway Device client (#651): just what forwarding a port needs, over `std::net`. Finds the
//! router with an SSDP search, reads its description for the WAN connection service, and calls that service's
//! `AddPortMapping`, `DeletePortMapping` and `GetExternalIPAddress` over SOAP (UPnP IGD 1.0 and 2.0).
//!
//! The router is trusted only as far as it must be: its description and control addresses must be on the address
//! that answered the search, and every reply is read with a size limit and a timeout.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

/// Where SSDP searches go.
const SSDP: &str = "239.255.255.250:1900";

/// What a search asks for: the device, then its two kinds of WAN connection service.
const SEARCH_TARGETS: [&str; 3] = [
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
];

/// How long one HTTP exchange with the router may take.
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);

/// The most of a reply that is read.
const MAX_REPLY: u64 = 256 * 1024;

/// UPnP error 718: the port is already forwarded to another computer.
pub(super) const CONFLICT: u16 = 718;
/// UPnP error 725: the router takes only permanent forwardings.
pub(super) const ONLY_PERMANENT: u16 = 725;
/// UPnP error 606: the router does not let programs forward ports.
pub(super) const NOT_AUTHORIZED: u16 = 606;

/// Why a call to the router failed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    /// The router answered with a UPnP error code.
    Upnp(u16),
    /// Anything else, in a few words.
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Upnp(code) => write!(f, "UPnP error {code}"),
            Error::Other(why) => f.write_str(why),
        }
    }
}

/// A router's WAN connection service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Gateway {
    /// The router's HTTP address.
    pub addr: SocketAddr,
    /// The service's control path there.
    pub control: String,
    /// The service type, as the router named it (`urn:schemas-upnp-org:service:WANIPConnection:1` or similar).
    pub service: String,
}

/// Finds the router on this network within `timeout`.
pub(super) fn search(timeout: Duration) -> Result<Gateway, String> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|error| format!("no network ({error})"))?;
    for target in SEARCH_TARGETS {
        let request =
            format!("M-SEARCH * HTTP/1.1\r\nHOST: {SSDP}\r\nST: {target}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\n\r\n");
        socket
            .send_to(request.as_bytes(), SSDP)
            .map_err(|error| format!("could not search the network ({error})"))?;
    }
    let deadline = Instant::now() + timeout;
    let mut buffer = [0u8; 2048];
    let mut tried = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
        socket.set_read_timeout(Some(left)).map_err(|error| error.to_string())?;
        let Ok((length, from)) = socket.recv_from(&mut buffer) else {
            break;
        };
        let Some(location) = ssdp_location(&String::from_utf8_lossy(&buffer[..length])) else {
            continue;
        };
        if tried.contains(&location) {
            continue;
        }
        tried.push(location.clone());
        match describe(&location, from.ip()) {
            Ok(gateway) => return Ok(gateway),
            Err(why) => log::info!("team create: {location} is not a usable router: {why}"),
        }
    }
    Err("no router on this network answered: UPnP may be turned off on it".to_owned())
}

/// The `LOCATION` header of an SSDP reply: where the device describes itself.
fn ssdp_location(reply: &str) -> Option<String> {
    reply.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("location")
            .then(|| value.trim().to_owned())
    })
}

/// Reads the description at `location` (which must be on `responder`) for its WAN connection service.
fn describe(location: &str, responder: IpAddr) -> Result<Gateway, String> {
    let (addr, path) = parse_url(location)?;
    if addr.ip() != responder {
        return Err(format!("its description is on another address ({})", addr.ip()));
    }
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (status, body) = exchange(addr, request.as_bytes())?;
    if status != 200 {
        return Err(format!("its description answered {status}"));
    }
    let (service, control) = wan_service(&body).ok_or("it has no WAN connection service")?;
    let control = if control.starts_with("http://") {
        let (at, path) = parse_url(&control)?;
        if at.ip() != responder {
            return Err(format!("its control address is on another address ({})", at.ip()));
        }
        path
    } else if control.starts_with('/') {
        control
    } else {
        format!("/{control}")
    };
    Ok(Gateway { addr, control, service })
}

/// The address and path of an `http://host:port/path` URL whose host is an IP address.
fn parse_url(url: &str) -> Result<(SocketAddr, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("not an http address: {url}"))?;
    let (host, path) = match rest.find('/') {
        Some(at) => (&rest[..at], rest[at..].to_owned()),
        None => (rest, "/".to_owned()),
    };
    let addr = match host.parse::<SocketAddr>() {
        Ok(addr) => addr,
        Err(_) => SocketAddr::new(
            host.trim_start_matches('[')
                .trim_end_matches(']')
                .parse()
                .map_err(|_| format!("not an IP address: {host}"))?,
            80,
        ),
    };
    Ok((addr, path))
}

/// The first WAN IP or PPP connection service in a device description: its type and control URL.
fn wan_service(description: &str) -> Option<(String, String)> {
    let mut rest = description;
    while let Some(start) = rest.find("<service>") {
        let block = &rest[start..];
        let end = block.find("</service>")?;
        let service = &block[..end];
        rest = &block[end..];
        let Some(kind) = tag(service, "serviceType") else {
            continue;
        };
        if kind.contains(":WANIPConnection:") || kind.contains(":WANPPPConnection:") {
            return Some((kind.to_owned(), tag(service, "controlURL")?.to_owned()));
        }
    }
    None
}

/// The trimmed text of the first `<name>` element in `xml` (with or without a namespace prefix on the name).
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = xml.match_indices('<').map(|(at, _)| at).find(|&at| {
        let after = &xml[at + 1..];
        let local = after
            .split_once(':')
            .filter(|(prefix, _)| !prefix.contains(['>', ' ', '/']));
        let name_part = local.map_or(after, |(_, rest)| rest);
        name_part.starts_with(name) && name_part[name.len()..].starts_with(['>', ' '])
    })?;
    let body_start = open + xml[open..].find('>')? + 1;
    let body_end = body_start + xml[body_start..].find("</")?;
    Some(xml[body_start..body_end].trim())
}

/// Sends `request` to `addr` and reads the whole reply (the request asks the router to close the connection): its
/// status and body.
fn exchange(addr: SocketAddr, request: &[u8]) -> Result<(u16, String), String> {
    let mut stream = TcpStream::connect_timeout(&addr, HTTP_TIMEOUT)
        .map_err(|error| format!("could not reach the router ({error})"))?;
    stream
        .set_read_timeout(Some(HTTP_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(HTTP_TIMEOUT)))
        .and_then(|()| stream.write_all(request))
        .map_err(|error| format!("could not ask the router ({error})"))?;
    let mut reply = Vec::new();
    stream
        .take(MAX_REPLY)
        .read_to_end(&mut reply)
        .map_err(|error| format!("the router did not answer ({error})"))?;
    parse_reply(&reply)
}

/// An HTTP reply's status and body (de-chunked).
fn parse_reply(reply: &[u8]) -> Result<(u16, String), String> {
    let text = String::from_utf8_lossy(reply);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or("the router's answer had no end of headers")?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or("the router's answer had no status")?;
    let chunked = head.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("transfer-encoding") && value.to_ascii_lowercase().contains("chunked")
        })
    });
    let body = if chunked { dechunk(body)? } else { body.to_owned() };
    Ok((status, body))
}

/// A chunked body joined back together.
fn dechunk(mut body: &str) -> Result<String, String> {
    let mut out = String::new();
    loop {
        let (size, rest) = body.split_once("\r\n").ok_or("a broken chunked answer")?;
        let size = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "a broken chunked answer")?;
        if size == 0 {
            return Ok(out);
        }
        let chunk = rest.get(..size).ok_or("a short chunked answer")?;
        out.push_str(chunk);
        body = rest.get(size..).unwrap_or("").trim_start_matches("\r\n");
    }
}

/// `text` with XML's special characters escaped.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Gateway {
    /// Calls the service's `action` with `args`; its reply's body.
    fn call(&self, action: &str, args: &[(&str, String)]) -> Result<String, Error> {
        let mut fields = String::new();
        for (name, value) in args {
            fields.push_str(&format!("<{name}>{}</{name}>", escape(value)));
        }
        let envelope = format!(
            "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
             s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">\
             {fields}</u:{action}></s:Body></s:Envelope>",
            self.service
        );
        let request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{}#{action}\"\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{envelope}",
            self.control,
            self.addr,
            self.service,
            envelope.len()
        );
        let (status, body) = exchange(self.addr, request.as_bytes()).map_err(Error::Other)?;
        if status == 200 {
            return Ok(body);
        }
        match tag(&body, "errorCode").and_then(|code| code.parse().ok()) {
            Some(code) => Err(Error::Upnp(code)),
            None => Err(Error::Other(format!("the router answered {status}"))),
        }
    }

    /// The router's address outside, on the internet side.
    pub(super) fn external_ip(&self) -> Result<IpAddr, Error> {
        let body = self.call("GetExternalIPAddress", &[])?;
        tag(&body, "NewExternalIPAddress")
            .and_then(|ip| ip.parse().ok())
            .ok_or_else(|| Error::Other("the router did not say its outside address".to_owned()))
    }

    /// Forwards TCP `port` outside to the same port at `local`, for `lease` seconds (0: until taken back).
    pub(super) fn add_port(&self, port: u16, local: IpAddr, lease: u32, description: &str) -> Result<(), Error> {
        self.call(
            "AddPortMapping",
            &[
                ("NewRemoteHost", String::new()),
                ("NewExternalPort", port.to_string()),
                ("NewProtocol", "TCP".to_owned()),
                ("NewInternalPort", port.to_string()),
                ("NewInternalClient", local.to_string()),
                ("NewEnabled", "1".to_owned()),
                ("NewPortMappingDescription", description.to_owned()),
                ("NewLeaseDuration", lease.to_string()),
            ],
        )
        .map(drop)
    }

    /// Takes back the forwarding of TCP `port`.
    pub(super) fn remove_port(&self, port: u16) -> Result<(), Error> {
        self.call(
            "DeletePortMapping",
            &[
                ("NewRemoteHost", String::new()),
                ("NewExternalPort", port.to_string()),
                ("NewProtocol", "TCP".to_owned()),
            ],
        )
        .map(drop)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use super::*;

    const DESCRIPTION: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0"><device>
  <deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
  <serviceList><service>
    <serviceType>urn:schemas-upnp-org:service:Layer3Forwarding:1</serviceType>
    <controlURL>/ctl/L3F</controlURL>
  </service></serviceList>
  <deviceList><device><deviceList><device><serviceList>
    <service>
      <serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>
      <serviceId>urn:upnp-org:serviceId:WANIPConn1</serviceId>
      <controlURL>/ctl/IPConn</controlURL>
    </service>
  </serviceList></device></deviceList></device></deviceList>
</device></root>"#;

    #[test]
    fn the_wan_service_and_its_control_path_are_found() {
        assert_eq!(
            wan_service(DESCRIPTION),
            Some((
                "urn:schemas-upnp-org:service:WANIPConnection:1".to_owned(),
                "/ctl/IPConn".to_owned()
            ))
        );
        assert_eq!(
            wan_service("<root><service><serviceType>x</serviceType></service></root>"),
            None
        );
    }

    #[test]
    fn replies_and_urls_are_read() {
        let ssdp =
            "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nLocation: http://192.168.1.1:5000/rootDesc.xml\r\n\r\n";
        assert_eq!(
            ssdp_location(ssdp).as_deref(),
            Some("http://192.168.1.1:5000/rootDesc.xml")
        );
        assert_eq!(
            parse_url("http://192.168.1.1:5000/rootDesc.xml"),
            Ok(("192.168.1.1:5000".parse().unwrap(), "/rootDesc.xml".to_owned()))
        );
        assert_eq!(
            parse_url("http://10.0.0.1"),
            Ok(("10.0.0.1:80".parse().unwrap(), "/".to_owned()))
        );
        assert!(parse_url("https://10.0.0.1/").is_err());
        assert!(parse_url("http://router.local/").is_err(), "names are not looked up");
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(parse_reply(chunked), Ok((200, "hello world".to_owned())));
        let fault = "<s:Envelope><s:Body><s:Fault><detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\">\
                     <errorCode>718</errorCode><errorDescription>ConflictInMappingEntry</errorDescription>\
                     </UPnPError></detail></s:Fault></s:Body></s:Envelope>";
        assert_eq!(tag(fault, "errorCode"), Some("718"));
        assert_eq!(
            tag(
                "<u:Reply><NewExternalIPAddress> 203.0.113.5 </NewExternalIPAddress>",
                "NewExternalIPAddress"
            ),
            Some("203.0.113.5")
        );
    }

    /// A pretend router on localhost: serves `DESCRIPTION` and answers the WAN service's calls, keeping the
    /// forwardings it was asked for; `refuse` makes AddPortMapping fail with that UPnP error.
    pub(in crate::forward) struct FakeRouter {
        pub addr: SocketAddr,
        pub forwarded: Arc<Mutex<Vec<(u16, String, u32)>>>,
    }

    impl FakeRouter {
        pub(in crate::forward) fn start(external: &'static str, refuse: Option<u16>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let forwarded = Arc::new(Mutex::new(Vec::new()));
            let kept = Arc::clone(&forwarded);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 4096];
                    // Read the head, then the body by its length.
                    loop {
                        let n = stream.read(&mut buffer).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..n]);
                        let text = String::from_utf8_lossy(&request);
                        if let Some((head, body)) = text.split_once("\r\n\r\n") {
                            let length = head
                                .lines()
                                .find_map(|l| l.strip_prefix("Content-Length: "))
                                .and_then(|n| n.parse::<usize>().ok())
                                .unwrap_or(0);
                            if body.len() >= length {
                                break;
                            }
                        }
                    }
                    let text = String::from_utf8_lossy(&request).into_owned();
                    let (status, body) = if text.starts_with("GET /rootDesc.xml") {
                        (200, DESCRIPTION.to_owned())
                    } else if text.contains("#GetExternalIPAddress") {
                        (
                            200,
                            format!("<u:R><NewExternalIPAddress>{external}</NewExternalIPAddress></u:R>"),
                        )
                    } else if text.contains("#AddPortMapping") {
                        match refuse {
                            Some(code) => (500, format!("<UPnPError><errorCode>{code}</errorCode></UPnPError>")),
                            None => {
                                let port = tag(&text, "NewExternalPort").unwrap().parse().unwrap();
                                let client = tag(&text, "NewInternalClient").unwrap().to_owned();
                                let lease = tag(&text, "NewLeaseDuration").unwrap().parse().unwrap();
                                let mut list = kept.lock().unwrap();
                                list.retain(|(p, _, _)| *p != port);
                                list.push((port, client, lease));
                                (200, "<u:AddPortMappingResponse/>".to_owned())
                            }
                        }
                    } else if text.contains("#DeletePortMapping") {
                        let port: u16 = tag(&text, "NewExternalPort").unwrap().parse().unwrap();
                        kept.lock().unwrap().retain(|(p, _, _)| *p != port);
                        (200, "<u:DeletePortMappingResponse/>".to_owned())
                    } else {
                        (404, String::new())
                    };
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes());
                }
            });
            Self { addr, forwarded }
        }

        /// The router as a search would have found it.
        pub(in crate::forward) fn gateway(&self) -> Gateway {
            describe(&format!("http://{}/rootDesc.xml", self.addr), self.addr.ip()).unwrap()
        }
    }

    #[test]
    fn a_router_forwards_says_its_address_and_takes_the_forwarding_back() {
        let router = FakeRouter::start("203.0.113.5", None);
        let gateway = router.gateway();
        assert_eq!(gateway.control, "/ctl/IPConn");
        assert_eq!(gateway.external_ip(), Ok("203.0.113.5".parse().unwrap()));
        gateway
            .add_port(7350, "192.168.1.20".parse().unwrap(), 3600, "Metallic <Team> & Create")
            .unwrap();
        assert_eq!(
            *router.forwarded.lock().unwrap(),
            [(7350, "192.168.1.20".to_owned(), 3600)]
        );
        gateway.remove_port(7350).unwrap();
        assert!(router.forwarded.lock().unwrap().is_empty());
        let refusing = FakeRouter::start("203.0.113.5", Some(CONFLICT)).gateway();
        assert_eq!(
            refusing.add_port(7350, "192.168.1.20".parse().unwrap(), 3600, "x"),
            Err(Error::Upnp(CONFLICT))
        );
    }

    #[test]
    fn a_description_elsewhere_than_the_responder_is_refused() {
        let router = FakeRouter::start("203.0.113.5", None);
        let elsewhere = "10.9.9.9".parse().unwrap();
        assert!(describe(&format!("http://{}/rootDesc.xml", router.addr), elsewhere).is_err());
    }
}
