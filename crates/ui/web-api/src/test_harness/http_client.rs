//! Ergonomic HTTP client for integration tests, built on `tower::ServiceExt::oneshot`.

use axum::Router;
use axum::body::Body;
use http::Request;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// A thin wrapper around an Axum [`Router`] for ergonomic test requests.
///
/// The router is cloned for each request, so a single `TestClient` can
/// issue multiple sequential requests.
pub struct TestClient {
    router: Router,
}

impl TestClient {
    /// Create a new client for the given router.
    pub fn new(router: Router) -> Self {
        Self { router }
    }

    /// Start building a `GET` request.
    pub fn get(&self, uri: &str) -> RequestBuilder {
        RequestBuilder::new(self.router.clone(), http::Method::GET, uri)
    }

    /// Start building a `POST` request with a JSON body.
    pub fn post_json(&self, uri: &str, body: &impl serde::Serialize) -> RequestBuilder {
        RequestBuilder::new(self.router.clone(), http::Method::POST, uri).json_body(body)
    }

    /// Start building a `PUT` request with a JSON body.
    pub fn put_json(&self, uri: &str, body: &impl serde::Serialize) -> RequestBuilder {
        RequestBuilder::new(self.router.clone(), http::Method::PUT, uri).json_body(body)
    }

    /// Start building a `DELETE` request.
    pub fn delete(&self, uri: &str) -> RequestBuilder {
        RequestBuilder::new(self.router.clone(), http::Method::DELETE, uri)
    }

    /// Start building a `POST` request with an empty body.
    pub fn post_empty(&self, uri: &str) -> RequestBuilder {
        RequestBuilder::new(self.router.clone(), http::Method::POST, uri)
    }
}

/// Builder for a single HTTP request.
pub struct RequestBuilder {
    router: Router,
    method: http::Method,
    uri: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
}

impl RequestBuilder {
    fn new(router: Router, method: http::Method, uri: &str) -> Self {
        Self {
            router,
            method,
            uri: uri.to_string(),
            headers: Vec::new(),
            body: None,
        }
    }

    /// Set a JSON body.
    fn json_body(mut self, body: &impl serde::Serialize) -> Self {
        self.body = Some(serde_json::to_vec(body).expect("serialize body"));
        self.headers.push((
            http::header::CONTENT_TYPE.to_string(),
            "application/json".to_string(),
        ));
        self
    }

    /// Add an `Authorization: Bearer <token>` header.
    pub fn bearer(mut self, token: &str) -> Self {
        self.headers.push((
            http::header::AUTHORIZATION.to_string(),
            format!("Bearer {token}"),
        ));
        self
    }

    /// Add a custom header.
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }

    /// Execute the request and return the raw `http::Response`.
    pub async fn send(self) -> http::Response<Body> {
        let body = match self.body {
            Some(b) => Body::from(b),
            None => Body::empty(),
        };

        let mut builder = Request::builder().method(self.method).uri(&self.uri);
        for (k, v) in &self.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        let req = builder.body(body).expect("build request");

        self.router
            .oneshot(req)
            .await
            .expect("router oneshot failed")
    }

    /// Execute the request and return `(StatusCode, deserialized body)`.
    pub async fn send_json<T: serde::de::DeserializeOwned>(self) -> (http::StatusCode, T) {
        let resp = self.send().await;
        let status = resp.status();
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let value: T =
            serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                panic!(
                    "failed to deserialize response body as {}: {e}\nbody: {}",
                    std::any::type_name::<T>(),
                    String::from_utf8_lossy(&bytes),
                )
            });
        (status, value)
    }

    /// Execute the request and return just the status code.
    pub async fn send_status(self) -> http::StatusCode {
        self.send().await.status()
    }
}
