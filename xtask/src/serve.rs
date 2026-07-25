//! A throwaway static file server for `dist/`.
//!
//! Hand-rolled rather than pulled from crates.io because it needs to do exactly three things —
//! bind an ephemeral port, map five extensions to MIME types, and serve bytes from one directory —
//! and because `application/wasm` is not optional: Chrome's `instantiateStreaming` refuses any other
//! content type, so a server that guesses `application/octet-stream` breaks the whole pipeline in a
//! way that looks like a WASM bug.
//!
//! Not a general-purpose server. It binds loopback only, serves GET and HEAD, and lives as long as
//! the `Server` value.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};

/// A running server. Dropping it stops accepting new connections.
pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl Server {
    /// The URL the app is reachable at, with no trailing slash.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock the accept loop so the thread notices the flag and exits.
        let _ = TcpStream::connect(self.addr);
    }
}

/// Serve `root` on an ephemeral loopback port, in a background thread.
pub fn serve(root: &Path) -> Result<Server> {
    let root = root
        .canonicalize()
        .with_context(|| format!("no such directory: {}", root.display()))?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .context("could not bind a loopback port for the static server")?;
    let addr = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));

    let thread_stop = stop.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            if thread_stop.load(Ordering::Relaxed) {
                return;
            }
            let Ok(stream) = stream else { continue };
            let root = root.clone();
            // Thread per connection. Chrome opens a handful in parallel for the HTML, the JS
            // shim, the WASM module and the font; a single-threaded loop would serialize them.
            thread::spawn(move || {
                let _ = handle(stream, &root);
            });
        }
    });

    Ok(Server { addr, stop })
}

fn handle(mut stream: TcpStream, root: &Path) -> Result<()> {
    let mut request_line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or("/").to_string();

    if method != "GET" && method != "HEAD" {
        return respond(&mut stream, 405, "text/plain", b"method not allowed", false);
    }

    match resolve(root, &target) {
        Some(path) => {
            let mut body = Vec::new();
            std::fs::File::open(&path)?.read_to_end(&mut body)?;
            let mime = mime_for(&path);
            respond(&mut stream, 200, mime, &body, method == "HEAD")
        }
        None => respond(
            &mut stream,
            404,
            "text/plain",
            b"not found",
            method == "HEAD",
        ),
    }
}

/// Map a request target onto a file under `root`, or `None`.
///
/// Rejects any path that escapes `root` — the directory is build output, but a traversal bug in a
/// dev tool is still a traversal bug, and `..` in a URL is never legitimate here.
fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let path = if path == "/" { "/index.html" } else { path };

    let mut out = root.to_path_buf();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        let decoded = percent_decode(segment);
        let candidate = Path::new(&decoded);
        // One plain component only: no `..`, no absolute prefix, no nested separators.
        match candidate.components().next() {
            Some(Component::Normal(c)) if candidate.components().count() == 1 => out.push(c),
            _ => return None,
        }
    }
    let out = out.canonicalize().ok()?;
    (out.starts_with(root) && out.is_file()).then_some(out)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        // Non-negotiable: `WebAssembly.instantiateStreaming` rejects anything else.
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("ttf") => "font/ttf",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    mime: &str,
    body: &[u8],
    head_only: bool,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {mime}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )?;
    if !head_only {
        stream.write_all(body)?;
    }
    stream.flush()?;
    Ok(())
}
