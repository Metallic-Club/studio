//! The raw Luau C API, behind a small safe surface. Every `unsafe` block of the crate is in this module.
//!
//! mlua's safe API runs a Rust callback through a trampoline that costs over 300 ns per userdata field read (the
//! archived engine measured it), which is twice the budget for `part.Position`. The hot paths (`Instance`
//! members and the data types) are therefore plain C functions on tagged userdata, written against the C API
//! here, and mlua keeps everything else: the VM, compiling, loading, threads and the `task` library.
//!
//! # The shape of a C function
//!
//! [`lua_fn!`](crate::lua_fn) wraps a safe body `fn(&Call) -> Result<Out, Raise>` into a C function. The body reads
//! its arguments through [`Args`], which never allocates in the VM, and returns what to push as an [`Out`]. The
//! trampoline then pushes the results, or raises the error with the caller's position (`chunk:line: message`).
//! Two results need the VM after the body: [`Out::Yield`] yields the thread to the host (`Invoke`), and
//! [`Out::KeepRef`] anchors an argument in the registry (a function assigned to a callback member). Both happen in
//! the trampoline, after `catch_unwind`, like pushing.
//!
//! # How errors travel, and why the panic strategy must be `unwind`
//!
//! - **The mechanism.** mlua-sys builds Luau with `luau0-src` and never calls its `use_longjmp(true)`, so
//!   `LUA_USE_LONGJMP` is unset and `lua_error` is a C++ `throw`, caught by Luau's `try`/`catch` in
//!   `luaD_rawrunprotected` (around every `lua_resume` and `lua_pcall`).
//! - **The path.** A C function's error is thrown by `lua_error` in [`raise`]: the exception unwinds through
//!   `raise`, [`run`] and the `extern "C-unwind"` trampoline (Rust frames), then through the Luau VM to its catch.
//!   The same holds for the interrupt (a timeout), for out-of-memory while pushing results, and for mlua itself:
//!   its `create_function` callbacks call `lua_error` from Rust `extern "C-unwind"` frames, and its protected calls
//!   (`exec_raw`, `protect_lua!`) run Rust closures between `lua_pcall` and a possible throw.
//! - **Why it is sound.** With `panic = "unwind"`, a foreign exception entering Rust through a `"C-unwind"` call is
//!   defined behaviour: it unwinds the Rust frames, running their destructors. It must never reach a
//!   `catch_unwind` (Rust leaves that case unspecified, and it may abort), so bodies only read the stack, and
//!   everything that can throw (pushing, allocating, raising) happens after `catch_unwind` has returned.
//! - **Why not `abort`.** With `panic = "abort"`, any unwind reaching a Rust frame aborts the process ("panic in a
//!   function that cannot unwind"). Measured on the `dist` profile when it was `abort`: every error raised by an
//!   engine call or an mlua callback killed the process; only errors that stay inside Luau (`error()` in pure
//!   Luau code, running out of memory in Luau code) survived. Restructuring this module alone would not fix that,
//!   because mlua raises through its own Rust frames. So `dist` uses `unwind`, and `lib.rs` refuses to compile under
//!   `abort` (`compile_error!` on `cfg(panic = "abort")`). `examples/error_smoke.rs` runs every error path (bad
//!   arguments, unknown members, read-only writes, engine errors, writes to destroyed instances, mlua callbacks,
//!   listener errors,
//!   `pcall`, a yield where Luau cannot yield, errors the host raises where a thread waits, the interrupt, out of
//!   memory in Luau and in an engine call) and CI runs it in the `dist` profile.
//!
//! # Reaching the engine
//!
//! A C function has no captured state. While a backend call runs, [`FrameGuard`] publishes the call's
//! [`HostContext`] and the backend's [`Shared`] state in a thread-local; [`with_ctx`] lends the context out one
//! borrow at a time, so two `&mut` to it never exist together.
//!
//! # Instances pin what they refer to
//!
//! An `Instance` userdata pins its instance in the data model ([`DataModel::pin`]) for as long as it lives, so a
//! script can still read an instance after destroying it. The pin is taken when the userdata is created during a
//! backend call (or, for one made outside a call, the first time a call pushes it), and the userdata records the
//! [`DataModel::uid`] it pinned in. Luau runs a per-tag destructor when it collects the userdata (or closes the VM);
//! the destructor cannot reach the data model, and runs inside the garbage collector where it must not touch the VM,
//! so it only queues `(uid, id)` in a thread-local. [`FrameGuard`] gives the queued pins of its data model back when
//! a call starts and when it ends, and [`release_pins`] does it on demand. A pin queued for a data model that never
//! sees another call stays queued; it is 16 bytes and harms nothing.
#![allow(unsafe_code, reason = "the raw Luau C API; the module docs explain why it is needed")]

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use metallic_core::{CFrame, Color3, DataModel, Id, ListenerKey, ScriptValue, Value, Vector2, Vector3};
use metallic_script::{HostContext, SignalRef};
use mlua::ffi;
pub(crate) use mlua::ffi::lua_State;
pub(crate) use mlua::lua_CFunction;
use mlua::{Function, IntoLuaMulti, Lua, MultiValue, Table};

use crate::shared::Shared;
use crate::{atoms, convert};

/// The userdata tags of the engine's types. mlua uses tag 1 for userdata it has destroyed, so these stay well
/// away from the low numbers; Luau allows up to 127.
pub(crate) mod tag {
    use std::ffi::c_int;

    /// [`Id`](metallic_core::Id): an `Instance`.
    pub(crate) const INSTANCE: c_int = 64;
    /// [`Vector2`](metallic_core::Vector2).
    pub(crate) const VECTOR2: c_int = 65;
    /// [`CFrame`](metallic_core::CFrame).
    pub(crate) const CFRAME: c_int = 66;
    /// [`Color3`](metallic_core::Color3).
    pub(crate) const COLOR3: c_int = 67;
    /// [`EnumRef`](super::EnumRef): an `EnumItem`.
    pub(crate) const ENUM_ITEM: c_int = 68;
    /// [`SignalHandle`](super::SignalHandle): an `RBXScriptSignal`.
    pub(crate) const SIGNAL: c_int = 69;
    /// [`ConnectionHandle`](super::ConnectionHandle): an `RBXScriptConnection`.
    pub(crate) const CONNECTION: c_int = 70;
}

/// A plain value stored in a tagged userdata. `Copy` rules out a destructor, which these userdata never run.
pub(crate) trait Payload: Copy + 'static {
    /// The userdata tag that marks a value of this type.
    const TAG: c_int;
}

/// Read only: an `Instance` userdata holds an [`InstanceBlock`], whose id comes first. Only [`new_instance`] creates
/// one.
impl Payload for Id {
    const TAG: c_int = tag::INSTANCE;
}

/// The block of an `Instance` userdata.
#[repr(C)]
#[derive(Clone, Copy)]
struct InstanceBlock {
    /// The instance. First, so [`Args::payload`] reads it as the payload.
    id: Id,
    /// The [`DataModel::uid`] of the data model the userdata pinned the instance in, or 0 when it holds no pin.
    pinned_in: u64,
}

impl Payload for Vector2 {
    const TAG: c_int = tag::VECTOR2;
}

impl Payload for CFrame {
    const TAG: c_int = tag::CFRAME;
}

impl Payload for Color3 {
    const TAG: c_int = tag::COLOR3;
}

/// An enum item, as positions in the registry's enum list, so pushing one reuses a cached userdata and `==`
/// compares identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EnumRef {
    /// Index of the enum in [`EnumIndex`](crate::shared::EnumIndex).
    pub(crate) enum_index: u16,
    /// Index of the item in its enum's `items`.
    pub(crate) item_index: u16,
}

impl Payload for EnumRef {
    const TAG: c_int = tag::ENUM_ITEM;
}

/// A signal of one instance (`part.Touched`, `part:GetPropertyChangedSignal("Name")`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SignalHandle {
    /// The instance.
    pub(crate) target: Id,
    /// Which signal.
    pub(crate) signal: SignalRef,
}

impl Payload for SignalHandle {
    const TAG: c_int = tag::SIGNAL;
}

/// A connection made by `Connect` or `Once`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionHandle {
    /// The key the host chose for it.
    pub(crate) listener: ListenerKey,
}

impl Payload for ConnectionHandle {
    const TAG: c_int = tag::CONNECTION;
}

// ---------------------------------------------------------------------------------------------------------------
// The frame: the running call's context, reachable from C functions.
// ---------------------------------------------------------------------------------------------------------------

/// What a C function can reach while a backend call runs.
#[derive(Clone, Copy)]
struct Frame {
    ctx: *mut HostContext<'static>,
    shared: *const Shared,
}

thread_local! {
    /// The published frame. One thread-local read per C function call; everything else hangs off it.
    static FRAME: Cell<Option<Frame>> = const { Cell::new(None) };

    /// Pins of collected `Instance` userdata, as `(data model uid, id)`, until [`release_pins`] gives them back
    /// (see the module docs).
    static RELEASED: RefCell<Vec<(u64, Id)>> = const { RefCell::new(Vec::new()) };
}

impl Frame {
    /// The current frame, if a backend call is running.
    #[inline]
    fn current() -> Option<Self> {
        FRAME.with(Cell::get)
    }

    /// The frame's shared state.
    #[inline]
    fn shared<'a>(self) -> &'a Shared {
        // SAFETY: the live `FrameGuard` borrows the `Shared` for as long as the frame is published, and callers
        // only hold the reference during the call that found the frame; shared references may coexist.
        unsafe { &*self.shared }
    }

    /// Lends the context to `f`, once at a time (see [`with_ctx`]).
    #[inline]
    fn lend<R>(self, f: impl FnOnce(&mut HostContext<'_>) -> R) -> Result<R, Raise> {
        let lent = &self.shared().ctx_lent;
        if lent.replace(true) {
            return Err(Raise::from("the engine API was re-entered"));
        }
        /// Returns the context when `f` ends, even by panic.
        struct Return<'a>(&'a Cell<bool>);
        impl Drop for Return<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _return = Return(lent);
        // SAFETY: the pointer was made from the `&mut HostContext` that the live `FrameGuard` keeps borrowed, so
        // the context is alive and nothing else uses it; `ctx_lent` guarantees this is the only reference made
        // from it now. The `'static` in its type is never exposed: `f` accepts any lifetime.
        let ctx = unsafe { &mut *self.ctx };
        Ok(f(ctx))
    }
}

/// Publishes a backend call's context and shared state to C functions and mlua callbacks until dropped. It
/// borrows both for its whole life, so the caller cannot touch the context while Luau code may. Entering and leaving
/// give the data model back the pins of collected instance userdata ([`release_pins`]).
pub(crate) struct FrameGuard<'a> {
    frame: Frame,
    previous: Option<Frame>,
    _borrow: PhantomData<&'a mut ()>,
}

impl<'a> FrameGuard<'a> {
    /// Starts a frame for one backend call.
    pub(crate) fn enter(ctx: &'a mut HostContext<'_>, shared: &'a Shared) -> Self {
        release_pins(ctx.dm);
        let frame = Frame {
            ctx: ptr::from_mut(ctx).cast::<HostContext<'static>>(),
            shared: ptr::from_ref(shared),
        };
        let previous = FRAME.with(|cell| cell.replace(Some(frame)));
        Self {
            frame,
            previous,
            _borrow: PhantomData,
        }
    }
}

impl Drop for FrameGuard<'_> {
    fn drop(&mut self) {
        FRAME.with(|cell| cell.set(self.previous));
        if self.frame.shared().ctx_lent.get() {
            return;
        }
        // SAFETY: the guard still borrows the context, and the frame is no longer published, so no C function or
        // callback can lend it; `ctx_lent` is clear, so no lent reference is alive either. This is the only one.
        let ctx = unsafe { &mut *self.frame.ctx };
        release_pins(ctx.dm);
    }
}

/// Gives `dm` back the pins that collected `Instance` userdata took in it (see the module docs). Pins taken in
/// other data models stay queued for them.
pub(crate) fn release_pins(dm: &mut DataModel) {
    let uid = dm.uid();
    let _ = RELEASED.try_with(|released| {
        let Ok(mut released) = released.try_borrow_mut() else {
            return;
        };
        if released.is_empty() {
            return;
        }
        released.retain(|&(owner, id)| {
            if owner != uid {
                return true;
            }
            dm.unpin(id);
            false
        });
    });
}

/// Luau's destructor for `Instance` userdata, run by the garbage collector (or when the VM closes): queues the
/// userdata's pin for [`release_pins`]. It must not touch the VM or raise, and it does neither. (mlua-sys declares
/// destructors `extern "C"`, so a panic here would abort; it cannot happen, and is caught regardless.)
unsafe extern "C" fn instance_dtor(_state: *mut lua_State, data: *mut c_void) {
    if data.is_null() {
        return;
    }
    // SAFETY: Luau passes the block of a userdata with the INSTANCE tag, and every such userdata is created by
    // `new_instance` with an `InstanceBlock` written into it before anything can collect it.
    let block = unsafe { ptr::read_unaligned(data.cast::<InstanceBlock>()) };
    if block.pinned_in == 0 {
        return;
    }
    // Nothing here calls into Luau, so no foreign exception can meet this `catch_unwind`. A pin that cannot be
    // queued (thread exit, re-entry) is kept: harmless.
    let _ = catch_unwind(|| {
        let _ = RELEASED.try_with(|released| {
            if let Ok(mut released) = released.try_borrow_mut() {
                released.push((block.pinned_in, block.id));
            }
        });
    });
}

/// Why a C function or callback could not reach the engine.
pub(crate) const NO_FRAME: &str = "engine API used outside a running script";

/// The error for [`NO_FRAME`], built only when needed (an eager `ok_or` would allocate on every call).
fn no_frame() -> Raise {
    Raise::from(NO_FRAME)
}

/// Lends the running call's [`HostContext`] to `f`. Fails outside a call, and inside another `with_ctx` (which
/// would be a second `&mut`). `f` must not run Luau code; nothing it can reach does.
pub(crate) fn with_ctx<R>(f: impl FnOnce(&mut HostContext<'_>) -> R) -> Result<R, Raise> {
    Frame::current().ok_or_else(no_frame)?.lend(f)
}

/// Runs `f` with the running backend's shared state. Fails outside a call.
pub(crate) fn with_shared<R>(f: impl FnOnce(&Shared) -> R) -> Result<R, Raise> {
    Ok(f(Frame::current().ok_or_else(no_frame)?.shared()))
}

// ---------------------------------------------------------------------------------------------------------------
// Errors and results of C functions.
// ---------------------------------------------------------------------------------------------------------------

/// An error a C function raises in the calling script. The trampoline adds the caller's position.
#[derive(Debug)]
pub(crate) struct Raise(pub(crate) String);

impl From<&str> for Raise {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}

impl From<String> for Raise {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<metallic_core::ScriptError> for Raise {
    fn from(error: metallic_core::ScriptError) -> Self {
        Self(error.to_string())
    }
}

impl From<metallic_core::DmError> for Raise {
    fn from(error: metallic_core::DmError) -> Self {
        Self(error.to_string())
    }
}

/// One value a C function returns.
#[derive(Debug)]
pub(crate) enum Push {
    /// `nil`.
    Nil,
    /// A boolean.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string the function built.
    Text(String),
    /// A string known at compile time.
    Static(&'static str),
    /// A `Vector3` (Luau's native `vector`).
    Vector3(Vector3),
    /// A `Vector2`.
    Vector2(Vector2),
    /// A `CFrame`.
    CFrame(CFrame),
    /// A `Color3`.
    Color3(Color3),
    /// An instance (the same userdata every time while one is alive).
    Instance(Id),
    /// An enum item (its cached userdata).
    Enum(EnumRef),
    /// A signal object.
    Signal(SignalHandle),
    /// A connection object.
    Connection(ConnectionHandle),
    /// A property value.
    Value(Value),
    /// A value from an engine method; a tuple pushes each of its values.
    Script(ScriptValue),
    /// A data type's method as a function value (`local dot = v.Dot`): field `name` of the table with registry
    /// reference `table`.
    Field {
        /// `lua_ref` of the method table.
        table: c_int,
        /// The method's name.
        name: &'static str,
    },
    /// An instance method as a function value (`local destroy = part.Destroy`).
    InstanceMethod(&'static str),
}

/// What a C function returns.
#[derive(Debug)]
pub(crate) enum Out {
    /// No values.
    None,
    /// One value.
    One(Push),
    /// Several values.
    Many(Vec<Push>),
    /// Yield the running thread to the host, for the reason the body put in [`Shared::pending_yield`]. The values
    /// the host resumes the thread with become the C function's results (Luau's `lua_yield` without a
    /// continuation).
    Yield,
    /// Anchor argument `idx` (a function) in the registry under `listener`, in [`Shared::callbacks`]; no results.
    /// Anchoring allocates, so it cannot happen in the body.
    KeepRef {
        /// The argument's stack index.
        idx: c_int,
        /// The key the host chose for it.
        listener: ListenerKey,
    },
}

impl From<Push> for Out {
    fn from(push: Push) -> Self {
        Self::One(push)
    }
}

/// The result of a C function's body.
pub(crate) type Ret = Result<Out, Raise>;

/// The kind of a stack value, with the tag of a userdata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// No value at that index (past the arguments).
    None,
    /// `nil`.
    Nil,
    /// A boolean.
    Bool,
    /// A number (or a Luau 64-bit integer).
    Number,
    /// A string.
    String,
    /// A native vector: a `Vector3`.
    Vector,
    /// A table.
    Table,
    /// A function.
    Function,
    /// A coroutine.
    Thread,
    /// A full userdata and its tag.
    Userdata(c_int),
    /// A buffer.
    Buffer,
    /// Anything else (a light userdata).
    Other,
}

/// A C function's view of its arguments. Reading never allocates in the VM, so it never raises.
pub(crate) struct Args<'a> {
    state: *mut lua_State,
    _call: PhantomData<&'a ()>,
}

impl<'a> Args<'a> {
    /// How many arguments were passed.
    pub(crate) fn len(&self) -> c_int {
        // SAFETY: `state` is the running C function's state (see `run`).
        unsafe { ffi::lua_gettop(self.state) }
    }

    /// `idx` as a positive index, if a value is there. Negative indices count from the top (-1 is the last).
    fn slot(&self, idx: c_int) -> Option<c_int> {
        let top = self.len();
        match idx {
            1.. if idx <= top => Some(idx),
            ..=-1 if -idx <= top => Some(top + idx + 1),
            _ => None,
        }
    }

    /// The kind of value `idx` (1-based, or negative from the top).
    pub(crate) fn kind(&self, idx: c_int) -> Kind {
        let Some(idx) = self.slot(idx) else {
            return Kind::None;
        };
        // SAFETY: `idx` is a valid stack index of this call.
        match unsafe { ffi::lua_type(self.state, idx) } {
            ffi::LUA_TNIL => Kind::Nil,
            ffi::LUA_TBOOLEAN => Kind::Bool,
            ffi::LUA_TNUMBER | ffi::LUA_TINTEGER => Kind::Number,
            ffi::LUA_TSTRING => Kind::String,
            ffi::LUA_TVECTOR => Kind::Vector,
            ffi::LUA_TTABLE => Kind::Table,
            ffi::LUA_TFUNCTION => Kind::Function,
            ffi::LUA_TTHREAD => Kind::Thread,
            // SAFETY: as above; the value is a full userdata.
            ffi::LUA_TUSERDATA => Kind::Userdata(unsafe { ffi::lua_userdatatag(self.state, idx) }),
            ffi::LUA_TBUFFER => Kind::Buffer,
            _ => Kind::Other,
        }
    }

    /// Argument `idx` as a number, if it is one.
    pub(crate) fn number(&self, idx: c_int) -> Option<f64> {
        // SAFETY: `idx` is a valid stack index holding a number, which `lua_tonumberx` reads without converting.
        (self.kind(idx) == Kind::Number).then(|| unsafe { ffi::lua_tonumberx(self.state, idx, ptr::null_mut()) })
    }

    /// Argument `idx` as a boolean, if it is one.
    pub(crate) fn boolean(&self, idx: c_int) -> Option<bool> {
        // SAFETY: `idx` is a valid stack index.
        (self.kind(idx) == Kind::Bool).then(|| unsafe { ffi::lua_toboolean(self.state, idx) } != 0)
    }

    /// Whether argument `idx` is anything but `nil`, `false` or missing.
    pub(crate) fn truthy(&self, idx: c_int) -> bool {
        match self.kind(idx) {
            Kind::None | Kind::Nil => false,
            Kind::Bool => self.boolean(idx).unwrap_or(false),
            _ => true,
        }
    }

    /// Argument `idx` as bytes, if it is a string. Valid while the call runs.
    pub(crate) fn bytes(&self, idx: c_int) -> Option<&'a [u8]> {
        if self.kind(idx) != Kind::String {
            return None;
        }
        let mut len = 0usize;
        // SAFETY: the value is a string, so `lua_tolstring` returns its bytes without converting anything. They
        // stay alive while the value is on the stack, which is the whole call.
        unsafe {
            let data = ffi::lua_tolstring(self.state, idx, &mut len);
            (!data.is_null()).then(|| std::slice::from_raw_parts(data.cast::<u8>(), len))
        }
    }

    /// Argument `idx` as text, if it is a string of valid UTF-8.
    pub(crate) fn str(&self, idx: c_int) -> Option<&'a str> {
        self.bytes(idx).and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Argument `idx` as text and its atom (an id Luau caches in the string; -1 for strings that are not member
    /// names).
    pub(crate) fn str_atom(&self, idx: c_int) -> Option<(i32, &'a str)> {
        if self.kind(idx) != Kind::String {
            return None;
        }
        let (mut len, mut atom): (usize, c_int) = (0, -1);
        // SAFETY: the value is a string; `lua_tolstringatom` returns its bytes and (computing it once through our
        // `useratom` callback) its atom.
        let bytes = unsafe {
            let data = ffi::lua_tolstringatom(self.state, idx, &mut len, &mut atom);
            if data.is_null() {
                return None;
            }
            std::slice::from_raw_parts(data.cast::<u8>(), len)
        };
        std::str::from_utf8(bytes).ok().map(|s| (atom, s))
    }

    /// Upvalue `n` of the running C closure as text and its atom, if it is a string.
    pub(crate) fn upvalue_str_atom(&self, n: c_int) -> Option<(i32, &'a str)> {
        let idx = ffi::lua_upvalueindex(n);
        let (mut len, mut atom): (usize, c_int) = (0, -1);
        // SAFETY: an upvalue pseudo-index of the running closure; it is only read when it holds a string, and the
        // closure keeps the string alive.
        let bytes = unsafe {
            if ffi::lua_type(self.state, idx) != ffi::LUA_TSTRING {
                return None;
            }
            let data = ffi::lua_tolstringatom(self.state, idx, &mut len, &mut atom);
            if data.is_null() {
                return None;
            }
            std::slice::from_raw_parts(data.cast::<u8>(), len)
        };
        std::str::from_utf8(bytes).ok().map(|s| (atom, s))
    }

    /// The method name of a `__namecall` and its atom.
    pub(crate) fn namecall(&self) -> Option<(i32, &'a str)> {
        let mut atom: c_int = -1;
        // SAFETY: called from a `__namecall` metamethod, where Luau sets the name; otherwise it returns null.
        let name = unsafe { ffi::lua_namecallatom(self.state, &mut atom) };
        if name.is_null() {
            return None;
        }
        // SAFETY: Luau strings are NUL-terminated and the name stays alive during the call.
        let name = unsafe { CStr::from_ptr(name) };
        name.to_str().ok().map(|s| (atom, s))
    }

    /// Argument `idx` as a `Vector3`, if it is a native vector.
    pub(crate) fn vector(&self, idx: c_int) -> Option<Vector3> {
        if self.kind(idx) != Kind::Vector {
            return None;
        }
        // SAFETY: the value is a vector, so `lua_tovector` points at its three floats.
        unsafe {
            let v = ffi::lua_tovector(self.state, idx);
            (!v.is_null()).then(|| Vector3::new(*v, *v.add(1), *v.add(2)))
        }
    }

    /// Argument `idx` as the payload `T`, if it is a userdata with `T`'s tag.
    pub(crate) fn payload<T: Payload>(&self, idx: c_int) -> Option<T> {
        let idx = self.slot(idx)?;
        // SAFETY: `lua_touserdatatagged` returns null unless the value is a userdata with `T::TAG`; only this module
        // creates userdata with our tags, always with a `T` written at the start (see `push_payload`).
        unsafe {
            let data = ffi::lua_touserdatatagged(self.state, idx, T::TAG);
            (!data.is_null()).then(|| ptr::read_unaligned(data.cast::<T>()))
        }
    }

    /// Reads the array part of the table at `idx` through `f`, one element at a time; the element is value `-1`
    /// of the `Args` that `f` gets, which lives only for that element.
    pub(crate) fn each_array<E>(&self, idx: c_int, mut f: impl FnMut(&Args<'_>) -> Result<(), E>) -> Result<(), E> {
        if self.kind(idx) != Kind::Table {
            return Ok(());
        }
        let Some(idx) = self.slot(idx) else {
            return Ok(());
        };
        // SAFETY: `idx` is a table; each element is pushed, read and popped, so the stack holds at most one extra
        // slot (C functions have LUA_MINSTACK free slots, and `exec_raw` checks for three).
        let len = unsafe { ffi::lua_objlen_(self.state, idx) };
        for i in 1..=len {
            // SAFETY: as above.
            unsafe { ffi::lua_rawgeti_(self.state, idx, i) };
            let element = Args {
                state: self.state,
                _call: PhantomData,
            };
            let result = f(&element);
            // SAFETY: pops the element pushed above.
            unsafe { ffi::lua_pop(self.state, 1) };
            result?;
        }
        Ok(())
    }

    /// Reads every pair of the table at `idx` through `f`, with the key as text (`None` for a key that is not a
    /// string); the value is value `-1` of the `Args` that `f` gets, which lives only for that pair.
    pub(crate) fn each_pair<E>(
        &self,
        idx: c_int,
        mut f: impl FnMut(&Args<'_>, Option<&str>) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.kind(idx) != Kind::Table {
            return Ok(());
        }
        let Some(idx) = self.slot(idx) else {
            return Ok(());
        };
        // SAFETY: `idx` is a table; `lua_next` pushes a key and a value, and the value is popped before the next
        // step (two extra slots at most). Keys are read without conversion, which `lua_next` requires.
        unsafe { ffi::lua_pushnil(self.state) };
        // SAFETY: as above.
        while unsafe { ffi::lua_next(self.state, idx) } != 0 {
            let pair = Args {
                state: self.state,
                _call: PhantomData,
            };
            let key = if pair.kind(-2) == Kind::String {
                pair.str(-2)
            } else {
                None
            };
            let result = f(&pair, key);
            // SAFETY: pops the value; the key stays for `lua_next`, or is popped on an early return.
            unsafe { ffi::lua_pop(self.state, if result.is_ok() { 1 } else { 2 }) };
            result?;
        }
        Ok(())
    }

    /// The Luau type name of argument `idx` (`typeof` names for the engine's types), for error messages.
    pub(crate) fn type_name(&self, idx: c_int) -> &'static str {
        kind_name(self.kind(idx))
    }
}

/// The name `typeof` gives a value of this kind.
pub(crate) fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::None => "no value",
        Kind::Nil => "nil",
        Kind::Bool => "boolean",
        Kind::Number => "number",
        Kind::String => "string",
        Kind::Vector => "Vector3",
        Kind::Table => "table",
        Kind::Function => "function",
        Kind::Thread => "thread",
        Kind::Userdata(tag::INSTANCE) => "Instance",
        Kind::Userdata(tag::VECTOR2) => "Vector2",
        Kind::Userdata(tag::CFRAME) => "CFrame",
        Kind::Userdata(tag::COLOR3) => "Color3",
        Kind::Userdata(tag::ENUM_ITEM) => "EnumItem",
        Kind::Userdata(tag::SIGNAL) => "RBXScriptSignal",
        Kind::Userdata(tag::CONNECTION) => "RBXScriptConnection",
        Kind::Buffer => "buffer",
        Kind::Userdata(_) | Kind::Other => "userdata",
    }
}

/// What a C function's body gets: its arguments and, during a backend call, the backend's shared state.
pub(crate) struct Call<'a> {
    /// The arguments.
    pub(crate) args: Args<'a>,
    frame: Option<Frame>,
    shared: Option<&'a Shared>,
}

impl<'a> Call<'a> {
    /// The running backend's shared state.
    pub(crate) fn shared(&self) -> Result<&'a Shared, Raise> {
        self.shared.ok_or_else(no_frame)
    }

    /// [`with_ctx`] without reading the thread-local again.
    #[inline]
    pub(crate) fn with_ctx<R>(&self, f: impl FnOnce(&mut HostContext<'_>) -> R) -> Result<R, Raise> {
        self.frame.ok_or_else(no_frame)?.lend(f)
    }
}

/// Declares a C function whose body is the safe function `$body: fn(&Call) -> Ret`.
macro_rules! lua_fn {
    ($vis:vis $name:ident => $body:path) => {
        #[allow(unsafe_code, reason = "a C function for Luau; the unsafe part is `ffi::run`")]
        $vis unsafe extern "C-unwind" fn $name(state: *mut $crate::ffi::lua_State) -> ::std::ffi::c_int {
            // SAFETY: Luau calls this as a C function, with `state` running it.
            unsafe { $crate::ffi::run(state, $body) }
        }
    };
}
pub(crate) use lua_fn;

/// The body of every C function: runs `body`, then pushes its results or raises its error.
///
/// # Safety
///
/// `state` must be the state of a running C function call.
pub(crate) unsafe fn run(state: *mut lua_State, body: fn(&Call<'_>) -> Ret) -> c_int {
    let frame = Frame::current();
    let shared = frame.map(Frame::shared);
    let call = Call {
        args: Args {
            state,
            _call: PhantomData,
        },
        frame,
        shared,
    };
    // The body only reads the stack, so no Luau exception can cross this boundary.
    match catch_unwind(AssertUnwindSafe(|| body(&call))) {
        // SAFETY: `state` is running this call, as a C function called from Luau code; `lua_yield` raises (an
        // exception, like any raise here) when the call cannot yield, such as inside a metamethod.
        Ok(Ok(Out::Yield)) => unsafe { yield_to_host(state) },
        // SAFETY: `state` is running this call and `idx` is one of its arguments; anchoring may raise out of
        // memory, which unwinds through this frame to Luau.
        Ok(Ok(Out::KeepRef { idx, listener })) => unsafe { keep_ref(state, shared, idx, listener) },
        // SAFETY: `state` is running this call; pushing may raise (out of memory), which unwinds through this
        // frame to Luau, with nothing to clean up here but `out`, whose destructor runs.
        Ok(Ok(out)) => unsafe { push_out(state, shared, out) },
        // SAFETY: as above.
        Ok(Err(Raise(message))) => unsafe { raise(state, message) },
        // SAFETY: as above.
        Err(_) => unsafe { raise(state, "internal error in the engine's Luau bindings".to_owned()) },
    }
}

/// Raises `message` in the caller, prefixed with its position as `luaL_error` does.
///
/// # Safety
///
/// `state` must be running a C function.
unsafe fn raise(state: *mut lua_State, message: String) -> ! {
    // SAFETY: the caller's guarantee; level 1 is the function that called the C function.
    unsafe {
        ffi::luaL_where(state, 1);
        push_str(state, &message);
        drop(message);
        ffi::lua_concat(state, 2);
        ffi::lua_error(state)
    }
}

/// Yields the running thread to the host with the marker of a C-function yield; `outcome` reads the reason from
/// [`Shared::pending_yield`].
///
/// # Safety
///
/// `state` must be running a C function, which returns this function's result to Luau.
unsafe fn yield_to_host(state: *mut lua_State) -> c_int {
    // SAFETY: the caller's guarantee; two slots are reserved before pushing the two yielded values.
    unsafe {
        reserve(state, 2);
        ffi::lua_pushlightuserdata(state, yield_marker().0);
        ffi::lua_pushnumber(state, crate::outcome::CALL_YIELD);
        ffi::lua_yield(state, 2)
    }
}

/// Anchors value `idx` in the registry under `listener` (replacing, and unanchoring, what was there).
///
/// # Safety
///
/// `state` must be running a C function and `idx` must be one of its arguments.
unsafe fn keep_ref(state: *mut lua_State, shared: Option<&Shared>, idx: c_int, listener: ListenerKey) -> c_int {
    // SAFETY: the caller's guarantee; `lua_ref` copies the value into the registry without touching the stack, and
    // `lua_unref` frees a reference this module made.
    unsafe {
        let reference = ffi::lua_ref(state, idx);
        let replaced = shared
            .and_then(|s| s.callbacks.try_borrow_mut().ok())
            .map(|mut callbacks| callbacks.insert(listener, reference));
        match replaced {
            Some(Some(old)) => {
                ffi::lua_unref(state, old);
            }
            Some(None) => {}
            // No backend state to keep it in: let it go rather than leak it.
            None => {
                ffi::lua_unref(state, reference);
            }
        }
        0
    }
}

/// The function anchored under registry reference `reference` (see [`Out::KeepRef`]).
pub(crate) fn referenced_function(lua: &Lua, reference: c_int) -> mlua::Result<Function> {
    // SAFETY: the reference was made by `keep_ref` and is still anchored; the one pushed value is returned.
    unsafe {
        lua.exec_raw::<Function>((), |state| {
            ffi::lua_rawgeti_(state, ffi::LUA_REGISTRYINDEX, reference);
        })
    }
}

/// Frees registry reference `reference` (see [`Out::KeepRef`]).
pub(crate) fn unreference(lua: &Lua, reference: c_int) {
    // SAFETY: the reference was made by `keep_ref` and is freed once; `lua_unref` does not touch the stack.
    let _ = unsafe {
        lua.exec_raw::<()>((), |state| {
            ffi::lua_unref(state, reference);
        })
    };
}

/// Pushes the results of a C function and returns how many.
///
/// # Safety
///
/// `state` must be running a C function (or be inside `exec_raw`).
unsafe fn push_out(state: *mut lua_State, shared: Option<&Shared>, out: Out) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe {
        match out {
            // Handled by `run`; nothing to push.
            Out::None | Out::Yield | Out::KeepRef { .. } => 0,
            Out::One(push) => push_value(state, shared, push),
            Out::Many(pushes) => {
                let mut count = 0;
                for push in pushes {
                    count += push_value(state, shared, push);
                }
                count
            }
        }
    }
}

/// Makes room for `n` more values on the stack.
unsafe fn reserve(state: *mut lua_State, n: usize) {
    let n = c_int::try_from(n).unwrap_or(c_int::MAX);
    // SAFETY: the caller's guarantee that `state` is valid. Growing may raise out of memory, as any push may.
    unsafe {
        if ffi::lua_checkstack(state, n) == 0 {
            push_str(state, "stack overflow");
            ffi::lua_error(state);
        }
    }
}

/// Pushes `text` as a Luau string.
unsafe fn push_str(state: *mut lua_State, text: &str) {
    // SAFETY: the caller's guarantee that `state` is valid; the bytes are copied.
    unsafe { ffi::lua_pushlstring_(state, text.as_ptr().cast::<c_char>(), text.len()) }
}

/// Creates a tagged userdata holding `payload`, with its tag's metatable. Not for instances, whose block is larger
/// and whose destructor reads all of it ([`new_instance`]).
unsafe fn push_payload<T: Payload>(state: *mut lua_State, payload: T) {
    const { assert!(T::TAG != tag::INSTANCE, "instances are created by `new_instance`") };
    // SAFETY: every tag we use has a metatable registered before any script runs (`set_metatable`); the block is
    // `size_of::<T>()` bytes, written once, unaligned-safe.
    unsafe {
        let data = ffi::lua_newuserdatataggedwithmetatable(state, size_of::<T>(), T::TAG);
        ptr::write_unaligned(data.cast::<T>(), payload);
    }
}

/// Pushes one value and returns how many slots it took (a tuple takes one per element).
///
/// # Safety
///
/// `state` must be valid and have room for one value (more is reserved here when needed).
pub(crate) unsafe fn push_value(state: *mut lua_State, shared: Option<&Shared>, push: Push) -> c_int {
    // SAFETY: the caller's guarantee for every call below.
    unsafe {
        match push {
            Push::Nil => ffi::lua_pushnil(state),
            Push::Bool(b) => ffi::lua_pushboolean(state, c_int::from(b)),
            Push::Number(n) => ffi::lua_pushnumber(state, n),
            Push::Text(text) => push_str(state, &text),
            Push::Static(text) => push_str(state, text),
            Push::Vector3(v) => ffi::lua_pushvector(state, v.x, v.y, v.z),
            Push::Vector2(v) => push_payload(state, v),
            Push::CFrame(cf) => push_payload(state, cf),
            Push::Color3(c) => push_payload(state, c),
            Push::Instance(id) => push_instance(state, shared, id),
            Push::Enum(item) => push_enum(state, shared, item),
            Push::Signal(signal) => push_payload(state, signal),
            Push::Connection(connection) => push_payload(state, connection),
            Push::Value(value) => match value {
                Value::Bool(b) => ffi::lua_pushboolean(state, c_int::from(b)),
                Value::Int(i) => ffi::lua_pushnumber(state, i as f64),
                Value::Number(n) => ffi::lua_pushnumber(state, n),
                Value::String(s) => push_str(state, &s),
                Value::Vector2(v) => push_payload(state, v),
                Value::Vector3(v) => ffi::lua_pushvector(state, v.x, v.y, v.z),
                Value::CFrame(cf) => push_payload(state, cf),
                Value::Color3(c) => push_payload(state, c),
                Value::Ref(Some(id)) => push_instance(state, shared, id),
                Value::Ref(None) => ffi::lua_pushnil(state),
                Value::Enum(item) => return push_script(state, shared, ScriptValue::Enum(item)),
            },
            Push::Script(value) => return push_script(state, shared, value),
            Push::Field { table, name } => {
                ffi::lua_rawgeti_(state, ffi::LUA_REGISTRYINDEX, table);
                push_str(state, name);
                ffi::lua_rawget(state, -2);
                ffi::lua_remove(state, -2);
            }
            Push::InstanceMethod(name) => {
                push_str(state, name);
                ffi::lua_pushcclosurek(state, crate::instance::method_value, c"method".as_ptr(), 1, None);
            }
        }
        1
    }
}

/// Pushes the userdata of an instance: the cached one while a script holds it, so `==`, `rawequal` and table keys
/// see one object per instance. A cached userdata made outside a backend call is pinned now.
unsafe fn push_instance(state: *mut lua_State, shared: Option<&Shared>, id: Id) {
    let cache = shared.map(|s| s.refs.instances);
    let slot = c_int::try_from(id.index()).ok().and_then(|i| i.checked_add(1));
    // SAFETY: the cache is a weak-valued table anchored in the registry for the backend's life; every push below
    // is balanced, and the stack has the slots `push_value` promised plus the two this uses (LUA_MINSTACK). A
    // userdata with the INSTANCE tag holds an `InstanceBlock` (`new_instance`).
    unsafe {
        let (Some(cache), Some(slot)) = (cache, slot) else {
            new_instance(state, id);
            return;
        };
        ffi::lua_rawgeti_(state, ffi::LUA_REGISTRYINDEX, cache);
        ffi::lua_rawgeti_(state, -1, slot);
        let data = ffi::lua_touserdatatagged(state, -1, tag::INSTANCE).cast::<InstanceBlock>();
        if !data.is_null() {
            let cached = ptr::read_unaligned(data);
            if cached.id == id {
                if cached.pinned_in == 0 {
                    ptr::write_unaligned(
                        data,
                        InstanceBlock {
                            id,
                            pinned_in: pin_in_frame(id),
                        },
                    );
                }
                ffi::lua_remove(state, -2);
                return;
            }
        }
        ffi::lua_pop(state, 1);
        new_instance(state, id);
        ffi::lua_pushvalue(state, -1);
        ffi::lua_rawseti_(state, -3, slot);
        ffi::lua_remove(state, -2);
    }
}

/// Creates a new userdata for `id`, pinning the instance in the running call's data model when there is one.
///
/// # Safety
///
/// `state` must be valid and have room for one value.
unsafe fn new_instance(state: *mut lua_State, id: Id) {
    // SAFETY: the INSTANCE tag has its metatable (and destructor) registered before any script runs. The pin is
    // taken only once the allocation (which may raise) succeeded, so a failed one leaves no pin; from here on the
    // destructor gives it back. Nothing between allocating and writing the block can run the collector.
    unsafe {
        let data = ffi::lua_newuserdatataggedwithmetatable(state, size_of::<InstanceBlock>(), tag::INSTANCE);
        let block = InstanceBlock {
            id,
            pinned_in: pin_in_frame(id),
        };
        ptr::write_unaligned(data.cast::<InstanceBlock>(), block);
    }
}

/// Pins `id` in the running call's data model and returns that data model's uid, or 0 when no pin was taken: no
/// call is running, its context is lent out, or the id names nothing that can be read.
fn pin_in_frame(id: Id) -> u64 {
    let Some(frame) = Frame::current() else {
        return 0;
    };
    frame
        .lend(|ctx| if ctx.dm.pin(id) { ctx.dm.uid() } else { 0 })
        .unwrap_or(0)
}

/// Pushes the cached userdata of an enum item.
unsafe fn push_enum(state: *mut lua_State, shared: Option<&Shared>, item: EnumRef) {
    // SAFETY: the item table is anchored in the registry; one balanced push and remove.
    unsafe {
        match shared.and_then(|s| s.enums.flat_index(item).map(|i| (s.refs.enum_items, i))) {
            Some((table, index)) => {
                ffi::lua_rawgeti_(state, ffi::LUA_REGISTRYINDEX, table);
                ffi::lua_rawgeti_(state, -1, index);
                ffi::lua_remove(state, -2);
            }
            None => push_payload(state, item),
        }
    }
}

/// Pushes a script value: a tuple as several values, a list or map as a new table.
unsafe fn push_script(state: *mut lua_State, shared: Option<&Shared>, value: ScriptValue) -> c_int {
    // SAFETY: the caller's guarantee; nested values are pushed one at a time and stored into the table at -2.
    unsafe {
        match value {
            ScriptValue::Nil => ffi::lua_pushnil(state),
            ScriptValue::Bool(b) => ffi::lua_pushboolean(state, c_int::from(b)),
            ScriptValue::Number(n) => ffi::lua_pushnumber(state, n),
            ScriptValue::String(s) => push_str(state, &s),
            ScriptValue::Vector2(v) => push_payload(state, v),
            ScriptValue::Vector3(v) => ffi::lua_pushvector(state, v.x, v.y, v.z),
            ScriptValue::CFrame(cf) => push_payload(state, cf),
            ScriptValue::Color3(c) => push_payload(state, c),
            ScriptValue::Instance(id) => push_instance(state, shared, id),
            ScriptValue::Enum(item) => match shared.and_then(|s| s.enums.find(&item)) {
                Some(item) => push_enum(state, shared, item),
                None => push_str(state, &item.to_string()),
            },
            ScriptValue::List(items) => {
                reserve(state, 2);
                ffi::lua_createtable(state, c_int::try_from(items.len()).unwrap_or(0), 0);
                for (i, item) in items.into_iter().enumerate() {
                    let n = push_script(state, shared, item);
                    if n != 1 {
                        ffi::lua_pop(state, n);
                        ffi::lua_pushnil(state);
                    }
                    ffi::lua_rawseti_(state, -2, c_int::try_from(i + 1).unwrap_or(c_int::MAX));
                }
            }
            ScriptValue::Map(entries) => {
                reserve(state, 3);
                ffi::lua_createtable(state, 0, c_int::try_from(entries.len()).unwrap_or(0));
                for (key, item) in entries {
                    push_str(state, &key);
                    let n = push_script(state, shared, item);
                    if n != 1 {
                        ffi::lua_pop(state, n);
                        ffi::lua_pushnil(state);
                    }
                    ffi::lua_rawset(state, -3);
                }
            }
            ScriptValue::Tuple(values) => {
                reserve(state, values.len());
                let mut count = 0;
                for value in values {
                    count += push_script(state, shared, value);
                }
                return count;
            }
        }
        1
    }
}

// ---------------------------------------------------------------------------------------------------------------
// VM hooks: the interrupt and the atom callback.
// ---------------------------------------------------------------------------------------------------------------

/// The message of a timeout; [`Shared::timed_out`] tells it apart from a script's own error.
pub(crate) const TIMEOUT_MESSAGE: &str = "script exceeded its time budget";

/// Luau's interrupt: called at every safepoint (loop back-edge, call, return). It polls the call's
/// [`Deadline`](metallic_script::Deadline), which reads the clock once per few hundred polls, and stops the
/// thread once it has passed. No mlua trampoline is involved: that alone made hot loops 1.5–1.9× slower.
unsafe extern "C-unwind" fn interrupt(state: *mut lua_State, gc: c_int) {
    // A GC step cannot survive an exception; the next safepoint will see the deadline.
    if gc >= 0 {
        return;
    }
    let Some(frame) = Frame::current().filter(|frame| !frame.shared().ctx_lent.get()) else {
        return;
    };
    // SAFETY: the frame's context is alive and not lent out (checked above), so this short `&mut` to its deadline
    // is the only reference in use.
    let expired = unsafe { (*frame.ctx).deadline.poll() };
    if !expired {
        return;
    }
    // SAFETY: `state` is the running thread; level 0 is the Luau function at the safepoint.
    let line = unsafe {
        let mut ar: ffi::lua_Debug = std::mem::zeroed();
        (ffi::lua_getinfo(state, 0, c"l".as_ptr(), &mut ar) != 0 && ar.currentline > 0)
            .then(|| u32::try_from(ar.currentline).ok())
            .flatten()
    };
    // SAFETY: the frame's `Shared` is alive (see `FrameGuard`). Raising from the interrupt is how Luau stops a
    // thread; nothing in this frame needs dropping.
    unsafe {
        (*frame.shared).timed_out.set(Some(line));
        push_str(state, TIMEOUT_MESSAGE);
        ffi::lua_error(state);
    }
}

/// Luau's atom callback: gives each member name an id once, cached in the string, so member lookups index an
/// array instead of hashing (see `atoms.rs`).
unsafe extern "C-unwind" fn useratom(_state: *mut lua_State, s: *const c_char, len: usize) -> i16 {
    // SAFETY: Luau passes the string's bytes and their length.
    let bytes = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len) };
    catch_unwind(|| atoms::atom_of(bytes)).unwrap_or(-1)
}

// ---------------------------------------------------------------------------------------------------------------
// Set-up through mlua: callbacks, metatables, references and the bridge between mlua values and payloads.
// ---------------------------------------------------------------------------------------------------------------

/// Installs the interrupt, the atom callback and the `Instance` destructor. Called once, before any script is
/// loaded.
pub(crate) fn install_callbacks(lua: &Lua) -> mlua::Result<()> {
    // SAFETY: `exec_raw` hands us a valid state; the callbacks are VM-wide and these two are not used by mlua, and
    // neither is the INSTANCE tag, whose destructor is set before any userdata of it exists.
    unsafe {
        lua.exec_raw::<()>((), |state| {
            let callbacks = ffi::lua_callbacks(state);
            (*callbacks).interrupt = Some(interrupt);
            (*callbacks).useratom = Some(useratom);
            ffi::lua_setuserdatadtor(state, tag::INSTANCE, Some(instance_dtor));
        })
    }
}

/// Installs (`true`) or removes the interrupt. Without it no time budget is enforced.
pub(crate) fn set_interrupt(lua: &Lua, enabled: bool) -> mlua::Result<()> {
    // SAFETY: as in `install_callbacks`.
    unsafe {
        lua.exec_raw::<()>((), |state| {
            (*ffi::lua_callbacks(state)).interrupt = if enabled { Some(interrupt) } else { None };
        })
    }
}

/// Wraps one of this crate's C functions (declared with `lua_fn!`) as an mlua function.
pub(crate) fn c_function(lua: &Lua, f: ffi::lua_CFunction) -> mlua::Result<Function> {
    // SAFETY: every function passed here is a `lua_fn!` trampoline, which follows the C function protocol.
    unsafe { lua.create_c_function(f) }
}

/// Wraps one of this crate's C functions as a closure whose upvalue 1 is `name` (a method fetched as a value
/// reads its name there).
pub(crate) fn c_closure(lua: &Lua, f: ffi::lua_CFunction, name: &str) -> mlua::Result<Function> {
    // SAFETY: the name is pushed, then captured as the closure's only upvalue; the closure is the one result.
    unsafe {
        lua.exec_raw::<Function>((), |state| {
            push_str(state, name);
            ffi::lua_pushcclosurek(state, f, c"method".as_ptr(), 1, None);
        })
    }
}

/// Runs Luau's sandbox on the globals: every library table and the globals become read-only, and imports and
/// builtins are resolved at load time (`safeenv`). Then `writable` is added to the globals, as tables scripts may
/// write (`_G` and `shared`, as in Roblox).
pub(crate) fn sandbox(lua: &Lua, writable: &[(&str, &Table)]) -> mlua::Result<()> {
    // SAFETY: `luaL_sandbox` works on the globals of the state `exec_raw` hands us; the writes below go through
    // `lua_rawset` on the globals with read-only lifted for the moment, and leave the stack balanced.
    unsafe {
        lua.exec_raw::<()>((), |state| ffi::luaL_sandbox(state, 1))?;
        for (name, table) in writable {
            lua.exec_raw::<()>(*table, |state| {
                ffi::lua_setreadonly(state, ffi::LUA_GLOBALSINDEX, 0);
                push_str(state, name);
                ffi::lua_pushvalue(state, -2);
                ffi::lua_rawset(state, ffi::LUA_GLOBALSINDEX);
                ffi::lua_setreadonly(state, ffi::LUA_GLOBALSINDEX, 1);
                ffi::lua_pop(state, 1);
            })?;
        }
    }
    Ok(())
}

/// Makes `metatable` the metatable of every userdata with `tag`. Once per tag.
pub(crate) fn set_metatable(lua: &Lua, tag: c_int, metatable: &Table) -> mlua::Result<()> {
    // SAFETY: the table is the only argument, on top; `lua_setuserdatametatable` pops it.
    unsafe { lua.exec_raw::<()>(metatable, |state| ffi::lua_setuserdatametatable(state, tag)) }
}

/// Makes `metatable` the metatable of Luau's `vector` type (every `Vector3`).
pub(crate) fn set_vector_metatable(lua: &Lua, metatable: &Table) -> mlua::Result<()> {
    // SAFETY: the table is the argument at -1; a vector is pushed, given it as metatable (which applies to the
    // whole type) and popped with the table.
    unsafe {
        lua.exec_raw::<()>(metatable, |state| {
            ffi::lua_pushvector(state, 0.0, 0.0, 0.0);
            ffi::lua_pushvalue(state, -2);
            ffi::lua_setmetatable(state, -2);
            ffi::lua_pop(state, 2);
        })
    }
}

/// Anchors `value` in the registry and returns its reference, for C functions to push it fast.
pub(crate) fn registry_ref(lua: &Lua, value: impl IntoLuaMulti) -> mlua::Result<c_int> {
    let mut reference = -1;
    // SAFETY: the value is the argument on top; `lua_ref` copies it into the registry without popping.
    unsafe {
        lua.exec_raw::<()>(value, |state| {
            reference = ffi::lua_ref(state, -1);
            ffi::lua_pop(state, 1);
        })?;
    }
    Ok(reference)
}

/// Pushes `values` (engine values) and returns them as mlua values: resume arguments, event arguments.
pub(crate) fn to_lua(lua: &Lua, shared: &Shared, values: Vec<ScriptValue>) -> mlua::Result<MultiValue> {
    // SAFETY: `exec_raw` runs the closure protected, so an out-of-memory error while pushing becomes an mlua
    // error; every pushed value is returned.
    unsafe {
        lua.exec_raw::<MultiValue>((), |state| {
            reserve(state, values.len());
            for value in values {
                push_script(state, Some(shared), value);
            }
        })
    }
}

/// Pushes `pushes` and returns them as mlua values. Without `shared`, instances and enum items get new userdata
/// instead of their cached ones (only while the backend is being built).
pub(crate) fn pushes_to_lua(lua: &Lua, shared: Option<&Shared>, pushes: Vec<Push>) -> mlua::Result<MultiValue> {
    // SAFETY: as in `to_lua`.
    unsafe {
        lua.exec_raw::<MultiValue>((), |state| {
            reserve(state, pushes.len());
            for push in pushes {
                push_value(state, shared, push);
            }
        })
    }
}

/// Reads mlua values as engine values (return values, yield arguments). A value with no engine equivalent (a
/// function, a coroutine) is an error naming it.
pub(crate) fn from_lua(lua: &Lua, shared: &Shared, values: MultiValue) -> Result<Vec<ScriptValue>, String> {
    let count = values.len();
    let mut out = Ok(Vec::with_capacity(count));
    // SAFETY: the values are pushed as arguments, so they are the top `count` slots; reading them never raises.
    let ran = unsafe {
        lua.exec_raw::<()>(values, |state| {
            let args = Args {
                state,
                _call: PhantomData,
            };
            let top = args.len();
            let first = top - c_int::try_from(count).unwrap_or(0) + 1;
            out = (first..=top)
                .map(|i| convert::read_script(&args, i, shared))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.0);
            ffi::lua_settop(state, top - c_int::try_from(count).unwrap_or(0));
        })
    };
    ran.map_err(|e| e.to_string())?;
    out
}

/// The payload of `value` if it is a userdata with `T`'s tag.
pub(crate) fn payload_of<T: Payload>(lua: &Lua, value: &mlua::Value) -> Option<T> {
    if !matches!(value, mlua::Value::UserData(_)) {
        return None;
    }
    let mut out = None;
    // SAFETY: the value is the argument on top; reading a payload never raises.
    unsafe {
        lua.exec_raw::<()>(value, |state| {
            let args = Args {
                state,
                _call: PhantomData,
            };
            out = args.payload::<T>(args.len());
            ffi::lua_pop(state, 1);
        })
        .ok()?;
    }
    out
}

/// A light userdata that marks the yields of `task.wait` and `signal:Wait()`. Scripts cannot get hold of it.
pub(crate) fn yield_marker() -> mlua::LightUserData {
    static MARKER: u8 = 0;
    mlua::LightUserData(ptr::from_ref(&MARKER).cast::<c_void>().cast_mut())
}
