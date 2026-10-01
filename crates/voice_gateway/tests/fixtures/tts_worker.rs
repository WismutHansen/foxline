//! Independently implemented std-only worker used by executable adapter tests.
use std::{
    io::{self, Read, Write},
    time::Duration,
};

fn frame(kind: u8, id: u32, payload: &[u8]) {
    let mut bytes = vec![kind];
    bytes.extend_from_slice(&id.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(payload);
    // Force the receiver to handle partial headers and bodies.
    let mut out = io::stdout().lock();
    for part in bytes.chunks(3) {
        out.write_all(part).unwrap();
        out.flush().unwrap();
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap();
    if let Some(pid_file) = std::env::args().nth(2) {
        std::fs::write(pid_file, std::process::id().to_string()).unwrap();
    }
    if mode == "stderr-error" {
        io::stderr().write_all(&vec![b'x'; 1024 * 1024]).unwrap();
        writeln!(io::stderr(), "\nmodel stderr error").unwrap();
        return;
    }
    if mode == "timeout" {
        std::thread::sleep(Duration::from_secs(60));
        return;
    }
    if mode == "startup-error" {
        frame(5, 0, b"missing model asset");
        return;
    }
    if mode == "invalid-ready" {
        frame(1, 0, &24000_u32.to_le_bytes());
        return;
    }
    if mode == "stderr" {
        io::stderr().write_all(&vec![b'x'; 1024 * 1024]).unwrap();
    }
    frame(1, 0, &[]);
    let mut input = io::stdin().lock();
    let mut requests = 0;
    loop {
        let mut header = [0_u8; 9];
        if input.read_exact(&mut header).is_err() {
            return;
        }
        let id = u32::from_le_bytes(header[1..5].try_into().unwrap());
        let len = u32::from_le_bytes(header[5..9].try_into().unwrap()) as usize;
        let mut payload = vec![0; len];
        input.read_exact(&mut payload).unwrap();
        match header[0] {
            1 => {
                let text = std::str::from_utf8(&payload).unwrap();
                assert!(!text.starts_with('{'), "speak must be raw UTF-8, not JSON");
                if text != "fixture" {
                    assert_eq!(text, "Guten Morgen, Straße.");
                }
                assert!(id != 0);
                requests += 1;
                if mode == "crash" {
                    return;
                }
                if mode == "truncated" {
                    io::stdout()
                        .write_all(&[3, 1, 0, 0, 0, 10, 0, 0, 0, 1, 2])
                        .unwrap();
                    return;
                }
                if mode == "request-error" && requests == 1 {
                    frame(5, id, b"unsupported voice");
                    continue;
                }
                if mode == "wrong-order" {
                    frame(3, id, &[1, 0]);
                    return;
                }
                frame(2, id, &48000_u32.to_le_bytes());
                if mode == "late" && requests == 1 {
                    std::thread::sleep(Duration::from_millis(100));
                }
                if mode == "odd-pcm" {
                    frame(3, id, &[1]);
                    return;
                }
                frame(3, id, &[1, 0, 255, 127]);
                frame(4, id, &[]);
            }
            2 => {
                assert!(payload.is_empty());
            }
            3 => {
                assert!(payload.is_empty());
                return;
            }
            other => panic!("unexpected input {other}"),
        }
    }
}
