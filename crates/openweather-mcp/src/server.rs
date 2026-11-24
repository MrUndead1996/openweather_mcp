use openweather_onecall_sdk::{ClientV4, Error, TimelineQuery, Units, WeatherQuery};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cache::{Tool, WeatherCache};

pub struct WeatherServer {
    client: ClientV4,
    cache: Option<WeatherCache>,
    tool_router: ToolRouter<Self>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for WeatherServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "OpenWeather One Call 4.0. Weather tools use coordinates, not city names. \
                 Timeline calls return one page. To follow next or prev, resend the same \
                 location, units, and language with the returned start and cnt (if present).",
            )
    }
}

#[tool_router]
impl WeatherServer {
    pub fn new(client: ClientV4, cache_enabled: bool) -> Self {
        Self {
            client,
            cache: cache_enabled.then(WeatherCache::new),
            tool_router: Self::tool_router(),
        }
    }

    async fn cached<T: Serialize>(
        &self,
        tool: Tool,
        args: &impl Serialize,
        fetch: impl Future<Output = Result<T, Error>>,
    ) -> CallToolResult {
        match &self.cache {
            Some(cache) => {
                cache
                    .get_or_fetch(tool, args, async { result(fetch.await) })
                    .await
            }
            None => result(fetch.await),
        }
    }

    #[tool(description = "Get the current weather and active alert IDs for a location.")]
    async fn get_current_weather(&self, Parameters(args): Parameters<Location>) -> CallToolResult {
        let query = args.into();
        self.cached(Tool::Current, &query, self.client.current_weather(&query))
            .await
    }

    #[tool(description = "Get the minute-by-minute precipitation forecast for the next hour.")]
    async fn get_minute_forecast(&self, Parameters(args): Parameters<Location>) -> CallToolResult {
        let query = args.into();
        self.cached(Tool::Minute, &query, self.client.minute_forecast(&query))
            .await
    }

    #[tool(description = "Get one page of the 15-minute forecast for the next 48 hours.")]
    async fn get_15_minute_forecast(
        &self,
        Parameters(args): Parameters<Timeline>,
    ) -> CallToolResult {
        let query = args.into();
        self.cached(
            Tool::FifteenMinute,
            &query,
            self.client.fifteen_minute_forecast(&query),
        )
        .await
    }

    #[tool(
        description = "Get one page of hourly historical weather or the next 48 hours of forecasts."
    )]
    async fn get_hourly_timeline(&self, Parameters(args): Parameters<Timeline>) -> CallToolResult {
        let query = args.into();
        self.cached(Tool::Hourly, &query, self.client.hourly_timeline(&query))
            .await
    }

    #[tool(
        description = "Get one page of daily historical weather or forecasts up to 1.5 years ahead."
    )]
    async fn get_daily_timeline(&self, Parameters(args): Parameters<Timeline>) -> CallToolResult {
        let query = args.into();
        self.cached(Tool::Daily, &query, self.client.daily_timeline(&query))
            .await
    }

    #[tool(description = "Get detailed information for an alert ID returned by a weather tool.")]
    async fn get_weather_alert(&self, Parameters(args): Parameters<Alert>) -> CallToolResult {
        self.cached(
            Tool::Alert,
            &args.alert_id,
            self.client.weather_alert(&args.alert_id),
        )
        .await
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Location {
    #[schemars(description = "Latitude in decimal degrees, from -90 to 90.")]
    #[schemars(range(min = -90, max = 90))]
    lat: f64,
    #[schemars(description = "Longitude in decimal degrees, from -180 to 180.")]
    #[schemars(range(min = -180, max = 180))]
    lon: f64,
    #[schemars(description = "Weather units. OpenWeather defaults to standard (Kelvin).")]
    units: Option<RequestedUnits>,
    #[schemars(description = "OpenWeather language code for localized weather descriptions.")]
    lang: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Timeline {
    #[schemars(description = "Latitude in decimal degrees, from -90 to 90.")]
    #[schemars(range(min = -90, max = 90))]
    lat: f64,
    #[schemars(description = "Longitude in decimal degrees, from -180 to 180.")]
    #[schemars(range(min = -180, max = 180))]
    lon: f64,
    #[schemars(description = "Weather units. OpenWeather defaults to standard (Kelvin).")]
    units: Option<RequestedUnits>,
    #[schemars(description = "OpenWeather language code for localized weather descriptions.")]
    lang: Option<String>,
    #[schemars(
        description = "Timeline start in Unix seconds (UTC). For pagination, use next.start or prev.start."
    )]
    start: Option<i64>,
    #[schemars(
        description = "Optional record count. When paginating, copy cnt from the cursor if present."
    )]
    #[schemars(range(min = 1))]
    cnt: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Alert {
    #[schemars(description = "OpenWeather alert ID, copied from an alerts array.")]
    alert_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum RequestedUnits {
    Standard,
    Metric,
    Imperial,
}

impl From<RequestedUnits> for Units {
    fn from(units: RequestedUnits) -> Self {
        match units {
            RequestedUnits::Standard => Self::Standard,
            RequestedUnits::Metric => Self::Metric,
            RequestedUnits::Imperial => Self::Imperial,
        }
    }
}

impl From<Location> for WeatherQuery {
    fn from(location: Location) -> Self {
        Self {
            lat: location.lat,
            lon: location.lon,
            units: location.units.map(Units::from),
            lang: location.lang,
        }
    }
}

impl From<Timeline> for TimelineQuery {
    fn from(timeline: Timeline) -> Self {
        Self {
            weather: Location {
                lat: timeline.lat,
                lon: timeline.lon,
                units: timeline.units,
                lang: timeline.lang,
            }
            .into(),
            start: timeline.start,
            cnt: timeline.cnt,
        }
    }
}

fn result<T: Serialize>(outcome: Result<T, Error>) -> CallToolResult {
    match outcome {
        Ok(value) => match serde_json::to_value(value) {
            Ok(json) => CallToolResult::structured(json),
            Err(_) => CallToolResult::structured_error(json!({
                "code": "internal_error",
                "message": "Could not encode the weather result",
            })),
        },
        Err(error) => {
            let code = match error {
                Error::InvalidCoordinates | Error::InvalidAlertId | Error::InvalidPageSize => {
                    "invalid_input"
                }
                Error::BadRequest | Error::InvalidRequest => "bad_request",
                Error::Unauthorized => "unauthorized",
                Error::NotFound => "not_found",
                Error::RateLimited => "rate_limited",
                Error::Timeout => "timeout",
                Error::Transport => "transport_error",
                Error::Upstream(_) | Error::UnexpectedStatus(_) => "upstream_error",
                Error::InvalidResponse | Error::InvalidPagination => "invalid_response",
                _ => "internal_error",
            };
            tracing::debug!(code, "OpenWeather MCP tool returned an error");
            CallToolResult::structured_error(json!({"code": code, "message": error.to_string()}))
        }
    }
}
use std::future::Future;
