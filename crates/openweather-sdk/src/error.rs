use std::{error, fmt};

use reqwest::StatusCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    MissingApiKey,
    InvalidBaseUrl,
    InvalidCoordinates,
    InvalidAlertId,
    InvalidPageSize,
    ClientSetup,
    InvalidRequest,
    Timeout,
    Transport,
    BadRequest,
    Unauthorized,
    NotFound,
    RateLimited,
    Upstream(u16),
    UnexpectedStatus(u16),
    InvalidResponse,
    InvalidPagination,
}

impl Error {
    pub(crate) fn from_status(status: StatusCode) -> Self {
        match status {
            StatusCode::BAD_REQUEST => Self::BadRequest,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Self::Unauthorized,
            StatusCode::NOT_FOUND => Self::NotFound,
            StatusCode::TOO_MANY_REQUESTS => Self::RateLimited,
            status if status.is_server_error() => Self::Upstream(status.as_u16()),
            status => Self::UnexpectedStatus(status.as_u16()),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::MissingApiKey => "OpenWeather API key is empty",
            Self::InvalidBaseUrl => {
                "OpenWeather base URL must be an absolute HTTP(S) URL without credentials, a query, or a fragment"
            }
            Self::InvalidCoordinates => {
                "latitude must be within [-90, 90] and longitude within [-180, 180]"
            }
            Self::InvalidAlertId => "invalid OpenWeather alert ID",
            Self::InvalidPageSize => "page size must be positive",
            Self::ClientSetup => "could not initialize the OpenWeather HTTP client",
            Self::InvalidRequest => "could not build the OpenWeather request",
            Self::Timeout => "OpenWeather request timed out",
            Self::Transport => "could not complete the OpenWeather HTTP request",
            Self::BadRequest => "OpenWeather rejected the request parameters",
            Self::Unauthorized => {
                "OpenWeather denied access; check the API key and One Call subscription"
            }
            Self::NotFound => "OpenWeather did not find the requested data",
            Self::RateLimited => "OpenWeather API request limit reached",
            Self::Upstream(status) => {
                return write!(f, "OpenWeather service error (HTTP {status})");
            }
            Self::UnexpectedStatus(status) => {
                return write!(f, "unexpected OpenWeather HTTP status {status}");
            }
            Self::InvalidResponse => "OpenWeather returned an invalid response",
            Self::InvalidPagination => "OpenWeather returned invalid pagination data",
        };
        f.write_str(message)
    }
}

impl error::Error for Error {}
