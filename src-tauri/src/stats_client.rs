//! Connexion à la Stats API officielle de Rocket League (socket TCP local).
//! Aucune injection : on lit uniquement ce que le jeu diffuse sur 127.0.0.1.

use serde_json::Value;
use std::io::{ErrorKind, Read};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub enum ClientEvent {
    Connected,
    Disconnected,
    Message(String, Value),
}

/// Découpe le flux TCP (objets JSON collés sans séparateur) en comptant les accolades.
#[derive(Default)]
pub struct JsonStreamParser {
    buf: Vec<u8>,
    depth: i32,
    in_string: bool,
    escaped: bool,
}

impl JsonStreamParser {
    pub fn feed(&mut self, chunk: &[u8], out: &mut Vec<Value>) {
        for &c in chunk {
            if self.depth == 0 && c != b'{' {
                continue;
            }
            self.buf.push(c);
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if c == b'\\' {
                    self.escaped = true;
                } else if c == b'"' {
                    self.in_string = false;
                }
                continue;
            }
            match c {
                b'"' => self.in_string = true,
                b'{' => self.depth += 1,
                b'}' => {
                    self.depth -= 1;
                    if self.depth == 0 {
                        if let Ok(v) = serde_json::from_slice::<Value>(&self.buf) {
                            out.push(v);
                        }
                        self.buf.clear();
                    }
                }
                _ => {}
            }
            if self.buf.len() > 8 * 1024 * 1024 {
                *self = JsonStreamParser::default();
            }
        }
    }
}

/// Transforme un message brut en (nom d'événement, données).
/// Le champ Data peut être un objet ou une chaîne JSON selon les versions.
pub fn decode_message(msg: Value) -> Option<(String, Value)> {
    let event = msg.get("Event")?.as_str()?.to_string();
    let data = match msg.get("Data") {
        Some(Value::String(s)) => serde_json::from_str(s).ok()?,
        Some(v) => v.clone(),
        None => Value::Object(Default::default()),
    };
    Some((event, data))
}

/// Lance le thread de connexion. Il se reconnecte tout seul (le jeu n'ouvre le socket qu'en match)
/// et suit les changements de port.
pub fn spawn<F>(port: Arc<AtomicU16>, on_event: F)
where
    F: Fn(ClientEvent) + Send + 'static,
{
    thread::spawn(move || loop {
        let current = port.load(Ordering::SeqCst);
        let addr = SocketAddr::from(([127, 0, 0, 1], current));
        let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        on_event(ClientEvent::Connected);

        let mut parser = JsonStreamParser::default();
        let mut buf = [0u8; 16 * 1024];
        let mut messages = Vec::new();
        loop {
            if port.load(Ordering::SeqCst) != current {
                break;
            }
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    parser.feed(&buf[..n], &mut messages);
                    for msg in messages.drain(..) {
                        if let Some((event, data)) = decode_message(msg) {
                            on_event(ClientEvent::Message(event, data));
                        }
                    }
                }
                Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
                Err(_) => break,
            }
        }
        on_event(ClientEvent::Disconnected);
        thread::sleep(Duration::from_millis(500));
    });
}
