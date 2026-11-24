mod cache;
#[cfg(test)]
mod cache_tests;
mod secret;
mod server;
#[cfg(target_os = "linux")]
mod update;

use std::{env, error::Error, ffi::OsStr, process::ExitCode};

use openweather_onecall_sdk::ClientV4;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::Layer;
use tracing_subscriber::{filter::LevelFilter, filter::Targets, layer::SubscriberExt};

use crate::server::WeatherServer;

enum ApiVersion {
    V4,
}

impl ApiVersion {
    fn load() -> Result<Self, &'static str> {
        match env::var_os("OPENWEATHER_API_VERSION").as_deref() {
            None => Ok(Self::V4),
            Some(version) if version == OsStr::new("v4") => Ok(Self::V4),
            Some(_) => Err("unsupported OPENWEATHER_API_VERSION; supported value: v4"),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("openweather-mcp: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    if let Some(arg) = args.next() {
        if args.next().is_some() {
            return Err("usage: openweather-mcp [--version|update]".into());
        }
        match arg.to_str() {
            Some("--version") => println!("openweather-mcp {}", env!("CARGO_PKG_VERSION")),
            Some("-h" | "--help") => {
                println!(
                    "Usage: openweather-mcp [--version|update]\nWithout arguments, start the MCP server over stdio."
                );
            }
            #[cfg(target_os = "linux")]
            Some("update") => update::run().await?,
            #[cfg(not(target_os = "linux"))]
            Some("update") => return Err("self-update is supported only on Linux".into()),
            _ => return Err("usage: openweather-mcp [--version|update]".into()),
        }
        return Ok(());
    }
    init_logging()?;
    match ApiVersion::load()? {
        ApiVersion::V4 => serve_v4().await,
    }
}

fn init_logging() -> Result<(), &'static str> {
    let level = match env::var_os("OPENWEATHER_LOG").as_deref() {
        None => LevelFilter::WARN,
        Some(level) if level == "off" => LevelFilter::OFF,
        Some(level) if level == "warn" => LevelFilter::WARN,
        Some(level) if level == "debug" => LevelFilter::DEBUG,
        Some(_) => return Err("unsupported OPENWEATHER_LOG level; use off, warn, or debug"),
    };
    // Do not enable dependency logging: transport diagnostics may include request URLs.
    let targets = Targets::new()
        .with_target("openweather_mcp", level)
        .with_target("openweather_onecall_sdk", level);
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_filter(targets);
    tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer))
        .map_err(|_| "could not initialize OpenWeather logging")
}

async fn serve_v4() -> Result<(), Box<dyn Error>> {
    let cache_enabled = match env::var_os("OPENWEATHER_CACHE").as_deref() {
        None => true,
        Some(value) if value == "on" => true,
        Some(value) if value == "off" => false,
        Some(_) => return Err("unsupported OPENWEATHER_CACHE value; use on or off".into()),
    };
    let server = WeatherServer::new(ClientV4::new(secret::load()?)?, cache_enabled);
    tracing::debug!(
        api_version = "v4",
        cache_enabled,
        "Starting OpenWeather MCP stdio server"
    );
    let service = server
        .serve(stdio())
        .await
        .map_err(|_| "could not initialize the MCP stdio connection")?;
    service
        .waiting()
        .await
        .map_err(|_| "MCP stdio service stopped unexpectedly")?;
    Ok(())
}
