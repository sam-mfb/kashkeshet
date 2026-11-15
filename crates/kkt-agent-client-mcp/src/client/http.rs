//! Plain HTTP for a program's test client. Protocol-specific calls and
//! assertions belong to that client's tests, not this transport.

use futures::StreamExt;

/// GET and POST requests with redirects disabled and TCP_NODELAY enabled.
pub struct Http {
    client: reqwest::Client,
}

/// A response whose HTTP library and streaming body stay private.
pub struct HttpReply {
    response: reqwest::Response,
}

impl Http {
    /// A client with the fake product's existing connection settings.
    pub fn new() -> Result<Self, String> {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .tcp_nodelay(true)
            .build()
            .map(|client| Self { client })
            .map_err(|e| e.to_string())
    }
    fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        bearer: Option<&str>,
        headers: &[(&str, &str)],
    ) -> reqwest::RequestBuilder {
        let mut request = self.client.request(method, url);
        for &(name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        request
    }
    /// Sends a GET, with the supplied headers and optional bearer.
    pub async fn get(
        &self,
        url: &str,
        bearer: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<HttpReply, String> {
        self.request(reqwest::Method::GET, url, bearer, headers)
            .send()
            .await
            .map(|response| HttpReply { response })
            .map_err(|e| e.to_string())
    }
    /// Sends the body as POST bytes, with the supplied headers and optional bearer.
    pub async fn post(
        &self,
        url: &str,
        bearer: Option<&str>,
        headers: &[(&str, &str)],
        body: String,
    ) -> Result<HttpReply, String> {
        self.request(reqwest::Method::POST, url, bearer, headers)
            .body(body)
            .send()
            .await
            .map(|response| HttpReply { response })
            .map_err(|e| e.to_string())
    }
}

impl HttpReply {
    /// The numeric HTTP status.
    pub fn status(&self) -> u16 {
        self.response.status().as_u16()
    }
    /// One header's bytes, using HTTP's case-insensitive name lookup.
    pub fn header(&self, name: &str) -> Option<&[u8]> {
        self.response.headers().get(name).map(|v| v.as_bytes())
    }
    /// One header as text, with the HTTP library's existing validation.
    pub fn header_text(&self, name: &str) -> Option<&str> {
        self.response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
    }
    /// Reads the body as text with the existing HTTP decoding behavior.
    pub async fn text(self) -> Result<String, String> {
        self.response.text().await.map_err(|e| e.to_string())
    }
    /// Streams body chunks without exposing the HTTP library's types.
    pub fn bytes_stream(self) -> futures::stream::BoxStream<'static, Result<Vec<u8>, String>> {
        Box::pin(
            self.response
                .bytes_stream()
                .map(|chunk| chunk.map(|b| b.to_vec()).map_err(|e| e.to_string())),
        )
    }
}
