const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy)]
enum State {
    Array,
    ArrayTail,
    Object,
    ObjectTail,
}

pub(super) struct Strings {
    pub escaped: usize,
    pub surrogate_bytes_saved: usize,
}

pub(super) fn validate(input: &[u8]) -> Option<Strings> {
    let mut scanner = Scanner::new(input);
    skip_value(&mut scanner, 0)?;
    scanner.whitespace();
    scanner.input.is_empty().then_some(scanner.strings)
}

pub(super) fn fields<'a>(
    input: &'a [u8],
    mut visit: impl FnMut(&'a [u8], &'a [u8]) -> Option<()>,
) -> Option<()> {
    let mut scanner = Scanner::new(input);
    scanner.expect(b'{')?;
    if !scanner.take(b'}') {
        loop {
            scanner.whitespace();
            let key = scanner.input;
            scanner.expect(b'"')?;
            scanner.string()?;
            let key = &key[..key.len() - scanner.input.len()];
            scanner.expect(b':')?;
            scanner.whitespace();
            let value = scanner.input;
            skip_value(&mut scanner, 1)?;
            visit(key, &value[..value.len() - scanner.input.len()])?;
            if scanner.take(b'}') {
                break;
            }
            scanner.expect(b',')?;
        }
    }
    scanner.whitespace();
    scanner.input.is_empty().then_some(())
}

fn skip_value(scanner: &mut Scanner<'_>, parent_depth: usize) -> Option<()> {
    let mut stack = [State::Array; MAX_DEPTH];
    let mut depth = parent_depth;
    scanner.value(&mut stack, &mut depth)?;
    while depth != parent_depth {
        match stack[depth - 1] {
            State::Array if scanner.take(b']') => depth -= 1,
            State::Array => {
                stack[depth - 1] = State::ArrayTail;
                scanner.value(&mut stack, &mut depth)?;
            }
            State::ArrayTail if scanner.take(b']') => depth -= 1,
            State::ArrayTail => {
                scanner.expect(b',')?;
                scanner.value(&mut stack, &mut depth)?;
            }
            State::Object if scanner.take(b'}') => depth -= 1,
            State::Object => {
                stack[depth - 1] = State::ObjectTail;
                scanner.member(&mut stack, &mut depth)?;
            }
            State::ObjectTail if scanner.take(b'}') => depth -= 1,
            State::ObjectTail => {
                scanner.expect(b',')?;
                scanner.member(&mut stack, &mut depth)?;
            }
        }
    }
    Some(())
}

struct Scanner<'a> {
    input: &'a [u8],
    strings: Strings,
}

impl<'a> Scanner<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            strings: Strings {
                escaped: 0,
                surrogate_bytes_saved: 0,
            },
        }
    }

    fn whitespace(&mut self) {
        while matches!(self.input.first(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.input = &self.input[1..];
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.input.first() == Some(&byte) {
            self.input = &self.input[1..];
            true
        } else {
            false
        }
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        self.take(byte).then_some(())
    }

    fn member(&mut self, stack: &mut [State; MAX_DEPTH], depth: &mut usize) -> Option<()> {
        self.expect(b'"')?;
        self.string()?;
        self.expect(b':')?;
        self.value(stack, depth)
    }

    fn value(&mut self, stack: &mut [State; MAX_DEPTH], depth: &mut usize) -> Option<()> {
        self.whitespace();
        match *self.input.first()? {
            b'[' | b'{' => {
                *stack.get_mut(*depth)? = if self.input[0] == b'[' {
                    State::Array
                } else {
                    State::Object
                };
                *depth += 1;
                self.input = &self.input[1..];
            }
            b'"' => {
                self.input = &self.input[1..];
                self.string()?;
            }
            b't' => self.input = self.input.strip_prefix(b"true")?,
            b'f' => self.input = self.input.strip_prefix(b"false")?,
            b'n' => self.input = self.input.strip_prefix(b"null")?,
            b'-' | b'0'..=b'9' => self.number()?,
            _ => return None,
        }
        Some(())
    }

    fn string(&mut self) -> Option<()> {
        let start = self.input.len();
        let mut unescape = false;
        let mut saved = 0;
        loop {
            let (&byte, rest) = self.input.split_first()?;
            self.input = rest;
            match byte {
                b'"' => break,
                b'\\' => {
                    let (&escape, rest) = self.input.split_first()?;
                    self.input = rest;
                    match escape {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                            unescape = true;
                            saved += 1;
                        }
                        b'u' => {
                            let code = self.hex()?;
                            if (0xd800..=0xdbff).contains(&code) {
                                self.input = self.input.strip_prefix(b"\\u")?;
                                if !(0xdc00..=0xdfff).contains(&self.hex()?) {
                                    return None;
                                }
                                self.strings.surrogate_bytes_saved += 8;
                                saved += 8;
                            } else if (0xdc00..=0xdfff).contains(&code) {
                                return None;
                            } else {
                                unescape = true;
                                saved += match code {
                                    0..=0x7f => 5,
                                    0x80..=0x7ff => 4,
                                    _ => 3,
                                };
                            }
                        }
                        _ => return None,
                    }
                }
                0..=31 => return None,
                _ => (),
            }
        }
        if unescape {
            self.strings.escaped = self
                .strings
                .escaped
                .max(start - self.input.len() - 1 - saved);
        }
        Some(())
    }

    fn hex(&mut self) -> Option<u16> {
        let (digits, rest) = self.input.split_at_checked(4)?;
        self.input = rest;
        hex(digits)
    }

    fn digits(&mut self) -> Option<()> {
        let count = self
            .input
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if count == 0 {
            return None;
        }
        self.input = &self.input[count..];
        Some(())
    }

    fn number(&mut self) -> Option<()> {
        if let Some(rest) = self.input.strip_prefix(b"-") {
            self.input = rest;
        }
        if let Some(rest) = self.input.strip_prefix(b"0") {
            self.input = rest;
        } else {
            self.digits()?;
        }
        if let Some(rest) = self.input.strip_prefix(b".") {
            self.input = rest;
            self.digits()?;
        }
        if matches!(self.input.first(), Some(b'e' | b'E')) {
            self.input = &self.input[1..];
            if matches!(self.input.first(), Some(b'+' | b'-')) {
                self.input = &self.input[1..];
            }
            self.digits()?;
        }
        Some(())
    }
}

pub(super) fn hex(digits: &[u8]) -> Option<u16> {
    digits.iter().try_fold(0, |value, &digit| {
        Some(value * 16 + u16::try_from(char::from(digit).to_digit(16)?).ok()?)
    })
}
