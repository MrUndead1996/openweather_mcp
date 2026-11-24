use std::collections::BTreeMap;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

use super::*;

const KEY: &str = "test-key+a&b";

struct Mock {
    base: Url,
    request: JoinHandle<String>,
}

impl Mock {
    async fn new(status: u16, body: &str, delay: Duration, redirect: bool) -> Self {
        Self::with_body(status, |_| body.to_owned(), delay, redirect).await
    }

    async fn with_body(
        status: u16,
        body: impl FnOnce(&Url) -> String,
        delay: Duration,
        redirect: bool,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!(
            "http://{}/data/4.0/onecall/",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let location = if redirect {
            format!("Location: {}current\r\n", base)
        } else {
            String::new()
        };
        let body = body(&base);
        let response = format!(
            "HTTP/1.1 {status} Mock\r\n{location}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let request = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut first_line = String::new();
            stream.read_line(&mut first_line).await.unwrap();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            sleep(delay).await;
            let _ = stream.get_mut().write_all(response.as_bytes()).await;
            first_line
        });
        Self { base, request }
    }

    async fn ok(body: Value) -> Self {
        Self::new(200, &body.to_string(), Duration::ZERO, false).await
    }

    fn client(&self) -> ClientV4 {
        ClientV4::with_config(
            KEY,
            ClientV4Config {
                base_url: self.base.as_str().trim_end_matches('/').into(),
                timeout: Duration::from_secs(2),
            },
        )
        .unwrap()
    }

    async fn assert_request(self, path: &str, params: &[(&str, &str)]) {
        let line = timeout(Duration::from_secs(3), self.request)
            .await
            .unwrap()
            .unwrap();
        let mut tokens = line.split_whitespace();
        assert_eq!(tokens.next(), Some("GET"));
        let url = self.base.join(tokens.next().unwrap()).unwrap();
        assert_eq!(url.path(), format!("/data/4.0/onecall/{path}"));
        let pairs: Vec<_> = url.query_pairs().collect();
        assert_eq!(pairs.len(), params.len() + 1);
        let actual: BTreeMap<_, _> = pairs
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref()))
            .collect();
        let expected = BTreeMap::from_iter(
            params
                .iter()
                .copied()
                .chain(std::iter::once(("appid", KEY))),
        );
        assert_eq!(actual, expected);
    }
}

fn weather(record: Value) -> Value {
    json!({
        "lat": 51.5,
        "lon": -0.1,
        "timezone": "Europe/London",
        "timezone_offset": 3600,
        "data": [record],
    })
}

fn query() -> WeatherQuery {
    WeatherQuery::new(51.5, -0.1)
}

const LOCATION: &[(&str, &str)] = &[("lat", "51.5"), ("lon", "-0.1")];

#[tokio::test]
async fn current_weather_preserves_documented_field_names() {
    let body = weather(json!({
        "dt": 1777449371,
        "temp": 13.27,
        "humidity": 58,
        "rain": {"1h": 0.5},
        "alerts": ["alert-id"],
        "weather": [{"id": 500, "main": "Rain", "description": "light rain", "icon": "10d"}]
    }));
    let mock = Mock::ok(body.clone()).await;
    let mut query = query();
    query.units = Some(Units::Metric);
    query.lang = Some("pt_br".into());
    let page = mock.client().current_weather(&query).await.unwrap();
    assert_eq!(page.data[0].rain.as_ref().unwrap().hourly, 0.5);
    assert_eq!(page.data[0].sunrise, None);
    assert_eq!(serde_json::to_value(page).unwrap(), body);
    mock.assert_request(
        "current",
        &[
            ("lat", "51.5"),
            ("lon", "-0.1"),
            ("units", "metric"),
            ("lang", "pt_br"),
        ],
    )
    .await;
}

#[tokio::test]
async fn minute_forecast_uses_documented_path_and_leaves_defaults_to_api() {
    let mock = Mock::ok(weather(json!({"dt": 1777451940, "precipitation": 0.2}))).await;
    let page = mock.client().minute_forecast(&query()).await.unwrap();
    assert_eq!(page.data[0].precipitation, 0.2);
    assert_eq!(page.data[0].alerts, None);
    mock.assert_request("timeline/1min", LOCATION).await;
}

#[tokio::test]
async fn fifteen_minute_forecast_exposes_safe_cursor_and_rebuilds_request() {
    let path = "timeline/15min";
    let mock = Mock::with_body(
        200,
        |base| {
            let mut body = weather(json!({"dt": 1777452300, "temp": 287.95}));
            body["next"] = json!(format!(
                "{base}{path}?start=1777497300&cnt=15&appid=secret-from-upstream"
            ));
            body["prev"] = json!(format!("{base}{path}?start=1777451400&appid={KEY}"));
            body.to_string()
        },
        Duration::ZERO,
        false,
    )
    .await;
    let query = TimelineQuery::from(query());
    let first = mock.client().fifteen_minute_forecast(&query).await.unwrap();
    let wire = serde_json::to_value(&first).unwrap();
    assert_eq!(wire["next"], json!({"start": 1777497300, "cnt": 15}));
    assert_eq!(wire["prev"], json!({"start": 1777451400}));
    assert!(!wire.to_string().contains("secret-from-upstream"));
    assert!(!wire.to_string().contains(KEY));
    mock.assert_request(path, LOCATION).await;

    let mock = Mock::ok(weather(json!({"dt": 1777497300}))).await;
    let page = mock
        .client()
        .fifteen_minute_forecast(&query.with_cursor(first.next.unwrap()))
        .await
        .unwrap();
    assert_eq!(page.weather.data[0].dt, 1777497300);
    mock.assert_request(
        path,
        &[
            ("lat", "51.5"),
            ("lon", "-0.1"),
            ("start", "1777497300"),
            ("cnt", "15"),
        ],
    )
    .await;
}

#[tokio::test]
async fn hourly_timeline_accepts_historical_start() {
    let mut query = TimelineQuery::from(query());
    query.start = Some(1_000_000_000);
    let mock = Mock::ok(weather(json!({"dt": 1000000000, "pop": 0.25}))).await;
    let result = mock.client().hourly_timeline(&query).await.unwrap();
    assert_eq!(result.weather.data[0].conditions.pop, Some(0.25));
    assert_eq!(result.next, None);
    mock.assert_request(
        "timeline/1h",
        &[("lat", "51.5"), ("lon", "-0.1"), ("start", "1000000000")],
    )
    .await;
}

#[tokio::test]
async fn daily_timeline_decodes_grouped_temperatures() {
    let body = weather(json!({
        "dt": 1777460400,
        "temp": {"min": 280.7, "max": 290.05, "day": 288.16, "night": 285.7, "eve": 289.38, "morn": 282.78},
        "feels_like": {"day": 286.89, "night": 284.58, "eve": 288.31, "morn": 279.89}
    }));
    let mock = Mock::ok(body.clone()).await;
    let page = mock.client().daily_timeline(&query().into()).await.unwrap();
    assert_eq!(page.weather.data[0].temp.as_ref().unwrap().min, 280.7);
    assert_eq!(serde_json::to_value(page).unwrap(), body);
    mock.assert_request("timeline/1day", LOCATION).await;
}

#[tokio::test]
async fn alert_carries_localized_descriptions_and_encodes_id_as_one_segment() {
    let id = "2.49.0:FR615/alert?x#y";
    let body = json!({
        "id": id, "sender_name": "METEO-FRANCE", "event": "",
        "start": 1787817940, "end": 1787868000,
        "description": [{"language": "fr-FR", "description": "Vent fort"}],
        "tags": ["Thunderstorm"]
    });
    let mock = Mock::ok(body.clone()).await;
    let alert = mock.client().weather_alert(id).await.unwrap();
    assert_eq!(alert.description[0].language, "fr-FR");
    assert_eq!(serde_json::to_value(alert).unwrap(), body);
    mock.assert_request("alert/2.49.0:FR615%2Falert%3Fx%23y", &[])
        .await;
}

#[tokio::test]
async fn rejects_invalid_input_before_using_the_network() {
    assert!(matches!(ClientV4::new(" \n "), Err(Error::MissingApiKey)));
    let client = ClientV4::new(KEY).unwrap();
    assert_eq!(
        client
            .current_weather(&WeatherQuery::new(f64::NAN, 0.0))
            .await,
        Err(Error::InvalidCoordinates)
    );
    assert_eq!(client.weather_alert("..").await, Err(Error::InvalidAlertId));
    let mut query = TimelineQuery::from(query());
    query.cnt = Some(0);
    assert_eq!(
        client.daily_timeline(&query).await,
        Err(Error::InvalidPageSize)
    );
}

#[tokio::test]
async fn http_failures_do_not_reveal_upstream_body_or_request() {
    for (status, expected) in [
        (400, Error::BadRequest),
        (401, Error::Unauthorized),
        (403, Error::Unauthorized),
        (404, Error::NotFound),
        (429, Error::RateLimited),
        (503, Error::Upstream(503)),
        (302, Error::UnexpectedStatus(302)),
        (200, Error::InvalidResponse),
    ] {
        let mock = Mock::new(status, KEY, Duration::ZERO, status == 302).await;
        let error = mock.client().current_weather(&query()).await.unwrap_err();
        assert_eq!(error, expected);
        assert!(!format!("{error:?} {error}").contains(KEY));
        mock.assert_request("current", LOCATION).await;
    }
}

#[tokio::test]
async fn classifies_timeout() {
    let mock = Mock::new(200, "{}", Duration::from_millis(500), false).await;
    let client = ClientV4::with_config(
        KEY,
        ClientV4Config {
            base_url: mock.base.to_string(),
            timeout: Duration::from_millis(100),
        },
    )
    .unwrap();
    assert_eq!(client.current_weather(&query()).await, Err(Error::Timeout));
    mock.assert_request("current", LOCATION).await;
}

#[test]
fn rejects_wrong_origin_wrong_path_and_invalid_cursor_values() {
    let endpoint = parse_base_url(&ClientV4Config::default().base_url)
        .unwrap()
        .join("timeline/1h")
        .unwrap();
    for link in [
        "https://weather.example/data/4.0/onecall/timeline/1h?start=1",
        "https://api.openweathermap.org/data/4.0/onecall/timeline/1day?start=1",
        "https://api.openweathermap.org/data/4.0/onecall/timeline/1h?appid=secret",
        "https://api.openweathermap.org/data/4.0/onecall/timeline/1h?start=1&start=2",
        "https://api.openweathermap.org/data/4.0/onecall/timeline/1h?start=1&cnt=0",
    ] {
        assert_eq!(cursor(link, &endpoint), Err(Error::InvalidPagination));
    }
}

#[test]
fn rejects_base_urls_that_cannot_be_used_as_clean_http_endpoints() {
    for base_url in [
        "relative/path",
        "ftp://example.test/onecall/",
        "https://user:password@example.test/onecall/",
        "https://example.test/onecall/?appid=other",
        "https://example.test/onecall/#fragment",
    ] {
        assert!(matches!(
            ClientV4::with_config(
                KEY,
                ClientV4Config {
                    base_url: base_url.into(),
                    ..ClientV4Config::default()
                }
            ),
            Err(Error::InvalidBaseUrl)
        ));
    }
}
