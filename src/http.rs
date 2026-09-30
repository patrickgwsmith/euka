use crate::session::Resource;
use std::time::{Duration, Instant};

const MAX_RESOURCE_BYTES: u64 = 4 * 1024 * 1024;

pub fn origin(url: &str) -> Result<String, String> {
    let uri: ureq::http::Uri = url
        .parse()
        .map_err(|error| format!("invalid URL: {error}"))?;
    let scheme = uri.scheme_str().ok_or("URL has no scheme")?;
    if !matches!(scheme, "http" | "https") {
        return Err(format!("unsupported URL scheme: {scheme}"));
    }
    let host = uri.host().ok_or("URL has no host")?;
    let port = uri
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    Ok(format!("{scheme}://{}:{port}", host.to_ascii_lowercase()))
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_idle_connections_per_host(1)
        .max_idle_age(Duration::from_secs(60))
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .new_agent()
}

pub fn load(agent: &ureq::Agent, url: &str) -> Result<Resource, String> {
    let mut response = agent.get(url).call().map_err(|error| error.to_string())?;
    if response.status().is_redirection() {
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("(no Location header)");
        return Err(format!(
            "HTTP {} redirects to {location}; enter the destination URL to load it",
            response.status()
        ));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    if media_type != "unknown"
        && !media_type.starts_with("text/")
        && !matches!(media_type, "application/json" | "application/xml")
        && !media_type.ends_with("+json")
        && !media_type.ends_with("+xml")
    {
        return Err(format!("resource is not text ({content_type})"));
    }
    let content = response
        .body_mut()
        .with_config()
        .limit(MAX_RESOURCE_BYTES)
        .read_to_string()
        .map_err(|error| error.to_string())?;
    Ok(Resource {
        url: url.to_owned(),
        content_type,
        content,
    })
}

pub fn head(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let start = Instant::now();
    let response = agent.head(url).call().map_err(|error| error.to_string())?;
    let elapsed = start.elapsed();
    let mut output = format!("HTTP {:?}\n", response.status());
    for (name, value) in response.headers() {
        output.push_str(&format!(
            "{}: {}\n",
            name,
            value.to_str().unwrap_or("<non-UTF-8>")
        ));
    }
    output.push_str(&format!(
        "Time to headers: {:.1} ms",
        elapsed.as_secs_f64() * 1000.0
    ));
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::origin;

    #[test]
    fn origin_includes_scheme_and_port() {
        assert_eq!(
            origin("https://EXAMPLE.com/a").unwrap(),
            "https://example.com:443"
        );
        assert_eq!(
            origin("http://example.com:8080/").unwrap(),
            "http://example.com:8080"
        );
        assert!(origin("https:///missing-host").is_err());
    }
}
