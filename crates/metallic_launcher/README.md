# Metallic Launcher core

This library implements Launcher L1 Task 2. It creates no window, starts no process and stores no credentials.

`DeepLink::parse` accepts only the four protocol shapes in the launcher plan. Query values are form-decoded once;
known duplicate fields fail, unknown fields are discarded, and `to_uri` writes a canonical transport URI. Tickets
are redacted by Debug. The canonical URI contains the ticket and must never be logged.

`LauncherConfig::from_args` receives arguments without the executable name. A `metallic:` argument at any position
suppresses all flags and persisted developer settings; a malformed link fails closed. Normal developer URLs require
developer mode and HTTPS, or HTTP on localhost/127.0.0.1/::1. No production domain is guessed: the checked-in origin
file is a placeholder, so `embedded_site_origin` returns `NotConfigured`.

`UrlPolicy` classifies actual page URLs separately from navigation. Only exact site/developer and bundled local
origins grant their corresponding capability category. `about:blank` can load but remains `Other`. Other HTTPS URLs
open in the system browser; dangerous schemes, credentials, downgrade URLs and malformed inputs are blocked.

`SettingsStore` defaults to the user config directory and supports an explicit path for tests. Missing, corrupt,
invalid and oversized settings fall back to defaults without logging input. Edits validate before mutation. Saves
write and sync a unique sibling temporary file, close its write handle, and atomically replace the destination.
Permission/sharing failures receive 10/20/40 ms retries; persistent failure returns an error and removes the temporary
file. Concurrent whole-file saves use last-writer-wins semantics, with no partial TOML exposed to readers. Atomic
replacement does not guarantee directory durability across power loss or merge concurrent edits.

Input limits: protocol/origin/individual argument 2 KiB, 64 ordinary arguments, site path 512 bytes, navigation URL
16 KiB, settings file 64 KiB. These checks precede parsing or accumulation. No Windows conditional code appears in
the library; Windows-specific share-lock behavior is covered by an integration test.
