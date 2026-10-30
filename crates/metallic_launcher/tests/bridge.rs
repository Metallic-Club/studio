//! Tests for the bridge between pages and the Launcher, with a recording fake host.

use std::sync::Mutex;

use metallic_launcher::{
    BRIDGE_VERSION, Bridge, BridgeCommand, BridgeErrorCode, BridgeEvent, BridgeHost, DeepLink,
    LauncherConfig, LauncherInfo, LauncherSettings, MAX_IN_FLIGHT, MAX_MESSAGE_BYTES, PageOrigin,
    PlaceId, SettingsPatch, SiteOrigin, UrlPolicy, allowed,
};
use serde_json::{Value, json};

const SITE: &str = "https://metallic.example/games/1";
const SITE_ROOT: &str = "https://metallic.example/";
const LOCAL: &str = "http://metallic-app.localhost/settings.html";
const DEV: &str = "http://127.0.0.1:5173/";
const PLAY: &str = "metallic://play?place=42&ticket=abcdefgh12345678";

fn bridge() -> Bridge {
    let config = LauncherConfig {
        site: SiteOrigin::parse("https://metallic.example", false),
        dev_origin: SiteOrigin::parse("http://127.0.0.1:5173", true).ok(),
        dev_mode: true,
        start_link: None,
        start_hidden: false,
        exit_after_load: false,
    };
    Bridge::new(UrlPolicy::new(&config))
}

/// Records which host methods ran (names only) and keeps real settings.
#[derive(Default)]
struct FakeHost {
    calls: Mutex<Vec<String>>,
    settings: Mutex<LauncherSettings>,
    play_error: Mutex<Option<BridgeErrorCode>>,
}

impl FakeHost {
    fn record(&self, name: &str) {
        self.calls.lock().unwrap().push(name.to_string());
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl BridgeHost for FakeHost {
    fn info(&self) -> LauncherInfo {
        self.record("info");
        LauncherInfo {
            launcher_version: "1.2.3".to_string(),
            bridge_version: BRIDGE_VERSION,
            platform: "windows",
            channel: "stable".to_string(),
            dev_mode: false,
        }
    }
    fn play(&self, link: DeepLink) -> Result<(), BridgeErrorCode> {
        self.record("play");
        assert!(matches!(link, DeepLink::Play { .. }));
        match *self.play_error.lock().unwrap() {
            Some(code) => Err(code),
            None => Ok(()),
        }
    }
    fn install_status(&self) -> Value {
        self.record("install_status");
        json!({"player": "notInstalled", "studio": "notInstalled", "launcher": "upToDate"})
    }
    fn install_studio(&self) -> Result<(), BridgeErrorCode> {
        self.record("install_studio");
        Err(BridgeErrorCode::NotAvailable)
    }
    fn open_studio(&self, place: Option<PlaceId>) -> Result<(), BridgeErrorCode> {
        self.record(&format!("open_studio {:?}", place.map(PlaceId::get)));
        Ok(())
    }
    fn settings(&self) -> LauncherSettings {
        self.record("settings");
        self.settings.lock().unwrap().clone()
    }
    fn update_settings(&self, patch: SettingsPatch) -> Result<LauncherSettings, BridgeErrorCode> {
        self.record("update_settings");
        let mut settings = self.settings.lock().unwrap();
        settings
            .apply(patch)
            .map_err(|_| BridgeErrorCode::BadRequest)?;
        Ok(settings.clone())
    }
    fn sign_in(&self) -> Result<Value, BridgeErrorCode> {
        self.record("sign_in");
        Ok(json!({"userCode": "ABCD-EFGH", "verificationUri": "https://login.example/device"}))
    }
    fn sign_out(&self) -> Result<(), BridgeErrorCode> {
        self.record("sign_out");
        Ok(())
    }
    fn github_sign_in(&self) -> Result<Value, BridgeErrorCode> {
        self.record("github_sign_in");
        Err(BridgeErrorCode::NotAvailable)
    }
    fn retry_site(&self) {
        self.record("retry_site");
    }
    fn restart_to_update(&self) -> Result<(), BridgeErrorCode> {
        self.record("restart_to_update");
        Err(BridgeErrorCode::Failed)
    }
}

/// Sends `body` from `page` and returns the script the page would run, if any.
fn call(bridge: &Bridge, host: &FakeHost, page: &str, body: &str) -> Option<String> {
    let reply = bridge.handle(page, body, host)?;
    bridge.deliver(reply, page)
}

/// The JSON argument of `window.__metallicBridge.resolve(<id>,<json>)`.
fn answer(script: &str) -> Value {
    let rest = script
        .strip_prefix("window.__metallicBridge.resolve(")
        .expect("a resolve call");
    let (_, json_text) = rest.split_once(',').expect("an id and an answer");
    let json_text = json_text.strip_suffix(')').expect("a closing bracket");
    serde_json::from_str(json_text).expect("the answer is JSON")
}

fn is_not_allowed(script: Option<String>) -> bool {
    answer(&script.expect("an answer"))["error"] == "notAllowed"
}

fn error_of(script: Option<String>) -> String {
    let value = answer(&script.expect("an answer"));
    value["error"].as_str().expect("an error code").to_string()
}

#[test]
fn site_get_info_round_trips() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let r = bridge
        .handle(SITE, r#"{"id":7,"cmd":"getInfo"}"#, &host)
        .unwrap();
    let js = bridge.deliver(r, SITE).unwrap();
    assert!(js.starts_with("window.__metallicBridge.resolve(7,"));
    assert!(js.contains(r#""bridgeVersion":1"#));
    let value = answer(&js);
    assert_eq!(value["ok"]["launcherVersion"], "1.2.3");
    assert_eq!(value["ok"]["platform"], "windows");
    assert_eq!(value["ok"]["devMode"], false);
    assert_eq!(value["ok"]["channel"], "stable");
}

#[test]
fn other_origin_is_ignored() {
    let (bridge, host) = (bridge(), FakeHost::default());
    for u in [
        "https://evil.test/",
        "http://metallic.example/",
        "https://metallic.example.evil.test/",
        "https://metallic.example@evil.test/",
        "https://metallic.example:8443/",
        "file:///C:/x.html",
        "about:blank",
        "not a url",
        "",
    ] {
        assert!(
            bridge
                .handle(u, r#"{"id":1,"cmd":"getInfo"}"#, &host)
                .is_none(),
            "{u}"
        );
    }
    assert!(host.calls().is_empty());
}

#[test]
fn dev_origin_and_bundled_pages_may_call() {
    let (bridge, host) = (bridge(), FakeHost::default());
    for page in [DEV, LOCAL, "metallic-app://local/settings.html"] {
        let js = call(&bridge, &host, page, r#"{"id":1,"cmd":"getInfo"}"#).unwrap();
        assert!(answer(&js)["ok"].is_object(), "{page}");
    }
}

#[test]
fn reply_dropped_when_page_changed_origin() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let msg = r#"{"id":1,"cmd":"getInfo"}"#;
    // The page navigated to another site, to the other trusted kinds, or to nothing useful.
    for now in [
        "https://evil.test/",
        "http://metallic.example/",
        "https://metallic.example.evil.test/",
        "about:blank",
        LOCAL,
        DEV,
    ] {
        let r = bridge.handle(SITE, msg, &host).unwrap();
        assert!(bridge.deliver(r, now).is_none(), "{now}");
    }
    // A bundled page that became the site is dropped too.
    let r = bridge.handle(LOCAL, msg, &host).unwrap();
    assert!(bridge.deliver(r, SITE).is_none());
    // Another page of the same origin still gets its answer.
    let r = bridge.handle(SITE, msg, &host).unwrap();
    assert!(bridge.deliver(r, SITE_ROOT).is_some());
}

#[test]
fn unknown_command_and_fields_are_bad_request() {
    let (bridge, host) = (bridge(), FakeHost::default());
    for body in [
        r#"{"id":1,"cmd":"runShell"}"#,
        r#"{"id":1,"cmd":"getInfo","args":{"x":1}}"#,
        r#"{"id":1,"cmd":"getInfo","extra":true}"#,
        r#"{"id":1,"cmd":"GETINFO"}"#,
        r#"{"id":1,"cmd":5}"#,
        r#"{"id":1}"#,
        r#"{"id":1,"cmd":"play"}"#,
        r#"{"id":1,"cmd":"play","args":{"launchUri":5}}"#,
        r#"{"id":1,"cmd":"play","args":{"launchUri":"x","more":1}}"#,
        r#"{"id":1,"cmd":"play","args":{"launch_uri":"x"}}"#,
        r#"{"id":1,"cmd":"setSettings","args":{"patch":{"closeToTray":"yes"}}}"#,
        r#"{"id":1,"cmd":"setSettings","args":{"patch":{"colour":1}}}"#,
        r#"{"id":1,"cmd":"openStudio","args":{"place":7}}"#,
        r#"{"id":1,"cmd":"openStudio","args":[]}"#,
    ] {
        let js = call(&bridge, &host, SITE, body);
        assert_eq!(error_of(js), "badRequest", "{body}");
    }
    assert!(host.calls().is_empty());
}

#[test]
fn messages_without_a_readable_id_get_no_answer() {
    let (bridge, host) = (bridge(), FakeHost::default());
    for body in [
        "",
        "not json",
        "[]",
        "null",
        r#"{"cmd":"getInfo"}"#,
        r#"{"id":"1","cmd":"getInfo"}"#,
        r#"{"id":-1,"cmd":"getInfo"}"#,
        r#"{"id":1.5,"cmd":"getInfo"}"#,
        r#"{"id":4294967296,"cmd":"getInfo"}"#,
        "local:retry",
    ] {
        assert!(bridge.handle(SITE, body, &host).is_none(), "{body}");
    }
    assert!(host.calls().is_empty());
}

#[test]
fn oversized_message_rejected() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let fill = |total: usize| {
        let base = r#"{"id":1,"cmd":"getInfo","pad":""}"#;
        let pad = "a".repeat(total - base.len());
        format!(r#"{{"id":1,"cmd":"getInfo","pad":"{pad}"}}"#)
    };
    let too_big = fill(MAX_MESSAGE_BYTES + 1);
    assert_eq!(too_big.len(), MAX_MESSAGE_BYTES + 1);
    assert!(bridge.handle(SITE, &too_big, &host).is_none());
    assert!(host.calls().is_empty());
    // Exactly the limit is read (and refused here only for its extra field).
    let at_limit = fill(MAX_MESSAGE_BYTES);
    let js = call(&bridge, &host, SITE, &at_limit);
    assert_eq!(error_of(js), "badRequest");
}

#[test]
fn local_page_cannot_play() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let body = json!({"id": 1, "cmd": "play", "args": {"launchUri": PLAY}}).to_string();
    let js = call(&bridge, &host, LOCAL, &body);
    assert_eq!(error_of(js), "notAllowed");
    assert!(host.calls().is_empty());
}

#[test]
fn each_origin_may_send_only_its_commands() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let site_only = [
        "play",
        "installStudio",
        "openStudio",
        "signIn",
        "signOut",
        "restartToUpdate",
    ];
    let local_only = ["githubSignIn", "retrySite"];
    let both = ["getInfo", "installStatus", "getSettings"];
    for cmd in site_only {
        let body = json!({"id": 1, "cmd": cmd, "args": {"launchUri": PLAY}}).to_string();
        let args = if cmd == "play" {
            body
        } else {
            format!(r#"{{"id":1,"cmd":"{cmd}"}}"#)
        };
        assert!(!is_not_allowed(call(&bridge, &host, SITE, &args)), "{cmd}");
        assert!(is_not_allowed(call(&bridge, &host, LOCAL, &args)), "{cmd}");
    }
    for cmd in local_only {
        let body = format!(r#"{{"id":1,"cmd":"{cmd}"}}"#);
        assert!(is_not_allowed(call(&bridge, &host, SITE, &body)), "{cmd}");
        assert!(is_not_allowed(call(&bridge, &host, DEV, &body)), "{cmd}");
        assert!(!is_not_allowed(call(&bridge, &host, LOCAL, &body)), "{cmd}");
    }
    for cmd in both {
        let body = format!(r#"{{"id":1,"cmd":"{cmd}"}}"#);
        for page in [SITE, LOCAL] {
            assert!(answer(&call(&bridge, &host, page, &body).unwrap())["ok"].is_object());
        }
    }
    // The table itself, for every origin, including pages that are not the site.
    let get_info = BridgeCommand::GetInfo;
    assert!(!allowed(PageOrigin::Other, &get_info));
    assert!(allowed(PageOrigin::Site, &get_info));
    assert!(allowed(PageOrigin::Local, &get_info));
}

#[test]
fn site_cannot_enable_developer_mode() {
    let (bridge, host) = (bridge(), FakeHost::default());
    for patch in [
        json!({"developerMode": true}),
        json!({"devUrl": "http://127.0.0.1:1"}),
        json!({"devUrl": null}),
        json!({"closeToTray": false, "developerMode": true}),
    ] {
        let body = json!({"id": 1, "cmd": "setSettings", "args": {"patch": patch}}).to_string();
        let js = call(&bridge, &host, SITE, &body);
        assert_eq!(error_of(js), "notAllowed", "{patch}");
    }
    assert!(host.calls().is_empty());
    assert_eq!(*host.settings.lock().unwrap(), LauncherSettings::default());

    // closeToTray is the one thing the site may change, and it sees nothing more than that.
    let body = r#"{"id":2,"cmd":"setSettings","args":{"patch":{"closeToTray":false}}}"#;
    let value = answer(&call(&bridge, &host, SITE, body).unwrap());
    assert_eq!(value, json!({"ok": {"closeToTray": false}}));
    assert!(!host.settings.lock().unwrap().close_to_tray);
}

#[test]
fn bundled_settings_page_can_change_everything_and_see_everything() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let set = |patch: Value| {
        let body = json!({"id": 3, "cmd": "setSettings", "args": {"patch": patch}}).to_string();
        answer(&call(&bridge, &host, LOCAL, &body).unwrap())
    };
    let value = set(json!({"developerMode": true, "devUrl": "http://127.0.0.1:5173"}));
    assert_eq!(
        value,
        json!({"ok": {"closeToTray": true, "developerMode": true, "devUrl": "http://127.0.0.1:5173"}})
    );
    // null clears the address; a bad address changes nothing.
    let value = set(json!({"devUrl": null}));
    assert_eq!(value["ok"]["devUrl"], Value::Null);
    let value = set(json!({"devUrl": "https://metallic.example/with/path", "closeToTray": false}));
    assert_eq!(value, json!({"error": "badRequest"}));
    assert!(host.settings.lock().unwrap().close_to_tray);
}

#[test]
fn play_starts_the_player_with_a_play_link() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let body = json!({"id": 9, "cmd": "play", "args": {"launchUri": PLAY}}).to_string();
    let js = call(&bridge, &host, SITE, &body).unwrap();
    assert_eq!(answer(&js), json!({"ok": {}}));
    assert_eq!(host.calls(), ["play"]);
}

#[test]
fn play_rejects_non_play_links() {
    let (bridge, host) = (bridge(), FakeHost::default());
    for uri in [
        "metallic://install",
        "metallic://open?path=/a",
        "metallic://studio/open",
        "https://x",
        "metallic://play?place=1",
        "metallic://play?place=1&ticket=short",
        "metallic://play?place=1&ticket=abc def ghi",
        "javascript:alert(1)",
        "",
    ] {
        let body = json!({"id": 1, "cmd": "play", "args": {"launchUri": uri}}).to_string();
        assert_eq!(
            error_of(call(&bridge, &host, SITE, &body)),
            "badLink",
            "{uri}"
        );
    }
    let long = format!("metallic://play?place=1&ticket={}", "a".repeat(3000));
    let body = json!({"id": 1, "cmd": "play", "args": {"launchUri": long}}).to_string();
    assert_eq!(error_of(call(&bridge, &host, SITE, &body)), "badLink");
    assert!(host.calls().is_empty());
}

#[test]
fn play_is_rate_limited() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let body = json!({"id": 1, "cmd": "play", "args": {"launchUri": PLAY}}).to_string();
    assert_eq!(
        answer(&call(&bridge, &host, SITE, &body).unwrap()),
        json!({"ok": {}})
    );
    assert_eq!(error_of(call(&bridge, &host, SITE, &body)), "busy");
    assert_eq!(host.calls(), ["play"]);
    // A refused (bad) link does not use up the slot of a later good one.
    let (bridge, host) = (self::bridge(), FakeHost::default());
    let bad =
        json!({"id": 1, "cmd": "play", "args": {"launchUri": "metallic://install"}}).to_string();
    assert_eq!(error_of(call(&bridge, &host, SITE, &bad)), "badLink");
    assert_eq!(
        answer(&call(&bridge, &host, SITE, &body).unwrap()),
        json!({"ok": {}})
    );
}

#[test]
fn host_errors_reach_the_page_as_codes_only() {
    let (bridge, host) = (bridge(), FakeHost::default());
    *host.play_error.lock().unwrap() = Some(BridgeErrorCode::NotInstalled);
    let body = json!({"id": 1, "cmd": "play", "args": {"launchUri": PLAY}}).to_string();
    let js = call(&bridge, &host, SITE, &body).unwrap();
    assert_eq!(
        js,
        r#"window.__metallicBridge.resolve(1,{"error":"notInstalled"})"#
    );
    // Every code is a bare camelCase word.
    for (code, word) in [
        (BridgeErrorCode::BadRequest, "badRequest"),
        (BridgeErrorCode::NotAllowed, "notAllowed"),
        (BridgeErrorCode::BadLink, "badLink"),
        (BridgeErrorCode::NotInstalled, "notInstalled"),
        (BridgeErrorCode::NotAvailable, "notAvailable"),
        (BridgeErrorCode::Busy, "busy"),
        (BridgeErrorCode::UpdateRequired, "updateRequired"),
        (BridgeErrorCode::Failed, "failed"),
    ] {
        assert_eq!(serde_json::to_value(code).unwrap(), json!(word));
    }
    // The placeholder commands and the update restart.
    let js = call(&bridge, &host, SITE, r#"{"id":2,"cmd":"installStudio"}"#);
    assert_eq!(error_of(js), "notAvailable");
    let js = call(&bridge, &host, SITE, r#"{"id":3,"cmd":"restartToUpdate"}"#);
    assert_eq!(error_of(js), "failed");
}

#[test]
fn arguments_may_be_left_out_or_empty_where_nothing_is_required() {
    let (bridge, host) = (bridge(), FakeHost::default());
    for body in [
        r#"{"id":1,"cmd":"getInfo","args":{}}"#,
        r#"{"id":1,"cmd":"getInfo","args":null}"#,
        r#"{"id":1,"cmd":"openStudio"}"#,
        r#"{"id":1,"cmd":"openStudio","args":{}}"#,
        r#"{"id":1,"cmd":"openStudio","args":{"place":null}}"#,
        r#"{"id":1,"cmd":"openStudio","args":{"place":"42"}}"#,
    ] {
        let js = call(&bridge, &host, SITE, body).unwrap();
        assert!(answer(&js).get("ok").is_some(), "{body}");
    }
    assert!(host.calls().contains(&"open_studio Some(42)".to_string()));
    assert!(host.calls().contains(&"open_studio None".to_string()));
    for place in ["0", "012", "-1", "4x", "99999999999999999999999"] {
        let body = json!({"id": 1, "cmd": "openStudio", "args": {"place": place}}).to_string();
        assert_eq!(
            error_of(call(&bridge, &host, SITE, &body)),
            "badRequest",
            "{place}"
        );
    }
}

#[test]
fn reply_json_is_safe_in_script() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let nasty = "</script>\u{2028}\u{2029}<!--&\"\\'";
    host.settings.lock().unwrap().dev_url = Some(nasty.to_string());
    let js = call(&bridge, &host, LOCAL, r#"{"id":5,"cmd":"getSettings"}"#).unwrap();
    for bad in ['<', '>', '&', '\u{2028}', '\u{2029}', '\n', '\r'] {
        assert!(!js.contains(bad), "script contains {bad:?}");
    }
    // The script is one call, and its argument gives back exactly the original text.
    assert!(js.starts_with("window.__metallicBridge.resolve(5,{"));
    assert!(js.ends_with("})"));
    assert_eq!(answer(&js)["ok"]["devUrl"], nasty);
}

#[test]
fn events_go_only_to_the_site_and_bundled_pages() {
    let bridge = bridge();
    let event = BridgeEvent::InstallProgress {
        product: "player".to_string(),
        percent: 40,
    };
    for page in [SITE, SITE_ROOT, DEV, LOCAL] {
        let js = bridge.event_script(&event, page).unwrap();
        assert!(js.starts_with("window.__metallicBridge.emit({"), "{page}");
        let value: Value = serde_json::from_str(
            js.strip_prefix("window.__metallicBridge.emit(")
                .unwrap()
                .strip_suffix(')')
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            value,
            json!({"event": "installProgress", "product": "player", "percent": 40})
        );
    }
}

#[test]
fn events_not_sent_to_other_origins() {
    let bridge = bridge();
    for event in [
        BridgeEvent::SiteStatus { up: true },
        BridgeEvent::UpdateReady {
            version: "1.0.1".to_string(),
            required: false,
        },
        BridgeEvent::SignIn {
            state: "pending".to_string(),
            user_code: Some("ABCD-EFGH".to_string()),
        },
    ] {
        for page in [
            "https://evil.test/",
            "http://metallic.example/",
            "https://metallic.example.evil.test/",
            "about:blank",
            "",
        ] {
            assert!(bridge.event_script(&event, page).is_none(), "{page}");
        }
    }
}

#[test]
fn events_have_the_names_and_fields_the_site_reads() {
    let bridge = bridge();
    let to_json = |event: BridgeEvent| -> Value {
        let js = bridge.event_script(&event, SITE).unwrap();
        serde_json::from_str(
            js.strip_prefix("window.__metallicBridge.emit(")
                .unwrap()
                .strip_suffix(')')
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(
        to_json(BridgeEvent::InstallFinished {
            product: "studio".to_string(),
            ok: true
        }),
        json!({"event": "installFinished", "product": "studio", "ok": true})
    );
    assert_eq!(
        to_json(BridgeEvent::UpdateReady {
            version: "1.0.1".to_string(),
            required: true
        }),
        json!({"event": "updateReady", "version": "1.0.1", "required": true})
    );
    assert_eq!(
        to_json(BridgeEvent::SiteStatus { up: false }),
        json!({"event": "siteStatus", "up": false})
    );
    assert_eq!(
        to_json(BridgeEvent::SignIn {
            state: "pending".to_string(),
            user_code: Some("ABCD-EFGH".to_string())
        }),
        json!({"event": "signIn", "state": "pending", "userCode": "ABCD-EFGH"})
    );
    assert_eq!(
        to_json(BridgeEvent::SignIn {
            state: "signedIn".to_string(),
            user_code: None
        }),
        json!({"event": "signIn", "state": "signedIn"})
    );
    // Over-large percentages are shown as 100.
    assert_eq!(
        to_json(BridgeEvent::InstallProgress {
            product: "player".to_string(),
            percent: 250
        })["percent"],
        100
    );
}

#[test]
fn only_the_five_sign_in_states_are_sent() {
    let bridge = bridge();
    for state in ["pending", "signedIn", "denied", "expired", "failed"] {
        let event = BridgeEvent::SignIn {
            state: state.to_string(),
            user_code: None,
        };
        assert!(bridge.event_script(&event, SITE).is_some(), "{state}");
    }
    for state in ["", "Pending", "signedin", "ok", "</script>"] {
        let event = BridgeEvent::SignIn {
            state: state.to_string(),
            user_code: None,
        };
        assert!(bridge.event_script(&event, SITE).is_none(), "{state}");
    }
}

#[test]
fn at_most_32_calls_wait_for_an_answer() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let msg = r#"{"id":1,"cmd":"getInfo"}"#;
    let mut waiting: Vec<_> = (0..MAX_IN_FLIGHT)
        .map(|_| bridge.handle(SITE, msg, &host).unwrap())
        .collect();
    let extra = bridge.handle(SITE, msg, &host).unwrap();
    assert_eq!(error_of(bridge.deliver(extra, SITE)), "busy");
    assert_eq!(
        host.calls().len(),
        MAX_IN_FLIGHT,
        "the extra call never reached the host"
    );
    // Delivering (or dropping) an answer frees its place.
    let first = waiting.pop().unwrap();
    assert!(bridge.deliver(first, SITE).is_some());
    let again = bridge.handle(SITE, msg, &host).unwrap();
    assert!(bridge.deliver(again, SITE).is_some());
    waiting.clear();
    assert!(call(&bridge, &host, SITE, msg).is_some());
}

#[test]
fn debug_output_never_shows_a_ticket() {
    let (bridge, host) = (bridge(), FakeHost::default());
    let command: BridgeCommand =
        serde_json::from_value(json!({"cmd": "play", "args": {"launchUri": PLAY}})).unwrap();
    assert_eq!(format!("{command:?}"), "play");
    let body = json!({"id": 1, "cmd": "play", "args": {"launchUri": PLAY}}).to_string();
    let reply = bridge.handle(SITE, &body, &host).unwrap();
    assert_eq!(format!("{reply:?}"), "PendingReply(id 1)");
    let link = DeepLink::parse(PLAY).unwrap();
    assert!(!format!("{link:?}").contains("abcdefgh12345678"));
}

#[test]
fn the_page_script_has_the_origins_and_the_fixed_shape() {
    let script = bridge().init_script();
    assert!(!script.contains("__METALLIC_ORIGINS__"));
    assert!(script.contains(r#""https://metallic.example""#));
    assert!(script.contains(r#""http://127.0.0.1:5173""#));
    assert!(script.contains(r#""http://metallic-app.localhost""#));
    assert!(script.contains("bridgeVersion: 1"));
    assert!(script.contains("window.top !== window"));
    assert!(script.contains("window.ipc"));
    // No site means only the bundled pages are listed.
    let config = LauncherConfig {
        site: Err(metallic_launcher::LauncherError::NotConfigured),
        dev_origin: None,
        dev_mode: false,
        start_link: None,
        start_hidden: false,
        exit_after_load: false,
    };
    let script = Bridge::new(UrlPolicy::new(&config)).init_script();
    assert!(!script.contains("metallic.example"));
    assert!(script.contains("metallic-app"));
}
