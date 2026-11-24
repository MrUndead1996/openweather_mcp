use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    Standard,
    Metric,
    Imperial,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherQuery {
    pub lat: f64,
    pub lon: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<Units>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

impl WeatherQuery {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self {
            lat,
            lon,
            units: None,
            lang: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineQuery {
    #[serde(flatten)]
    pub weather: WeatherQuery,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cnt: Option<u32>,
}

impl From<WeatherQuery> for TimelineQuery {
    fn from(weather: WeatherQuery) -> Self {
        Self {
            weather,
            start: None,
            cnt: None,
        }
    }
}

impl TimelineQuery {
    pub fn with_cursor(mut self, cursor: PageCursor) -> Self {
        self.start = Some(cursor.start);
        self.cnt = cursor.cnt;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageCursor {
    pub start: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cnt: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeatherData<T> {
    pub lat: f64,
    pub lon: f64,
    pub timezone: String,
    pub timezone_offset: i32,
    pub data: Vec<T>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelinePage<T> {
    #[serde(flatten)]
    pub weather: WeatherData<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<PageCursor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev: Option<PageCursor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MinuteForecast {
    pub dt: i64,
    pub precipitation: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alerts: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub dt: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunrise: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunset: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feels_like: Option<f64>,
    #[serde(flatten)]
    pub conditions: Conditions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rain: Option<HourlyPrecipitation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snow: Option<HourlyPrecipitation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyForecast {
    pub dt: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunrise: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sunset: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moonrise: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moonset: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moon_phase: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp: Option<DailyTemperature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feels_like: Option<DayParts>,
    #[serde(flatten)]
    pub conditions: Conditions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rain: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snow: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyTemperature {
    pub min: f64,
    pub max: f64,
    #[serde(flatten)]
    pub parts: DayParts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayParts {
    pub morn: f64,
    pub day: f64,
    pub eve: f64,
    pub night: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conditions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pressure: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub humidity: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dew_point: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uvi: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clouds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_deg: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wind_gust: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pop: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weather: Option<Vec<WeatherCondition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alerts: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HourlyPrecipitation {
    #[serde(rename = "1h")]
    pub hourly: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherCondition {
    pub id: u32,
    pub main: String,
    pub description: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeatherAlert {
    pub id: String,
    pub sender_name: String,
    pub event: String,
    pub start: i64,
    pub end: i64,
    pub description: Vec<AlertDescription>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertDescription {
    pub language: String,
    pub description: String,
}
