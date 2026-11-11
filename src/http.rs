//! Sends an HTTP request (called from a worker thread).

use std::time::{Duration, Instant};
use ureq::http;

pub struct Outgoing {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

pub struct Response {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub elapsed: Duration,
}

const MAX_BODY: u64 = 100 * 1024 * 1024;

pub fn send(req: Outgoing) -> Result<Response, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();

    let mut url = req.url.trim().to_string();
    if !url.contains("://") {
        url = format!("http://{url}");
    }

    let mut builder = http::Request::builder().method(req.method.as_str()).uri(&url);
    let mut has_content_type = false;
    for (k, v) in &req.headers {
        has_content_type |= k.eq_ignore_ascii_case("content-type");
        builder = builder.header(k, v);
    }
    // Convenience: a JSON body without a Content-Type gets application/json.
    if !req.body.is_empty() && !has_content_type && serde_json::from_str::<serde_json::Value>(&req.body).is_ok() {
        builder = builder.header("Content-Type", "application/json");
    }

    let start = Instant::now();
    let result = if req.body.is_empty() {
        agent.run(builder.body(()).map_err(|e| e.to_string())?)
    } else {
        agent.run(builder.body(req.body.into_bytes()).map_err(|e| e.to_string())?)
    };
    let mut resp = result.map_err(|e| e.to_string())?;

    let status = resp.status();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
        .collect();
    let body = resp.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| e.to_string())?;

    Ok(Response {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        body,
        elapsed: start.elapsed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn post_json_to_local_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            let raw = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = "{\"ok\":true}";
            write!(
                s,
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            raw
        });
        let resp = send(Outgoing {
            method: "POST".into(),
            url: format!("127.0.0.1:{port}/users"),
            headers: vec![("X-Test".into(), "1".into())],
            body: "{\"a\":1}".into(),
        })
        .unwrap();
        let raw = server.join().unwrap().to_lowercase();
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, b"{\"ok\":true}");
        assert!(raw.starts_with("post /users"));
        assert!(raw.contains("x-test: 1"));
        assert!(raw.contains("content-type: application/json"));
    }
}
