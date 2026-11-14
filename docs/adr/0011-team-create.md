# ADR 0011: Offline Team Create and multiplayer playtesting

- **Status:** Accepted (2026-11-14)
- **Issues:** #628 (networking and identity), #629 (projects), #630 (scene sync and presence), #631 (collaborative
  scripts), #632 (playtesting)
- **Builds on:** [ADR 0001](0001-project-layout.md) (`crates/net`, reserved for M10), [ADR 0002](0002-object-model.md)
  (the data model), [ADR 0006](0006-project-format.md) (projects and scenes), [ADR 0009](0009-studio-foundation.md)
  (Studio), [ADR 0010](0010-play-in-studio.md) (Play in Studio).

## Context

Several people should edit one project at once, and test it together, before Metallic has accounts or servers. That
is **Offline Mode**: a Studio hosts the project it has open, and others join it by address and port, on a LAN or over
the internet with the port forwarded. **Online Mode** (accounts, cloud and group projects, invites, sessions the
platform hosts) comes later and is not built now, but nothing in Offline Mode may assume that the host is a person's
Studio or that a project is a local file: Online Mode must reuse the same session logic behind another transport and
another authority.

## Decision

### 1. Concerns, kept apart

| Concern | Where | Online Mode changes |
|---|---|---|
| wire format | `metallic_net::codec`: bounded binary reads, exact float bits | nothing |
| transport and hosting | `metallic_net::transport` (framed TCP on threads), `collab::{HostServer, ClientLink}` | a relay transport feeding the same sessions |
| discovery and connection | `collab::probe`, the projects page's entries | the platform's project list |
| identity and presence | `metallic_net::identity` (display name, colour, session user id), presence messages | an account id beside the name |
| ownership and permissions | `collab::host::Host`: the host admits, names and numbers users, and validates every operation | the platform's roles |
| scene sync | `metallic_net::collab::scene` | nothing |
| collaborative script text | `metallic_net::collab::text` (a sequence CRDT) | nothing |
| persistence | the host's Studio, through `EditorState::save` | the cloud's store |
| playtesting | `metallic_net::replication`, Studio's play modes | a platform server |

`collab::host::Host` and `collab::client::Client` never touch a socket: the transport feeds them events and drains
their outboxes, so the same logic runs over TCP, over memory in tests, and later behind a relay.

### 2. The host is the authority

Participants propose; the host validates, applies and broadcasts, in one order. Every peer applies the host's order,
so every peer converges on the host's state. The host's Studio is also a participant of its own session (user 1).

### 3. Transport

One TCP connection per participant, frames of `[len u32][channel u8][payload]`:

- **Reliable** (scene and text operations, joins, snapshots): every frame, in order.
- **Presence** (cameras, selections, cursors): latest-only, throttled by the sender to 10 Hz; a frame not yet written
  is replaced by a newer one, so a slow link drops stale presence instead of delaying edits.
- **Limits**: frames up to 8 MiB from participants (16 MiB from the host, whose project arrives in 1 MiB chunks), a
  byte rate budget, a message rate budget, heartbeats every 2 s and a 12 s timeout. Breaking one closes the
  connection. Nothing on the caller's thread blocks: connecting, reading and writing each have a thread, and the
  caller polls channels. A waker redraws Studio when something arrives.

TCP over UDP: one port to forward, ordering for free, and presence kept latest-only on top of it. Game replication
(§8) uses the same transport; a UDP transport can replace it behind the same session types when M10's games need it.

### 4. Joining, in order

connect → `Hello` (protocol and engine versions, the class registry's fingerprint, display name and colour, the
project id expected) → `Welcome` (user id, unique name, session, users) or `Refused` (a reason the user can act on) →
the project: its members as they are in the host's memory, unsaved edits included, the start scene saved from the
host's scene at that moment → `ProjectEnd` → live operations. The host makes the snapshot and makes the participant
live in the same frame, after flushing its own pending operations, so every operation after the snapshot follows it
on the same stream: nothing is missed or applied twice. The participant keeps those operations until its copy is
open, applies them in order, and says `Ready`; then the others see it as editing.

A participant's copy is written to Studio's cache folder (`<cache>/team-create/<project id>/<name>.mtproj`) and
opened like any project. Only the host saves; the copy is never the project. Member paths are checked twice (the
protocol's rules, then `check_member_path`) before anything is written.

### 5. Scene sync (#630)

- **Ids.** Every instance in the session has a `NetId`. Those of the scene at the start are its scene ids; an
  instance made during the session gets `(user << 40) | n` from its maker, so ids never collide and a participant can
  refer to what it just made.
- **Diff, not hooks.** Each peer keeps a shadow of the synced state (class, parent and place, name, saved
  properties, attributes) and, at most 20 times a second, drains the data model's change consumer and compares the
  changed instances with the shadow. Every edit path (tools, Properties, undo and redo, paste, scripts in the editor)
  is caught without hooking each, and drags are coalesced by the rate.
- **Operations**: `Create` (a subtree, for an instance entering the tree from outside it), `Move` (parent and the
  sibling it comes before), `Remove` (out of the tree: a delete keeps the instance, as undo does), `Set`
  (property), `Name`, `Attr`. The current camera's view properties are each user's own and never sync.
- **Conflicts.** The host's order decides: last writer wins per property, per name, per attribute and per place. A
  participant's own operations apply at once (optimistically); while one is unacknowledged, the host's operations on
  the same key are skipped, so its acknowledgement leaves the key at the host's value. A rejected operation (a cycle,
  a class that cannot be made, a value out of type) comes back with the host's state of that key. A structural
  operation that cannot apply yet waits for the acknowledgements in flight; if it still cannot, the participant
  asks for a resync.
- **Nothing silently corrupted.** The host sends a digest of the scene every 10 s; a participant with no operation
  in flight whose digest differs asks for a resync, which reconciles the scene in place (no reopen) from the host's
  full state.

### 6. Collaborative script text (#631)

Each script file in the session is an RGA sequence CRDT: every character has an id `(user, clock)` and is placed after
its left origin, concurrent inserts at one place ordered by id; deletes leave tombstones. Only operations travel (an
insert of a run, a delete of ids); a joining or reconnecting participant receives the document's state with its
tombstones. The editor's text is the CRDT's visible text; each frame's local change is turned into operations by a
diff. **Undo** undoes your own operations (an insert by deleting its characters, a delete by inserting the text
again where it was), never someone else's. **Presence** puts cursors and selections at character ids, so they stay
put through others' edits.

### 7. Persistence

The host saves through the existing save path (`EditorState::save`, an archive written beside the project and
renamed over it), debounced: a few seconds after the last change and at most every 30 s while edits keep coming, and
at End session. A host that crashes leaves the last saved archive intact. Participants never write the project.

### 8. Playtesting (#632)

A dropdown beside Play in the top bar: **Play Solo** (the default: Server and Game tabs, the copy-on-Play of ADR 0010),
**Team Create Play** (the host runs one server; every participant joins it as a player), **Multiple Clients** (a
local server in the Server view and N Player processes, `Player1`..`PlayerN`, that connect by themselves). The server
replicates its data model (but `ServerStorage` and `ServerScriptService`) to clients through
`metallic_net::replication`; each client owns its character's movement. Nothing built in moves a character or points
the camera at it (#638): the game's own controller does, and the client sends where its character is. Stop ends the
server, the clients and the processes.

### 9. Security

Every message is decoded with limits before it is acted on; an invalid one drops the peer. Participants can change
only the scene and script text, through the operations above, each validated by the host; they cannot run editor
commands, read files, or name paths outside the project. Only the project's members are sent.

## Consequences

- One port to open for the host; nothing else to install or run.
- The editor's data model stays the only truth on every peer; sync is a mirror of it, as rendering is.
- A host that ends or loses the session leaves participants with a copy they cannot save; they rejoin to go on.
- Imports of new asset files during a session reach participants when they next join (the scene's references sync
  at once).
