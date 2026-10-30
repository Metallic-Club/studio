//! Windows 11: WebView2 settings, the system browser, the "WebView2 missing" message box, the
//! single-instance pipe, the `metallic://` registry keys, starting programs and the tray icon.

use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use ::windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PATH_NOT_FOUND,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_WRITE, GetLastError, HANDLE, HLOCAL, LocalFree,
    WIN32_ERROR,
};
use ::windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use ::windows::Win32::Security::{
    EqualSid, GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use ::windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_NONE, FlushFileBuffers, OPEN_EXISTING,
    PIPE_ACCESS_INBOUND, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use ::windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT, WaitNamedPipeW,
};
use ::windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};
use ::windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use ::windows::Win32::UI::Shell::ShellExecuteW;
use ::windows::Win32::UI::WindowsAndMessaging::{
    IDYES, MB_ICONINFORMATION, MB_YESNO, MessageBoxW, SW_SHOWNORMAL,
};
use ::windows::core::{BOOL, HSTRING, PCWSTR, PWSTR, w};
use metallic_launcher::single_instance::body_length;
use metallic_launcher::{
    Acquired, AppEvent, EventSink, HandoffListener, HandoffMessage, HandoffSender, LauncherError,
    LocalPage, OfflineReason, Spawner, TrayAction, UriRegistry, UrlPolicy, decode, encode,
    protocol,
};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use url::Url;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_WEB_ERROR_STATUS, COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
};
use webview2_com::{NavigationCompletedEventHandler, NavigationStartingEventHandler, take_pwstr};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

/// The text of the message box. The wording is fixed by the product text.
const MISSING_TEXT: &str = "Metallic needs the Microsoft Edge WebView2 Runtime. Install it, then start Metallic again.\n\nOpen the download page now?";

pub fn open_in_browser(url: &Url) -> Result<(), LauncherError> {
    let address = HSTRING::from(url.as_str());
    // SAFETY: both strings are valid, null-terminated UTF-16 for the duration of the call.
    let result = unsafe { ShellExecuteW(None, w!("open"), &address, None, None, SW_SHOWNORMAL) };
    // ShellExecuteW reports success with a value above 32.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(LauncherError::Platform(
            "the system browser could not be started".to_string(),
        ))
    }
}

/// Shows the message box. True when the user chose to open the download page.
pub fn show_webview_missing() -> bool {
    let text = HSTRING::from(MISSING_TEXT);
    // SAFETY: the strings are valid for the duration of the call; no owner window.
    let answer = unsafe { MessageBoxW(None, &text, w!("Metallic"), MB_YESNO | MB_ICONINFORMATION) };
    answer == IDYES
}

pub fn webview_data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(base)
            .join("Metallic")
            .join("Launcher")
            .join("WebView2"),
    )
}

pub fn webview_local_url(url: &Url) -> Url {
    let mut text = format!("http://metallic-app.localhost{}", url.path());
    if let Some(query) = url.query() {
        text.push('?');
        text.push_str(query);
    }
    Url::parse(&text).unwrap_or_else(|_| url.clone())
}

pub fn configure_builder(builder: WebViewBuilder<'_>, dev_mode: bool) -> WebViewBuilder<'_> {
    // Browser shortcuts (reload, print, find, zoom, save) only make sense for a developer.
    builder.with_browser_accelerator_keys(dev_mode)
}

pub fn watch_navigation_failures(view: &WebView, sink: Arc<dyn EventSink>) {
    let webview = view.webview();
    let handler = NavigationCompletedEventHandler::create(Box::new(move |webview, args| {
        let (Some(webview), Some(args)) = (webview, args) else {
            return Ok(());
        };
        // SAFETY: plain getters on live WebView2 objects, called on the UI thread that owns them.
        unsafe {
            let mut success = BOOL::default();
            args.IsSuccess(&mut success)?;
            if success.as_bool() {
                return Ok(());
            }
            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
            args.WebErrorStatus(&mut status)?;
            // A navigation that our own rules refused is cancelled; that is not a load failure.
            if status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                return Ok(());
            }
            // A failure on a bundled page must not start a loop.
            let mut source = PWSTR::null();
            webview.Source(&mut source)?;
            let source = take_pwstr(source);
            if let Ok(url) = Url::parse(&source)
                && metallic_launcher::site::is_local_page(&url)
            {
                return Ok(());
            }
        }
        sink.send(AppEvent::ShowLocal(LocalPage::Offline(
            OfflineReason::LoadFailed,
        )));
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: the handler outlives the registration (WebView2 keeps its own reference).
    if let Err(error) = unsafe { webview.add_NavigationCompleted(&handler, &mut token) } {
        log::warn!("could not watch for failed page loads: {error}");
    }
}

/// WebView2 reports a frame's navigation only through `FrameNavigationStarting`; wry's own
/// navigation handler sees the top level alone. Anything the policy does not allow is cancelled.
/// (Pop-ups from frames arrive through the same `NewWindowRequested` event wry already handles.)
pub fn watch_frame_navigations(view: &WebView, policy: Arc<UrlPolicy>) {
    let webview = view.webview();
    let handler = NavigationStartingEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        // SAFETY: plain getter and setter on a live event-args object, on the UI thread.
        unsafe {
            let mut uri = PWSTR::null();
            args.Uri(&mut uri)?;
            let uri = take_pwstr(uri);
            if !policy.frame_navigation(&uri) {
                log::info!("blocked a frame navigation");
                args.SetCancel(true)?;
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: WebView2 keeps its own reference to the handler.
    if let Err(error) = unsafe { webview.add_FrameNavigationStarting(&handler, &mut token) } {
        log::warn!("could not watch frame navigations: {error}");
    }
}

// ---- Handles and users -------------------------------------------------------------------

fn platform_error(what: &str, error: impl std::fmt::Display) -> LauncherError {
    LauncherError::Platform(format!("{what}: {error}"))
}

/// A Windows handle that is closed when dropped.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by this program and is closed once, here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// The user a process runs as: the bytes of a `TOKEN_USER` (and the SID inside it).
struct UserSid {
    // u64 elements keep the buffer aligned for TOKEN_USER.
    buffer: Vec<u64>,
}

impl UserSid {
    /// The user of the process token `token` (opened with `TOKEN_QUERY`).
    fn of_token(token: &OwnedHandle) -> Result<UserSid, LauncherError> {
        let mut needed = 0u32;
        // SAFETY: asking for the size with no buffer; the call fails and sets `needed`.
        let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut needed) };
        if needed == 0 {
            return Err(platform_error("token", "no size"));
        }
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: the buffer holds at least `needed` bytes and is 8-byte aligned.
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                needed,
                &mut needed,
            )
        }
        .map_err(|e| platform_error("could not read the user", e))?;
        Ok(UserSid { buffer })
    }

    fn psid(&self) -> PSID {
        // SAFETY: the buffer was filled with a TOKEN_USER by GetTokenInformation.
        unsafe { (*self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    /// The SID as text, like `S-1-5-21-...`.
    fn to_text(&self) -> Result<String, LauncherError> {
        let mut text = PWSTR::null();
        // SAFETY: the SID is valid; the returned string is freed with LocalFree below.
        unsafe { ConvertSidToStringSidW(self.psid(), &mut text) }
            .map_err(|e| platform_error("could not write the user id", e))?;
        // SAFETY: `text` is a valid null-terminated string from the call above.
        let result = unsafe { text.to_string() }
            .map_err(|e| platform_error("could not read the user id", e));
        // SAFETY: the string was allocated by the call above and is freed once.
        let _ = unsafe { LocalFree(Some(HLOCAL(text.0.cast()))) };
        result
    }

    fn same_as(&self, other: &UserSid) -> bool {
        // SAFETY: both SIDs are valid for the duration of the call.
        unsafe { EqualSid(self.psid(), other.psid()) }.is_ok()
    }
}

fn current_user() -> Result<UserSid, LauncherError> {
    let mut token = HANDLE::default();
    // SAFETY: opens a query token of this process; closed by OwnedHandle.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| platform_error("could not open the process token", e))?;
    UserSid::of_token(&OwnedHandle(token))
}

fn user_of_process(pid: u32) -> Result<UserSid, LauncherError> {
    // SAFETY: opens a limited query handle; closed by OwnedHandle.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|e| platform_error("could not open the other Launcher", e))?;
    let process = OwnedHandle(process);
    let mut token = HANDLE::default();
    // SAFETY: as above.
    unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) }
        .map_err(|e| platform_error("could not open the other Launcher's token", e))?;
    UserSid::of_token(&OwnedHandle(token))
}

// ---- Single instance: a named pipe --------------------------------------------------------

/// The listening end. One pipe instance serves one client at a time.
struct PipeListener(OwnedHandle);

// SAFETY: a pipe handle may be used from any thread; only the listener thread uses it.
unsafe impl Send for PipeListener {}

fn read_exact(handle: HANDLE, buffer: &mut [u8]) -> Result<(), LauncherError> {
    let mut done = 0;
    while done < buffer.len() {
        let mut read = 0u32;
        // SAFETY: the slice is valid for writes; `read` receives the count.
        unsafe { ReadFile(handle, Some(&mut buffer[done..]), Some(&mut read), None) }
            .map_err(|e| platform_error("pipe read", e))?;
        if read == 0 {
            return Err(LauncherError::BadHandoff("the message ended early"));
        }
        done += read as usize;
    }
    Ok(())
}

impl HandoffListener for PipeListener {
    fn accept(&mut self) -> Result<HandoffMessage, LauncherError> {
        let handle = self.0.0;
        // SAFETY: a valid pipe handle; waits for a client.
        if let Err(error) = unsafe { ConnectNamedPipe(handle, None) }
            && error.code() != ERROR_PIPE_CONNECTED.to_hresult()
        {
            return Err(platform_error("pipe connect", error));
        }
        let result = (|| {
            let mut header = [0u8; 4];
            read_exact(handle, &mut header)?;
            // The length is checked before the body is read.
            let length = body_length(header)?;
            let mut frame = vec![0u8; 4 + length];
            frame[..4].copy_from_slice(&header);
            read_exact(handle, &mut frame[4..])?;
            decode(&frame)
        })();
        // SAFETY: ends this client's connection so the next one can connect.
        let _ = unsafe { DisconnectNamedPipe(handle) };
        result
    }
}

/// The sending end, already checked to belong to a Launcher of the same user.
struct PipeSender(OwnedHandle);

impl HandoffSender for PipeSender {
    fn send(&mut self, message: &HandoffMessage) -> Result<(), LauncherError> {
        let frame = encode(message);
        // Refuse here what the other side would refuse.
        decode(&frame)?;
        // SAFETY: the buffer is valid for reads; the handle is a connected pipe.
        unsafe {
            WriteFile(self.0.0, Some(&frame), None, None)
                .map_err(|e| platform_error("pipe write", e))?;
            // Returns once the first Launcher has read everything.
            FlushFileBuffers(self.0.0).map_err(|e| platform_error("pipe flush", e))
        }
    }
}

/// Becomes the first Launcher of this user, or connects to the one that runs.
pub fn acquire_instance() -> Result<Acquired, LauncherError> {
    let user = current_user()?;
    let name = format!(r"\\.\pipe\Metallic.Launcher.{}", user.to_text()?);
    acquire_named(&name, &user)
}

fn acquire_named(name: &str, user: &UserSid) -> Result<Acquired, LauncherError> {
    // Only the current user may open the pipe.
    let sddl = HSTRING::from(format!("D:P(A;;GA;;;{})", user.to_text()?));
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: valid string in, descriptor out (freed below).
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &sddl,
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(|e| platform_error("could not build the pipe's access rule", e))?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: BOOL(0),
    };
    let wide = HSTRING::from(name);
    // SAFETY: the name and the attributes are valid for the call; FIRST_PIPE_INSTANCE makes it
    // fail when a pipe of this name exists, and REJECT_REMOTE_CLIENTS keeps other PCs out.
    let handle = unsafe {
        CreateNamedPipeW(
            &wide,
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            8192,
            8192,
            0,
            Some(&attributes),
        )
    };
    // SAFETY: freeing the descriptor that the conversion allocated.
    let _ = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    if handle.is_invalid() {
        // SAFETY: reads the error of the call above on this thread.
        let error: WIN32_ERROR = unsafe { GetLastError() };
        if error == ERROR_ACCESS_DENIED || error == ERROR_PIPE_BUSY {
            return connect(name, user).map(|sender| Acquired::Secondary(Box::new(sender)));
        }
        return Err(platform_error("could not create the pipe", error.0));
    }
    Ok(Acquired::Primary(Box::new(PipeListener(OwnedHandle(
        handle,
    )))))
}

fn connect(name: &str, user: &UserSid) -> Result<PipeSender, LauncherError> {
    let wide = HSTRING::from(name);
    let mut handle = None;
    for _ in 0..20 {
        // SAFETY: valid name; identification level only, so the server cannot act as us.
        let opened = unsafe {
            CreateFileW(
                &wide,
                GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        match opened {
            Ok(opened) => {
                handle = Some(OwnedHandle(opened));
                break;
            }
            Err(error)
                if error.code() == ERROR_PIPE_BUSY.to_hresult()
                    || error.code() == ERROR_FILE_NOT_FOUND.to_hresult() =>
            {
                // The first Launcher is between two clients; wait a moment and try again.
                // SAFETY: valid name.
                let _ = unsafe { WaitNamedPipeW(&wide, 200) };
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(platform_error("could not reach the other Launcher", error)),
        }
    }
    let handle = handle.ok_or_else(|| platform_error("the other Launcher", "did not answer"))?;
    // Before anything is sent: the program on the other end must run as this user.
    let mut pid = 0u32;
    // SAFETY: a connected pipe handle and a valid out pointer.
    unsafe { GetNamedPipeServerProcessId(handle.0, &mut pid) }
        .map_err(|e| platform_error("could not identify the other Launcher", e))?;
    if !user_of_process(pid)?.same_as(user) {
        return Err(platform_error(
            "refused to send",
            "the pipe belongs to another user",
        ));
    }
    Ok(PipeSender(handle))
}

// ---- Registry ------------------------------------------------------------------------------

/// The registry under `HKEY_CURRENT_USER` (no administrator rights needed).
pub struct WinRegistry;

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn reg_error(what: &str, code: WIN32_ERROR) -> LauncherError {
    platform_error(what, format!("error {}", code.0))
}

impl UriRegistry for WinRegistry {
    fn set(&self, key: &str, name: Option<&str>, value: &str) -> Result<(), LauncherError> {
        let key = HSTRING::from(key);
        let mut hkey = HKEY::default();
        // SAFETY: valid strings; the key is closed below.
        let created = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &key,
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut hkey,
                None,
            )
        };
        if created.0 != 0 {
            return Err(reg_error("could not open the registry key", created));
        }
        let name = name.map(HSTRING::from);
        let data: Vec<u8> = utf16(value)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        let name_ptr = name
            .as_ref()
            .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
        // SAFETY: `hkey` is open; the name is null (default value) or a valid HSTRING.
        let set = unsafe { RegSetValueExW(hkey, name_ptr, None, REG_SZ, Some(&data)) };
        // SAFETY: closing the key opened above.
        let _ = unsafe { RegCloseKey(hkey) };
        if set.0 != 0 {
            return Err(reg_error("could not write the registry value", set));
        }
        Ok(())
    }

    fn get(&self, key: &str, name: Option<&str>) -> Result<Option<String>, LauncherError> {
        let key = HSTRING::from(key);
        let name = name.map(HSTRING::from);
        let name_ptr = name
            .as_ref()
            .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
        let mut size = 0u32;
        // SAFETY: asking for the size only.
        let probe = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &key,
                name_ptr,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if probe == ERROR_FILE_NOT_FOUND || probe == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        if probe.0 != 0 && probe != ERROR_MORE_DATA {
            return Err(reg_error("could not read the registry value", probe));
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2).max(1)];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: the buffer holds `size` bytes.
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &key,
                name_ptr,
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if read == ERROR_FILE_NOT_FOUND || read == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        if read.0 != 0 {
            return Err(reg_error("could not read the registry value", read));
        }
        let units = (size as usize / 2).min(buffer.len());
        let text = String::from_utf16_lossy(&buffer[..units]);
        Ok(Some(text.trim_end_matches('\0').to_string()))
    }

    fn delete_tree(&self, key: &str) -> Result<(), LauncherError> {
        let key = HSTRING::from(key);
        // SAFETY: valid key name under HKCU.
        let deleted = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &key) };
        if deleted.0 == 0 || deleted == ERROR_FILE_NOT_FOUND || deleted == ERROR_PATH_NOT_FOUND {
            Ok(())
        } else {
            Err(reg_error("could not delete the registry key", deleted))
        }
    }
}

pub fn register_protocol(exe: &Path) -> Result<(), LauncherError> {
    protocol::register(&WinRegistry, exe)
}

pub fn unregister_protocol(exe: &Path) -> Result<(), LauncherError> {
    protocol::unregister(&WinRegistry, exe)
}

pub fn protocol_registered_to(exe: &Path) -> bool {
    protocol::is_registered_to(&WinRegistry, exe)
}

// ---- Install, update, uninstall ------------------------------------------------------------

/// Registers `metallic://` for the running copy.
fn register_this_copy() {
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(error) = register_protocol(&exe) {
                log::warn!("could not register metallic:// links: {error}");
            }
        }
        Err(error) => log::warn!("could not find this program: {error}"),
    }
}

/// Removes the registration (if it is ours) and the Launcher's data folders. Only the two
/// `Metallic\Launcher` folders are touched.
fn clean_up_this_copy() {
    if let Ok(exe) = std::env::current_exe()
        && let Err(error) = unregister_protocol(&exe)
    {
        log::warn!("could not remove the metallic:// registration: {error}");
    }
    for variable in ["LOCALAPPDATA", "APPDATA"] {
        let Some(base) = std::env::var_os(variable).filter(|base| !base.is_empty()) else {
            continue;
        };
        let folder = PathBuf::from(base).join("Metallic").join("Launcher");
        if let Err(error) = std::fs::remove_dir_all(&folder)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("could not remove a data folder: {error}");
        }
    }
}

/// Velopack's start-up logic with the Launcher's hooks. These run inside the installer's
/// critical stages and must stay short.
pub fn run_velopack() {
    metallic_update::velopack::VelopackApp::build()
        .on_after_install_fast_callback(|_| register_this_copy())
        .on_after_update_fast_callback(|_| register_this_copy())
        .on_before_uninstall_fast_callback(|_| clean_up_this_copy())
        .run();
}

// ---- Starting programs ---------------------------------------------------------------------

/// Windows flag: do not create a console window for the new process.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Starts programs with `std::process::Command`: no shell, no console window, no inherited
/// standard handles.
pub struct SystemSpawner;

impl Spawner for SystemSpawner {
    fn spawn(&self, exe: &Path, args: &[OsString]) -> Result<u32, LauncherError> {
        let child = Command::new(exe)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| platform_error("could not start the program", e))?;
        Ok(child.id())
    }
}

// ---- Tray ----------------------------------------------------------------------------------

/// The tray icon. It disappears when this value is dropped.
pub struct Tray(#[allow(dead_code)] TrayIcon);

/// Shows the tray icon with `items`. Menu choices and a left click on the icon are posted to
/// `sink` as [`AppEvent::Tray`]. Must be called on the thread that runs the event loop.
pub fn create_tray(
    items: &[(TrayAction, &'static str)],
    icon_rgba: Vec<u8>,
    width: u32,
    height: u32,
    sink: Arc<dyn EventSink>,
) -> Result<Tray, LauncherError> {
    let icon = tray_icon::Icon::from_rgba(icon_rgba, width, height)
        .map_err(|e| platform_error("tray icon", e))?;
    let menu = Menu::new();
    for (action, text) in items {
        let item = MenuItem::with_id(MenuId::new(action.id()), *text, true, None);
        menu.append(&item)
            .map_err(|e| platform_error("tray menu", e))?;
    }
    {
        let sink = sink.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(action) = TrayAction::from_id(event.id.as_ref()) {
                sink.send(AppEvent::Tray(action));
            }
        }));
    }
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            sink.send(AppEvent::Tray(TrayAction::Show));
        }
    }));
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Metallic")
        .with_icon(icon)
        .build()
        .map(Tray)
        .map_err(|e| platform_error("could not create the tray icon", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_pipe(tag: &str) -> String {
        format!(
            r"\\.\pipe\Metallic.Launcher.test.{}.{tag}",
            std::process::id()
        )
    }

    #[test]
    fn second_instance_hands_link_to_first() {
        let user = current_user().unwrap();
        let name = test_pipe("handoff");
        let Acquired::Primary(mut listener) = acquire_named(&name, &user).unwrap() else {
            panic!("the first start must be the primary");
        };
        let Acquired::Secondary(mut sender) = acquire_named(&name, &user).unwrap() else {
            panic!("the second start must be a secondary");
        };
        let reader = std::thread::spawn(move || {
            let first = listener.accept();
            let second = listener.accept();
            (first, second)
        });
        sender
            .send(&HandoffMessage::Link("metallic://install".into()))
            .unwrap();
        // A second secondary can follow the first one (the pipe serves one client after another).
        let Acquired::Secondary(mut again) = acquire_named(&name, &user).unwrap() else {
            panic!("a later start must be a secondary");
        };
        again.send(&HandoffMessage::Focus).unwrap();
        let (first, second) = reader.join().unwrap();
        assert_eq!(
            first.unwrap(),
            HandoffMessage::Link("metallic://install".into())
        );
        assert_eq!(second.unwrap(), HandoffMessage::Focus);
    }

    #[test]
    fn user_sid_text_looks_like_a_sid() {
        let text = current_user().unwrap().to_text().unwrap();
        assert!(text.starts_with("S-1-"), "{text}");
    }

    #[test]
    fn real_registry_round_trip_under_a_test_key() {
        let key = format!(r"Software\Classes\metallic-test-{}", std::process::id());
        let exe = PathBuf::from(r"C:\Users\A B\Metallic.exe");
        let reg = WinRegistry;
        let result = (|| {
            protocol::register_at(&reg, &key, &exe)?;
            assert!(protocol::is_registered_to_at(&reg, &key, &exe));
            assert_eq!(reg.get(&key, Some("URL Protocol"))?, Some(String::new()));
            assert_eq!(
                reg.get(&key, None)?,
                Some("URL:Metallic Protocol".to_string())
            );
            protocol::unregister_at(&reg, &key, &exe)?;
            assert_eq!(reg.get(&key, None)?, None);
            Ok::<(), LauncherError>(())
        })();
        // Whatever happened, the test key is gone.
        reg.delete_tree(&key).unwrap();
        result.unwrap();
    }

    /// Starts the stand-in Player (built as an example) and checks it got exactly one argument.
    #[test]
    fn spawner_passes_exactly_one_argument() {
        let Some(stand_in) = stand_in_player() else {
            eprintln!("stand-in player not built; run `cargo test` to build examples");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("metallic-player.exe");
        std::fs::copy(stand_in, &exe).unwrap();
        let uri = "metallic://play?place=1&ticket=jt_abcdefgh";
        SystemSpawner.spawn(&exe, &[OsString::from(uri)]).unwrap();
        let out = dir.path().join("metallic-player.exe.args.txt");
        for _ in 0..100 {
            if out.exists() {
                std::thread::sleep(Duration::from_millis(100));
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let text = std::fs::read_to_string(&out).unwrap();
        assert_eq!(text.lines().collect::<Vec<_>>(), [uri]);
    }

    fn stand_in_player() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let path = exe
            .parent()?
            .parent()?
            .join("examples")
            .join("stand_in_player.exe");
        path.is_file().then_some(path)
    }
}
