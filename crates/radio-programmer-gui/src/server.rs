//! Bounded loopback HTTP boundary and process launcher.

use crate::{GuiError, GuiSession, GuiState, INDEX_HTML, MAX_PROJECT_TEXT_BYTES};
use radio_programmer::VerifiedConfigurationReceipt;
use radio_programmer_serial::is_supported_baud;
use radio_storage::ObjectKind;
use std::{
    fmt::{self, Write as _},
    fs::File,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    time::Duration,
};

const DEFAULT_LISTEN: &str = "127.0.0.1:8765";
const MAX_HEADER_BYTES: usize = 16 * 1024;
const SESSION_TOKEN_BYTES: usize = 32;
const CONFIRMATION: &str = "replace-configuration";
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum canonical restore image bytes accepted by the local server.
pub const MAX_RESTORE_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// Stable command help text.
pub const HELP: &str = "AFIK programmer GUI\n\
\n\
Usage:\n\
  afik-programmer-gui --sim [--listen LOOPBACK:PORT]\n\
  afik-programmer-gui --device PATH --baud BAUD [--listen LOOPBACK:PORT]\n\
  afik-programmer-gui --help\n\
  afik-programmer-gui --version\n\
\n\
The interface only accepts an explicit loopback IP address.\n\
Supported BAUD: 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200\n";

#[derive(Clone, Debug, Eq, PartialEq)]
enum Backend {
    Simulator,
    Serial { path: PathBuf, baud: u32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Invocation {
    Help,
    Version,
    Serve {
        backend: Backend,
        listen: SocketAddr,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UsageError(String);

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Parses one invocation and either prints help/version or serves until stopped.
pub fn main_entry(arguments: &[String]) -> i32 {
    match parse_invocation(arguments) {
        Ok(Invocation::Help) => {
            print!("{HELP}");
            0
        }
        Ok(Invocation::Version) => {
            println!("afik-programmer-gui {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Ok(Invocation::Serve { backend, listen }) => match run_server(backend, listen) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            2
        }
    }
}

fn parse_invocation(arguments: &[String]) -> Result<Invocation, UsageError> {
    if arguments == ["--help"] || arguments == ["-h"] || arguments == ["help"] {
        return Ok(Invocation::Help);
    }
    if arguments == ["--version"] {
        return Ok(Invocation::Version);
    }
    if arguments.is_empty() {
        return Err(UsageError(
            "missing backend; select --sim or --device PATH --baud BAUD".into(),
        ));
    }

    let mut simulator = false;
    let mut device = None;
    let mut baud = None;
    let mut listen = None;
    let mut offset = 0;
    while let Some(argument) = arguments.get(offset) {
        match argument.as_str() {
            "--sim" => {
                if simulator {
                    return Err(UsageError("--sim was provided more than once".into()));
                }
                simulator = true;
                offset += 1;
            }
            "--device" => {
                if device.is_some() {
                    return Err(UsageError("--device was provided more than once".into()));
                }
                device = Some(PathBuf::from(require_value(arguments, offset, "--device")?));
                offset += 2;
            }
            "--baud" => {
                if baud.is_some() {
                    return Err(UsageError("--baud was provided more than once".into()));
                }
                let value = require_value(arguments, offset, "--baud")?;
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| UsageError(format!("invalid baud: {value}")))?;
                if !is_supported_baud(parsed) {
                    return Err(UsageError(format!("unsupported baud: {parsed}")));
                }
                baud = Some(parsed);
                offset += 2;
            }
            "--listen" => {
                if listen.is_some() {
                    return Err(UsageError("--listen was provided more than once".into()));
                }
                let value = require_value(arguments, offset, "--listen")?;
                let address = value
                    .parse::<SocketAddr>()
                    .map_err(|_| UsageError(format!("invalid listen address: {value}")))?;
                if !address.ip().is_loopback() {
                    return Err(UsageError(format!(
                        "listen address must be loopback: {address}"
                    )));
                }
                listen = Some(address);
                offset += 2;
            }
            unknown => return Err(UsageError(format!("unknown argument: {unknown}"))),
        }
    }

    let backend = match (simulator, device, baud) {
        (true, None, None) => Backend::Simulator,
        (false, Some(path), Some(baud)) => Backend::Serial { path, baud },
        (false, None, None) => {
            return Err(UsageError(
                "select exactly one backend: --sim or --device PATH --baud BAUD".into(),
            ));
        }
        (true, _, _) => {
            return Err(UsageError(
                "--sim conflicts with --device and --baud".into(),
            ));
        }
        (false, Some(_), None) => return Err(UsageError("--device requires --baud".into())),
        (false, None, Some(_)) => return Err(UsageError("--baud requires --device".into())),
    };
    let listen = listen.unwrap_or_else(|| {
        DEFAULT_LISTEN
            .parse::<SocketAddr>()
            .expect("constant default address is valid")
    });
    Ok(Invocation::Serve { backend, listen })
}

fn require_value<'a>(
    arguments: &'a [String],
    option_offset: usize,
    option: &str,
) -> Result<&'a str, UsageError> {
    arguments
        .get(option_offset + 1)
        .map(String::as_str)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| UsageError(format!("{option} requires a value")))
}

fn run_server(backend: Backend, listen: SocketAddr) -> Result<(), ServerError> {
    let session = match backend {
        Backend::Simulator => GuiSession::connect_simulator(),
        Backend::Serial { path, baud } => GuiSession::connect_serial(&path, baud),
    }
    .map_err(ServerError::Gui)?;
    let listener = TcpListener::bind(listen).map_err(ServerError::Io)?;
    let local_address = listener.local_addr().map_err(ServerError::Io)?;
    let token = generate_session_token().map_err(ServerError::Io)?;
    println!("AFIK programmer GUI listening at http://{local_address}/");
    println!("Press Ctrl-C to stop.");
    serve(&listener, session, &token)
}

fn serve(listener: &TcpListener, mut session: GuiSession, token: &str) -> Result<(), ServerError> {
    for incoming in listener.incoming() {
        let stream = incoming.map_err(ServerError::Io)?;
        if let Err(error) = serve_connection(stream, &mut session, token) {
            eprintln!("warning: local client connection failed: {error}");
        }
    }
    Ok(())
}

fn serve_connection(
    mut stream: TcpStream,
    session: &mut GuiSession,
    token: &str,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let response = match read_request(&mut stream) {
        Ok(Some(request)) => handle_request(session, token, &request),
        Ok(None) => return Ok(()),
        Err(HttpReadError::TooLarge) => Response::text(413, "request is too large"),
        Err(HttpReadError::BadRequest(detail)) => Response::text(400, detail),
        Err(HttpReadError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Response::text(408, "request timed out")
        }
        Err(HttpReadError::Io(error)) => return Err(error),
    };
    write_response(&mut stream, &response)
}

#[derive(Debug)]
enum ServerError {
    Gui(GuiError),
    Io(io::Error),
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gui(error) => error.fmt(formatter),
            Self::Io(error) => write!(formatter, "local server I/O failed: {error}"),
        }
    }
}

fn generate_session_token() -> io::Result<String> {
    let mut random = [0_u8; SESSION_TOKEN_BYTES];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let mut token = String::with_capacity(SESSION_TOKEN_BYTES * 2);
    for byte in random {
        write!(&mut token, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(token)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Request {
    method: String,
    path: String,
    session_token: Option<String>,
    confirmation: Option<String>,
    body: Vec<u8>,
}

#[derive(Debug)]
enum HttpReadError {
    BadRequest(&'static str),
    TooLarge,
    Io(io::Error),
}

impl From<io::Error> for HttpReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn read_request<R: Read>(stream: &mut R) -> Result<Option<Request>, HttpReadError> {
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    while header.len() < MAX_HEADER_BYTES {
        let read = stream.read(&mut byte)?;
        if read == 0 {
            return if header.is_empty() {
                Ok(None)
            } else {
                Err(HttpReadError::BadRequest("request headers ended early"))
            };
        }
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            return parse_header_and_body(stream, &header).map(Some);
        }
    }
    Err(HttpReadError::TooLarge)
}

fn parse_header_and_body<R: Read>(stream: &mut R, header: &[u8]) -> Result<Request, HttpReadError> {
    let text = std::str::from_utf8(header)
        .map_err(|_| HttpReadError::BadRequest("request headers are not UTF-8"))?;
    let mut lines = text[..text.len() - 4].split("\r\n");
    let request_line = lines
        .next()
        .ok_or(HttpReadError::BadRequest("missing request line"))?;
    let mut request_fields = request_line.split(' ');
    let method = request_fields
        .next()
        .filter(|field| !field.is_empty())
        .ok_or(HttpReadError::BadRequest("missing request method"))?;
    let path = request_fields
        .next()
        .filter(|field| field.starts_with('/'))
        .ok_or(HttpReadError::BadRequest("invalid request path"))?;
    if request_fields.next() != Some("HTTP/1.1") || request_fields.next().is_some() {
        return Err(HttpReadError::BadRequest("unsupported request line"));
    }

    let mut content_length = None;
    let mut session_token = None;
    let mut confirmation = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or(HttpReadError::BadRequest("malformed request header"))?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(HttpReadError::BadRequest("duplicate content length"));
            }
            content_length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| HttpReadError::BadRequest("invalid content length"))?,
            );
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(HttpReadError::BadRequest(
                "transfer encoding is not supported",
            ));
        } else if name.eq_ignore_ascii_case("x-afik-session") {
            if session_token.is_some() {
                return Err(HttpReadError::BadRequest("duplicate session token"));
            }
            session_token = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("x-afik-confirm") {
            if confirmation.is_some() {
                return Err(HttpReadError::BadRequest("duplicate confirmation"));
            }
            confirmation = Some(value.to_owned());
        }
    }
    let content_length = content_length.unwrap_or(0);
    if content_length > MAX_RESTORE_IMAGE_BYTES {
        return Err(HttpReadError::TooLarge);
    }
    if method == "POST" && content_length == 0 {
        return Err(HttpReadError::BadRequest(
            "POST request requires a non-empty body",
        ));
    }
    let mut body = vec![0; content_length];
    stream.read_exact(&mut body)?;
    Ok(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        session_token,
        confirmation,
        body,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Response {
    status: u16,
    content_type: &'static str,
    disposition: Option<&'static str>,
    body: Vec<u8>,
}

impl Response {
    fn bytes(content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type,
            disposition: None,
            body,
        }
    }

    fn download(name: &'static str, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            disposition: Some(name),
            body,
        }
    }

    fn text(status: u16, message: impl fmt::Display) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            disposition: None,
            body: format!("{message}\n").into_bytes(),
        }
    }
}

fn handle_request(session: &mut GuiSession, token: &str, request: &Request) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => Response::bytes(
            "text/html; charset=utf-8",
            INDEX_HTML
                .replace("__AFIK_SESSION_TOKEN__", token)
                .into_bytes(),
        ),
        ("GET", "/app.css") => Response::bytes(
            "text/css; charset=utf-8",
            crate::APP_CSS.as_bytes().to_vec(),
        ),
        ("GET", "/app.js") => Response::bytes(
            "text/javascript; charset=utf-8",
            crate::APP_JS.as_bytes().to_vec(),
        ),
        ("GET", "/api/state") => match session.state() {
            Ok(state) => Response::bytes(
                "application/json; charset=utf-8",
                state_json(&state).into_bytes(),
            ),
            Err(error) => operation_error(error),
        },
        ("POST", "/api/compile") => {
            let project = match project_text(&request.body) {
                Ok(project) => project,
                Err(response) => return response,
            };
            match session.compile_project(project) {
                Ok(download) => Response::download(
                    "attachment; filename=\"afik-configuration.afik\"",
                    download.image,
                ),
                Err(error) => operation_error(error),
            }
        }
        ("POST", "/api/write") => {
            if let Some(response) = require_mutation_headers(request, token) {
                return response;
            }
            let project = match project_text(&request.body) {
                Ok(project) => project,
                Err(response) => return response,
            };
            match session.write_project(project) {
                Ok(receipt) => receipt_response("Write", receipt),
                Err(error) => operation_error(error),
            }
        }
        ("GET", "/api/backup") => match session.backup() {
            Ok(backup) => {
                Response::download("attachment; filename=\"afik-backup.afik\"", backup.image)
            }
            Err(error) => operation_error(error),
        },
        ("POST", "/api/restore") => {
            if let Some(response) = require_mutation_headers(request, token) {
                return response;
            }
            match session.restore(&request.body) {
                Ok(receipt) => receipt_response("Restore", receipt),
                Err(error) => operation_error(error),
            }
        }
        (method, path) if route_exists(path) && !matches!(method, "GET" | "POST") => {
            Response::text(405, "method not allowed")
        }
        (_, path) if route_exists(path) => Response::text(405, "method not allowed"),
        _ => Response::text(404, "not found"),
    }
}

fn route_exists(path: &str) -> bool {
    matches!(
        path,
        "/" | "/app.css"
            | "/app.js"
            | "/api/state"
            | "/api/compile"
            | "/api/write"
            | "/api/backup"
            | "/api/restore"
    )
}

fn require_mutation_headers(request: &Request, token: &str) -> Option<Response> {
    if request.session_token.as_deref() != Some(token) {
        Some(Response::text(403, "invalid local session token"))
    } else if request.confirmation.as_deref() != Some(CONFIRMATION) {
        Some(Response::text(
            409,
            "explicit configuration replacement confirmation is required",
        ))
    } else {
        None
    }
}

fn project_text(body: &[u8]) -> Result<&str, Response> {
    if body.len() > MAX_PROJECT_TEXT_BYTES {
        return Err(Response::text(413, "project text is too large"));
    }
    std::str::from_utf8(body).map_err(|_| Response::text(400, "project text is not UTF-8"))
}

fn operation_error(error: GuiError) -> Response {
    Response::text(422, error)
}

fn receipt_response(action: &str, receipt: VerifiedConfigurationReceipt) -> Response {
    Response::text(
        200,
        format_args!(
            "{action} verified at generation {} ({} objects, {} bytes, {} generated channels)",
            receipt.generation,
            receipt.report.object_count,
            receipt.report.storage_bytes,
            receipt.report.generated_channels
        ),
    )
}

fn state_json(state: &GuiState) -> String {
    let capabilities = state.capabilities;
    let mut json = format!(
        "{{\"generation\":{},\"capabilities\":{{\"Protocol version\":{},\"Storage version\":{},\"Maximum frame payload\":{},\"Maximum objects\":{},\"Maximum object bytes\":{},\"Plan encodings\":{}}},\"objects\":[",
        state.listing.generation,
        capabilities.protocol_version,
        capabilities.storage_version,
        capabilities.max_frame_payload,
        capabilities.max_objects,
        capabilities.max_object_size,
        capabilities.plan_encodings
    );
    for (index, object) in state.listing.objects.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        let kind = match object.key.kind {
            ObjectKind::GeneratedBank => "generated-bank",
        };
        write!(
            &mut json,
            "{{\"kind\":\"{kind}\",\"id\":{},\"bytes\":{}}}",
            object.key.id, object.encoded_len
        )
        .expect("writing to a String cannot fail");
    }
    json.push_str("]}");
    json
}

fn write_response<W: Write>(stream: &mut W, response: &Response) -> io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Content Too Large",
        422 => "Unprocessable Content",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; connect-src 'self'; img-src 'self' blob:; style-src 'self'; script-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\n",
        response.status,
        response.content_type,
        response.body.len()
    )?;
    if let Some(disposition) = response.disposition {
        write!(stream, "Content-Disposition: {disposition}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(&response.body)
}

#[cfg(test)]
mod tests {
    use super::{
        handle_request, parse_invocation, read_request, state_json, write_response, Backend,
        HttpReadError, Invocation, Request, CONFIRMATION, MAX_PROJECT_TEXT_BYTES,
        MAX_RESTORE_IMAGE_BYTES,
    };
    use crate::GuiSession;
    use std::{io::Cursor, net::SocketAddr, path::PathBuf};

    const TOKEN: &str = "test-session-token";
    const PROJECT: &[u8] = b"1:PMR446:446006250:12500:16:licence-free\n";

    fn request(method: &str, path: &str, body: &[u8]) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            session_token: None,
            confirmation: None,
            body: body.into(),
        }
    }

    fn confirmed_request(path: &str, body: &[u8]) -> Request {
        Request {
            session_token: Some(TOKEN.into()),
            confirmation: Some(CONFIRMATION.into()),
            ..request("POST", path, body)
        }
    }

    fn parse_raw_request(raw: &[u8]) -> Result<Option<Request>, HttpReadError> {
        read_request(&mut Cursor::new(raw))
    }

    fn raw_http_round_trip(raw: &[u8]) -> Vec<u8> {
        let mut session = GuiSession::connect_simulator().unwrap();
        let request = parse_raw_request(raw).unwrap().unwrap();
        let handled = handle_request(&mut session, TOKEN, &request);
        let mut encoded = Vec::new();
        write_response(&mut encoded, &handled).unwrap();
        encoded
    }

    #[test]
    fn launcher_accepts_only_one_backend_and_loopback_listeners() {
        let default = parse_invocation(&["--sim".into()]).unwrap();
        assert_eq!(
            default,
            Invocation::Serve {
                backend: Backend::Simulator,
                listen: "127.0.0.1:8765".parse::<SocketAddr>().unwrap(),
            }
        );
        let serial = parse_invocation(&[
            "--device".into(),
            "/dev/ttyUSB0".into(),
            "--baud".into(),
            "9600".into(),
            "--listen".into(),
            "[::1]:9000".into(),
        ])
        .unwrap();
        assert_eq!(
            serial,
            Invocation::Serve {
                backend: Backend::Serial {
                    path: PathBuf::from("/dev/ttyUSB0"),
                    baud: 9600,
                },
                listen: "[::1]:9000".parse::<SocketAddr>().unwrap(),
            }
        );
        assert!(
            parse_invocation(&["--sim".into(), "--listen".into(), "0.0.0.0:9".into()]).is_err()
        );
        assert!(parse_invocation(&["--device".into(), "/dev/null".into()]).is_err());
        assert!(parse_invocation(&["--sim".into(), "--baud".into(), "9600".into()]).is_err());
    }

    #[test]
    fn interface_injects_token_and_returns_deterministic_state() {
        let mut first = GuiSession::connect_simulator().unwrap();
        let mut second = GuiSession::connect_simulator().unwrap();
        let document = handle_request(&mut first, TOKEN, &request("GET", "/", &[]));
        assert_eq!(document.status, 200);
        let document = String::from_utf8(document.body).unwrap();
        assert!(document.contains(TOKEN));
        assert!(!document.contains("__AFIK_SESSION_TOKEN__"));

        let first_state = handle_request(&mut first, TOKEN, &request("GET", "/api/state", &[]));
        let second_state = handle_request(&mut second, TOKEN, &request("GET", "/api/state", &[]));
        assert_eq!(first_state, second_state);
        assert_eq!(
            String::from_utf8(first_state.body).unwrap(),
            state_json(&first.state().unwrap())
        );
    }

    #[test]
    fn mutations_require_token_and_explicit_confirmation_without_side_effects() {
        let mut session = GuiSession::connect_simulator().unwrap();
        let denied = handle_request(&mut session, TOKEN, &request("POST", "/api/write", PROJECT));
        assert_eq!(denied.status, 403);
        let mut token_only = request("POST", "/api/write", PROJECT);
        token_only.session_token = Some(TOKEN.into());
        assert_eq!(handle_request(&mut session, TOKEN, &token_only).status, 409);
        assert_eq!(session.state().unwrap().listing.generation, 0);

        let written = handle_request(
            &mut session,
            TOKEN,
            &confirmed_request("/api/write", PROJECT),
        );
        assert_eq!(written.status, 200);
        assert_eq!(session.state().unwrap().listing.generation, 1);
        let backup = handle_request(&mut session, TOKEN, &request("GET", "/api/backup", &[]));
        assert_eq!(backup.status, 200);

        let mut restored = GuiSession::connect_simulator().unwrap();
        let receipt = handle_request(
            &mut restored,
            TOKEN,
            &confirmed_request("/api/restore", &backup.body),
        );
        assert_eq!(receipt.status, 200);
        assert_eq!(restored.state().unwrap().listing.generation, 1);
    }

    #[test]
    fn routes_bound_project_text_and_reject_wrong_methods() {
        let mut session = GuiSession::connect_simulator().unwrap();
        let compiled = handle_request(
            &mut session,
            TOKEN,
            &request("POST", "/api/compile", PROJECT),
        );
        assert_eq!(compiled.status, 200);
        assert_eq!(compiled.content_type, "application/octet-stream");
        let too_large = vec![b'x'; MAX_PROJECT_TEXT_BYTES + 1];
        assert_eq!(
            handle_request(
                &mut session,
                TOKEN,
                &request("POST", "/api/compile", &too_large)
            )
            .status,
            413
        );
        assert_eq!(
            handle_request(&mut session, TOKEN, &request("POST", "/api/state", b"x")).status,
            405
        );
        assert_eq!(
            handle_request(&mut session, TOKEN, &request("GET", "/missing", &[])).status,
            404
        );
    }

    #[test]
    fn wire_parser_bounds_bodies_and_rejects_ambiguous_headers() {
        let parsed = parse_raw_request(
            b"POST /api/compile HTTP/1.1\r\nContent-Length: 1\r\nX-Afik-Session: x\r\n\r\ny",
        )
        .unwrap()
        .unwrap();
        assert_eq!(parsed.path, "/api/compile");
        assert_eq!(parsed.body, b"y");

        let oversized = format!(
            "POST /api/restore HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_RESTORE_IMAGE_BYTES + 1
        );
        assert!(matches!(
            parse_raw_request(oversized.as_bytes()),
            Err(HttpReadError::TooLarge)
        ));
        assert!(matches!(
            parse_raw_request(
                b"POST /api/write HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx"
            ),
            Err(HttpReadError::BadRequest("duplicate content length"))
        ));
        assert!(matches!(
            parse_raw_request(
                b"POST /api/write HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"
            ),
            Err(HttpReadError::BadRequest(
                "transfer encoding is not supported"
            ))
        ));
    }

    #[test]
    fn http_round_trip_frames_secure_embedded_document() {
        let response = raw_http_round_trip(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Cache-Control: no-store\r\n"));
        assert!(response.contains("Content-Security-Policy: default-src 'self'"));
        assert!(response.contains("X-Content-Type-Options: nosniff\r\n"));
        assert!(response.contains(TOKEN));
    }
}
