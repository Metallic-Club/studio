//! Lua 5.1 pattern matching (`string.find`, `match`, `gmatch`, `gsub`) with a step
//! counter.
//!
//! Luau's own implementations run uninterruptibly in C, so a pattern such as `.-b` on a
//! large string could hang the host. These versions follow the same semantics (they are a
//! port of the reference matcher) but count steps and give up with `pattern too complex`
//! after [`MAX_STEPS`] steps in one match attempt, or once the script's time budget is gone.
//! Deadline polling continues across attempts, including scans with many short failures.

use std::rc::Rc;
use std::time::Instant;

use mlua::{Error, Function, Lua, LuaString, MultiValue, Result, Table, Value};

use crate::task_lib::{Scheduler, TIMEOUT_MSG};
use crate::vm::script_location;

/// Hard cap on matcher steps per starting-position attempt.
pub(crate) const MAX_STEPS: u64 = 10_000_000;
/// Charged-work interval for deadline checks, independent of attempt boundaries.
const DEADLINE_POLL_STEPS: u64 = 4096;
/// Cap on the nesting depth of the matcher (as in the reference implementation).
const MAX_DEPTH: u32 = 200;
/// Cap on captures per pattern.
const MAX_CAPTURES: usize = 32;
/// Cap on the size of a `gsub` result.
const MAX_RESULT: usize = 64 * 1024 * 1024;
/// Pattern bytes of a `[...]` class read per extra matcher step (#166). Shorter reads cost
/// nothing extra, so ordinary patterns count exactly as before; a long class is charged in
/// proportion to its length, so the step cap and the deadline check bound it.
const CLASS_BYTES_PER_STEP: u64 = 16;

const CAP_UNFINISHED: isize = -1;
const CAP_POSITION: isize = -2;

/// Why matching stopped without a result.
enum Stop {
    /// The script's time budget ran out.
    Timeout,
    /// Step cap exhausted.
    TooComplex,
    /// Malformed pattern or bad capture reference.
    Msg(&'static str),
}

impl From<Stop> for Error {
    fn from(s: Stop) -> Error {
        match s {
            Stop::TooComplex => Error::runtime("pattern too complex"),
            Stop::Timeout => Error::runtime(TIMEOUT_MSG),
            Stop::Msg(m) => Error::runtime(m),
        }
    }
}

type R<T> = std::result::Result<T, Stop>;

/// Matcher state for one call.
struct State<'a> {
    src: &'a [u8],
    pat: &'a [u8],
    level: usize,
    caps: [(usize, isize); MAX_CAPTURES],
    /// Complexity charged to the current starting-position attempt.
    steps: u64,
    /// Charged work since the last deadline poll, retained across attempts.
    poll_steps: u64,
    deadline: Option<Instant>,
    depth: u32,
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn match_class(c: u8, cl: u8) -> bool {
    let res = match cl.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c < 32 || c == 127,
        b'd' => c.is_ascii_digit(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => is_space(c),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        _ => return cl == c,
    };
    if cl.is_ascii_uppercase() { !res } else { res }
}

impl<'a> State<'a> {
    fn new(src: &'a [u8], pat: &'a [u8], deadline: Option<Instant>) -> Self {
        State {
            src,
            pat,
            level: 0,
            caps: [(0, 0); MAX_CAPTURES],
            steps: 0,
            poll_steps: 0,
            deadline,
            depth: 0,
        }
    }

    /// Starts an independent attempt without resetting aggregate deadline polling.
    fn begin_attempt(&mut self) {
        self.level = 0;
        self.steps = 0;
    }

    fn tick(&mut self, n: u64) -> R<()> {
        self.steps = self.steps.saturating_add(n);
        if self.steps > MAX_STEPS {
            return Err(Stop::TooComplex);
        }
        self.poll_steps = self.poll_steps.saturating_add(n);
        if self.poll_steps >= DEADLINE_POLL_STEPS {
            self.poll_steps %= DEADLINE_POLL_STEPS;
            if let Some(d) = self.deadline
                && Instant::now() >= d
            {
                return Err(Stop::Timeout);
            }
        }
        Ok(())
    }

    /// Charges for reading `bytes` bytes of a `[...]` class.
    fn charge_class(&mut self, bytes: usize) -> R<()> {
        match bytes as u64 / CLASS_BYTES_PER_STEP {
            0 => Ok(()),
            extra => self.tick(extra),
        }
    }

    fn class_end(&mut self, mut p: usize) -> R<usize> {
        let start = p;
        let pat = self.pat;
        let c = pat[p];
        p += 1;
        if c == b'%' {
            if p >= pat.len() {
                return Err(Stop::Msg("malformed pattern (ends with '%')"));
            }
            return Ok(p + 1);
        }
        if c == b'[' {
            if pat.get(p) == Some(&b'^') {
                p += 1;
            }
            loop {
                if p >= pat.len() {
                    return Err(Stop::Msg("malformed pattern (missing ']')"));
                }
                let cc = pat[p];
                p += 1;
                if cc == b'%' && p < pat.len() {
                    p += 1;
                }
                if pat.get(p) == Some(&b']') {
                    break;
                }
            }
            self.charge_class(p - start)?;
            return Ok(p + 1);
        }
        Ok(p)
    }

    /// `p` points at `[`, `ec` at the closing `]`. Charges for the class bytes read.
    fn match_bracket_class(&mut self, c: u8, p: usize, ec: usize) -> R<bool> {
        let (res, end) = bracket_class(self.pat, c, p, ec);
        self.charge_class(end - p)?;
        Ok(res)
    }

    fn single_match(&mut self, c: u8, p: usize, ep: usize) -> R<bool> {
        Ok(match self.pat[p] {
            b'.' => true,
            b'%' => match_class(c, self.pat[p + 1]),
            b'[' => return self.match_bracket_class(c, p, ep - 1),
            pc => pc == c,
        })
    }

    fn match_balance(&mut self, s: usize, p: usize) -> R<Option<usize>> {
        if p + 1 >= self.pat.len() {
            return Err(Stop::Msg("missing arguments to '%b'"));
        }
        if s >= self.src.len() || self.src[s] != self.pat[p] {
            return Ok(None);
        }
        let (b, e) = (self.pat[p], self.pat[p + 1]);
        let mut cont = 1;
        let mut i = s + 1;
        while i < self.src.len() {
            self.tick(1)?;
            let c = self.src[i];
            if c == e {
                cont -= 1;
                if cont == 0 {
                    return Ok(Some(i + 1));
                }
            } else if c == b {
                cont += 1;
            }
            i += 1;
        }
        Ok(None)
    }

    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> R<Option<usize>> {
        let mut i = 0;
        while s + i < self.src.len() && self.single_match(self.src[s + i], p, ep)? {
            self.tick(1)?;
            i += 1;
        }
        loop {
            if let Some(r) = self.do_match(s + i, ep + 1)? {
                return Ok(Some(r));
            }
            if i == 0 {
                return Ok(None);
            }
            i -= 1;
        }
    }

    fn min_expand(&mut self, mut s: usize, p: usize, ep: usize) -> R<Option<usize>> {
        loop {
            if let Some(r) = self.do_match(s, ep + 1)? {
                return Ok(Some(r));
            }
            if s < self.src.len() && self.single_match(self.src[s], p, ep)? {
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }

    fn start_capture(&mut self, s: usize, p: usize, what: isize) -> R<Option<usize>> {
        if self.level >= MAX_CAPTURES {
            return Err(Stop::Msg("too many captures"));
        }
        self.caps[self.level] = (s, what);
        self.level += 1;
        let res = self.do_match(s, p)?;
        if res.is_none() {
            self.level -= 1;
        }
        Ok(res)
    }

    fn end_capture(&mut self, s: usize, p: usize) -> R<Option<usize>> {
        let l = (0..self.level)
            .rev()
            .find(|&l| self.caps[l].1 == CAP_UNFINISHED)
            .ok_or(Stop::Msg("invalid pattern capture"))?;
        self.caps[l].1 = (s - self.caps[l].0) as isize;
        let res = self.do_match(s, p)?;
        if res.is_none() {
            self.caps[l].1 = CAP_UNFINISHED;
        }
        Ok(res)
    }

    fn match_capture(&self, s: usize, digit: u8) -> R<Option<usize>> {
        let l = (digit as usize).wrapping_sub(b'1' as usize);
        if l >= self.level || self.caps[l].1 == CAP_UNFINISHED {
            return Err(Stop::Msg("invalid capture index"));
        }
        let (init, len) = self.caps[l];
        if len < 0 {
            return Ok(None);
        }
        let len = len as usize;
        if self.src.len() - s >= len && self.src[init..init + len] == self.src[s..s + len] {
            Ok(Some(s + len))
        } else {
            Ok(None)
        }
    }

    fn do_match(&mut self, s: usize, p: usize) -> R<Option<usize>> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Stop::TooComplex);
        }
        let r = self.match_inner(s, p);
        self.depth -= 1;
        r
    }

    fn match_inner(&mut self, mut s: usize, mut p: usize) -> R<Option<usize>> {
        let plen = self.pat.len();
        loop {
            self.tick(1)?;
            if p >= plen {
                return Ok(Some(s));
            }
            let next = self.pat.get(p + 1).copied();
            match self.pat[p] {
                b'(' => {
                    return if next == Some(b')') {
                        self.start_capture(s, p + 2, CAP_POSITION)
                    } else {
                        self.start_capture(s, p + 1, CAP_UNFINISHED)
                    };
                }
                b')' => return self.end_capture(s, p + 1),
                b'$' if p + 1 == plen => return Ok((s == self.src.len()).then_some(s)),
                b'%' if next == Some(b'b') => match self.match_balance(s, p + 2)? {
                    Some(ns) => {
                        s = ns;
                        p += 4;
                        continue;
                    }
                    None => return Ok(None),
                },
                b'%' if next == Some(b'f') => {
                    p += 2;
                    if self.pat.get(p) != Some(&b'[') {
                        return Err(Stop::Msg("missing '[' after '%f' in pattern"));
                    }
                    let ep = self.class_end(p)?;
                    let prev = if s == 0 { 0 } else { self.src[s - 1] };
                    let cur = self.src.get(s).copied().unwrap_or(0);
                    if !self.match_bracket_class(prev, p, ep - 1)?
                        && self.match_bracket_class(cur, p, ep - 1)?
                    {
                        p = ep;
                        continue;
                    }
                    return Ok(None);
                }
                b'%' if next.is_some_and(|c| c.is_ascii_digit()) => {
                    match self.match_capture(s, self.pat[p + 1])? {
                        Some(ns) => {
                            s = ns;
                            p += 2;
                            continue;
                        }
                        None => return Ok(None),
                    }
                }
                _ => {}
            }
            let ep = self.class_end(p)?;
            let m = s < self.src.len() && self.single_match(self.src[s], p, ep)?;
            match self.pat.get(ep) {
                Some(b'?') => {
                    if m && let Some(r) = self.do_match(s + 1, ep + 1)? {
                        return Ok(Some(r));
                    }
                    p = ep + 1;
                }
                Some(b'*') => return self.max_expand(s, p, ep),
                Some(b'+') => {
                    return if m {
                        self.max_expand(s + 1, p, ep)
                    } else {
                        Ok(None)
                    };
                }
                Some(b'-') => return self.min_expand(s, p, ep),
                _ => {
                    if !m {
                        return Ok(None);
                    }
                    s += 1;
                    p = ep;
                }
            }
        }
    }

    fn one_capture(&self, lua: &Lua, i: usize, s: usize, e: usize) -> Result<Value> {
        if i >= self.level {
            if i == 0 {
                return Ok(Value::String(lua.create_string(&self.src[s..e])?));
            }
            return Err(Error::runtime("invalid capture index"));
        }
        let (init, len) = self.caps[i];
        match len {
            CAP_UNFINISHED => Err(Error::runtime("unfinished capture")),
            CAP_POSITION => Ok(Value::Number((init + 1) as f64)),
            len => Ok(Value::String(
                lua.create_string(&self.src[init..init + len as usize])?,
            )),
        }
    }

    /// All captures; the whole match if the pattern has none and `whole` is set.
    fn captures(&self, lua: &Lua, s: usize, e: usize, whole: bool) -> Result<Vec<Value>> {
        let n = if self.level == 0 && whole {
            1
        } else {
            self.level
        };
        (0..n).map(|i| self.one_capture(lua, i, s, e)).collect()
    }
}

/// Tests `c` against the `[...]` class at `p` (closing `]` at `ec`). Returns the result and
/// where reading stopped, so the caller can charge for the bytes read.
fn bracket_class(pat: &[u8], c: u8, mut p: usize, ec: usize) -> (bool, usize) {
    let mut sig = true;
    if pat.get(p + 1) == Some(&b'^') {
        sig = false;
        p += 1;
    }
    loop {
        p += 1;
        if p >= ec {
            break;
        }
        if pat[p] == b'%' {
            p += 1;
            if match_class(c, pat[p]) {
                return (sig, p);
            }
        } else if pat.get(p + 1) == Some(&b'-') && p + 2 < ec {
            p += 2;
            if pat[p - 2] <= c && c <= pat[p] {
                return (sig, p);
            }
        } else if pat[p] == c {
            return (sig, p);
        }
    }
    (!sig, p)
}

/// Lua-style relative start position (as in Luau): 1-based, negative counts from the end.
fn pos_relat(pos: f64, len: usize) -> usize {
    let pos = pos.trunc();
    if pos > 0.0 {
        pos as usize
    } else if pos.is_nan() || pos == 0.0 || pos < -(len as f64) {
        1
    } else {
        (len as f64 + pos + 1.0) as usize
    }
}

fn has_specials(pat: &[u8]) -> bool {
    pat.iter().any(|c| b"^$*+?.([%-".contains(c))
}

/// Recognizes one literal byte conservatively; malformed magic stays on the matcher.
fn literal_byte(pat: &[u8]) -> Option<u8> {
    match pat {
        [byte] if !b"^$*+?.()[]%-".contains(byte) => Some(*byte),
        _ => None,
    }
}

/// Uses the same sticky timeout reporting as the general matcher.
fn check_deadline(lua: &Lua, sched: &Scheduler) -> Result<()> {
    if let Some(deadline) = sched.deadline.get()
        && Instant::now() >= deadline
    {
        return Err(stop_error(lua, sched, Stop::Timeout));
    }
    Ok(())
}

fn match_literal(
    lua: &Lua,
    sched: &Scheduler,
    src: &[u8],
    byte: u8,
    init: usize,
) -> Result<MultiValue> {
    for chunk in src[init..].chunks(DEADLINE_POLL_STEPS as usize) {
        check_deadline(lua, sched)?;
        if chunk.contains(&byte) {
            let result = lua.create_string([byte])?;
            check_deadline(lua, sched)?;
            return Ok(MultiValue::from_vec(vec![Value::String(result)]));
        }
    }
    check_deadline(lua, sched)?;
    Ok(MultiValue::from_vec(vec![Value::Nil]))
}

fn gsub_literal(
    lua: &Lua,
    sched: &Scheduler,
    src: &[u8],
    byte: u8,
    replacement: u8,
    max_s: usize,
) -> Result<MultiValue> {
    check_deadline(lua, sched)?;
    if src.len() > MAX_RESULT {
        return Err(Error::runtime(
            "string.gsub: result too large (limit 64 MB)",
        ));
    }
    let mut out = Vec::with_capacity(src.len());
    let mut count = 0;
    for chunk in src.chunks(DEADLINE_POLL_STEPS as usize) {
        check_deadline(lua, sched)?;
        let start = out.len();
        out.extend_from_slice(chunk);
        if count >= max_s {
            continue;
        }
        for current in &mut out[start..] {
            if *current == byte {
                *current = replacement;
                count += 1;
                if count >= max_s {
                    break;
                }
            }
        }
    }
    let result = lua.create_string(&out)?;
    check_deadline(lua, sched)?;
    Ok(MultiValue::from_vec(vec![
        Value::String(result),
        num(count),
    ]))
}

fn find_plain(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

fn num(n: usize) -> Value {
    Value::Number(n as f64)
}

fn find_aux(
    sched: &Scheduler,
    lua: &Lua,
    find: bool,
    (s, pat, init, plain): (LuaString, LuaString, Option<f64>, Option<Value>),
) -> Result<MultiValue> {
    let (src, pat) = (s.as_bytes(), pat.as_bytes());
    let init = pos_relat(init.unwrap_or(1.0), src.len()) - 1;
    if init > src.len() {
        return Ok(MultiValue::from_vec(vec![Value::Nil]));
    }
    let plain = plain.is_some_and(|v| !matches!(v, Value::Nil | Value::Boolean(false)));
    if find && (plain || !has_specials(&pat)) {
        return Ok(MultiValue::from_vec(match find_plain(&src, &pat, init) {
            Some(i) => vec![num(i + 1), num(i + pat.len())],
            None => vec![Value::Nil],
        }));
    }
    if !find && let Some(byte) = literal_byte(&pat) {
        return match_literal(lua, sched, &src, byte, init);
    }
    let anchor = pat.first() == Some(&b'^');
    let pstart = usize::from(anchor);
    let mut st = State::new(&src, &pat, sched.deadline.get());
    let mut s1 = init;
    loop {
        st.begin_attempt();
        if let Some(e) = st
            .do_match(s1, pstart)
            .map_err(|e| stop_error(lua, sched, e))?
        {
            let mut out = Vec::new();
            if find {
                out.push(num(s1 + 1));
                out.push(num(e));
            }
            out.extend(st.captures(lua, s1, e, !find)?);
            return Ok(MultiValue::from_vec(out));
        }
        s1 += 1;
        if s1 > src.len() || anchor {
            return Ok(MultiValue::from_vec(vec![Value::Nil]));
        }
    }
}

fn gmatch(sched: &Rc<Scheduler>, lua: &Lua, (s, pat): (LuaString, LuaString)) -> Result<Function> {
    let (src, pat) = (s.as_bytes().to_vec(), pat.as_bytes().to_vec());
    let pos = std::cell::Cell::new(0usize);
    let sched = sched.clone();
    lua.create_function(move |lua, ()| {
        let mut st = State::new(&src, &pat, sched.deadline.get());
        let mut s1 = pos.get();
        while s1 <= src.len() {
            st.begin_attempt();
            if let Some(e) = st.do_match(s1, 0).map_err(|e| stop_error(lua, &sched, e))? {
                pos.set(if e == s1 { e + 1 } else { e });
                return Ok(MultiValue::from_vec(st.captures(lua, s1, e, true)?));
            }
            s1 += 1;
        }
        pos.set(src.len() + 1);
        Ok(MultiValue::from_vec(vec![Value::Nil]))
    })
}

fn add_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    if out.len() + bytes.len() > MAX_RESULT {
        return Err(Error::runtime(
            "string.gsub: result too large (limit 64 MB)",
        ));
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn value_bytes(lua: &Lua, v: &Value) -> Result<Vec<u8>> {
    match v {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        other => match lua.coerce_string(other.clone())? {
            Some(s) => Ok(s.as_bytes().to_vec()),
            None => Err(Error::runtime(format!(
                "invalid replacement value (a {})",
                other.type_name()
            ))),
        },
    }
}

fn gsub(
    sched: &Scheduler,
    lua: &Lua,
    (s, pat, repl, max_n): (LuaString, LuaString, Value, Option<f64>),
) -> Result<MultiValue> {
    let (src, pat) = (s.as_bytes(), pat.as_bytes());
    if !matches!(
        repl,
        Value::String(_)
            | Value::Number(_)
            | Value::Integer(_)
            | Value::Function(_)
            | Value::Table(_)
    ) {
        return Err(Error::runtime(format!(
            "bad argument #3 to 'gsub' (string/function/table expected, got {})",
            repl.type_name()
        )));
    }
    let repl_str: Option<Vec<u8>> = match &repl {
        Value::Function(_) | Value::Table(_) => None,
        v => Some(value_bytes(lua, v)?),
    };
    let max_s = max_n.map_or(src.len() + 1, |n| if n < 0.0 { 0 } else { n as usize });
    if let Some(byte) = literal_byte(&pat)
        && let Some(replacement) = &repl_str
        && let [replacement] = replacement.as_slice()
        && *replacement != b'%'
    {
        return gsub_literal(lua, sched, &src, byte, *replacement, max_s);
    }
    let anchor = pat.first() == Some(&b'^');
    let pstart = usize::from(anchor);
    let mut st = State::new(&src, &pat, sched.deadline.get());
    let mut out: Vec<u8> = Vec::new();
    let (mut pos, mut n) = (0usize, 0usize);
    while n < max_s {
        st.begin_attempt();
        let e = st
            .do_match(pos, pstart)
            .map_err(|er| stop_error(lua, sched, er))?;
        if let Some(e) = e {
            n += 1;
            match &repl_str {
                Some(r) => {
                    let mut i = 0;
                    while i < r.len() {
                        if r[i] != b'%' {
                            add_bytes(&mut out, &r[i..=i])?;
                        } else {
                            i += 1;
                            match r.get(i) {
                                None => {
                                    return Err(Error::runtime(
                                        "invalid use of '%' in replacement string",
                                    ));
                                }
                                Some(b'0') => add_bytes(&mut out, &src[pos..e])?,
                                Some(d) if d.is_ascii_digit() => {
                                    let v = st.one_capture(lua, usize::from(*d - b'1'), pos, e)?;
                                    add_bytes(&mut out, &value_bytes(lua, &v)?)?;
                                }
                                Some(c) => add_bytes(&mut out, &[*c])?,
                            }
                        }
                        i += 1;
                    }
                }
                None => {
                    let result = match &repl {
                        Value::Table(t) => {
                            let key = st.one_capture(lua, 0, pos, e)?;
                            let t: &Table = t;
                            t.get::<Value>(key)?
                        }
                        Value::Function(f) => {
                            let args = MultiValue::from_vec(st.captures(lua, pos, e, true)?);
                            f.call::<Value>(args)
                                .map_err(|err| Error::runtime(crate::vm::format_error(&err)))?
                        }
                        _ => Value::Nil,
                    };
                    match result {
                        Value::Nil | Value::Boolean(false) => add_bytes(&mut out, &src[pos..e])?,
                        v => add_bytes(&mut out, &value_bytes(lua, &v)?)?,
                    }
                }
            }
        }
        match e {
            Some(e) if e > pos => pos = e,
            _ if pos < src.len() => {
                add_bytes(&mut out, &src[pos..=pos])?;
                pos += 1;
            }
            _ => break,
        }
        if anchor {
            break;
        }
    }
    if pos < src.len() {
        add_bytes(&mut out, &src[pos..])?;
    }
    Ok(MultiValue::from_vec(vec![
        Value::String(lua.create_string(&out)?),
        num(n),
    ]))
}

/// Replaces `string.find`, `match`, `gmatch` and `gsub` with the step-counted versions.
pub(crate) fn install(lua: &Lua, sched: &Rc<Scheduler>) -> Result<()> {
    let string: Table = lua.globals().get("string")?;
    let s = sched.clone();
    string.set(
        "find",
        lua.create_function(move |lua, a| find_aux(&s, lua, true, a))?,
    )?;
    let s = sched.clone();
    string.set(
        "match",
        lua.create_function(move |lua, (a, b, c): (LuaString, LuaString, Option<f64>)| {
            find_aux(&s, lua, false, (a, b, c, None))
        })?,
    )?;
    let s = sched.clone();
    string.set(
        "gmatch",
        lua.create_function(move |lua, a| gmatch(&s, lua, a))?,
    )?;
    let s = sched.clone();
    string.set("gsub", lua.create_function(move |lua, a| gsub(&s, lua, a))?)?;
    Ok(())
}

/// Converts a matcher stop into a Lua error; a timeout also records where the script was.
fn stop_error(lua: &Lua, sched: &Scheduler, stop: Stop) -> Error {
    if matches!(stop, Stop::Timeout) && sched.timeout_loc.borrow().is_none() {
        *sched.timeout_loc.borrow_mut() = Some(script_location(lua, 1));
    }
    stop.into()
}

#[cfg(test)]
mod attempt_budget_tests {
    use super::*;
    use std::time::Duration;

    struct Quiet;
    impl crate::vm::OutputSink for Quiet {
        fn print(&self, _: &str) {}
        fn warn(&self, _: &str) {}
        fn error(&self, _: &str) {}
    }

    #[test]
    fn expired_literal_paths_report_timeout_and_mark_it_sticky() {
        let lua = Lua::new();
        let sched = Scheduler::new(Rc::new(Quiet));
        sched.deadline.set(Some(Instant::now()));
        for result in [
            match_literal(&lua, &sched, b"", b'a', 0),
            match_literal(&lua, &sched, b"a", b'a', 0),
            gsub_literal(&lua, &sched, b"", b'a', b'b', 1),
            gsub_literal(&lua, &sched, b"a", b'a', b'b', 0),
            gsub_literal(&lua, &sched, b"a", b'a', b'b', 1),
        ] {
            let error = result.unwrap_err();
            assert!(crate::vm::format_error(&error).contains(TIMEOUT_MSG));
        }
        assert!(sched.timeout_loc.borrow().is_some());
    }

    #[test]
    fn independent_attempts_receive_their_own_complexity_allowance() {
        let mut state = State::new(b"", b"", None);
        assert!(state.tick(MAX_STEPS).is_ok());
        assert!(matches!(state.tick(1), Err(Stop::TooComplex)));
        state.begin_attempt();
        assert!(state.tick(MAX_STEPS).is_ok());
        assert!(matches!(state.tick(1), Err(Stop::TooComplex)));
    }

    #[test]
    fn many_short_failed_attempts_still_reach_the_deadline_poll() {
        let expired = Instant::now() - Duration::from_secs(1);
        let mut state = State::new(b"a", b"x", Some(expired));
        for attempt in 1..=DEADLINE_POLL_STEPS {
            state.begin_attempt();
            match state.do_match(0, 0) {
                Ok(None) => assert!(attempt < DEADLINE_POLL_STEPS),
                Err(Stop::Timeout) => assert_eq!(attempt, DEADLINE_POLL_STEPS),
                _ => panic!("unexpected result for a one-step failed attempt"),
            }
        }
    }

    #[test]
    fn attempt_reset_preserves_partial_poll_work() {
        let expired = Instant::now() - Duration::from_secs(1);
        let mut state = State::new(b"", b"", Some(expired));
        assert!(state.tick(DEADLINE_POLL_STEPS - 1).is_ok());
        state.begin_attempt();
        assert!(matches!(state.tick(1), Err(Stop::Timeout)));
    }
}
