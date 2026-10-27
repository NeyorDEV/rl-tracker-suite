//! Petit serveur HTTP local pour OBS : sert les overlays et pousse les mises à jour
//! en direct (Server-Sent Events). Écoute uniquement sur 127.0.0.1.
//!
//! Deux canaux partagent le même mécanisme : l'overlay de session (`/overlay`, `/events`) et
//! l'overlay du lobby LobbyTracker (`/lobby`, `/lobby/events`).

use serde_json::Value;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const OVERLAY_HTML: &str = include_str!("../../ui/overlay.html");
const LOBBY_HTML: &str = include_str!("../../ui/lobby-overlay.html");

type StateFn = Arc<dyn Fn() -> Value + Send + Sync>;

/// Un flux SSE : les navigateurs abonnés et la fonction qui donne l'état courant.
#[derive(Clone)]
struct Channel {
    clients: Arc<Mutex<Vec<TcpStream>>>,
    get_state: StateFn,
}

impl Channel {
    fn new(get_state: StateFn) -> Self {
        Channel { clients: Arc::default(), get_state }
    }

    fn subscribe(&self, mut stream: TcpStream) {
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
        let first = format!("retry: 2000\n\ndata: {}\n\n", (self.get_state)());
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        if stream.write_all(head.as_bytes()).is_ok() && stream.write_all(first.as_bytes()).is_ok() {
            self.clients.lock().unwrap().push(stream);
        }
    }

    fn send_raw(&self, bytes: &[u8]) {
        let mut clients = self.clients.lock().unwrap();
        clients.retain_mut(|c| c.write_all(bytes).and_then(|_| c.flush()).is_ok());
    }
}

#[derive(Clone)]
pub struct WebServer {
    session: Channel,
    lobby: Channel,
    error: Arc<Mutex<Option<String>>>,
}

impl WebServer {
    pub fn start(port: u16, session_state: StateFn, lobby_state: StateFn) -> WebServer {
        let server = WebServer {
            session: Channel::new(session_state),
            lobby: Channel::new(lobby_state),
            error: Arc::default(),
        };

        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                let channels = (server.session.clone(), server.lobby.clone());
                thread::spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let channels = channels.clone();
                        thread::spawn(move || handle(stream, channels));
                    }
                });
                let keepalive = server.clone();
                thread::spawn(move || loop {
                    thread::sleep(Duration::from_secs(20));
                    keepalive.session.send_raw(b": ping\n\n");
                    keepalive.lobby.send_raw(b": ping\n\n");
                });
            }
            Err(err) => {
                let message = if err.kind() == ErrorKind::AddrInUse {
                    format!("Le port {port} est déjà utilisé. Change-le dans la section OBS, puis relance l'appli.")
                } else {
                    format!("Serveur local indisponible : {err}")
                };
                *server.error.lock().unwrap() = Some(message);
            }
        }
        server
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    pub fn broadcast(&self, state: &Value) {
        self.session.send_raw(format!("data: {state}\n\n").as_bytes());
    }

    pub fn broadcast_lobby(&self, state: &Value) {
        self.lobby.send_raw(format!("data: {state}\n\n").as_bytes());
    }
}

fn read_request_path(stream: &mut TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let target = head.lines().next()?.split_whitespace().nth(1)?.to_string();
    Some(target.split('?').next().unwrap_or("/").to_string())
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn handle(mut stream: TcpStream, (session, lobby): (Channel, Channel)) {
    let Some(path) = read_request_path(&mut stream) else { return };
    match path.as_str() {
        "/events" => session.subscribe(stream),
        "/lobby/events" => lobby.subscribe(stream),
        "/api/state" => respond(&mut stream, "200 OK", "application/json; charset=utf-8", (session.get_state)().to_string().as_bytes()),
        "/lobby/api/state" => respond(&mut stream, "200 OK", "application/json; charset=utf-8", (lobby.get_state)().to_string().as_bytes()),
        "/" | "/overlay" | "/overlay.html" => respond(&mut stream, "200 OK", "text/html; charset=utf-8", OVERLAY_HTML.as_bytes()),
        "/lobby" | "/lobby/" => respond(&mut stream, "200 OK", "text/html; charset=utf-8", LOBBY_HTML.as_bytes()),
        _ => respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", b"Introuvable"),
    }
}
