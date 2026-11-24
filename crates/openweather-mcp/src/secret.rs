use std::{env, error::Error, ffi::OsString, fmt, fs};

const KEY: &str = "OPENWEATHER_API_KEY";
const KEY_FILE: &str = "OPENWEATHER_API_KEY_FILE";

pub fn load() -> Result<String, SecretError> {
    load_from(env::var_os(KEY), env::var_os(KEY_FILE))
}

fn load_from(key: Option<OsString>, file: Option<OsString>) -> Result<String, SecretError> {
    let secret = match (key, file) {
        (Some(_), Some(_)) => return Err(SecretError::Conflict),
        (None, None) => return Err(SecretError::Missing),
        (Some(key), None) => key.into_string().map_err(|_| SecretError::InvalidKey)?,
        (None, Some(path)) if path.is_empty() => return Err(SecretError::EmptyPath),
        (None, Some(path)) => fs::read_to_string(path).map_err(|_| SecretError::UnreadableFile)?,
    };
    let key = secret.trim();
    if key.is_empty() {
        return Err(SecretError::EmptyKey);
    }
    Ok(key.to_owned())
}

#[derive(Debug, PartialEq, Eq)]
pub enum SecretError {
    Missing,
    Conflict,
    InvalidKey,
    EmptyPath,
    UnreadableFile,
    EmptyKey,
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing | Self::Conflict => write!(f, "set exactly one of {KEY} or {KEY_FILE}"),
            Self::InvalidKey => write!(f, "{KEY} must contain UTF-8 text"),
            Self::EmptyPath => write!(f, "{KEY_FILE} is empty"),
            Self::UnreadableFile => write!(f, "could not read a UTF-8 secret file from {KEY_FILE}"),
            Self::EmptyKey => f.write_str("OpenWeather API key is empty"),
        }
    }
}

impl Error for SecretError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_key_and_rejects_missing_empty_or_conflicting_sources() {
        assert_eq!(load_from(Some(" key \n".into()), None).unwrap(), "key");
        assert_eq!(load_from(None, None), Err(SecretError::Missing));
        assert_eq!(
            load_from(Some("key".into()), Some("file".into())),
            Err(SecretError::Conflict)
        );
        assert_eq!(
            load_from(Some(" \n".into()), None),
            Err(SecretError::EmptyKey)
        );
        assert_eq!(
            load_from(None, Some("".into())),
            Err(SecretError::EmptyPath)
        );
    }

    #[test]
    fn reads_utf8_file_and_rejects_missing_empty_or_invalid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        let load = || load_from(None, Some(path.clone().into_os_string()));
        assert_eq!(load(), Err(SecretError::UnreadableFile));
        fs::write(&path, "  secret-value\r\n").unwrap();
        assert_eq!(load().unwrap(), "secret-value");
        fs::write(&path, "\r\n").unwrap();
        assert_eq!(load(), Err(SecretError::EmptyKey));
        fs::write(&path, [0xff]).unwrap();
        assert_eq!(load(), Err(SecretError::UnreadableFile));
    }
}
