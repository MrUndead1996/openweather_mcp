# OpenWeather MCP

A local MCP server for the [OpenWeather One Call API 4.0](https://openweathermap.org/api/one-call-4.md). It provides current conditions, forecasts, historical weather, and official weather alerts. The MCP client launches the server as a subprocess and communicates with it over `stdio`.

This repository contains two crates: [`openweather-mcp`](crates/openweather-mcp), the server, and [`openweather-onecall-sdk`](crates/openweather-sdk), a standalone async Rust client with typed requests and responses.

## Installation

Download the archive for your Linux architecture from [GitHub Releases](https://github.com/MrUndead1996/openweather_mcp/releases):

| Architecture | Archive |
| --- | --- |
| `x86_64` (Intel/AMD 64-bit) | `openweather-mcp-linux-x86_64.tar.gz` |
| `arm64` (`aarch64`) | `openweather-mcp-linux-aarch64.tar.gz` |

Extract the archive and keep the absolute path to `openweather-mcp` for your client configuration. The binary is statically linked; Rust is not needed to run it.

```sh
tar -xzf openweather-mcp-linux-x86_64.tar.gz
```

## Setup

1. Activate a separate [One Call by Call](https://home.openweathermap.org/subscriptions) subscription and use an API key from the same account. The standard **Free** plan does not include One Call 4.0. The first 1,000 calls per day are free; to avoid charges, set your daily limit to 1,000 in your OpenWeather account.
2. Add the server to your MCP client. This example is for clients that use an `mcpServers` configuration. Replace both paths with your own absolute paths:

   ```json
   {
     "mcpServers": {
       "openweather": {
         "command": "/absolute/path/to/openweather-mcp",
         "env": {
           "OPENWEATHER_API_KEY_FILE": "/absolute/path/to/openweather-key"
         }
       }
     }
   }
   ```

### Environment variables

| Variable | Description |
| --- | --- |
| `OPENWEATHER_API_KEY` | The OpenWeather API key itself. Leave unset if you use `OPENWEATHER_API_KEY_FILE`. |
| `OPENWEATHER_API_KEY_FILE` | Path to a UTF-8 file containing the API key. The server reads it once at startup and trims leading and trailing whitespace, including newlines. Leave unset if you use `OPENWEATHER_API_KEY`. |
| `OPENWEATHER_API_VERSION` | Optional. Defaults to `v4`, the only supported value. |
| `OPENWEATHER_LOG` | Optional. `warn` (default) logs upstream failures; `debug` also logs successful requests and timings; `off` disables these logs. |
| `OPENWEATHER_CACHE` | Optional. `on` (default) enables the in-memory response cache; `off` sends each valid call to OpenWeather. |

Set **exactly one** of the two key variables. For Docker Secrets, set `OPENWEATHER_API_KEY_FILE` to the path inside the container, such as `/run/secrets/openweather_api_key`. This project does not ship a Docker image.

## Tools

| Tool | Returns |
| --- | --- |
| `get_current_weather` | Current conditions and active alert IDs, if any |
| `get_minute_forecast` | Minute-by-minute precipitation forecast for the next hour |
| `get_15_minute_forecast` | Forecast at 15-minute intervals for the next 48 hours |
| `get_hourly_timeline` | Hourly historical weather or forecasts for the next 48 hours |
| `get_daily_timeline` | Daily historical weather or forecasts up to 1.5 years ahead |
| `get_weather_alert` | Alert details for an `alert_id` from the `alerts` array |

Weather requests take `lat` and `lon` in decimal degrees. City-name lookup is not supported. You can optionally pass `units` (`standard`, `metric`, `imperial`) and a language code as `lang` (for example, `en`). Example arguments for `get_current_weather`:

```json
{ "lat": 51.5, "lon": -0.1, "units": "metric", "lang": "en" }
```

Tools return structured JSON. The 15-minute, hourly, and daily timelines return one page per call. To continue, repeat the request with the same coordinates, units, and language, using `start` (Unix timestamp in seconds, UTC) and, if present, `cnt` from `next` or `prev`. A page not served from the cache requires a separate OpenWeather API call.

If `get_current_weather` returns `unauthorized`, check that the API key is active and the One Call subscription is enabled on the same account.

Diagnostic logs go to `stderr`, not the MCP `stdout` stream. For upstream failures they show the endpoint and failure stage, plus the HTTP status when available. JSON decode warnings include the error category, line and column, and the tracked response path (which may point to the containing object). Logs exclude API keys, full URLs, query parameters, and raw response bodies.

### Response cache

The MCP server keeps up to 256 successful responses in memory per process, keyed by tool and arguments (including units, language, and pagination). Concurrent identical calls share one upstream request. Errors are not cached, and restarting the server clears the cache. `OPENWEATHER_LOG=debug` shows cache hits and misses without argument values.

| Response | Cached for |
| --- | ---: |
| Minute forecast | 30 seconds |
| Current weather and alert details | 1 minute |
| 15-minute and hourly timelines | 2 minutes |
| Daily timeline | 10 minutes |

These are local freshness limits, not HTTP revalidation intervals: a cache hit makes no request to OpenWeather.

## Development

### Build from source (optional)

From the repository root, with stable Rust installed:

```sh
cargo build --release --locked -p openweather-mcp
```

Use `target/release/openweather-mcp` in place of the downloaded binary.

### Checks

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
```

Tests do not require a real API key. GitHub Actions runs these checks on branch pushes and pull requests. Pushing a `v*` tag that matches both crate versions triggers the Linux builds, creates a GitHub Release, and publishes the SDK to crates.io if authentication is configured.

## License

[MIT](LICENSE).
