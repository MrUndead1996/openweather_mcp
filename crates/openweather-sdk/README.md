# OpenWeather One Call SDK

An async, typed Rust client for the [OpenWeather One Call API 4.0](https://openweathermap.org/api/one-call-4.md). Use it directly in a Rust application; the MCP server is not required.

## Installation

Add the SDK and a Tokio runtime to your `Cargo.toml`:

```toml
[dependencies]
openweather-onecall-sdk = "1"
tokio = { version = "1", features = ["macros", "rt"] }
```

Use an [OpenWeather API key](https://home.openweathermap.org/api_keys) from an account with an active [One Call by Call subscription](https://home.openweathermap.org/subscriptions). The standard Free plan alone does not grant access to One Call 4.0.

## Current weather

Set `OPENWEATHER_API_KEY` in your application's environment, then run this example:

```rust
use openweather_onecall_sdk::{ClientV4, Units, WeatherQuery};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = ClientV4::new(std::env::var("OPENWEATHER_API_KEY")?)?;
    let mut query = WeatherQuery::new(51.5, -0.1);
    query.units = Some(Units::Metric);
    query.lang = Some("en".into());

    let weather = client.current_weather(&query).await?;
    if let Some(temp) = weather.data.first().and_then(|record| record.temp) {
        println!("London: {temp:.1}°C");
    }
    Ok(())
}
```

The SDK takes the key as a constructor argument. It does not read environment variables or secret files itself. Queries use latitude and longitude in decimal degrees; the SDK does not provide geocoding. If `units` and `lang` are omitted, OpenWeather supplies the defaults.

## Supported endpoints

| `ClientV4` method | Returns |
| --- | --- |
| `current_weather` | Current conditions and active alert IDs, if any |
| `minute_forecast` | Minute-by-minute precipitation forecast for the next hour |
| `fifteen_minute_forecast` | Forecast at 15-minute intervals for the next 48 hours |
| `hourly_timeline` | Hourly historical weather and forecasts |
| `daily_timeline` | Daily historical weather and forecasts |
| `weather_alert` | Details for an alert ID from a weather response |

## Pagination

The 15-minute, hourly, and daily methods fetch one page per call. Use a `next` or `prev` cursor with the same query to continue. Cursors contain only `start` (Unix seconds, UTC) and an optional `cnt`, not the upstream URL or API key.

```rust
use openweather_onecall_sdk::{ClientV4, Error, TimelineQuery, WeatherQuery};

async fn print_hourly_timestamps(client: &ClientV4) -> Result<(), Error> {
    let query = TimelineQuery::from(WeatherQuery::new(51.5, -0.1));
    let first = client.hourly_timeline(&query).await?;
    for record in &first.weather.data {
        println!("{}", record.dt);
    }

    if let Some(cursor) = first.next {
        let second = client.hourly_timeline(&query.with_cursor(cursor)).await?;
        for record in &second.weather.data {
            println!("{}", record.dt);
        }
    }
    Ok(())
}
```

Each page counts as a separate OpenWeather API call. The SDK does not fetch additional pages or retry requests automatically.

## Configuration and errors

`ClientV4::new` uses the official One Call 4.0 endpoint and a 15-second timeout. Use `ClientV4::with_config` with `ClientV4Config` to change the timeout or point to a trusted endpoint. The SDK sends the API key to the configured URL; use HTTPS with real credentials.

Methods return a typed `Error`. `Error::Unauthorized` can mean either a problem with the key or missing access to One Call. `Error::RateLimited` indicates the request limit was reached, and `Error::Timeout` indicates the request timed out.

See the [API reference on docs.rs](https://docs.rs/openweather-onecall-sdk) for the public types. Licensed under [MIT](LICENSE).
