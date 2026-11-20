//! Servidor HTTP minimo (solo std) para la WebUI y la API de IA Optimizer.
//!
/// Soporta lo justo: GET (index.html, /api/*) y POST con cuerpo JSON plano.
/// Sin dependencias externas para mantener el binario pequeno y portable.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
}

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".into(),
            body: body.as_bytes().to_vec(),
        }
    }
    pub fn json(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "application/json; charset=utf-8".into(),
            body: body.as_bytes().to_vec(),
        }
    }
    pub fn html(body: &str) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".into(),
            body: body.as_bytes().to_vec(),
        }
    }
    pub fn png(data: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "image/png".into(),
            body: data,
        }
    }
    pub fn image(content_type: &str, data: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: content_type.into(),
            body: data,
        }
    }
    pub fn not_found() -> Self {
        Self {
            status: 404,
            content_type: "text/plain; charset=utf-8".into(),
            body: b"no encontrado".to_vec(),
        }
    }
}

pub fn serve<F>(listener: TcpListener, handler: F)
where
    F: Fn(&Request) -> Response + Send + Sync + 'static,
{
    let handler = std::sync::Arc::new(handler);
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let h = handler.clone();
                std::thread::spawn(move || {
                    let _ = handle(s, &*h);
                });
            }
            Err(_) => continue,
        }
    }
}

fn handle<F>(mut stream: TcpStream, handler: &F) -> std::io::Result<()>
where
    F: Fn(&Request) -> Response,
{
    let peer = stream.peer_addr().ok();
    let mut reader = BufReader::new(stream.try_clone()?);

    // linea de peticion
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let raw_path = parts.next().unwrap_or("/").to_string();

    // cabeceras
    let mut content_length = 0usize;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 {
            break;
        }
        let t = h.trim_end();
        if t.is_empty() {
            break;
        }
        if let Some(v) = t.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }

    // cuerpo
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let body = String::from_utf8_lossy(&body).to_string();

    let req = Request {
        method,
        path: raw_path.split('?').next().unwrap_or("/").to_string(),
        query: raw_path.split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default(),
        body,
    };
    let resp = handler(&req);

    // Registro de acceso minimo (util para depurar desde adb logcat / archivo)
    if let Some(p) = peer {
        eprintln!("{} {} <- {}", req.method, req.path, p);
    }

    write_response(&mut stream, resp)
}

fn write_response(stream: &mut TcpStream, r: Response) -> std::io::Result<()> {
    let reason = match r.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        r.status,
        reason,
        r.content_type,
        r.body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&r.body)?;
    stream.flush()
}
