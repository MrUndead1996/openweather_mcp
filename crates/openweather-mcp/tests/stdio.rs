#[path = "../src/cache.rs"]
mod cache;
#[path = "../src/server.rs"]
mod server;

use std::{process::Stdio, time::Duration};

use openweather_onecall_sdk::{ClientV4, ClientV4Config};
use rmcp::ServiceExt;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, duplex, split},
    net::TcpListener,
    process::Command,
    time::timeout,
};
use tracing_subscriber::{Layer, filter::Targets, layer::SubscriberExt};

const KEY: &str = "local-mock-only-key";
const DEADLINE: Duration = Duration::from_secs(5);

struct Peer<R, W> {
    reader: BufReader<R>,
    writer: W,
    id: u64,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Peer<R, W> {
    fn new(reader: R, writer: W) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
            id: 0,
        }
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        timeout(DEADLINE, self.writer.write_all(line.as_bytes()))
            .await
            .unwrap()
            .unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(json!({
            "jsonrpc": "2.0", "id": self.id, "method": method, "params": params,
        }))
        .await;

        let mut line = String::new();
        timeout(DEADLINE, self.reader.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        let reply: Value = serde_json::from_str(&line).expect("expected JSON-RPC on stdout");
        assert_eq!(reply["id"], self.id, "{reply}");
        assert!(!reply["result"].is_null(), "{reply}");
        reply["result"].clone()
    }

    async fn initialize(&mut self) {
        let reply = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "stdio-test", "version": "1.0.0"},
                }),
            )
            .await;
        assert_eq!(reply["serverInfo"]["name"], "openweather-mcp");
        assert!(reply["capabilities"]["tools"].is_object());
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
    }

    async fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
            .await
    }
}

fn binary() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_openweather-mcp"));
    cmd.env_remove("OPENWEATHER_API_KEY")
        .env_remove("OPENWEATHER_API_KEY_FILE")
        .env_remove("OPENWEATHER_API_VERSION")
        .env_remove("OPENWEATHER_LOG")
        .env_remove("OPENWEATHER_CACHE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

#[tokio::test]
async fn binary_initializes_from_secret_file_and_lists_all_six_tools() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), format!("{KEY}\n")).unwrap();
    let mut child = binary()
        .env("OPENWEATHER_API_KEY_FILE", file.path())
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut peer = Peer::new(child.stdout.take().unwrap(), child.stdin.take().unwrap());
    peer.initialize().await;

    let reply = peer.request("tools/list", json!({})).await;
    let tools = reply["tools"].as_array().unwrap();
    let mut names: Vec<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "get_15_minute_forecast",
            "get_current_weather",
            "get_daily_timeline",
            "get_hourly_timeline",
            "get_minute_forecast",
            "get_weather_alert",
        ]
    );
    let reply = peer
        .call("get_current_weather", json!({"lat": 91, "lon": 0}))
        .await;
    assert_eq!(reply["isError"], true);
    assert_eq!(reply["structuredContent"]["code"], "invalid_input");

    drop(peer);
    let output = timeout(DEADLINE, child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[tokio::test]
async fn binary_rejects_conflicting_sources_without_printing_secret_values() {
    let output = timeout(
        DEADLINE,
        binary()
            .env("OPENWEATHER_API_KEY", KEY)
            .env("OPENWEATHER_API_KEY_FILE", "/tmp/private-secret-file")
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("set exactly one"));
    assert!(!stderr.contains(KEY));
    assert!(!stderr.contains("private-secret-file"));
}

#[tokio::test]
async fn binary_accepts_explicit_v4() {
    let mut child = binary()
        .env("OPENWEATHER_API_VERSION", "v4")
        .env("OPENWEATHER_LOG", "debug")
        .env("OPENWEATHER_CACHE", "off")
        .env("OPENWEATHER_API_KEY", KEY)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut peer = Peer::new(child.stdout.take().unwrap(), child.stdin.take().unwrap());
    peer.initialize().await;
    drop(peer);
    let output = timeout(DEADLINE, child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Starting OpenWeather MCP stdio server"));
    assert!(stderr.contains("cache_enabled=false"));
    assert!(!stderr.contains(KEY));
}

#[tokio::test]
async fn binary_rejects_unsupported_api_version_before_loading_credentials() {
    let output = timeout(
        DEADLINE,
        binary().env("OPENWEATHER_API_VERSION", "v3").output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unsupported OPENWEATHER_API_VERSION; supported value: v4")
    );
}

#[tokio::test]
async fn stdio_protocol_returns_safe_timeline_pages_and_upstream_errors() {
    let logs = tempfile::NamedTempFile::new().unwrap();
    let log_writer = logs.reopen().unwrap();
    let targets = Targets::new()
        .with_target("stdio", tracing::Level::DEBUG)
        .with_target("openweather_onecall_sdk", tracing::Level::DEBUG);
    let layer = tracing_subscriber::fmt::layer()
        .without_time()
        .with_ansi(false)
        .with_writer(move || log_writer.try_clone().unwrap())
        .with_filter(targets);
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let mock = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = mock.local_addr().unwrap();
    let client = ClientV4::with_config(
        KEY,
        ClientV4Config {
            base_url: format!("http://{addr}/data/4.0/onecall/"),
            timeout: DEADLINE,
        },
    )
    .unwrap();
    let server = server::WeatherServer::new(client, true);
    let mock_task = tokio::spawn(async move {
        let first = json!({
            "lat": 51.5, "lon": -0.1, "timezone": "Europe/London", "timezone_offset": 3600,
            "data": [{"dt": 1777452300, "temp": 12.5}],
            "next": format!("http://{addr}/data/4.0/onecall/timeline/1h?start=1777455900&cnt=20&appid={KEY}"),
        });
        let second = json!({
            "lat": 51.5, "lon": -0.1, "timezone": "Europe/London", "timezone_offset": 3600,
            "data": [{"dt": 1777455900, "temp": 13.1}],
        });
        let invalid = json!({
            "lat": 51.5, "lon": -0.1, "timezone": "Europe/London", "timezone_offset": 3600,
            "data": [{"dt": 1777455900, "pressure": KEY}],
        });
        let invalid_cursor = json!({
            "lat": 51.5, "lon": -0.1, "timezone": "Europe/London", "timezone_offset": 3600,
            "data": [],
            "next": format!("http://{addr}/data/4.0/onecall/timeline/1day?start={KEY}&appid={KEY}"),
        });
        let response_cases = [
            ("200 OK", first.to_string()),
            ("200 OK", second.to_string()),
            (
                "429 Too Many Requests",
                format!("untrusted body containing {KEY}"),
            ),
            ("200 OK", invalid.to_string()),
            ("200 OK", invalid_cursor.to_string()),
            ("429 Too Many Requests", String::new()),
        ];
        let mut requests = Vec::new();
        for (status, body) in response_cases {
            let (conn, _) = mock.accept().await.unwrap();
            let mut conn = BufReader::new(conn);
            let mut request_line = String::new();
            conn.read_line(&mut request_line).await.unwrap();
            requests.push(request_line);
            loop {
                let mut header = String::new();
                assert_ne!(conn.read_line(&mut header).await.unwrap(), 0);
                if header == "\r\n" {
                    break;
                }
            }
            conn.get_mut()
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
        requests
    });
    let (server_stream, client_stream) = duplex(16384);
    let service = tokio::spawn(async move {
        server
            .serve(split(server_stream))
            .await
            .unwrap()
            .waiting()
            .await
    });
    let (reader, writer) = split(client_stream);
    let mut peer = Peer::new(reader, writer);
    peer.initialize().await;

    let args = json!({"lat": 51.5, "lon": -0.1, "units": "metric", "lang": "ru"});
    let first = peer.call("get_hourly_timeline", args.clone()).await;
    assert_eq!(first["isError"], false);
    assert_eq!(first["structuredContent"]["data"][0]["temp"], 12.5);
    let cursor = &first["structuredContent"]["next"];
    assert_eq!(*cursor, json!({"start": 1777455900, "cnt": 20}));
    assert!(!first.to_string().contains(KEY));
    assert!(!first.to_string().contains("appid"));
    let repeat = peer.call("get_hourly_timeline", args.clone()).await;
    assert_eq!(repeat, first);

    let mut next_args = args.clone();
    next_args["start"] = cursor["start"].clone();
    next_args["cnt"] = cursor["cnt"].clone();
    let second = peer.call("get_hourly_timeline", next_args).await;
    assert_eq!(second["isError"], false);
    assert_eq!(second["structuredContent"]["data"][0]["dt"], 1777455900);
    assert!(second["structuredContent"].get("next").is_none());

    let failed = peer.call("get_current_weather", args.clone()).await;
    assert_eq!(failed["isError"], true);
    assert_eq!(failed["structuredContent"]["code"], "rate_limited");
    assert!(!failed.to_string().contains(KEY));

    for message in [
        "OpenWeather returned an invalid response",
        "OpenWeather returned invalid pagination data",
    ] {
        let failed = peer.call("get_daily_timeline", args.clone()).await;
        assert_eq!(failed["isError"], true);
        assert_eq!(failed["structuredContent"]["code"], "invalid_response");
        assert_eq!(failed["structuredContent"]["message"], message);
        assert!(!failed.to_string().contains(KEY));
    }
    let retry = peer.call("get_current_weather", args).await;
    assert_eq!(retry, failed);

    let requests = timeout(DEADLINE, mock_task).await.unwrap().unwrap();
    assert!(requests[0].starts_with("GET /data/4.0/onecall/timeline/1h?"));
    assert!(!requests[0].contains("start="));
    assert!(requests[1].contains("start=1777455900&cnt=20"));
    assert!(requests[2].starts_with("GET /data/4.0/onecall/current?"));
    assert_eq!(requests[5], requests[2]);
    for request in requests {
        assert!(request.contains("units=metric") && request.contains("lang=ru"));
        assert!(request.contains(&format!("appid={KEY}")));
    }

    drop(peer);
    timeout(DEADLINE, service).await.unwrap().unwrap().unwrap();
    let logs = std::fs::read_to_string(logs.path()).unwrap();
    for detail in [
        "status=429",
        "Could not decode OpenWeather response",
        "status=200",
        "timeline/1day",
        "$.data[0]",
        "category=Data",
        "Invalid OpenWeather pagination",
        "invalid start",
        "OpenWeather cache hit",
        "OpenWeather cache miss",
    ] {
        assert!(logs.contains(detail), "missing {detail}: {logs}");
    }
    assert!(!logs.contains(KEY));
    assert!(!logs.contains("appid"));
    assert!(!logs.contains("untrusted body"));
    assert!(!logs.contains("http://"));
}
