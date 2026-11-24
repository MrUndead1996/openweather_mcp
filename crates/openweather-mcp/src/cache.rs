use std::{
    future::Future,
    time::{Duration, Instant},
};

use moka::{future::Cache, policy::Expiry};
use rmcp::model::CallToolResult;
use serde::Serialize;
use tracing::debug;

const CAPACITY: u64 = 256;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum Tool {
    Current,
    Minute,
    FifteenMinute,
    Hourly,
    Daily,
    Alert,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Self::Current => "get_current_weather",
            Self::Minute => "get_minute_forecast",
            Self::FifteenMinute => "get_15_minute_forecast",
            Self::Hourly => "get_hourly_timeline",
            Self::Daily => "get_daily_timeline",
            Self::Alert => "get_weather_alert",
        }
    }

    fn ttl(self) -> Duration {
        Duration::from_secs(match self {
            Self::Minute => 30,
            Self::Current | Self::Alert => 60,
            Self::FifteenMinute | Self::Hourly => 120,
            Self::Daily => 600,
        })
    }
}

#[derive(Hash, PartialEq, Eq)]
struct Key {
    tool: Tool,
    args: String,
}

struct ToolExpiry;

impl Expiry<Key, CallToolResult> for ToolExpiry {
    fn expire_after_create(
        &self,
        key: &Key,
        _value: &CallToolResult,
        _created_at: Instant,
    ) -> Option<Duration> {
        Some(key.tool.ttl())
    }
}

pub(crate) struct WeatherCache(Cache<Key, CallToolResult>);

impl WeatherCache {
    pub(crate) fn new() -> Self {
        Self(
            Cache::builder()
                .max_capacity(CAPACITY)
                .expire_after(ToolExpiry)
                .build(),
        )
    }

    pub(crate) async fn get_or_fetch(
        &self,
        tool: Tool,
        args: &impl Serialize,
        fetch: impl Future<Output = CallToolResult>,
    ) -> CallToolResult {
        // The serialized query contains tool arguments only, never the upstream appid.
        let Ok(args) = serde_json::to_string(args) else {
            return fetch.await;
        };
        let key = Key { tool, args };
        if let Some(result) = self.0.get(&key).await {
            debug!(tool = tool.name(), "OpenWeather cache hit");
            return result;
        }
        debug!(tool = tool.name(), "OpenWeather cache miss");
        self.0
            .try_get_with(key, async {
                let result = fetch.await;
                if result.is_error == Some(true) {
                    Err(result)
                } else {
                    Ok(result)
                }
            })
            .await
            .unwrap_or_else(|error| (*error).clone())
    }
}
