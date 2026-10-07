//! Client for the Resume Reader API.
//!
//! ```no_run
//! # async fn run() -> Result<(), resumereaderapi::Error> {
//! let client = resumereaderapi::Client::new("your_api_key");
//! let parsed = client.parse_text("Jane Doe, Senior Data Analyst, Manchester", &Default::default()).await?;
//! println!("{}", parsed["resume"]["contact"]["full_name"]);
//! # Ok(()) }
//! ```

use base64::Engine;
use serde_json::{json, Map, Value};
use std::path::Path;
use std::time::Duration;

const BASE: &str = "https://www.resumereaderapi.com/api/";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("API status {status}: {message}")]
    Api { status: i64, message: String, body: Value },
}

fn status_text(status: i64) -> &'static str {
    match status {
        400 => "missing or invalid input",
        401 => "invalid API key",
        402 => "insufficient credits, nothing was billed",
        413 => "document beyond 20 pages, about 30,000 tokens or 10 MB, nothing was billed",
        422 => "file could not be read as a resume",
        429 => "rate limit exceeded, 30 requests per 60 seconds per IP",
        _ => "API error",
    }
}

/// Optional parse parameters.
#[derive(Default, Clone)]
pub struct ParseOptions {
    /// "en" (default) or "fr" for French JSON keys.
    pub field_names: Option<String>,
    pub exclude_sensitive: bool,
    pub anonymize: bool,
    pub max_pages: Option<u32>,
    pub sections: Vec<String>,
    pub language: Option<String>,
}

/// Result of a batched normalization call.
#[derive(Debug)]
pub struct Normalized {
    pub results: Vec<Value>,
    pub credits_used: f64,
    pub remaining_credits: Option<f64>,
}

pub struct Client {
    api_key: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("resumereaderapi-rust/", env!("CARGO_PKG_VERSION"), " (+https://www.resumereaderapi.com)"))
            .build()
            .expect("http client");
        Client { api_key: api_key.into(), http }
    }

    async fn post(&self, endpoint: &str, payload: Value) -> Result<Value, Error> {
        let body: Value = self.http.post(format!("{BASE}{endpoint}")).json(&payload).send().await?.json().await?;
        let status = body.get("status").and_then(Value::as_i64).unwrap_or(200);
        if status != 200 {
            return Err(Error::Api { status, message: status_text(status).to_string(), body });
        }
        Ok(body)
    }

    fn parse_payload(&self, source: (&str, Value), extra: Option<(&str, Value)>, o: &ParseOptions) -> Value {
        let mut m = Map::new();
        m.insert("api_key".into(), json!(self.api_key));
        m.insert("schema_version".into(), json!(2));
        m.insert(source.0.into(), source.1);
        if let Some((k, v)) = extra {
            m.insert(k.into(), v);
        }
        if let Some(f) = &o.field_names {
            m.insert("field_names".into(), json!(f));
        }
        if o.exclude_sensitive {
            m.insert("exclude_sensitive".into(), json!(true));
        }
        if o.anonymize {
            m.insert("anonymize".into(), json!(true));
        }
        if let Some(p) = o.max_pages {
            m.insert("max_pages".into(), json!(p));
        }
        if !o.sections.is_empty() {
            m.insert("sections".into(), json!(o.sections));
        }
        if let Some(l) = &o.language {
            m.insert("language".into(), json!(l));
        }
        Value::Object(m)
    }

    pub async fn parse_text(&self, text: &str, o: &ParseOptions) -> Result<Value, Error> {
        self.post("parse.php", self.parse_payload(("text", json!(text)), None, o)).await
    }

    pub async fn parse_file(&self, path: impl AsRef<Path>, o: &ParseOptions) -> Result<Value, Error> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        self.post("parse.php", self.parse_payload(("file_base64", json!(b64)), Some(("filename", json!(name))), o)).await
    }

    pub async fn parse_url(&self, file_url: &str, o: &ParseOptions) -> Result<Value, Error> {
        self.post("parse.php", self.parse_payload(("file_url", json!(file_url)), None, o)).await
    }

    async fn normalize(&self, endpoint: &str, key: &str, items: &[String]) -> Result<Normalized, Error> {
        let mut out = Normalized { results: Vec::new(), credits_used: 0.0, remaining_credits: None };
        for chunk in items.chunks(100) {
            let body = self.post(endpoint, json!({ "api_key": self.api_key, key: chunk })).await?;
            if let Some(list) = body.get("results").and_then(Value::as_array) {
                out.results.extend(list.iter().cloned());
            }
            out.credits_used += body.get("credits_used").and_then(Value::as_f64).unwrap_or(0.0);
            out.remaining_credits = body.get("remaining_credits").and_then(Value::as_f64).or(out.remaining_credits);
        }
        Ok(out)
    }

    pub async fn normalize_titles(&self, titles: &[String]) -> Result<Normalized, Error> {
        self.normalize("normalize_title.php", "titles", titles).await
    }

    pub async fn normalize_skills(&self, skills: &[String]) -> Result<Normalized, Error> {
        self.normalize("normalize_skills.php", "skills", skills).await
    }

    pub async fn normalize_locations(&self, locations: &[String]) -> Result<Normalized, Error> {
        self.normalize("normalize_locations.php", "locations", locations).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_carries_options() {
        let c = Client::new("k");
        let o = ParseOptions { anonymize: true, field_names: Some("fr".into()), ..Default::default() };
        let p = c.parse_payload(("text", json!("cv")), None, &o);
        assert_eq!(p["schema_version"], 2);
        assert_eq!(p["anonymize"], true);
        assert_eq!(p["field_names"], "fr");
        assert!(p.get("exclude_sensitive").is_none());
    }
}
