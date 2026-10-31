//! Number fields accept small arithmetic expressions: `3*4`, `2+0.5`, `-(1/4)`, `10%3`.
//!
//! A leading `+` or `*` etc. is relative to the current value: `+2` adds 2, `*2` doubles, `/2`
//! halves, so a creator can type `+2` into Position.Y to lift a part by two studs. Properties
//! and (later) typed gizmo values (#214) use [`eval_relative`].

/// Longest text accepted (a pasted novel is not a number).
pub const MAX_LEN: usize = 256;
/// Deepest nesting of brackets and signs accepted, so `((((...` cannot overflow the stack.
pub const MAX_DEPTH: usize = 64;

/// Evaluates `text` as an expression. `None` when it is not a valid finite expression, longer
/// than [`MAX_LEN`] bytes, or nested deeper than [`MAX_DEPTH`].
pub fn eval(text: &str) -> Option<f64> {
    if text.len() > MAX_LEN {
        return None;
    }
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    let v = p.expr()?;
    p.ws();
    (p.i == p.s.len() && v.is_finite()).then_some(v)
}

/// Like [`eval`], but a text starting with `+`, `*` or `/` applies to `current`
/// (`+2` -> current + 2). A leading `-` stays a negative number.
pub fn eval_relative(text: &str, current: f64) -> Option<f64> {
    let t = text.trim();
    let (op, rest) = match t.as_bytes().first() {
        Some(b'+') => (b'+', &t[1..]),
        Some(b'*') => (b'*', &t[1..]),
        Some(b'/') => (b'/', &t[1..]),
        _ => return eval(t),
    };
    let v = eval(rest)?;
    let out = match op {
        b'+' => current + v,
        b'*' => current * v,
        _ => current / v,
    };
    out.is_finite().then_some(out)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(c) = self.peek() {
            match c {
                b'+' => {
                    self.i += 1;
                    v += self.term()?;
                }
                b'-' => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        Some(v)
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(c) = self.peek() {
            match c {
                b'*' => {
                    self.i += 1;
                    v *= self.unary()?;
                }
                b'/' => {
                    self.i += 1;
                    v /= self.unary()?;
                }
                b'%' => {
                    self.i += 1;
                    v %= self.unary()?;
                }
                _ => break,
            }
        }
        Some(v)
    }

    fn unary(&mut self) -> Option<f64> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        self.depth += 1;
        let v = self.unary_inner();
        self.depth -= 1;
        v
    }

    fn unary_inner(&mut self) -> Option<f64> {
        match self.peek()? {
            b'-' => {
                self.i += 1;
                Some(-self.unary()?)
            }
            b'+' => {
                self.i += 1;
                self.unary()
            }
            b'(' => {
                self.i += 1;
                let v = self.expr()?;
                (self.peek()? == b')').then(|| self.i += 1)?;
                Some(v)
            }
            _ => self.number(),
        }
    }

    fn number(&mut self) -> Option<f64> {
        self.ws();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
            self.i += 1;
        }
        if self.i < self.s.len() && (self.s[self.i] == b'e' || self.s[self.i] == b'E') {
            let save = self.i;
            self.i += 1;
            if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
                self.i += 1;
            }
            let digits = self.i;
            while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                self.i += 1;
            }
            if self.i == digits {
                self.i = save;
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()?
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_nesting_and_long_input_are_rejected_not_a_crash() {
        let deep = "(".repeat(100_000) + "1" + &")".repeat(100_000);
        assert_eq!(eval(&deep), None);
        assert_eq!(eval(&"-".repeat(100_000)), None);
        let ok = "(".repeat(40) + "2" + &")".repeat(40);
        assert_eq!(eval(&ok), Some(2.0));
        let long = "1+".repeat(200) + "1";
        assert_eq!(eval(&long), None);
        assert_eq!(eval_relative(&deep, 1.0), None);
    }

    #[test]
    fn arithmetic_with_precedence() {
        assert_eq!(eval("3*4"), Some(12.0));
        assert_eq!(eval("2 + 3 * 4"), Some(14.0));
        assert_eq!(eval("(2 + 3) * 4"), Some(20.0));
        assert_eq!(eval("-1.5"), Some(-1.5));
        assert_eq!(eval("10 % 3"), Some(1.0));
        assert_eq!(eval("1e2"), Some(100.0));
    }

    #[test]
    fn rejects_junk_and_infinity() {
        assert_eq!(eval(""), None);
        assert_eq!(eval("3*"), None);
        assert_eq!(eval("abc"), None);
        assert_eq!(eval("1/0"), None);
        assert_eq!(eval("(1"), None);
    }

    #[test]
    fn relative_forms() {
        assert_eq!(eval_relative("+2", 5.0), Some(7.0));
        assert_eq!(eval_relative("*2", 5.0), Some(10.0));
        assert_eq!(eval_relative("/2", 5.0), Some(2.5));
        assert_eq!(eval_relative("-2", 5.0), Some(-2.0));
        assert_eq!(eval_relative("3*4", 5.0), Some(12.0));
    }
}
