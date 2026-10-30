//! The events the rest of the Launcher sends to the window, and the way to send them.

use url::Url;

use crate::deep_link::DeepLink;
use crate::local_pages::LocalPage;
use crate::site_check::SiteStatus;

/// Something the window should do. Other parts post these through an [`EventSink`] and never
/// touch the web view themselves.
#[derive(Debug)]
pub enum AppEvent {
    /// A `metallic://` link arrived (from the web view or another process).
    DeepLink(DeepLink),
    /// Show the window and bring it to the front.
    Focus,
    /// A page posted a message to the Launcher. `page_url` is the page that sent it; the
    /// receiver must check it before acting.
    BridgeMessage {
        /// The address of the page that sent the message.
        page_url: String,
        /// The message text.
        body: String,
    },
    /// Run a script in the current page.
    RunScript(String),
    /// A site check finished.
    SiteStatus(SiteStatus),
    /// Load this address in the main view (only the site and bundled pages are accepted).
    Navigate(Url),
    /// Show a bundled page.
    ShowLocal(LocalPage),
    /// Close the Launcher.
    Quit,
}

/// Where events are sent. Implemented by the shell on top of the event loop; tests use a list.
pub trait EventSink: Send + Sync {
    /// Queues `event` for the window.
    fn send(&self, event: AppEvent);
}
