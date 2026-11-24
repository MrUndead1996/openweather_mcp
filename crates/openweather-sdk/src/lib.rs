mod error;
mod model;

pub use error::Error;
pub use model::{
    AlertDescription, Conditions, DailyForecast, DailyTemperature, DayParts, HourlyPrecipitation,
    MinuteForecast, Observation, PageCursor, TimelinePage, TimelineQuery, Units, WeatherAlert,
    WeatherCondition, WeatherData, WeatherQuery,
};

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Url, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

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
        self.get(self.endpoint("current"), query).await
    }

    pub async fn minute_forecast(
        &self,
        query: &WeatherQuery,
    ) -> Result<WeatherData<MinuteForecast>, Error> {
        validate_location(query)?;
        self.get(self.endpoint("timeline/1min"), query).await
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
        self.get(url, &query).await
    }

    fn endpoint(&self, path: &str) -> Url {
        self.base.join(path).expect("static One Call endpoint path")
    }

    async fn timeline<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &TimelineQuery,
    ) -> Result<TimelinePage<T>, Error> {
        validate_location(&query.weather)?;
        if query.cnt == Some(0) {
            return Err(Error::InvalidPageSize);
        }
        let url = self.endpoint(path);
        let raw: RawTimeline<T> = self.get(url.clone(), query).await?;
        Ok(TimelinePage {
            weather: raw.weather,
            next: raw
                .next
                .as_deref()
                .map(|link| cursor(link, &url))
                .transpose()?,
            prev: raw
                .prev
                .as_deref()
                .map(|link| cursor(link, &url))
                .transpose()?,
        })
    }

    async fn get<T: DeserializeOwned>(
        &self,
        url: Url,
        query: &(impl Serialize + ?Sized),
    ) -> Result<T, Error> {
        let response = self
            .http
            .get(url)
            .query(query)
            .query(&[("appid", self.api_key.as_ref())])
            .send()
            .await
            .map_err(map_http_error)?;
        if !response.status().is_success() {
            return Err(Error::from_status(response.status()));
        }
        response.json().await.map_err(map_http_error)
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
    #[serde(flatten)]
    weather: WeatherData<T>,
    next: Option<String>,
    prev: Option<String>,
}

fn validate_location(query: &WeatherQuery) -> Result<(), Error> {
    if !(-90.0..=90.0).contains(&query.lat) || !(-180.0..=180.0).contains(&query.lon) {
        return Err(Error::InvalidCoordinates);
    }
    Ok(())
}

fn cursor(link: &str, endpoint: &Url) -> Result<PageCursor, Error> {
    let url = Url::parse(link).map_err(|_| Error::InvalidPagination)?;
    if url.origin() != endpoint.origin()
        || url.path() != endpoint.path()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::InvalidPagination);
    }

    // Never keep the upstream URL or its appid, even if it differs from our key.
    let mut start = None;
    let mut cnt = None;
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "start" if start.is_none() => {
                start = Some(value.parse().map_err(|_| Error::InvalidPagination)?)
            }
            "cnt" if cnt.is_none() => {
                let count = value.parse().map_err(|_| Error::InvalidPagination)?;
                if count == 0 {
                    return Err(Error::InvalidPagination);
                }
                cnt = Some(count);
            }
            "start" | "cnt" => return Err(Error::InvalidPagination),
            _ => {}
        }
    }
    Ok(PageCursor {
        start: start.ok_or(Error::InvalidPagination)?,
        cnt,
    })
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
