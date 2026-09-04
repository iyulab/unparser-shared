//! A minimal synchronous mock HTTP server for the `ai` feature's integration tests.
//!
//! Deliberately hand-rolled on `tiny_http` rather than a mocking framework
//! (`httpmock`/`wiremock`) — see the `ai` dev-dependency comment in `Cargo.toml` for
//! why. It serves exactly the `responses` given, one per request received, in order.

use std::thread::JoinHandle;

pub struct MockServer {
    url: String,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Starts a server that hands out `responses` (status code, body) in order, one
    /// per request received, then stops.
    pub fn serving(responses: Vec<(u16, String)>) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock server");
        let url = format!("http://{}", server.server_addr());
        let handle = std::thread::spawn(move || {
            for (status, body) in responses {
                let Ok(request) = server.recv() else {
                    return;
                };
                let header =
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                        .expect("valid header");
                let response = tiny_http::Response::from_string(body)
                    .with_status_code(status)
                    .with_header(header);
                let _ = request.respond(response);
            }
        });
        Self {
            url,
            handle: Some(handle),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
