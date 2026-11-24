mod secret;
mod server;

use std::{env, error::Error, ffi::OsStr, process::ExitCode};

use openweather_sdk::ClientV4;
use rmcp::{ServiceExt, transport::stdio};

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
    match ApiVersion::load()? {
        ApiVersion::V4 => serve_v4().await,
    }
}

async fn serve_v4() -> Result<(), Box<dyn Error>> {
    let server = WeatherServer::new(ClientV4::new(secret::load()?)?);
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
