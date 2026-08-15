//! Minimal static-file HTTP/1.1 server, for `certo doc --serve` (BACKLOG
//! item 155). No new dependency — a small, self-contained, single-threaded
//! GET-only server, in the same spirit as this project's other hand-rolled
//! implementations (`crates/cli/src/diff.rs`'s unified diff) rather than
//! pulling in a crate for one CLI flag. Not a general-purpose production web
//! server: no keep-alive, no HTTP/1.1 pipelining, no range requests — just
//! enough to browse locally-generated docs during development.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

/// Serve `dir` over HTTP on `127.0.0.1:port` until the process is killed
/// (Ctrl+C). Blocks the calling thread — the caller should print its own
/// "serving at ..." message *before* calling this, since `bind` happening
/// first is what actually confirms the port is available.
pub fn serve_dir(dir: &Path, port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream, dir) {
                    eprintln!("certo doc --serve: connection error: {}", e);
                }
            }
            Err(e) => eprintln!("certo doc --serve: accept error: {}", e),
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream, dir: &Path) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Drain the rest of the request headers (we don't use them, but the
    // client is entitled to have sent them before we write a response).
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let path = match parse_request_path(&request_line) {
        Some(p) => p,
        None => return write_response(&mut stream, 400, "Bad Request", "text/plain", b"Bad Request"),
    };

    match resolve_static_path(dir, &path) {
        Some(file_path) => match std::fs::read(&file_path) {
            Ok(body) => {
                let ctype = content_type_for(&file_path);
                write_response(&mut stream, 200, "OK", ctype, &body)
            }
            Err(_) => write_response(&mut stream, 404, "Not Found", "text/plain", b"404 Not Found"),
        },
        None => write_response(&mut stream, 404, "Not Found", "text/plain", b"404 Not Found"),
    }
}

fn write_response(stream: &mut TcpStream, code: u16, reason: &str, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        code, reason, content_type, body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

/// Extract the request path from an HTTP request line (`GET /foo HTTP/1.1`),
/// percent-decoded and with any query string stripped. `None` if the line
/// isn't a well-formed request line (fewer than the 3 space-separated parts
/// every valid one has) — matches how browsers/curl never send anything
/// else, so a malformed line means a bad/incomplete request, not a `/`.
fn parse_request_path(request_line: &str) -> Option<String> {
    let mut parts = request_line.trim().split(' ');
    let _method = parts.next()?;
    let raw_path = parts.next()?;
    parts.next()?; // HTTP version — required to exist, not otherwise used
    let path = raw_path.split('?').next().unwrap_or(raw_path);
    Some(percent_decode(path))
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

/// Resolve a request path to a real file under `dir`, refusing to serve
/// anything outside it (`..` traversal) — the one security property a
/// "serve a folder" server must not get wrong, even a local-dev-only one.
/// `/` and any path ending in `/` map to `index.html` inside that directory.
fn resolve_static_path(dir: &Path, req_path: &str) -> Option<PathBuf> {
    let trimmed = req_path.trim_start_matches('/');
    let rel = if trimmed.is_empty() || req_path.ends_with('/') {
        format!("{}index.html", trimmed)
    } else {
        trimmed.to_string()
    };
    // Reject any segment that could escape `dir` — simpler and safer than
    // canonicalizing and comparing prefixes (which requires the file to
    // already exist to canonicalize at all).
    if rel.split('/').any(|seg| seg == "..") {
        return None;
    }
    Some(dir.join(rel))
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css"          => "text/css; charset=utf-8",
        "js"           => "text/javascript; charset=utf-8",
        "json"         => "application/json",
        "svg"          => "image/svg+xml",
        "png"          => "image/png",
        "txt"          => "text/plain; charset=utf-8",
        _              => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ordinary_get_request_line() {
        assert_eq!(parse_request_path("GET / HTTP/1.1\r\n"), Some("/".into()));
        assert_eq!(parse_request_path("GET /foo/bar.html HTTP/1.1\r\n"), Some("/foo/bar.html".into()));
    }

    #[test]
    fn strips_query_string() {
        assert_eq!(parse_request_path("GET /page?x=1&y=2 HTTP/1.1\r\n"), Some("/page".into()));
    }

    #[test]
    fn percent_decodes_the_path() {
        assert_eq!(parse_request_path("GET /a%20b.html HTTP/1.1\r\n"), Some("/a b.html".into()));
    }

    #[test]
    fn rejects_malformed_request_lines() {
        assert_eq!(parse_request_path(""), None);
        assert_eq!(parse_request_path("GET"), None);
        assert_eq!(parse_request_path("GET /"), None); // missing HTTP version
    }

    #[test]
    fn root_and_trailing_slash_map_to_index_html() {
        let dir = Path::new("/docs");
        assert_eq!(resolve_static_path(dir, "/"), Some(PathBuf::from("/docs/index.html")));
        assert_eq!(resolve_static_path(dir, "/sub/"), Some(PathBuf::from("/docs/sub/index.html")));
    }

    #[test]
    fn ordinary_path_maps_directly() {
        let dir = Path::new("/docs");
        assert_eq!(resolve_static_path(dir, "/style.css"), Some(PathBuf::from("/docs/style.css")));
    }

    #[test]
    fn parent_traversal_is_rejected() {
        let dir = Path::new("/docs");
        assert_eq!(resolve_static_path(dir, "/../secret.txt"), None);
        assert_eq!(resolve_static_path(dir, "/a/../../secret.txt"), None);
        assert_eq!(resolve_static_path(dir, "/..%2f..%2fsecret.txt".replace("%2f", "/").as_str()), None);
    }

    #[test]
    fn content_type_guessed_from_extension() {
        assert_eq!(content_type_for(Path::new("x.html")), "text/html; charset=utf-8");
        assert_eq!(content_type_for(Path::new("x.CSS")), "text/css; charset=utf-8");
        assert_eq!(content_type_for(Path::new("x.unknown")), "application/octet-stream");
        assert_eq!(content_type_for(Path::new("noext")), "application/octet-stream");
    }
}
