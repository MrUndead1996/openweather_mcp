mod error;
mod model;

pub use error::Error;
pub use model::{
    AlertDescription, Conditions, DailyForecast, DailyTemperature, DayParts, HourlyPrecipitation,
    MinuteForecast, Observation, PageCursor, TimelinePage, TimelineQuery, Units, WeatherAlert,
    WeatherCondition, WeatherData, WeatherQuery,
};

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::{Url, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_path_to_error::{Path, Segment};
use tracing::{debug, warn};

const OPENWEATHER_HOST: &str = "api.openweathermap.org";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientV4Config {
    pub base_url: String,
    pub timeout: Duration,
}

impl Default for ClientV4Config {
    fn default() -> Self {
        Self {
            base_url: "https://api.openweathermap.org/data/4.0/onecall/".into(),
            timeout: Duration::from_secs(15),
        }
    }
}

#[derive(Clone)]
pub struct ClientV4 {
    http: reqwest::Client,
    base: Url,
    api_key: Arc<str>,
}

impl ClientV4 {
    pub fn new(api_key: impl AsRef<str>) -> Result<Self, Error> {
        Self::with_config(api_key, ClientV4Config::default())
    }

    pub fn with_config(api_key: impl AsRef<str>, config: ClientV4Config) -> Result<Self, Error> {
        let api_key = api_key.as_ref().trim();
        if api_key.is_empty() {
            return Err(Error::MissingApiKey);
        }
        let base = parse_base_url(&config.base_url)?;
        // Keep TLS provider selection local to this client, not process-wide.
        let tls = rustls::ClientConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::ClientSetup)?
        .with_root_certificates(rustls::RootCertStore::from_iter(
            webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
        ))
        .with_no_client_auth();
        let http = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .timeout(config.timeout)
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| Error::ClientSetup)?;
        Ok(Self {
            http,
            base,
            api_key: Arc::from(api_key),
        })
    }

    pub async fn current_weather(
        &self,
        query: &WeatherQuery,
    ) -> Result<WeatherData<Observation>, Error> {
        validate_location(query)?;
        self.get("current", self.endpoint("current"), query).await
    }

    pub async fn minute_forecast(
        &self,
        query: &WeatherQuery,
    ) -> Result<WeatherData<MinuteForecast>, Error> {
        validate_location(query)?;
        self.get("timeline/1min", self.endpoint("timeline/1min"), query)
            .await
    }

    pub async fn fifteen_minute_forecast(
        &self,
        query: &TimelineQuery,
    ) -> Result<TimelinePage<Observation>, Error> {
        self.timeline("timeline/15min", query).await
    }

    pub async fn hourly_timeline(
        &self,
        query: &TimelineQuery,
    ) -> Result<TimelinePage<Observation>, Error> {
        self.timeline("timeline/1h", query).await
    }

    pub async fn daily_timeline(
        &self,
        query: &TimelineQuery,
    ) -> Result<TimelinePage<DailyForecast>, Error> {
        self.timeline("timeline/1day", query).await
    }

    pub async fn weather_alert(&self, alert_id: &str) -> Result<WeatherAlert, Error> {
        if alert_id.trim().is_empty() || matches!(alert_id, "." | "..") {
            return Err(Error::InvalidAlertId);
        }
        let mut url = self.endpoint("alert/");
        url.path_segments_mut()
            .expect("HTTP URL")
            .pop_if_empty()
            .push(alert_id);
        let query: [(&str, &str); 0] = [];
        self.get("alert", url, &query).await
    }

    fn endpoint(&self, path: &str) -> Url {
        self.base.join(path).expect("static One Call endpoint path")
    }

    async fn timeline<T: DeserializeOwned>(
        &self,
        path: &'static str,
        query: &TimelineQuery,
    ) -> Result<TimelinePage<T>, Error> {
        validate_location(&query.weather)?;
        if query.cnt == Some(0) {
            return Err(Error::InvalidPageSize);
        }
        let url = self.endpoint(path);
        let raw: RawTimeline<T> = self.get(path, url.clone(), query).await?;
        let decode_cursor = |link: Option<String>, direction| {
            link.as_deref()
                .map(|link| {
                    parse_cursor(link, &url).map_err(|reason| {
                        warn!(
                            endpoint = path,
                            direction, reason, "Invalid OpenWeather pagination"
                        );
                        Error::InvalidPagination
                    })
                })
                .transpose()
        };
        Ok(TimelinePage {
            weather: WeatherData {
                lat: raw.lat,
                lon: raw.lon,
                timezone: raw.timezone,
                timezone_offset: raw.timezone_offset,
                data: raw.data,
            },
            next: decode_cursor(raw.next, "next")?,
            prev: decode_cursor(raw.prev, "prev")?,
        })
    }

    async fn get<T: DeserializeOwned>(
        &self,
        endpoint: &'static str,
        url: Url,
        query: &(impl Serialize + ?Sized),
    ) -> Result<T, Error> {
        let started = Instant::now();
        let elapsed_ms = || started.elapsed().as_millis() as u64;
        debug!(endpoint, "Sending OpenWeather request");
        let response = self
            .http
            .get(url)
            .query(query)
            .query(&[("appid", self.api_key.as_ref())])
            .send()
            .await
            .map_err(|error| {
                let error = map_http_error(error);
                warn!(endpoint, elapsed_ms = elapsed_ms(), error = ?error, "OpenWeather request failed");
                error
            })?;
        let status = response.status();
        if !status.is_success() {
            let error = Error::from_status(status);
            warn!(endpoint, status = status.as_u16(), elapsed_ms = elapsed_ms(), error = ?error, "OpenWeather rejected the request");
            return Err(error);
        }
        let status = status.as_u16();
        let body = response.bytes().await.map_err(|error| {
            let error = map_http_error(error);
            warn!(endpoint, status, elapsed_ms = elapsed_ms(), error = ?error, "Could not read OpenWeather response body");
            error
        })?;
        let body_bytes = body.len();
        let mut json = serde_json::Deserializer::from_slice(&body);
        let report = |error: &serde_json::Error, path: &str| {
            // serde's error text may include arbitrary values from the response.
            warn!(endpoint, status, body_bytes, elapsed_ms = elapsed_ms(), path,
                category = ?error.classify(), line = error.line(), column = error.column(),
                "Could not decode OpenWeather response");
            Error::InvalidResponse
        };
        let decoded = serde_path_to_error::deserialize(&mut json)
            .map_err(|error| report(error.inner(), &safe_json_path(error.path())))?;
        json.end().map_err(|error| report(&error, "$"))?;
        debug!(
            endpoint,
            status,
            body_bytes,
            elapsed_ms = elapsed_ms(),
            "Decoded OpenWeather response"
        );
        Ok(decoded)
    }
}

fn parse_base_url(value: &str) -> Result<Url, Error> {
    let mut url = Url::parse(value).map_err(|_| Error::InvalidBaseUrl)?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidBaseUrl);
    }
    if !url.path().ends_with('/') {
        url.path_segments_mut()
            .map_err(|_| Error::InvalidBaseUrl)?
            .push("");
    }
    Ok(url)
}

#[derive(Deserialize)]
struct RawTimeline<T> {
    lat: f64,
    lon: f64,
    timezone: String,
    timezone_offset: i32,
    data: Vec<T>,
    next: Option<String>,
    prev: Option<String>,
}

fn validate_location(query: &WeatherQuery) -> Result<(), Error> {
    if !(-90.0..=90.0).contains(&query.lat) || !(-180.0..=180.0).contains(&query.lon) {
        return Err(Error::InvalidCoordinates);
    }
    Ok(())
}

fn parse_cursor(link: &str, endpoint: &Url) -> Result<PageCursor, &'static str> {
    let url = Url::parse(link).map_err(|_| "invalid URL")?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL contains credentials");
    }
    // Production sometimes advertises its HTTP origin. We extract numbers only;
    // requests always use the configured endpoint, never the advertised URL.
    let is_production_http = endpoint.scheme() == "https"
        && endpoint.host_str() == Some(OPENWEATHER_HOST)
        && endpoint.port().is_none()
        && url.scheme() == "http"
        && url.host_str() == Some(OPENWEATHER_HOST)
        && url.port().is_none();
    if url.origin() != endpoint.origin() && !is_production_http {
        return Err("URL origin does not match the endpoint");
    }
    if url.path() != endpoint.path() {
        return Err("URL path does not match the endpoint");
    }

    // Never keep the upstream URL or its appid, even if it differs from our key.
    let mut start = None;
    let mut cnt = None;
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "start" if start.is_none() => start = Some(value.parse().map_err(|_| "invalid start")?),
            "cnt" if cnt.is_none() => {
                let count = value.parse().map_err(|_| "invalid cnt")?;
                if count == 0 {
                    return Err("cnt must be positive");
                }
                cnt = Some(count);
            }
            "start" | "cnt" => return Err("duplicate cursor parameter"),
            _ => {}
        }
    }
    Ok(PageCursor {
        start: start.ok_or("missing start")?,
        cnt,
    })
}

fn safe_json_path(path: &Path) -> String {
    use std::fmt::Write as _;

    let mut safe = String::from("$");
    for segment in path {
        match segment {
            Segment::Seq { index } => {
                let _ = write!(safe, "[{index}]");
            }
            Segment::Map { key }
                if matches!(
                    key.as_str(),
                    "lat"
                        | "lon"
                        | "timezone"
                        | "timezone_offset"
                        | "data"
                        | "prev"
                        | "next"
                        | "dt"
                        | "sunrise"
                        | "sunset"
                        | "moonrise"
                        | "moonset"
                        | "moon_phase"
                        | "temp"
                        | "feels_like"
                        | "pressure"
                        | "humidity"
                        | "dew_point"
                        | "uvi"
                        | "clouds"
                        | "wind_speed"
                        | "wind_deg"
                        | "wind_gust"
                        | "pop"
                        | "weather"
                        | "alerts"
                        | "visibility"
                        | "rain"
                        | "snow"
                        | "precipitation"
                        | "min"
                        | "max"
                        | "morn"
                        | "day"
                        | "eve"
                        | "night"
                        | "id"
                        | "main"
                        | "description"
                        | "icon"
                        | "sender_name"
                        | "event"
                        | "start"
                        | "end"
                        | "tags"
                        | "language"
                        | "1h"
                ) =>
            {
                safe.push('.');
                safe.push_str(key);
            }
            _ => safe.push_str(".?"),
        }
    }
    safe
}

fn map_http_error(error: reqwest::Error) -> Error {
    // The transport error can contain the request URL including appid.
    if error.is_timeout() {
        Error::Timeout
    } else if error.is_decode() {
        Error::InvalidResponse
    } else if error.is_builder() {
        Error::InvalidRequest
    } else {
        Error::Transport
    }
}

#[cfg(test)]
mod tests;
