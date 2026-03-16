use std::collections::HashMap;
use std::time::Duration;

use orbit_types::OrbitError;
use reqwest::Method;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::template::{TemplateContext, render};

#[derive(Debug, Clone, Deserialize)]
pub struct ApiSpec {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub timeout_seconds: Option<u64>,
    #[serde(default = "default_status_codes")]
    pub expected_status_codes: Vec<u16>,
}

fn default_status_codes() -> Vec<u16> {
    vec![200]
}

pub fn execute(
    spec_config: &Value,
    template_context: &TemplateContext,
    timeout_seconds: u64,
) -> Result<Value, OrbitError> {
    let spec: ApiSpec = serde_json::from_value(spec_config.clone())
        .map_err(|error| OrbitError::InvalidInput(format!("invalid api spec_config: {error}")))?;
    let method = Method::from_bytes(spec.method.as_bytes()).map_err(|error| {
        OrbitError::InvalidInput(format!("invalid api method '{}': {error}", spec.method))
    })?;
    let url = render(&spec.url, template_context)?;
    let body = spec
        .body
        .as_deref()
        .map(|value| render(value, template_context))
        .transpose()?;

    let client = Client::builder()
        .timeout(Duration::from_secs(
            spec.timeout_seconds.unwrap_or(timeout_seconds),
        ))
        .build()
        .map_err(|error| OrbitError::Execution(format!("failed to build api client: {error}")))?;

    let mut request = client.request(method, url);
    for (key, value) in &spec.headers {
        request = request.header(key, render(value, template_context)?);
    }
    if let Some(body) = body {
        request = request.body(body);
    }

    let response = request
        .send()
        .map_err(|error| OrbitError::Execution(format!("api request failed: {error}")))?;
    let status = response.status().as_u16();
    let raw_body = response
        .text()
        .map_err(|error| OrbitError::Execution(format!("failed to read api response: {error}")))?;

    if !spec.expected_status_codes.contains(&status) {
        return Err(OrbitError::Execution(format!(
            "api returned status {status}; expected one of {:?}; body: {}",
            spec.expected_status_codes, raw_body
        )));
    }

    match serde_json::from_str::<Value>(&raw_body) {
        Ok(value) => Ok(value),
        Err(_) => Ok(json!({ "body": raw_body })),
    }
}
