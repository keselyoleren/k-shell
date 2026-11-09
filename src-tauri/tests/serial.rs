//! Serial console sessions against a pseudo-terminal pair: K-Shell opens one end by its
//! path, like `/dev/ttyUSB0`, and the test plays the device on the other end.
#![cfg(unix)]

use kshell_lib::terminal::{Cmd, SerialOptions, Terminals};
use serialport::{SerialPort, TTYPort};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::{Channel, InvokeResponseBody};

type Captured = (Channel<InvokeResponseBody>, Arc<Mutex<Vec<u8>>>, Arc<Mutex<Vec<String>>>);

fn capture() -> Captured {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let ev = Arc::new(Mutex::new(Vec::new()));
    let (b, e) = (buf.clone(), ev.clone());
    let ch = Channel::new(move |m| {
        match m {
            InvokeResponseBody::Raw(r) => b.lock().unwrap().extend(r),
            InvokeResponseBody::Json(j) => e.lock().unwrap().push(j),
        }
        Ok(())
    });
    (ch, buf, ev)
}

fn wait<T>(secs: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(v) = f() {
            return Some(v);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

#[test]
// macOS sets the speed with IOSSIOSPEED, which only real serial drivers accept, not ptys.
#[cfg_attr(target_os = "macos", ignore = "macOS pseudo-terminals reject the serial speed ioctl")]
fn serial_session_round_trip() {
    let (mut device, end) = TTYPort::pair().expect("pseudo-terminal pair");
    let path = end.name().expect("pty path");
    device.set_timeout(Duration::from_millis(100)).unwrap();

    let terms = Arc::new(Terminals::default());
    let (ch, buf, ev) = capture();
    let opts = SerialOptions { path: path.clone(), baud_rate: 115_200, ..Default::default() };
    terms.open_serial("s1".into(), opts, None, ch).expect("open serial");

    // device -> terminal
    device.write_all(b"Router> ").unwrap();
    device.flush().unwrap();
    let got = wait(5, || {
        let b = buf.lock().unwrap();
        String::from_utf8_lossy(&b).contains("Router> ").then_some(())
    });
    assert!(got.is_some(), "device output not received: {:?}", String::from_utf8_lossy(&buf.lock().unwrap()));

    // terminal -> device
    terms.send("s1", Cmd::Write(b"show version\r".to_vec())).unwrap();
    let mut seen = Vec::new();
    let got = wait(5, || {
        let mut b = [0u8; 64];
        if let Ok(n) = device.read(&mut b) {
            seen.extend_from_slice(&b[..n]);
        }
        String::from_utf8_lossy(&seen).contains("show version").then_some(())
    });
    assert!(got.is_some(), "typed text not received: {:?}", String::from_utf8_lossy(&seen));

    // resize is ignored, close ends the session with an exit event
    terms.send("s1", Cmd::Resize(120, 40)).unwrap();
    terms.close("s1");
    let exit = wait(5, || ev.lock().unwrap().first().cloned());
    assert_eq!(exit.as_deref(), Some(r#"{"code":null,"type":"exit"}"#));
    assert!(terms.send("s1", Cmd::Write(b"x".to_vec())).is_err(), "session still registered");
}

#[test]
fn missing_port_is_a_clear_error() {
    let error = |opts: SerialOptions| match opts.open() {
        Ok(_) => panic!("opened {}", opts.path),
        Err(e) => e,
    };
    let err = error(SerialOptions { path: "/dev/kshell-no-such-port".into(), ..Default::default() });
    assert!(err.starts_with("Couldn't open /dev/kshell-no-such-port"), "{err}");
    assert_eq!(error(SerialOptions::default()), "No serial port chosen");
}
