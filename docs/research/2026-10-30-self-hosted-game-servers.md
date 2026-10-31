# Self-hosted Game Servers: security, regions and player counts

Date: 2026-10-30. Status: proposal for the owner. Author: Claude (research session).
Scope: run Metallic Game Servers on the owner's own home servers. Strong security. $0 until paid regions. Fit as many
players as possible into one game.
Builds on `docs/research/2026-10-30-platform-architecture.md` (Game Server, matchmaker, outbound-only control, UDP with
join ticket, fixed host pool, no Kubernetes) and design spec sections 9 and 10 (milestone 2 = multiplayer + Team Create).

How to read the evidence. **Sourced** = I fetched a page or a search result said it (link and date given; all pages
were read on 2026-10-30). **Judgment** = my reasoning or general knowledge that I did not confirm in this session.
**Estimate** = a number I computed from stated assumptions; it must be measured before anyone relies on it. Many
search results were third-party guides, not the vendor's own page. I say so each time.

---

## 1. Short answer

1. **Phase 1 needs one forwarded UDP port and no paid service.** The owner's link is 340 Mbps up / 452 Mbps down, with
   24 ms to a Dallas test server. That is enough for roughly 100 to 150 players per game and a few hundred players in
   total in the first version. CPU, not bandwidth, will be the first limit (section 5).
2. **A home link cannot absorb a real DDoS attack.** Any flood above about 452 Mbps fills the download side and the
   whole household loses internet. The only fix is to keep the home IP hidden behind something that can take the hit.
   That is phase 3 (a relay) and phase 4 (paid regions with filtering) (section 4).
3. **No free tunnel carries public game UDP safely.** Cloudflare Tunnel and Tailscale Funnel do not carry public UDP.
   Cloudflare Spectrum does, but only on Enterprise. ngrok shows no UDP in its docs. playit.gg does UDP for free but
   is a third party we should use only for developer tests (section 3).
4. **We do not need dynamic DNS for game traffic.** Each host agent calls the backend over an outbound connection. The
   backend sees the host's current public address and hands it to players inside the join ticket. A domain name is
   only needed for the website.
5. **To fit more players in one game:** send each player only what is near them, send only changes, pack them tightly,
   and use several CPU cores. Then split the world into zones across processes. A real number needs a load test; the
   budgets below are estimates (section 6).

---

## 2. Where the lead's proposal holds, and where it needs correcting

| Lead's point | Verdict |
|---|---|
| One forwarded UDP port to an edge gateway | **Holds.** It is the right shape. Detail added: the edge must end (terminate) the QUIC connection itself, otherwise it cannot read the ticket and cannot route to the right game server (section 4.2). |
| Free dynamic DNS (Cloudflare DNS API) | **Not needed for games.** The backend learns the host address from the host agent and puts it in the ticket. Also, a DNS name that points at the home IP *publishes* the home IP. A Cloudflare name must be "DNS only" (grey cloud); the orange-cloud proxy does not carry game UDP. A domain is not free (about the cost of one a year; judgment). |
| Home IP never published, via a relay | **Only true with the relay.** In direct mode (phases 1 and 2) every player's client must know the home address. Do not claim the IP is hidden in those phases. |
| Free relay on Oracle Cloud Always Free | **Works, but smaller than the popular numbers.** Oracle's own page today says 2 OCPU / 12 GB for the Arm shape (not 4 / 24), 10 TB/month outbound, and a 50 Mbps cap on the tiny AMD shape. The relay can carry only about 31 Mbps on average all month (section 3.3). So it is a security shield and a CGNAT fix, not a way to carry all traffic. |
| Cloudflare Tunnel / Tailscale Funnel / ngrok / playit for UDP | **Mostly no.** See section 3.1. |
| nftables and XDP/eBPF | **Holds, in order.** nftables first (phase 2). XDP only if measurements show the kernel is the bottleneck (phase 2 or later). |
| quinn vs custom reliable UDP | **Start with quinn.** Known cost: the ticket can only be checked after the QUIC handshake, so an attacker can make us do handshake work. Mitigations in 4.2. A netcode.io-style custom layer is the fallback if measurements show this hurts. |
| Ed25519 tickets checked at the edge | **Holds, and is better than netcode.io's shared key.** The edge holds only the public key, so a stolen edge cannot mint tickets. |
| Containers with no inbound, egress only to backend | **Holds.** Extended in 4.4. |
| Max players: interest management, deltas, sharding | **Holds.** Numbers in section 6 are estimates; no public Roblox number could be confirmed. |
| "$0 until regions" | **Holds for software and services.** Not strictly $0: a domain for the website, and possibly a fee if the ISP charges for a static or non-CGNAT address (judgment). Both are small. |

---

## 3. Exposure: how players reach the home server

### 3.1 What free tunnels really support (checked 2026-10-30)

| Option | Public UDP? | Free? | Verdict for Metallic |
|---|---|---|---|
| Cloudflare Tunnel (cloudflared) | **No** for public hostnames. Third parties report the feature request is still open (cloudflared issue 964). UDP works only for private-network use where every user runs Cloudflare's WARP client. | Free | Not usable. [localtonet comparison](https://localtonet.com/blog/cloudflare-tunnel-alternative) (third party); [Cloudflare tunnel FAQ](https://developers.cloudflare.com/cloudflare-one/faq/tunnel/) |
| Cloudflare Spectrum | **Yes**, TCP and UDP | **No.** Cloudflare's docs: "Custom TCP/UDP applications require an Enterprise plan with Spectrum as a paid add-on." No public price. A forum post quotes a sales figure of 10,000 USD/month (anecdote). | Not for us. [Spectrum docs](https://developers.cloudflare.com/spectrum/) |
| Tailscale Funnel | **No.** TCP proxy, TLS only, ports 443, 8443, 10000 only, fixed bandwidth limits. | Free tier exists | Not usable for UDP. [Funnel docs](https://tailscale.com/kb/1223/tailscale-funnel) |
| Tailscale tailnet (private) | Yes, inside the tailnet only | Free tier | Good for the team and for host-to-backend links, not for the public. |
| ngrok | The TCP page does not mention UDP. TCP endpoints on a free plan need a payment method on file. | Partly | Not usable for UDP as far as I could find. [ngrok TCP docs](https://ngrok.com/docs/universal-gateway/tcp/) |
| playit.gg | **Yes** (UDP is reported to work on the free tier). | Free; paid extras | Developer tests only. Its terms (section 3.4) forbid reselling tunnels or bandwidth, ban hidden or obfuscated tunnels, and let them suspend you. All player traffic would go through a company we do not control. Free-tier limits come from third-party guides and I could not read them on playit's own pages: **uncertain**. [playit terms](https://playit.gg/terms); [Pinggy review](https://pinggy.io/blog/best_playit_gg_alternatives/) |
| Own relay VM + WireGuard | Yes | Free on Oracle Always Free (limits in 3.3) | **Recommended** for the "hide the home IP" and CGNAT cases. We control it. |

Terms note: I found no free-tier clause that forbids game hosting in the Cloudflare terms I could read (the
service-specific terms page does not mention Tunnel, Spectrum or game servers). The practical blocker is protocol
support, not terms. Check each provider's current terms again before building on it.

### 3.2 CGNAT: how to tell, and what to do

CGNAT (carrier-grade NAT) means the ISP shares one public IPv4 address between many homes. Port forwarding then cannot
work, because the router never receives the internet's packets first.

**The test (the owner can do it in five minutes):**
1. Log in to the router. Find the **WAN** (or "Internet") status page. Write down the WAN IPv4 address shown there.
2. On any device on the home network, open a "what is my IP" website. Write down the public IPv4 it shows.
3. Compare.
   - **Same address:** you have a real public IPv4. Port forwarding will work. (The PC having a "public IPv4" is not
     enough evidence. The PC may only see the address the router gave it. Compare the router's WAN address.)
   - **Different address, and the router WAN address starts with 100.64 to 100.127:** CGNAT (this range is reserved
     for it, RFC 6598).
   - **Different, and the WAN address is in a private range (10.x, 172.16 to 172.31, 192.168.x):** you are behind
     another router or CGNAT. Check whether an ISP modem in "router mode" sits in front of your router. If so, put the
     modem into bridge mode or forward through it too. If the ISP's device is the only one in front, it is CGNAT.
4. Also note whether the address changes after a router restart (dynamic IP). The design handles that (section 3.4).

**If it is CGNAT:** options, best first: (a) ask the ISP for a public IPv4. Some give one free or for a small fee
(judgment; ask them); (b) the relay in 3.3, which needs no inbound port at all; (c) playit.gg for developer tests only.
There is no IPv6 on this link, so the IPv6 "bonus path" is not available today.

### 3.3 The relay (phase 3)

Design: a small VM outside the house with a public IP. The home server opens a **WireGuard tunnel outbound** to it.
Players connect to the relay. The relay passes only checked traffic through the tunnel. The router then needs **no
forwarded port at all**, and the home IP is not in any ticket.

Oracle Cloud Always Free, from Oracle's own page ([Always Free resources](https://docs.oracle.com/en-us/iaas/Content/FreeTier/resourceref.htm), read 2026-10-30):
- Arm shape (A1 Flex): 1,500 OCPU-hours and 9,000 GB-hours a month = **2 OCPU and 12 GB**. Many guides still say 4 OCPU /
  24 GB; some 2026 pages say it was cut. The official page matches the smaller number.
- Two tiny AMD VMs (1/8 OCPU, 1 GB each), with **up to 50 Mbps** to the internet.
- **10 TB per month** outbound data. Inbound is not limited on that page.
- Idle machines can be reclaimed: over 7 days, CPU, network and (Arm) memory all under 20% at the 95th percentile.
  A quiet relay could look idle. Plan a light keep-alive job or accept the risk. Uncertain how strictly it is applied.
- Oracle asks for a payment card at sign-up to verify the account (general knowledge, not on the page I read:
  **uncertain**). The owner does the sign-up.

What that means for capacity (estimate, simple arithmetic):
- 10 TB a month = about 31 Mbps average, all month. At the full 140 Mbps plan in section 5 the cap would be used up in
  about 6.6 days.
- So the relay cannot carry the full game load for free. Use it as a shield for a small game, for CGNAT, or only for
  the first packets (the handshake and ticket check), and let players move to a direct path only when the owner
  accepts that the home IP becomes visible.

Two more rules for the relay:
- **Keep the real player IP visible at home.** If the relay just rewrites the source address (plain NAT), the home
  server sees every player as the relay and per-IP limits stop working. Run our `metallic-edge` on the relay. It ends
  the QUIC connection there, checks the ticket and the rate limits, and sends only authenticated frames through the
  tunnel, tagged with the player's address.
- **Do not leak the home IP.** The home server must not make direct connections to players, must not appear in old DNS
  records, and its outbound requests must go through the tunnel or only to the backend. Judgment.

### 3.4 Finding the server without DNS

- The host agent connects **out** to the backend (control channel). The backend sees the current public address.
- The backend puts address, port and the server's public-key fingerprint in the join ticket response.
- The client pins that key. No DNS name and no web certificate is needed for game traffic.
- The address changes after an ISP re-lease? The agent reconnects, the backend updates it. Existing players
  reconnect with a fresh ticket.
- A DNS name (Cloudflare "DNS only" record, or a free dynamic-DNS name) is optional and only useful for debugging.

---

## 4. Security design

### 4.1 Layers, outside in

| Layer | What it does | Phase |
|---|---|---|
| ISP / router | One UDP port forwarded. UPnP off. No DMZ. | 1 |
| Host firewall (nftables) | Default drop. Allow only that UDP port to the edge, and SSH from the admin network. | 1 (basic), 2 (rate limits) |
| Edge (`metallic-edge`) | Ends QUIC. Stateless address check (Retry). Ticket check. Per-IP and per-ticket limits. Routes to a game server. | 1 |
| Game server container | Unprivileged, no inbound from outside, egress only to backend and edge link. | 1 |
| Server-authoritative game logic | Clients send inputs. The server decides what happens. | 1 |
| Relay | Hides the home IP. Absorbs floods up to its own size. | 3 |
| Provider filtering | Upstream scrubbing for real attacks. | 4 |

### 4.2 Transport and tickets

**Join ticket** (signed by the backend, checked by the edge):
- Ed25519 signature. The edge holds only the public key. Key rotation by key id.
- Fields: player id, place id, server id, issue time, expiry (**60 seconds or less**), random id (to stop reuse), the
  server key fingerprint, and optionally the player's IP (off by default because phones change address).
- Single use: the edge keeps a small set of seen ids until they expire.
- Sizing: a ticket is roughly 150 bytes, which fits inside one datagram.

**Why a ticket is not enough alone.** With QUIC the ticket arrives *after* the handshake. An attacker can start
handshakes without a ticket and use up CPU. Defences, all cheap:
1. **Always send a QUIC Retry** to unknown addresses. That proves the sender can receive at its claimed address and
   stops spoofed floods. quinn supports it: `Incoming::retry()`, plus `remote_address_validated()`,
   `ignore()` and `refuse()` ([quinn docs](https://docs.rs/quinn/latest/quinn/struct.Incoming.html)).
   RFC 9000 requires servers to send no more than 3 times what they received before the address is validated
   (section 8.1; the exact numbers are in sections 8.1 and 21.3 of [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000.html); I did not read those paragraphs in full, so check before coding).
2. **Authentication deadline:** drop any connection that has not sent a valid ticket within 2 seconds.
3. **Caps:** maximum half-open handshakes in total and per source IP; token bucket on new connections per IP.
4. **Client Initial packets are at least 1,200 bytes** (RFC 9000 section 14.1), which already limits cheap
   amplification and makes small spoofed packets worthless.
5. **Cheap pre-checks first:** drop packets from a source that is already banned before any crypto.

A similar stateless-cookie idea exists in WireGuard (under load it replies with a cookie that the sender must echo).
I know this from the WireGuard paper but did not fetch it this session. **Judgment.**

**quinn or custom?**

| | quinn (QUIC) | Custom reliable UDP, netcode.io style |
|---|---|---|
| Encryption, key exchange | Built in (TLS 1.3) | You build it, or use a library |
| Reliable streams, multiple, no head-of-line blocking | Built in | You build it |
| Unreliable datagrams (for snapshots) | QUIC datagram extension | Natural |
| Ticket checked before expensive work | No, after handshake | Yes: one authenticated-encryption check on the first packet |
| Risk | Congestion control may slow snapshot datagrams under loss (**uncertain**, test it) | More code to write and audit |
| Time to M2 | Short | Long |

Recommendation: **quinn for M2.** Put a thin `metallic_net` layer over it so the transport can be swapped. If load
tests show handshake CPU is the weak spot, move to a netcode.io-style first packet
([netcode.io](https://github.com/networkprotocol/netcode.io/blob/master/README.md): short-lived connect tokens from a web backend, encrypted and signed
packets, designed against replay, spoofing and amplification) in phase 2.

### 4.3 One port, many game servers

One forwarded port can feed many game servers only if something routes by connection. Two ways:
- **A. Edge ends QUIC and relays frames to game servers over a private link** (recommended). The game server never
  sees the internet. Cost: one extra local hop (well under a millisecond) and some CPU. Benefit: the relay in phase 3 is
  the same program, and regions in phase 4 are the same program.
- B. The edge only forwards raw UDP by connection id. Needs servers to choose connection ids that encode their
  identity (as in the QUIC load-balancer design). More complex, less secure (the server handles the raw handshake).

### 4.4 Game server isolation

The owner already uses LXC. Per game server:
- Unprivileged container. No devices. Read-only game files. A memory and CPU limit. A process count limit.
- Network: only a private bridge to the edge and the host agent. **No route from the internet.** Egress allowed only to
  the backend (for registration and heartbeats) and the asset service. Block everything else, including the
  household network and the other servers. This also contains a hacked server and a script that tries to call out.
- **Outbound-only control**: the host agent opens the connection to the backend. The backend never opens a connection
  to the home. No SSH or admin port open to the internet.
- One process per match (already decided in the architecture document), so one crashed or exploited match does not touch others.
- Scripts run in the sandbox (Luau VM; Wasm later) inside that container: defence in depth, not the only wall.
- Secrets: the server holds no signing key and no database credentials. It can verify tickets with the public key only.
  It can read only its own place assets.
- Logs and metrics go out through the same outbound channel.

### 4.5 In-game abuse

- Server-authoritative simulation. Validate every input: bounds, rate (inputs per second), size, sequence.
- Rate-limit by player id and ticket id (not only by IP, which fails behind shared addresses and carriers).
- Cap per-player cost: messages per second, entities spawned, script time per tick. Kick on repeat offenders and ban
  the ticket subject through the backend.
- Reject oversize and malformed frames before parsing.

### 4.6 DDoS: what a home connection can and cannot do

Plain facts:
- The home link is 452 Mbps down. A flood larger than that fills the link. The router, the ISP, and the household all
  suffer, whatever the server does. Software filtering on the host cannot help because the packets already used the
  link before reaching it.
- Cheap attack services can send far more than 452 Mbps (general knowledge; I did not cite a figure). Treat a determined
  attacker as able to knock a single home link offline.
- Small floods (tens of Mbps or a few hundred thousand packets a second) can be handled by nftables and, if needed,
  XDP at the host: drop at the earliest point, cheaply. Fine for nuisance traffic.
- The ISP may also null-route the address or take the line down, and consumer terms often forbid running servers
  (**uncertain**: read the owner's ISP terms).
- Upload is the other limit: our own traffic. Section 5 shows how to cap it.

**When to add the relay or a provider:** when (a) any real player complains about outages tied to attacks, (b) the
site goes public with real traffic, or (c) the household feels it. Until then, a home link is acceptable for a
closed alpha.

**A relay hides the home IP but does not remove the attack.** The flood moves to the relay (50 Mbps on the tiny
free VMs; more on the Arm shape, size unconfirmed). The game goes offline, but the house stays online. For real
protection the relay needs to be in a provider with upstream filtering (next section).

---

## 5. A concrete budget for the owner's link (phase 1)

Link: **340 Mbps up, 452 Mbps down**, 24 ms to a Dallas test server, public IPv4 pending the CGNAT check, no IPv6,
owner has router access. Speed tests are often taken at quiet times. Measure again in the evening.

### 5.1 How much upload to reserve

| Item | Mbps up | Why |
|---|---|---|
| Household, always kept free | **100** (about 30%) | Video calls, streaming, uploads. Judgment. |
| Slack for ISP variation and bursts | about 40 | Evening speeds drop |
| **Game cap (hard limit)** | **200** | Shape the game traffic to this at the router or host |
| Planned average game load | **about 140** | 70% of the cap, so bursts do not hit the cap |

Set the cap in two places: the router (quality of service / SQM, if it has it) and the host (`tc` or nftables rate
limit on the game interface). Use a queue discipline that fights bufferbloat (fq_codel or CAKE) so the household's
latency stays low when the game is busy. Download need is tiny: each player sends inputs only, about 15 to 30 kbps.

### 5.2 Per-player cost, server to player (estimate)

Assume 20 snapshots a second, interest management, delta from the last acknowledged snapshot, quantized values, and
about 30 bytes of QUIC, UDP and IP header overhead per packet.

| Game type | Entities a player sees | Per-player server output |
|---|---|---|
| Light (lobby, social, slow) | up to about 30 | 30 kbps |
| **Typical (action, small world)** | up to about 100 | **60 kbps** |
| Heavy (crowded combat) | 300 or more | 150 kbps |

### 5.3 Players by upload (bandwidth only)

Usable = 70% of the upload given to the game. Players = usable / per-player rate.

| Upload given to games | Usable (70%) | Light 30 kbps | Typical 60 kbps | Heavy 150 kbps |
|---|---|---|---|---|
| 20 Mbps | 14 | 466 | 233 | 93 |
| 50 Mbps | 35 | 1,166 | 583 | 233 |
| 100 Mbps | 70 | 2,333 | 1,166 | 466 |
| **200 Mbps (owner's cap)** | **140** | **4,666** | **2,333** | **933** |
| 500 Mbps | 350 | 11,666 | 5,833 | 2,333 |
| 1000 Mbps | 700 | 23,333 | 11,666 | 4,666 |

Reading it: on the owner's link bandwidth allows over 900 players even in a heavy game. **So CPU decides, not
bandwidth.** (The 340 Mbps line is below the 500 and 1,000 rows; they are for future hosts and regions.)

### 5.4 CPU (estimate, must be measured)

Assumption: one simulation tick every 50 ms (20 Hz). A single thread has about 40 ms to spend. Per-player cost per tick
includes simulation, finding what the player can see, and building the snapshot.

| Game type | Cost per player per tick | Players per core |
|---|---|---|
| Light | 0.1 ms | about 400 |
| Typical | 0.25 ms | about 160 |
| Heavy | 0.5 ms | about 80 |

With several cores in one process, assume 70% efficiency: a process on 8 cores gives about 5 times one core.
Interest queries grow as players crowd together, so the real number is lower in packed scenes.
**All numbers in this section are guesses until `metallic-server` can be load-tested with bots.** The first M2 task
after the server runs is a bot load test that prints players, tick time, CPU and bandwidth.

### 5.5 Targets

| When | Players per game | Total across games | Notes |
|---|---|---|---|
| **M2 (phase 1)** | **50 comfortable, 100 stretch** | **300 to 500 at the same time** | Several single-thread game processes. Limited by CPU cores free on the home server. Bandwidth use stays under 40 Mbps. |
| After multi-threaded simulation and better compression (phase 2 to 3) | 200 to 300 | 500 to 1,000 | One game uses several cores. |
| With zone sharding (phase 5) | 1,000 or more | grows with machines | Bandwidth becomes the limit again; needs larger upload or more hosts. |

The total is bounded by whichever is lower: free CPU cores (section 5.4), the 140 Mbps average plan (section 5.3), or
the memory of the home server. I do not know the home server's core count, so I did not fix the total more tightly.

---

## 6. How to fit the most players in one game

### 6.1 Cut bandwidth per player (do these in order)

1. **Interest management.** Split the world into a grid. A player only gets entities in nearby cells. Cheap and the
   biggest win.
2. **Distance priority.** Near things 20 Hz, mid-range 10 Hz, far 2 to 5 Hz. A priority accumulator picks what fits in
   a per-player byte budget each tick (a method used since Tribes; from memory, judgment).
3. **Delta against what the client acknowledged.** Send only fields that changed since the last snapshot the client
   confirmed (the Quake 3 style). Lost packets cost nothing extra because the next delta is made from the confirmed state.
4. **Quantize.** Positions as 16-bit values relative to a cell. Rotations in "smallest three" form (about 32 bits).
   Velocities as small integers. Bit-pack and skip fields that did not change.
5. **Entropy coding** of the final stream only if measurements show it is worth the CPU.
6. **Events, not state,** for one-shot things (shots, sounds).

Rough result (estimate): from about 500 bytes per snapshot to about 100 to 150 bytes per player in a typical scene.

### 6.2 Use the CPU

- Run the simulation in fixed steps. Split work into jobs: physics islands, per-region updates, per-player snapshot
  building (this one parallelizes very well).
- Network send on a separate thread or threads. Group sends (`sendmmsg`, later GSO) to cut system calls.
- Keep data compact and cache-friendly (the engine's ECS or arena choice is left to the builder).
- Measure tick time, not average FPS. Report the 99th percentile.

### 6.3 Degrade gracefully

EVE Online slows game time on an overloaded node ("time dilation"), down to a floor of 10% speed, instead of lagging
unpredictably ([EVE: Introducing TiDi](https://eveonline.com/article/introducing-time-dilation-tidi);
[EVE University wiki](https://wiki.eveuniversity.org/Time_Dilation), read 2026-10-30). Metallic should plan for a
simple version: if tick time goes over budget, first lower far-away update rates, then lower everyone's rate, then
slow time, and only last drop players. Better than random lag.

### 6.4 Caps in the wild

- **Roblox:** no current official per-server maximum could be confirmed. Old developer-forum posts: the cap was 50 and
  was lifted to 100 in 2019 ([announcement](https://devforum.roblox.com/t/maximum-player-count-lifted-to-100/262089));
  200-player servers were beta; one post mentions a 700-player beta
  ([thread](https://devforum.roblox.com/t/max-player-count-increased-in-beta/722045)). All **uncertain and dated**.
  Commenters also report crashes from server memory. Use these only as a hint that 100 to 200 is a normal ceiling for
  a general-purpose platform.
- **PlanetSide 2:** the world is split into continent servers. Launch caps are reported around 2,000 per continent,
  later reduced; the latest forum figure was about 750 (250 per faction) in 2022. Fan posts, not official, **uncertain**
  ([Steam thread](https://steamcommunity.com/app/218230/discussions/0/3274688023496416213)). The lesson: caps get lowered
  after launch to protect performance.
- **EVE Online:** one solar system per node; thousands of players in one system causes dilation. Source above.

### 6.5 Spatial sharding (phase 5)

Idea: one game world, many processes. Each process owns a region (a "zone"). The client keeps **one** connection to the
edge. The edge forwards its traffic to whichever zone process owns it, so a zone change does not reconnect the player.
This is how Minecraft proxies such as Velocity work for separate servers: the proxy holds the player connection and
switches the backend. Velocity's "modern forwarding" signs player data with a shared secret so a backend only trusts
the proxy ([community guides](https://www.gameserverkings.com/knowledge-base/minecraft/multi-server-networks-with-velocity/), not the official docs, so check
before reusing the details). Metallic's edge plus signed tickets play the same role.

Design sketch (judgment; I could not find a usable source on SpatialOS internals in this session, so what follows is
general knowledge, not checked):
1. **Static zones first** (a map or instance per process, portals between them). Easiest, already enough for many games.
2. **Grid zones with a border strip.** Each zone also holds read-only copies ("ghosts") of entities within a margin
   of its border, so players near a border see across it.
3. **Authority handoff:** when an entity crosses the line, the old zone sends its full state to the new zone, then
   stops simulating it. Use a message with a sequence number and a confirm, to avoid two owners or none.
4. **Cross-zone actions** (a shot across the border) go through a short message to the owning zone; accept one or two
   ticks of delay.
5. **Hot spots:** if everyone rushes one border, the zone overloads. Answer with dilation (6.3), split a zone
   dynamically, or cap the area, as PlanetSide does with continent caps.
6. **Zone-to-zone links** are on a private network between hosts. Between regions the added latency is too high for
   one world; sharding is within one site.

Cost: a lot of engineering. It should wait until a real game needs more than one process can hold, and until the
load test shows where the single-process limit really is.

---

## 7. Paid regions (phase 4)

Same host agent, same edge, same containers, on a VPS in another place. The agent registers with a **region tag** over
its outbound channel. The matchmaker picks a region by measured round-trip time. The client pings each region's edge
(a tiny UDP echo or a QUIC handshake) and sends the results with its join request. Fall back to the nearest if none respond.
Valve's Steam Datagram Relay is a known example of ping-based routing (from memory, judgment).

Providers with included DDoS protection (prices change; check before buying):
- **OVHcloud:** its page says anti-DDoS is "deployed by default on all products", and advertises over 17 Tbps of
  global mitigation capacity for its dedicated ranges. The **Game** dedicated servers add "Game DDoS protection" for
  35+ games and protocols, from about 397 USD/month incl. VAT (GAME-1) with 1 Gbps unmetered. That is the dedicated range,
  expensive for a start. UDP is not named on the page. Sources: [OVHcloud Game servers](https://www.ovhcloud.com/en/bare-metal/game/) (read 2026-10-30).
  I did not find a current VPS-level price for protected game traffic, so a cheaper OVH path is **unconfirmed**.
- **Hetzner:** third-party sources say basic DDoS protection is included at no charge and that it is "basic" (filtering
  can lag, and the server can be null-routed under big attacks). I could not open Hetzner's own page (the URLs I tried
  returned errors), so this is **unconfirmed**. Source: [LowEndSpirit thread](https://lowendspirit.com/discussion/3235/any-first-hand-experience-with-hetzner-ddos-protection) (community, not official).
- A "protected game proxy" product (a paid filtering service in front of a VPS) can be added later without changing
  our design, because the edge already sits behind a single UDP port.
- Cloudflare Spectrum would be ideal technically but is Enterprise-only (section 3.1).

Region rules:
- Start with one extra region, in the continent with most players.
- Database and the matchmaker stay central. Game servers are the only part that moves.
- A region host is cattle: nothing precious on it, tickets and public keys only.

---

## 8. Phased plan

Cost column is money to the owner. Owner column is what must be done by hand (the owner does all sign-ups).

| Phase | We build | Cost | Owner does by hand |
|---|---|---|---|
| **1. M2 minimum, home, one port** | `metallic-server` headless. `metallic-edge` v0 (quinn, Retry, Ed25519 ticket check, routes to servers by ticket). `metallic_net` v1 on quinn. Host agent with outbound control channel and region tag field (always "home"). Ticket issue endpoint on the backend. nftables default-drop. LXC unprivileged container recipe with the egress rules. Bot load test tool. Upload shaping script. | **$0** | (1) Run the CGNAT test in 3.2. (2) In the router: reserve a fixed LAN address for the server host (DHCP reservation). (3) Add **one port-forward rule: protocol UDP only, external port N to the host's LAN address, internal port N** (N = a number the team picks, above 1024, not a well-known game port; write it in the backend config). (4) Turn **UPnP off**. Do not enable DMZ. Do not forward TCP. (5) If the router has QoS/SQM, cap game upload at the figures in 5.1. (6) Test from a phone on mobile data, not from the home Wi-Fi (many routers cannot reach their own public address from inside). |
| **2. Hardening** | Per-IP and per-ticket rate limits, handshake caps, auth deadline, ban list. Ticket replay set. Key rotation. nftables meters. Optional XDP drop program if measured need. Decide quinn vs netcode-style from load test. Dashboards. Multi-threaded snapshots. | $0 | Review router logs once. Choose who may SSH to the host. |
| **3. Relay** | `metallic-edge` runs on a relay VM. WireGuard tunnel outbound from home. Home port forward **removed**. Tag player addresses through the tunnel. Idle keep-alive. | $0 (Oracle Always Free; see limits in 3.3) | Sign up for the cloud account. Create the VM. Open one UDP port in its network rules. Remove the router port-forward rule. Optional: ISP request for a public IP if CGNAT. |
| **4. Paid regions** | Region-aware matchmaker. Ping probes. Host agent install script for a VPS. Region tag. Per-region keys. | Paid VPS per region, plus any game-DDoS add-on | Choose a provider and sign up. Pay. Give the agent its registration token through the platform (never in chat). |
| **5. Zone sharding** | Zone process type. Ghost entities. Authority handoff. Edge forwards by zone. Dilation. Load tests with thousands of bots. | Mostly engineering; hardware by need | Buy or free up more host machines. |

Notes:
- M2 also includes Team Create and the Studio multi-client test (architecture doc 1.8). Those reuse the same server and edge.
- The home server must never be the **only** copy of anything (backend data stays on the platform).
- Avoid starting phase 3 before phase 2's load test; the relay's 10 TB limit makes sense only after real traffic numbers exist.

---

## 9. Open questions for the owner and lead

1. **CGNAT result.** Router WAN address versus public address (3.2). Decides whether phase 1 can start with a port-forward or must start with the relay.
2. **How many CPU cores and how much RAM can the game host use?** This sets the real M2 total (5.5).
3. **Does the ISP's contract forbid hosting servers?** If so, a relay or a rental VPS may be needed earlier.
4. **Is a domain for the website already owned?** Only the website needs it.
5. **Edge ends QUIC (4.3-A) or forwards raw UDP (4.3-B)?** I recommend A.
6. **quinn first, with a netcode-style fallback after the load test?** I recommend yes.
7. **Hide the home IP only from phase 3, or require the relay for the public launch?** I recommend the relay before any public launch.

---

## 10. Sources (read 2026-10-30)

- Oracle, Always Free resources: https://docs.oracle.com/en-us/iaas/Content/FreeTier/resourceref.htm
- Cloudflare Spectrum docs: https://developers.cloudflare.com/spectrum/
- Cloudflare Tunnel FAQ: https://developers.cloudflare.com/cloudflare-one/faq/tunnel/
- Cloudflare service-specific terms (CDN section only): https://www.cloudflare.com/service-specific-terms-application-services/
- Tailscale Funnel: https://tailscale.com/kb/1223/tailscale-funnel
- ngrok TCP docs: https://ngrok.com/docs/universal-gateway/tcp/
- playit.gg terms: https://playit.gg/terms; third-party reviews: https://pinggy.io/blog/best_playit_gg_alternatives/
- Third-party Cloudflare Tunnel UDP summary: https://localtonet.com/blog/cloudflare-tunnel-alternative
- quinn `Incoming`: https://docs.rs/quinn/latest/quinn/struct.Incoming.html
- RFC 9000 (QUIC): https://www.rfc-editor.org/rfc/rfc9000.html (only the table of contents was read; section numbers 8.1, 14.1, 21.3 come from it)
- netcode.io: https://github.com/networkprotocol/netcode.io/blob/master/README.md
- OVHcloud Game servers: https://www.ovhcloud.com/en/bare-metal/game/
- Hetzner (community only): https://lowendspirit.com/discussion/3235/any-first-hand-experience-with-hetzner-ddos-protection
- EVE TiDi: https://eveonline.com/article/introducing-time-dilation-tidi ; https://wiki.eveuniversity.org/Time_Dilation
- Roblox player caps (dev forum, dated): https://devforum.roblox.com/t/maximum-player-count-lifted-to-100/262089 ; https://devforum.roblox.com/t/max-player-count-increased-in-beta/722045
- PlanetSide 2 caps (fan thread): https://steamcommunity.com/app/218230/discussions/0/3274688023496416213
- Velocity (community guide): https://www.gameserverkings.com/knowledge-base/minecraft/multi-server-networks-with-velocity/

Not verified this session (general knowledge, marked in text): WireGuard cookie design, Steam Datagram Relay,
SpatialOS handoff, priority accumulators, Quake 3 delta snapshots, Oracle card requirement, ISP terms.
