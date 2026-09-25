use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use repo_com_delivery_retry::{
    ManualClock, ObservedMessage, ReadError as RecoveryReadError, ReadPage, Reconciler,
    ReconciliationDecision, ReconciliationReader, ReconciliationRequest, RecoveryTarget,
};
use repo_com_state::StateStore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::runtime::{Builder, Runtime};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{header, method, path},
};

pub const REPOSITORY_ID: &str = "acme/e2e";
pub const WORKSPACE_ID: &str = "100000000000000001";
pub const CHANNEL_ID: &str = "200000000000000001";
pub const BOT_USER_ID: &str = "500000000000000001";
pub const HUMAN_USER_ID: &str = "400000000000000001";
pub const ACCEPTED_MESSAGE_ID: &str = "300000000000000001";
pub const HUMAN_MESSAGE_ID: &str = "300000000000000002";
pub const OUTBOUND_TEXT: &str = "Synthetic mocked release request.";
pub const HUMAN_REPLY_TEXT: &str = "Synthetic mocked reply for validation.";
pub const EVENT_TYPE: &str = "build_failed";
pub const DESTINATION_ALIAS: &str = "release";
pub const SEVERITY: &str = "high";
pub const TUPLE: &str = "build_failed/release/high";
pub const CONCURRENT_INVOCATIONS: usize = 100;

type CaptureLog = Arc<Mutex<Vec<(String, Vec<u8>, Vec<u8>)>>>;

const TOKEN_ENV: &str = "REPO_COM_DISCORD_TOKEN";
static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);
const CREATE_PATH: &str = "/api/v10/channels/200000000000000001/messages";
const PYTHON_PROXY: &str = r#"
import concurrent.futures
import socket
import ssl
import sys
import time

backend_host = sys.argv[1]
backend_port = int(sys.argv[2])
certificate = sys.argv[3]
private_key = sys.argv[4]
delay_seconds = float(sys.argv[5])

context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(certificate, private_key)
context.set_alpn_protocols(["http/1.1"])

hop_by_hop = {
    "connection",
    "proxy-connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}

def read_line_raw(stream):
    data = bytearray()
    while not data.endswith(b"\n"):
        chunk = stream.recv(1)
        if not chunk:
            raise RuntimeError("incomplete control line")
        data.extend(chunk)
        if len(data) > 8192:
            raise RuntimeError("control line exceeded bound")
    return bytes(data)

def read_http_request(stream):
    data = bytearray()
    while b"\r\n\r\n" not in data:
        chunk = stream.recv(8192)
        if not chunk:
            raise RuntimeError("incomplete request headers")
        data.extend(chunk)
        if len(data) > 131072:
            raise RuntimeError("request headers exceeded bound")
    header_end = data.index(b"\r\n\r\n") + 4
    lines = bytes(data[:header_end]).split(b"\r\n")
    request_line = lines[0].decode("ascii")
    headers = []
    for line in lines[1:]:
        if not line:
            continue
        name, value = line.split(b":", 1)
        headers.append((name.decode("ascii"), value.strip().decode("latin1")))
    content_length = 0
    for name, value in headers:
        if name.lower() == "content-length":
            content_length = int(value)
    body = bytearray(data[header_end:])
    while len(body) < content_length:
        chunk = stream.recv(min(8192, content_length - len(body)))
        if not chunk:
            raise RuntimeError("incomplete request body")
        body.extend(chunk)
    return request_line, headers, bytes(body[:content_length])

def read_http_response(stream):
    data = bytearray()
    while b"\r\n\r\n" not in data:
        chunk = stream.recv(8192)
        if not chunk:
            raise RuntimeError("incomplete response headers")
        data.extend(chunk)
        if len(data) > 1048576:
            raise RuntimeError("response exceeded bound")
    header_end = data.index(b"\r\n\r\n") + 4
    lines = bytes(data[:header_end]).split(b"\r\n")
    content_length = None
    for line in lines[1:]:
        if b":" not in line:
            continue
        name, value = line.split(b":", 1)
        if name.lower() == b"content-length":
            content_length = int(value.strip())
    if content_length is None:
        raise RuntimeError("response omitted content length")
    body = bytearray(data[header_end:])
    while len(body) < content_length:
        chunk = stream.recv(min(8192, content_length - len(body)))
        if not chunk:
            raise RuntimeError("incomplete response body")
        body.extend(chunk)
    return bytes(data[:header_end]) + bytes(body[:content_length])

def handle(connection):
    upstream = None
    tls = None
    stage = "connect"
    try:
        connection.settimeout(35)
        connect_line = read_line_raw(connection).decode("ascii").strip()
        if connect_line != "CONNECT discord.com:443 HTTP/1.1":
            connection.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            return
        while True:
            line = read_line_raw(connection)
            if line in (b"\r\n", b"\n"):
                break
        connection.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        stage = "tls"
        tls = context.wrap_socket(connection, server_side=True)
        stage = "request"
        request_line, headers, body = read_http_request(tls)
        method, request_path, _ = request_line.split(" ", 2)
        if method not in ("GET", "POST") or not request_path.startswith("/api/v10/"):
            tls.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            return
        forwarded = [request_line.encode("ascii")]
        for name, value in headers:
            if name.lower() not in hop_by_hop and name.lower() != "host":
                forwarded.append(f"{name}: {value}".encode("latin1"))
        forwarded.append(b"Host: discord.com")
        forwarded.append(b"Connection: close")
        stage = "backend-connect"
        upstream = socket.create_connection((backend_host, backend_port), timeout=5)
        upstream.settimeout(35)
        stage = "backend-write"
        upstream.sendall(b"\r\n".join(forwarded) + b"\r\n\r\n" + body)
        stage = "backend-read"
        response = read_http_response(upstream)
        if delay_seconds > 0:
            time.sleep(delay_seconds)
        stage = "client-write"
        tls.sendall(response)
        stage = "client-shutdown"
        try:
            tls.unwrap()
        except Exception:
            pass
    except Exception as error:
        import traceback
        print("loopback proxy handler error at " + stage + ": " + type(error).__name__ + ": " + str(error), file=sys.stderr, flush=True)
        traceback.print_exc(file=sys.stderr)
        try:
            if tls is not None:
                tls.sendall(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        except Exception:
            pass
    finally:
        if upstream is not None:
            try:
                upstream.close()
            except Exception:
                pass
        if tls is not None:
            try:
                tls.close()
            except Exception:
                pass
        else:
            try:
                connection.close()
            except Exception:
                pass

listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 0))
listener.listen(256)
print("READY " + str(listener.getsockname()[1]), flush=True)
with concurrent.futures.ThreadPoolExecutor(max_workers=160) as pool:
    while True:
        connection, _ = listener.accept()
        pool.submit(handle, connection)
"#;
const PYTHON_PTY: &str = r#"
import os
import pty
import select
import sys
import time

binary = sys.argv[1]
working_directory = sys.argv[2]
payload = sys.argv[3]
marker = sys.argv[4]
response = sys.argv[5]
arguments = sys.argv[6:]

pid, master = pty.fork()
if pid == 0:
    os.chdir(working_directory)
    os.execv(binary, [binary, *arguments])

os.write(master, payload.encode("utf-8") + b"\n\x04")
output = bytearray()
deadline = time.monotonic() + 20
prompt_sent = False
input_closed = False
try:
    while time.monotonic() < deadline:
        if not input_closed:
            readable, _, _ = select.select([master], [], [], 0.1)
            if readable:
                try:
                    chunk = os.read(master, 8192)
                except OSError:
                    input_closed = True
                else:
                    if chunk:
                        output.extend(chunk)
                    else:
                        input_closed = True
        if not prompt_sent and marker.encode("utf-8") in output:
            os.write(master, response.encode("utf-8") + b"\n")
            prompt_sent = True
        finished, status = os.waitpid(pid, os.WNOHANG)
        if finished == pid:
            while True:
                try:
                    chunk = os.read(master, 8192)
                except OSError:
                    break
                if not chunk:
                    break
                output.extend(chunk)
            sys.stdout.buffer.write(output)
            sys.stdout.buffer.flush()
            sys.exit(os.waitstatus_to_exitcode(status))
    os.kill(pid, 9)
    os.waitpid(pid, 0)
except Exception:
    try:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
    except Exception:
        pass
sys.stdout.buffer.write(output)
sys.exit(124)
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestCounts {
    pub create: usize,
    pub list: usize,
    pub point: usize,
    pub total: usize,
}

pub struct ConcurrentBatch {
    pub results: Vec<ProcessResult>,
    pub spawned: usize,
    pub alive_before_release: usize,
    pub release_group_size: usize,
}

struct PendingChild {
    child: Child,
    input: Option<std::process::ChildStdin>,
    stdout_reader: Option<std::thread::JoinHandle<Vec<u8>>>,
    stderr_reader: Option<std::thread::JoinHandle<Vec<u8>>>,
}

impl Drop for PendingChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct ProcessResult {
    pub status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ProcessResult {
    #[must_use]
    pub fn success_json(self, label: &str) -> Value {
        let value: Value = serde_json::from_slice(&self.stdout).unwrap_or_else(|_| {
            panic!(
                "{label} returned one protocol JSON object: status={:?}, stdout_bytes={}, stderr={}",
                self.status,
                self.stdout.len(),
                String::from_utf8_lossy(&self.stderr)
            )
        });
        assert_eq!(value["protocol_version"], 1, "{label} protocol version");
        assert!(
            self.status.success() && value["status"] == "success",
            "{label} process did not succeed: {} ({})",
            value["error"]["code"],
            value["error"]["message"]
        );
        value["data"].clone()
    }

    #[must_use]
    pub fn text(self, label: &str) -> String {
        assert!(self.status.success(), "{label} process did not succeed");
        String::from_utf8(self.stdout).unwrap_or_else(|_| panic!("{label} emitted UTF-8"))
    }
}

#[derive(Clone)]
pub struct JsonRunner {
    binary: PathBuf,
    root: PathBuf,
    config_path: PathBuf,
    state_path: PathBuf,
    user_data: PathBuf,
    home: PathBuf,
    temp: PathBuf,
    proxy_url: String,
    certificate: PathBuf,
    certificate_directory: PathBuf,
    token: String,
    captures: CaptureLog,
}

impl JsonRunner {
    pub fn run(&self, label: &str, route: &[&str], input: Value) -> ProcessResult {
        let payload = envelope(&route_protocol(route), input);
        let mut command = self.command(false);
        command.args(route);
        self.record(label, run_with_stdin(command, payload.as_bytes()))
    }

    pub fn run_released_batch(
        &self,
        label_prefix: &str,
        route: &[&str],
        input: Value,
        count: usize,
    ) -> ConcurrentBatch {
        const RELEASE_GROUP_SIZE: usize = 10;
        assert!(
            count > 0,
            "a released batch must contain at least one process"
        );
        let payload = envelope(&route_protocol(route), input);
        let mut pending = Vec::with_capacity(count);

        for _ in 0..count {
            let mut command = self.command(false);
            command.args(route);
            let mut child = command.spawn().expect("concurrent repo-com process starts");
            let input = child.stdin.take().expect("concurrent process stdin pipe");
            let stdout = child.stdout.take().expect("concurrent process stdout pipe");
            let stderr = child.stderr.take().expect("concurrent process stderr pipe");
            pending.push(PendingChild {
                child,
                input: Some(input),
                stdout_reader: Some(spawn_pipe_reader(stdout)),
                stderr_reader: Some(spawn_pipe_reader(stderr)),
            });
        }

        let mut alive_before_release = 0;
        for process in &mut pending {
            if process
                .child
                .try_wait()
                .expect("concurrent process readiness is observable")
                .is_none()
            {
                alive_before_release += 1;
            }
        }
        if alive_before_release != count {
            panic!(
                "all {count} child processes must remain alive before release; observed {alive_before_release}"
            );
        }

        let deadline = Instant::now() + Duration::from_secs(120);
        let mut results = Vec::with_capacity(count);
        let mut completed = 0;
        for group in pending.chunks_mut(RELEASE_GROUP_SIZE) {
            for process in group.iter_mut() {
                let mut input = process
                    .input
                    .take()
                    .expect("each child has one held input pipe");
                input
                    .write_all(payload.as_bytes())
                    .expect("concurrent process input releases");
            }
            for process in group.iter_mut() {
                let status = wait_for_child(&mut process.child, deadline);
                let stdout = process
                    .stdout_reader
                    .take()
                    .expect("one stdout reader per child")
                    .join()
                    .expect("concurrent stdout reader joins");
                let stderr = process
                    .stderr_reader
                    .take()
                    .expect("one stderr reader per child")
                    .join()
                    .expect("concurrent stderr reader joins");
                let output = std::process::Output {
                    status,
                    stdout,
                    stderr,
                };
                results.push(self.record(&format!("{label_prefix}.{completed}"), output));
                completed += 1;
            }
        }

        ConcurrentBatch {
            results,
            spawned: count,
            alive_before_release,
            release_group_size: RELEASE_GROUP_SIZE,
        }
    }

    fn run_tty(
        &self,
        label: &str,
        route: &[&str],
        input: Value,
        marker: &str,
        response: &str,
    ) -> ProcessResult {
        let payload = envelope(&route_protocol(route), input);
        let mut command = self.command(true);
        command
            .arg("-c")
            .arg(PYTHON_PTY)
            .arg(&self.binary)
            .arg(&self.root)
            .arg(payload)
            .arg(marker)
            .arg(response)
            .args(self.base_human_args())
            .args(route);
        self.record(label, command.output().expect("PTY helper completes"))
    }

    fn command(&self, tty: bool) -> Command {
        let mut command = if tty {
            Command::new("python3")
        } else {
            let mut command = Command::new(&self.binary);
            command.args(self.base_json_args());
            command
        };
        command.current_dir(&self.root);
        command.env(TOKEN_ENV, &self.token);
        command.env("HTTPS_PROXY", &self.proxy_url);
        command.env("https_proxy", &self.proxy_url);
        command.env("HTTP_PROXY", &self.proxy_url);
        command.env("http_proxy", &self.proxy_url);
        command.env("ALL_PROXY", &self.proxy_url);
        command.env("all_proxy", &self.proxy_url);
        command.env("NO_PROXY", "");
        command.env("no_proxy", "");
        command.env("SSL_CERT_FILE", &self.certificate);
        command.env("SSL_CERT_DIR", &self.certificate_directory);
        command.env("XDG_DATA_HOME", &self.user_data);
        command.env("XDG_CONFIG_HOME", &self.user_data);
        command.env("HOME", &self.home);
        command.env("APPDATA", &self.user_data);
        command.env("LOCALAPPDATA", &self.user_data);
        command.env("TMPDIR", &self.temp);
        command.env("NO_COLOR", "1");
        command.env_remove("REPO_COM_CONFIG");
        command.env_remove("REPO_COM_STATE");
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        command
    }

    fn base_json_args(&self) -> Vec<String> {
        vec![
            "--config".to_owned(),
            path_text(&self.config_path),
            "--state".to_owned(),
            path_text(&self.state_path),
            "--output".to_owned(),
            "json".to_owned(),
        ]
    }

    fn base_human_args(&self) -> Vec<String> {
        vec![
            "--config".to_owned(),
            path_text(&self.config_path),
            "--state".to_owned(),
            path_text(&self.state_path),
            "--color".to_owned(),
            "never".to_owned(),
            "--tty".to_owned(),
        ]
    }

    fn record(&self, label: &str, output: std::process::Output) -> ProcessResult {
        assert_capture_clean(label, &self.token, &output.stdout, &output.stderr);
        self.captures.lock().expect("capture lock").push((
            label.to_owned(),
            output.stdout.clone(),
            output.stderr.clone(),
        ));
        ProcessResult {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

pub struct E2eHarness {
    _proxy: TlsProxy,
    server: MockServer,
    _runtime: Runtime,
    runner: JsonRunner,
    workspace: Workspace,
    token: String,
}

impl E2eHarness {
    #[must_use]
    pub fn start(name: &str, response_delay: Duration) -> Self {
        assert_fixture_hygiene();
        require_command("python3", &["--version"]);
        require_command("openssl", &["version"]);
        let workspace = Workspace::create(name);
        let token = synthetic_token();
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("WireMock runtime starts");
        let server = runtime.block_on(MockServer::start());
        let proxy = TlsProxy::start(&workspace.root, *server.address(), response_delay);
        let runner = JsonRunner {
            binary: binary_path(),
            root: workspace.root.clone(),
            config_path: workspace.config_path.clone(),
            state_path: workspace.state_path.clone(),
            user_data: workspace.user_data.clone(),
            home: workspace.home.clone(),
            temp: workspace.temp.clone(),
            proxy_url: proxy.url.clone(),
            certificate: proxy.certificate.clone(),
            certificate_directory: workspace.certificate_directory.clone(),
            token: token.clone(),
            captures: Arc::new(Mutex::new(Vec::new())),
        };
        Self {
            _proxy: proxy,
            server,
            _runtime: runtime,
            runner,
            workspace,
            token,
        }
    }

    #[must_use]
    pub fn runner(&self) -> JsonRunner {
        self.runner.clone()
    }

    #[must_use]
    pub fn server(&self) -> &MockServer {
        &self.server
    }

    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn state_path(&self) -> &Path {
        &self.workspace.state_path
    }

    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.workspace.config_path
    }

    pub fn run(&self, label: &str, route: &[&str], input: Value) -> ProcessResult {
        self.runner.run(label, route, input)
    }

    pub fn run_tty(
        &self,
        label: &str,
        route: &[&str],
        input: Value,
        marker: &str,
        response: &str,
    ) -> ProcessResult {
        self.runner.run_tty(label, route, input, marker, response)
    }

    pub async fn mount_create(&self) {
        let accepted: Value = accepted_message_fixture();
        Mock::given(method("POST"))
            .and(path(CREATE_PATH))
            .and(header("authorization", format!("Bot {}", self.token)))
            .respond_with(ResponseTemplate::new(200).set_body_json(accepted))
            .expect(1)
            .mount(&self.server)
            .await;
    }

    pub async fn mount_inbound(&self) {
        let human: Value = human_reply_fixture();
        Mock::given(method("GET"))
            .and(path(CREATE_PATH))
            .and(header("authorization", format!("Bot {}", self.token)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([human])))
            .expect(1)
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{CREATE_PATH}/{HUMAN_MESSAGE_ID}")))
            .and(header("authorization", format!("Bot {}", self.token)))
            .respond_with(ResponseTemplate::new(200).set_body_json(human))
            .expect(1)
            .mount(&self.server)
            .await;
    }

    pub async fn mount_reconciliation(&self, exact_content: &str) {
        let message = json!({
            "id": "300000000000000003",
            "channel_id": CHANNEL_ID,
            "content": exact_content,
            "timestamp": "2099-01-01T00:00:06Z",
            "author": {"id": BOT_USER_ID, "bot": true},
            "mentions": [],
            "attachments": []
        });
        Mock::given(method("GET"))
            .and(path(CREATE_PATH))
            .and(header("authorization", format!("Bot {}", self.token)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([message])))
            .expect(1)
            .mount(&self.server)
            .await;
    }

    pub async fn request_counts(&self) -> RequestCounts {
        let requests = self
            .server
            .received_requests()
            .await
            .expect("WireMock request journal");
        let mut counts = RequestCounts {
            create: 0,
            list: 0,
            point: 0,
            total: requests.len(),
        };
        for request in &requests {
            assert!(request.url.path().starts_with("/api/v10/"));
            assert_eq!(
                request
                    .headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok()),
                Some(format!("Bot {}", self.token).as_str())
            );
            assert!(
                !String::from_utf8_lossy(&request.body)
                    .to_ascii_lowercase()
                    .contains(&self.token.to_ascii_lowercase()),
                "WireMock request body leaked the runtime credential"
            );
            match (request.method.as_str(), request.url.path()) {
                ("POST", CREATE_PATH) => counts.create += 1,
                ("GET", CREATE_PATH) => counts.list += 1,
                ("GET", path) if path == format!("{CREATE_PATH}/{HUMAN_MESSAGE_ID}") => {
                    counts.point += 1;
                }
                _ => panic!("WireMock observed an unexpected Discord operation"),
            }
        }
        counts
    }

    pub async fn create_request(&self) -> Request {
        let requests = self
            .server
            .received_requests()
            .await
            .expect("WireMock request journal");
        requests
            .into_iter()
            .find(|request| request.method.as_str() == "POST" && request.url.path() == CREATE_PATH)
            .expect("one Discord create request")
    }

    #[must_use]
    pub fn capture_count(&self) -> usize {
        self.runner.captures.lock().expect("capture lock").len()
    }
}

struct TlsProxy {
    url: String,
    certificate: PathBuf,
    child: Child,
}

impl TlsProxy {
    fn start(root: &Path, backend: SocketAddr, response_delay: Duration) -> Self {
        let tls = root.join("tls");
        fs::create_dir_all(&tls).expect("temporary TLS directory");
        let certificate_authority = tls.join("discord-ca-cert.pem");
        let certificate_authority_key = tls.join("discord-ca-key.pem");
        let certificate = tls.join("discord-cert.pem");
        let private_key = tls.join("discord-key.pem");
        let request = tls.join("discord.csr");
        let extensions = tls.join("discord.ext");
        fs::write(
            &extensions,
            "subjectAltName=DNS:discord.com\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n",
        )
        .expect("ephemeral certificate extension file writes");
        let certificate_authority_output =
            openssl_ca(&certificate_authority, &certificate_authority_key);
        assert!(
            certificate_authority_output.success(),
            "OpenSSL creates an ephemeral loopback CA"
        );
        let request_output = openssl_request(&private_key, &request);
        assert!(
            request_output.success(),
            "OpenSSL creates an ephemeral loopback request"
        );
        let certificate_output = Command::new("openssl")
            .arg("x509")
            .arg("-req")
            .arg("-in")
            .arg(path_text(&request))
            .arg("-CA")
            .arg(path_text(&certificate_authority))
            .arg("-CAkey")
            .arg(path_text(&certificate_authority_key))
            .arg("-CAcreateserial")
            .arg("-out")
            .arg(path_text(&certificate))
            .arg("-days")
            .arg("1")
            .arg("-extfile")
            .arg(path_text(&extensions))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("OpenSSL signs the loopback certificate");
        assert!(
            certificate_output.success(),
            "OpenSSL signs an ephemeral loopback certificate"
        );
        let delay = response_delay.as_secs_f64();
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(PYTHON_PROXY)
            .arg(backend.ip().to_string())
            .arg(backend.port().to_string())
            .arg(path_text(&certificate))
            .arg(path_text(&private_key))
            .arg(delay.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("loopback TLS proxy starts");
        let stdout = child.stdout.take().expect("proxy ready stream");
        let mut ready = String::new();
        BufReader::new(stdout)
            .read_line(&mut ready)
            .expect("proxy publishes its loopback port");
        let port = ready
            .strip_prefix("READY ")
            .and_then(|value| value.trim().parse::<u16>().ok())
            .unwrap_or_else(|| {
                let mut diagnostic = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut diagnostic);
                }
                panic!(
                    "proxy publishes a valid loopback port: {}{diagnostic}",
                    ready.trim()
                )
            });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            certificate: certificate_authority,
            child,
        }
    }
}

impl Drop for TlsProxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Workspace {
    root: PathBuf,
    config_path: PathBuf,
    state_path: PathBuf,
    user_data: PathBuf,
    home: PathBuf,
    temp: PathBuf,
    certificate_directory: PathBuf,
}

impl Workspace {
    fn create(name: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "repo-com-e2e-{name}-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        let config_path = root.join(".repo-com.toml");
        let state_path = root.join("state.sqlite3");
        let user_data = root.join("user-data");
        let home = root.join("home");
        let temp = root.join("tmp");
        let certificate_directory = root.join("empty-cert-dir");
        fs::create_dir_all(root.join(".git")).expect("temporary repository root");
        fs::create_dir_all(&user_data).expect("temporary user data");
        fs::create_dir_all(&home).expect("temporary home");
        fs::create_dir_all(&temp).expect("temporary process directory");
        fs::create_dir_all(&certificate_directory).expect("empty certificate directory");
        fs::copy(fixture_path("valid-config.toml"), &config_path)
            .expect("valid configuration fixture copies");
        restrict_permissions(&root);
        restrict_permissions(&config_path);
        Self {
            root,
            config_path,
            state_path,
            user_data,
            home,
            temp,
            certificate_directory,
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub struct WiremockRecoveryReader {
    address: SocketAddr,
    token: String,
    calls: usize,
}

impl WiremockRecoveryReader {
    #[must_use]
    pub fn new(address: SocketAddr, token: impl Into<String>) -> Self {
        Self {
            address,
            token: token.into(),
            calls: 0,
        }
    }

    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls
    }
}

impl ReconciliationReader for WiremockRecoveryReader {
    fn read_destination(
        &mut self,
        request: &ReconciliationRequest,
    ) -> Result<ReadPage, RecoveryReadError> {
        self.calls += 1;
        let target = &request.target;
        let mut stream =
            TcpStream::connect(self.address).map_err(|_| RecoveryReadError::Transport)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| RecoveryReadError::Transport)?;
        let wire_request = format!(
            "GET {CREATE_PATH}?limit=100&after=0 HTTP/1.1\r\nHost: discord.com\r\nAuthorization: Bot {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            self.token
        );
        stream
            .write_all(wire_request.as_bytes())
            .map_err(|_| RecoveryReadError::Transport)?;
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .map_err(|_| RecoveryReadError::Transport)?;
        let separator = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or(RecoveryReadError::Incomplete)?;
        let header = String::from_utf8(bytes[..separator].to_vec())
            .map_err(|_| RecoveryReadError::InvalidResponse)?;
        if !header.starts_with("HTTP/1.1 200") {
            return Err(RecoveryReadError::InvalidResponse);
        }
        let body: Value = serde_json::from_slice(&bytes[separator + 4..])
            .map_err(|_| RecoveryReadError::InvalidResponse)?;
        let message = body
            .as_array()
            .and_then(|messages| messages.first())
            .ok_or(RecoveryReadError::InvalidResponse)?;
        if message["channel_id"] != target.channel_id
            || message["author"]["id"] != target.bot_author_id
            || message["author"]["bot"] != true
            || message["content"] != target.exact_content
        {
            return Err(RecoveryReadError::InvalidResponse);
        }
        let message_id = message["id"]
            .as_str()
            .ok_or(RecoveryReadError::InvalidResponse)?;
        Ok(ReadPage::new(vec![ObservedMessage::new(
            message_id,
            &target.channel_id,
            &target.bot_author_id,
            &target.nonce,
            &target.exact_content,
        )]))
    }
}

#[must_use]
pub fn accepted_message_fixture() -> Value {
    serde_json::from_str(&fixture_text("accepted-message.json")).expect("accepted fixture JSON")
}

#[must_use]
pub fn human_reply_fixture() -> Value {
    serde_json::from_str(&fixture_text("human-reply.json")).expect("human reply fixture JSON")
}

#[must_use]
pub fn fixture_text(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).unwrap_or_else(|_| panic!("fixture {name} reads"))
}

pub fn assert_fixture_hygiene() {
    let config = fixture_text("valid-config.toml").to_ascii_lowercase();
    for forbidden in [
        "authorization",
        "bearer ",
        "bot ",
        "private key",
        "password",
    ] {
        assert!(!config.contains(forbidden));
    }
    for value in [accepted_message_fixture(), human_reply_fixture()] {
        assert_synthetic_json(&value);
    }
}

#[must_use]
pub fn tuple_hash() -> String {
    let canonical =
        br#"{"event_type":"build_failed","destination_alias":"release","severity":"high"}"#;
    let digest = Sha256::digest(canonical);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[must_use]
pub fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

#[must_use]
pub fn format_utc(seconds: u64) -> String {
    let days = seconds / 86_400;
    let day_seconds = seconds % 86_400;
    let shifted = i64::try_from(days).expect("fixture days fit i64") + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        day_seconds / 3_600,
        (day_seconds % 3_600) / 60,
        day_seconds % 60
    )
}

pub fn assert_create_request(request: &Request, exact_text: &str, content_nonce: &str) {
    assert_eq!(request.method.as_str(), "POST");
    assert_eq!(request.url.path(), CREATE_PATH);
    assert_eq!(
        request
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let body: Value = serde_json::from_slice(&request.body).expect("one create JSON body");
    let object = body.as_object().expect("create body object");
    let keys: Vec<&str> = object.keys().map(String::as_str).collect();
    assert_eq!(keys.len(), 3);
    assert!(keys.contains(&"content"));
    assert!(keys.contains(&"nonce"));
    assert!(keys.contains(&"allowed_mentions"));
    assert_eq!(body["content"], exact_text);
    assert_eq!(body["nonce"], content_nonce);
    assert_eq!(
        body["allowed_mentions"],
        json!({"parse": [], "roles": [], "users": [], "replied_user": false})
    );
}

pub fn assert_durable_accepted(path: &Path, repository_id: &str, attempt_id: &str) {
    let store = StateStore::open_path(path).expect("accepted state opens");
    let attempt = store
        .delivery_attempt(repository_id, attempt_id)
        .expect("accepted attempt reads")
        .expect("accepted attempt exists");
    assert_eq!(attempt.state, "accepted");
    assert_eq!(
        attempt.remote_message_id.as_deref(),
        Some(ACCEPTED_MESSAGE_ID)
    );
}

pub fn assert_durable_unknown(path: &Path, repository_id: &str, attempt_id: &str) {
    let store = StateStore::open_path(path).expect("unknown state opens");
    let attempt = store
        .delivery_attempt(repository_id, attempt_id)
        .expect("unknown attempt reads")
        .expect("unknown attempt exists");
    assert_eq!(attempt.state, "unknown");
    assert!(attempt.remote_message_id.is_none());
}

#[must_use]
pub fn matching_reconciliation(
    address: SocketAddr,
    token: &str,
    exact_text: &str,
    content_nonce: &str,
    now: u64,
) -> (usize, ReconciliationDecision) {
    let target = RecoveryTarget::new(
        DESTINATION_ALIAS,
        WORKSPACE_ID,
        CHANNEL_ID,
        BOT_USER_ID,
        content_nonce,
        exact_text,
        now,
    )
    .expect("complete recovery target");
    let reader = WiremockRecoveryReader::new(address, token);
    let mut reconciler = Reconciler::new(reader, ManualClock::new(now));
    let decision = reconciler
        .reconcile(&target)
        .expect("read-only reconciliation");
    let calls = reconciler.reader().calls();
    (calls, decision)
}

pub fn emit_evidence(scenario: &str, counts: RequestCounts, extra: Value) {
    let evidence = json!({
        "contract": "REL-E2E-1",
        "classification": "token-free-mocked-e2e",
        "scenario": scenario,
        "environment": {
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "binary": "repo-com",
            "network_boundary": "local WireMock through loopback-only TLS proxy"
        },
        "samples": {
            "contract_test_cases": 1,
            "scenario_observations": extra.clone()
        },
        "thresholds": {
            "external_network_max": 0,
            "wiremock_required": true,
            "accepted_message_ids_required": 1
        },
        "requests": {
            "discord_create": counts.create,
            "discord_list": counts.list,
            "discord_point": counts.point,
            "wiremock_total": counts.total,
            "external": 0
        },
        "claims": {
            "live_discord": false,
            "human_acceptance": false,
            "human_approval": false,
            "read_receipt": false,
            "response_analytics": false
        },
        "exclusions": ["live Discord", "human UX review", "security review", "release sign-off"],
        "observations": extra
    });
    println!(
        "{}",
        serde_json::to_string(&evidence).expect("machine-readable E2E evidence")
    );
}

fn assert_capture_clean(label: &str, token: &str, stdout: &[u8], stderr: &[u8]) {
    let token = token.to_ascii_lowercase();
    for (stream, bytes) in [("stdout", stdout), ("stderr", stderr)] {
        let text = String::from_utf8_lossy(bytes);
        let lower = text.to_ascii_lowercase();
        assert!(
            !lower.contains(&token),
            "{label} {stream} leaked the runtime credential"
        );
        for forbidden in [
            "authorization:",
            "\"authorization\"",
            "bearer ",
            "private key",
        ] {
            assert!(
                !lower.contains(forbidden),
                "{label} {stream} leaked forbidden credential marker {forbidden}"
            );
        }
    }
}

fn assert_synthetic_json(value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let lower = key.to_ascii_lowercase();
                assert!(!lower.contains("authorization"));
                assert!(!lower.contains("token"));
                assert!(!lower.contains("private_key"));
                if lower == "content" {
                    assert_eq!(child.as_str(), Some(HUMAN_REPLY_TEXT));
                }
                assert_synthetic_json(child);
            }
        }
        Value::Array(values) => values.iter().for_each(assert_synthetic_json),
        Value::String(value)
            if value.chars().all(|character| character.is_ascii_digit()) && value.len() >= 15 =>
        {
            assert!(
                [
                    WORKSPACE_ID,
                    CHANNEL_ID,
                    HUMAN_USER_ID,
                    ACCEPTED_MESSAGE_ID,
                    HUMAN_MESSAGE_ID,
                    BOT_USER_ID
                ]
                .contains(&value.as_str())
            );
        }
        _ => {}
    }
}

fn binary_path() -> PathBuf {
    let mut path = std::env::current_exe().expect("test executable path");
    path.pop();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join(format!("repo-com{}", std::env::consts::EXE_SUFFIX))
}

fn envelope(command: &str, input: Value) -> String {
    serde_json::to_string(&json!({
        "protocol_version": 1,
        "command": command,
        "input": input,
    }))
    .expect("protocol envelope serializes")
}

fn route_protocol(route: &[&str]) -> String {
    match route {
        ["config", "validate"] => "config.validate",
        ["policy", "status"] => "policy.status",
        ["policy", "activate"] => "policy.activate",
        ["draft", "create"] => "draft.create",
        ["draft", "show"] => "draft.show",
        ["draft", "update"] => "draft.update",
        ["draft", "preview"] => "draft.preview",
        ["draft", "approve"] => "draft.approve",
        ["draft", "secret-override"] => "draft.secret-override",
        ["send"] | ["send", "dispatch"] => "send",
        ["setup-check"] | ["setup", "check"] => "setup-check",
        ["inbox", "fetch"] => "inbox.fetch",
        ["inbox", "acknowledge"] => "inbox.acknowledge",
        ["inbox", "archive"] => "inbox.archive",
        ["reply", "draft-create"] => "reply.draft-create",
        ["audit", "query"] => "audit.query",
        ["state", "verify"] => "state.verify",
        ["state", "inspect"] | ["lifecycle", "inspect"] => "lifecycle.inspect",
        ["purge", "plan"] => "purge.plan",
        ["purge", "execute"] => "purge.execute",
        _ => panic!("test route has one canonical protocol command"),
    }
    .to_owned()
}

fn spawn_pipe_reader<R>(mut reader: R) -> std::thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .expect("concurrent process output pipe reads");
        bytes
    })
}

fn wait_for_child(child: &mut Child, deadline: Instant) -> ExitStatus {
    loop {
        match child
            .try_wait()
            .expect("concurrent process completion is observable")
        {
            Some(status) => return status,
            None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("concurrent process batch exceeded its completion deadline");
            }
        }
    }
}

fn run_with_stdin(mut command: Command, input: &[u8]) -> std::process::Output {
    let mut child = command.spawn().expect("repo-com process starts");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input)
        .expect("structured input writes");
    child
        .wait_with_output()
        .expect("repo-com process completes")
}

fn synthetic_token() -> String {
    format!("{}.{}.{}", "A".repeat(24), "B".repeat(6), "C".repeat(27))
}

fn fixture_path(name: &str) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = if manifest.join("tests").join("fixtures").is_dir() {
        manifest.join("tests").join("fixtures")
    } else {
        manifest.join("../repo-com-cli/tests/fixtures")
    };
    fixtures.join(name)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn openssl_ca(certificate: &Path, private_key: &Path) -> std::process::ExitStatus {
    Command::new("openssl")
        .arg("req")
        .arg("-x509")
        .arg("-newkey")
        .arg("rsa:2048")
        .arg("-nodes")
        .arg("-keyout")
        .arg(path_text(private_key))
        .arg("-out")
        .arg(path_text(certificate))
        .arg("-days")
        .arg("1")
        .arg("-subj")
        .arg("/CN=repo-com-e2e-ca")
        .arg("-addext")
        .arg("basicConstraints=critical,CA:TRUE")
        .arg("-addext")
        .arg("keyUsage=critical,keyCertSign,cRLSign")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("OpenSSL starts")
}

fn openssl_request(private_key: &Path, request: &Path) -> std::process::ExitStatus {
    Command::new("openssl")
        .arg("req")
        .arg("-new")
        .arg("-newkey")
        .arg("rsa:2048")
        .arg("-nodes")
        .arg("-keyout")
        .arg(path_text(private_key))
        .arg("-out")
        .arg(path_text(request))
        .arg("-subj")
        .arg("/CN=discord.com")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("OpenSSL starts")
}

fn require_command(name: &str, arguments: &[&str]) {
    let output = Command::new(name)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|_| panic!("{name} is available for isolated E2E support"));
    assert!(
        output.status.success(),
        "{name} starts successfully: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if path.is_dir() { 0o700 } else { 0o600 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .expect("restrict fixture permissions");
    }
    #[cfg(not(unix))]
    let _ = path;
}
