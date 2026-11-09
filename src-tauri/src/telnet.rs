//! Telnet (RFC 854) for terminal sessions: separates the server's option negotiation from
//! the text it prints, answers it, and encodes what is typed.
//!
//! Options K-Shell agrees to: the server echoing and suppressing go-ahead (character at a
//! time instead of line mode), and on our side terminal type (RFC 1091, `XTERM-256COLOR`)
//! and window size (NAWS, RFC 1073). Everything else is refused. No I/O happens here.

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const ECHO: u8 = 1;
const SGA: u8 = 3;
const TTYPE: u8 = 24;
const NAWS: u8 = 31;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;
const TERMINAL_TYPE: &[u8] = b"XTERM-256COLOR";
/// Subnegotiation data kept at most (we only read TTYPE SEND, which is one byte).
const MAX_SB: usize = 64;

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Data,
    /// After IAC.
    Iac,
    /// After IAC DO/DONT/WILL/WONT: the option byte comes next.
    Verb(u8),
    /// Inside IAC SB … (until IAC SE).
    Sub,
    /// IAC seen inside a subnegotiation.
    SubIac,
    /// After a CR from the server: a NUL that follows it is padding.
    Cr,
}

pub struct Telnet {
    state: State,
    sub: Vec<u8>,
    /// We said WILL NAWS and the server wants it: window size changes are sent.
    naws: bool,
    size: (u16, u16),
    /// Options we already answered, so a server repeating itself doesn't start a loop.
    answered_do: [bool; 256],
    answered_will: [bool; 256],
}

impl Telnet {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            state: State::Data,
            sub: Vec::new(),
            naws: false,
            size: (cols, rows),
            answered_do: [false; 256],
            answered_will: [false; 256],
        }
    }

    /// Feed bytes from the server: text for the terminal goes to `text`, the answers the
    /// server is waiting for go to `reply`.
    pub fn receive(&mut self, input: &[u8], text: &mut Vec<u8>, reply: &mut Vec<u8>) {
        for &b in input {
            self.state = match self.state {
                State::Data | State::Cr => {
                    let after_cr = self.state == State::Cr;
                    match b {
                        IAC => State::Iac,
                        0 if after_cr => State::Data, // CR NUL = a bare carriage return
                        b'\r' => {
                            text.push(b);
                            State::Cr
                        }
                        _ => {
                            text.push(b);
                            State::Data
                        }
                    }
                }
                State::Iac => match b {
                    IAC => {
                        text.push(IAC); // escaped 0xFF
                        State::Data
                    }
                    DO | DONT | WILL | WONT => State::Verb(b),
                    SB => {
                        self.sub.clear();
                        State::Sub
                    }
                    _ => State::Data, // NOP, GA, AYT…: nothing to show
                },
                State::Verb(verb) => {
                    self.negotiate(verb, b, reply);
                    State::Data
                }
                State::Sub => {
                    if b == IAC {
                        State::SubIac
                    } else {
                        if self.sub.len() < MAX_SB {
                            self.sub.push(b);
                        }
                        State::Sub
                    }
                }
                State::SubIac => match b {
                    SE => {
                        self.subnegotiation(reply);
                        State::Data
                    }
                    IAC => {
                        if self.sub.len() < MAX_SB {
                            self.sub.push(IAC);
                        }
                        State::Sub
                    }
                    _ => State::Sub,
                },
            };
        }
    }

    fn negotiate(&mut self, verb: u8, opt: u8, reply: &mut Vec<u8>) {
        match verb {
            // The server offers to do something.
            WILL => {
                let accept = matches!(opt, ECHO | SGA);
                if !self.answered_will[opt as usize] {
                    self.answered_will[opt as usize] = true;
                    reply.extend_from_slice(&[IAC, if accept { DO } else { DONT }, opt]);
                }
            }
            // The server asks us to do something.
            DO => {
                let accept = matches!(opt, TTYPE | NAWS | SGA);
                if !self.answered_do[opt as usize] {
                    self.answered_do[opt as usize] = true;
                    reply.extend_from_slice(&[IAC, if accept { WILL } else { WONT }, opt]);
                }
                if opt == NAWS {
                    self.naws = true;
                    reply.extend(self.naws_frame());
                }
            }
            DONT => {
                if opt == NAWS {
                    self.naws = false;
                }
                self.answered_do[opt as usize] = false;
            }
            WONT => self.answered_will[opt as usize] = false,
            _ => {}
        }
    }

    fn subnegotiation(&mut self, reply: &mut Vec<u8>) {
        if self.sub.as_slice() == [TTYPE, TTYPE_SEND] {
            reply.extend_from_slice(&[IAC, SB, TTYPE, TTYPE_IS]);
            reply.extend_from_slice(TERMINAL_TYPE);
            reply.extend_from_slice(&[IAC, SE]);
        }
    }

    /// The terminal was resized: the NAWS message to send, if the server asked for sizes.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Option<Vec<u8>> {
        self.size = (cols, rows);
        self.naws.then(|| self.naws_frame())
    }

    fn naws_frame(&self) -> Vec<u8> {
        let (c, r) = self.size;
        let mut v = vec![IAC, SB, NAWS];
        for b in [c.to_be_bytes(), r.to_be_bytes()].concat() {
            v.push(b);
            if b == IAC {
                v.push(IAC); // 255 in the data is doubled
            }
        }
        v.extend_from_slice(&[IAC, SE]);
        v
    }

    /// Typed text on its way to the server: 0xFF is doubled, and a bare CR (what Enter
    /// sends) becomes CR NUL as the protocol requires.
    pub fn encode(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len() + 4);
        let mut i = 0;
        while i < input.len() {
            let b = input[i];
            out.push(b);
            if b == IAC {
                out.push(IAC);
            } else if b == b'\r' && input.get(i + 1) != Some(&b'\n') {
                out.push(0);
            }
            i += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(t: &mut Telnet, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (mut text, mut reply) = (Vec::new(), Vec::new());
        t.receive(input, &mut text, &mut reply);
        (text, reply)
    }

    #[test]
    fn plain_text_passes_through() {
        let mut t = Telnet::new(80, 24);
        assert_eq!(feed(&mut t, b"login: "), (b"login: ".to_vec(), vec![]));
    }

    #[test]
    fn accepts_echo_and_sga_refuses_the_rest() {
        let mut t = Telnet::new(80, 24);
        let (text, reply) = feed(&mut t, &[IAC, WILL, ECHO, IAC, WILL, SGA, IAC, WILL, 42, b'x']);
        assert_eq!(text, b"x");
        assert_eq!(reply, [IAC, DO, ECHO, IAC, DO, SGA, IAC, DONT, 42]);
    }

    #[test]
    fn repeated_offers_are_answered_once() {
        let mut t = Telnet::new(80, 24);
        let (_, reply) = feed(&mut t, &[IAC, WILL, ECHO, IAC, WILL, ECHO]);
        assert_eq!(reply, [IAC, DO, ECHO]);
    }

    #[test]
    fn sends_window_size_when_asked_and_on_resize() {
        let mut t = Telnet::new(100, 30);
        let (_, reply) = feed(&mut t, &[IAC, DO, NAWS]);
        assert_eq!(reply, [IAC, WILL, NAWS, IAC, SB, NAWS, 0, 100, 0, 30, IAC, SE]);
        assert_eq!(t.resize(255, 40).unwrap(), [IAC, SB, NAWS, 0, 255, 255, 0, 40, IAC, SE]);
        feed(&mut t, &[IAC, DONT, NAWS]);
        assert_eq!(t.resize(80, 24), None);
    }

    #[test]
    fn window_size_not_sent_before_the_server_asks() {
        let mut t = Telnet::new(80, 24);
        assert_eq!(t.resize(120, 40), None);
    }

    #[test]
    fn answers_terminal_type() {
        let mut t = Telnet::new(80, 24);
        let (_, reply) = feed(&mut t, &[IAC, DO, TTYPE, IAC, SB, TTYPE, TTYPE_SEND, IAC, SE]);
        let mut want = vec![IAC, WILL, TTYPE, IAC, SB, TTYPE, TTYPE_IS];
        want.extend_from_slice(b"XTERM-256COLOR");
        want.extend_from_slice(&[IAC, SE]);
        assert_eq!(reply, want);
    }

    #[test]
    fn sequences_split_across_reads() {
        let mut t = Telnet::new(80, 24);
        let mut text = Vec::new();
        let mut reply = Vec::new();
        for chunk in [&[b'a', IAC][..], &[WILL][..], &[ECHO, b'b'][..]] {
            t.receive(chunk, &mut text, &mut reply);
        }
        assert_eq!(text, b"ab");
        assert_eq!(reply, [IAC, DO, ECHO]);
    }

    #[test]
    fn escaped_iac_and_cr_nul() {
        let mut t = Telnet::new(80, 24);
        let (text, reply) = feed(&mut t, &[b'a', IAC, IAC, b'\r', 0, b'b', b'\r', b'\n', IAC, 241]);
        assert_eq!(text, [b'a', 255, b'\r', b'b', b'\r', b'\n']);
        assert!(reply.is_empty());
    }

    #[test]
    fn encodes_typed_input() {
        assert_eq!(Telnet::encode(b"ls\r"), b"ls\r\0");
        assert_eq!(Telnet::encode(b"a\r\nb"), b"a\r\nb");
        assert_eq!(Telnet::encode(&[1, 255, 2]), [1, 255, 255, 2]);
    }
}
