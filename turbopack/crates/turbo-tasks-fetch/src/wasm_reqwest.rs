//! HTTP for wasm builds, in the shape of the `reqwest` API this crate consumes, served by the host.
//!
//! `reqwest` itself cannot serve wasm here. Its only wasm backend targets `wasm32-unknown-unknown`
//! and is built on the browser `fetch` API via `wasm-bindgen`. Under `wasm32-wasip1-threads` that
//! backend is still selected (it keys off `target_arch = "wasm32"`), and it is both
//! API-incompatible (no `ClientBuilder::connect_timeout`, no `ClientBuilder::timeout`, no
//! `Error::is_connect`) and — fatally — `!Send`, which `turbo-tasks` requires of every task future.
//! WASI preview1 has no sockets to build a native client on either.
//!
//! So the host provides HTTP: it registers a fetch with [`set_host_fetch`] (the Next.js bindings
//! do this with a napi callback into the JS `fetch`), and requests go through it behind the
//! existing [`crate::FetchClientConfig`] interface, so callers are unaffected. Without a host
//! fetch, building a client fails with a clear error, so features that fetch at build time, such as
//! `next/font/google`, report it instead of silently producing nothing.

use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{Arc, OnceLock},
    time::Duration,
};

pub type Result<T> = std::result::Result<T, Error>;

/// A GET request for the host to perform.
#[derive(Clone, Debug)]
pub struct HostRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Maximum time for the entire request, from [`crate::FetchClientConfig::timeout`].
    pub timeout: Option<Duration>,
}

/// The host's complete response.
#[derive(Debug)]
pub struct HostResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Performs a request on the host. Fails with a message when no response arrived (a network error,
/// a timeout, ...).
pub type HostFetch = Arc<
    dyn Fn(HostRequest) -> Pin<Box<dyn Future<Output = std::result::Result<HostResponse, String>> + Send>>
        + Send
        + Sync,
>;

static HOST_FETCH: OnceLock<HostFetch> = OnceLock::new();

/// Serves this crate's HTTP requests on wasm with the host's fetch. Returns `false` if one was
/// already set.
pub fn set_host_fetch(fetch: HostFetch) -> bool {
    HOST_FETCH.set(fetch).is_ok()
}

#[derive(Debug)]
pub enum Error {
    /// No host fetch was registered, so there is no HTTP client at all.
    Unsupported,
    /// The host's fetch failed before a response arrived.
    Request(String),
    /// The response had a 4xx or 5xx status.
    Status(StatusCode),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => f.write_str(
                "HTTP requests are not supported in this wasm build of Next.js: the host provides \
                 no HTTP client. Features that fetch at build time, such as `next/font/google`, \
                 cannot be used here — self-host the assets, or use a platform with native Next.js \
                 binaries.",
            ),
            Error::Request(message) => write!(f, "request failed: {message}"),
            Error::Status(status) => write!(f, "HTTP status {}", status.0),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// Retrying cannot help: the platform has no client.
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Error::Unsupported)
    }

    /// The host reports network failures and timeouts alike as a failed request.
    pub fn is_connect(&self) -> bool {
        matches!(self, Error::Request(_))
    }

    pub fn is_timeout(&self) -> bool {
        false
    }

    pub fn is_request(&self) -> bool {
        matches!(self, Error::Request(_))
    }

    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Error::Status(status) => Some(*status),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusCode(u16);

impl StatusCode {
    pub fn as_u16(self) -> u16 {
        self.0
    }

    pub fn is_server_error(self) -> bool {
        (500..600).contains(&self.0)
    }
}

#[derive(Clone)]
pub struct Client {
    fetch: HostFetch,
    timeout: Option<Duration>,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").field("timeout", &self.timeout).finish_non_exhaustive()
    }
}

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder { timeout: None }
    }

    pub fn get(&self, url: &str) -> RequestBuilder {
        RequestBuilder {
            client: self.clone(),
            url: url.to_string(),
            headers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClientBuilder {
    timeout: Option<Duration>,
}

impl ClientBuilder {
    /// The host's fetch has no separate connect phase; [`ClientBuilder::timeout`] bounds the
    /// whole request.
    pub fn connect_timeout(self, _timeout: Duration) -> Self {
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Fails without a host fetch, so callers get the error at client construction rather than
    /// per request.
    pub fn build(self) -> Result<Client> {
        let fetch = HOST_FETCH.get().cloned().ok_or(Error::Unsupported)?;
        Ok(Client {
            fetch,
            timeout: self.timeout,
        })
    }
}

#[derive(Clone, Debug)]
pub struct RequestBuilder {
    client: Client,
    url: String,
    headers: Vec<(String, String)>,
}

impl RequestBuilder {
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn try_clone(&self) -> Option<Self> {
        Some(self.clone())
    }

    pub async fn send(self) -> Result<Response> {
        let request = HostRequest {
            url: self.url,
            headers: self.headers,
            timeout: self.client.timeout,
        };
        let response = (self.client.fetch)(request).await.map_err(Error::Request)?;
        let mut headers = header::HeaderMap::new();
        for (name, value) in response.headers {
            headers.insert(&name, header::HeaderValue::new(value));
        }
        Ok(Response {
            status: StatusCode(response.status),
            headers,
            body: response.body,
        })
    }
}

#[derive(Debug)]
pub struct Response {
    status: StatusCode,
    headers: header::HeaderMap,
    body: Vec<u8>,
}

impl Response {
    pub fn error_for_status(self) -> Result<Self> {
        if self.status.0 >= 400 {
            Err(Error::Status(self.status))
        } else {
            Ok(self)
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &header::HeaderMap {
        &self.headers
    }

    pub async fn bytes(self) -> Result<Vec<u8>> {
        Ok(self.body)
    }
}

pub mod header {
    use rustc_hash::FxHashMap;

    pub const CACHE_CONTROL: &str = "cache-control";

    #[derive(Clone, Debug, Default)]
    pub struct HeaderValue(String);

    impl HeaderValue {
        pub fn from_static(value: &'static str) -> Self {
            HeaderValue(value.to_string())
        }

        pub(crate) fn new(value: String) -> Self {
            HeaderValue(value)
        }

        pub fn to_str(&self) -> Result<&str, std::str::Utf8Error> {
            Ok(&self.0)
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct HeaderMap(FxHashMap<String, HeaderValue>);

    impl HeaderMap {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn insert(&mut self, name: &str, value: HeaderValue) -> Option<HeaderValue> {
            self.0.insert(name.to_ascii_lowercase(), value)
        }

        pub fn get(&self, name: &str) -> Option<&HeaderValue> {
            self.0.get(&name.to_ascii_lowercase())
        }
    }
}
