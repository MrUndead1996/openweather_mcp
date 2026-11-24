use std::sync::atomic::{AtomicUsize, Ordering};

use rmcp::model::CallToolResult;
use serde_json::json;
use tokio::sync::Notify;

use crate::cache::{Tool, WeatherCache};

#[tokio::test]
async fn coalesces_identical_in_flight_calls_and_separates_tool_and_arguments() {
    let cache = WeatherCache::new();
    let args = json!({"lat": 53.2097, "lon": 50.1564, "units": "metric", "lang": "ru", "cnt": 4});
    let calls = AtomicUsize::new(0);
    let release = Notify::new();
    let fetch = || async {
        calls.fetch_add(1, Ordering::Relaxed);
        CallToolResult::structured(json!({"fetched": true}))
    };
    let (first, second, ()) = tokio::join!(
        cache.get_or_fetch(Tool::Daily, &args, async {
            release.notified().await;
            fetch().await
        }),
        cache.get_or_fetch(Tool::Daily, &args, fetch()),
        async { release.notify_one() },
    );
    assert_eq!(first, second);
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    cache.get_or_fetch(Tool::Hourly, &args, fetch()).await;
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    for (field, value) in [
        ("lat", json!(52.0)),
        ("lon", json!(51.0)),
        ("units", json!("imperial")),
        ("lang", json!("en")),
        ("cnt", json!(8)),
        ("start", json!(1795824000)),
    ] {
        let mut changed = args.clone();
        changed[field] = value;
        cache.get_or_fetch(Tool::Daily, &changed, fetch()).await;
    }
    assert_eq!(calls.load(Ordering::Relaxed), 8);
}
