//! Lets the wasm build of Turbopack make HTTP requests (for `next/font/google`) through the host's
//! `fetch`: Next.js registers a JS function with `registerFetch`, and `turbo-tasks-fetch` sends
//! its requests through it. Native builds have their own HTTP client and ignore the registration.

use napi::{
    Status,
    bindgen_prelude::{Buffer, Promise},
    threadsafe_function::ThreadsafeFunction,
};
use napi_derive::napi;

#[napi(object)]
pub struct NapiFetchHeader {
    pub name: String,
    pub value: String,
}

#[napi(object)]
pub struct NapiFetchRequest {
    pub url: String,
    pub headers: Vec<NapiFetchHeader>,
    /// Maximum time for the entire request.
    pub timeout_ms: Option<u32>,
}

#[napi(object)]
pub struct NapiFetchResponse {
    pub status: u32,
    pub headers: Vec<NapiFetchHeader>,
    pub body: Buffer,
}

type HostFetchFunction = ThreadsafeFunction<
    NapiFetchRequest,
    Promise<NapiFetchResponse>,
    NapiFetchRequest,
    Status,
    /* CalleeHandled */ false,
    /* Weak */ true,
>;

// Allow dead_code for test builds where napi exports are not entry points
#[allow(dead_code)]
#[napi]
pub fn register_fetch(
    #[napi(ts_arg_type = "(request: NapiFetchRequest) => Promise<NapiFetchResponse>")]
    fetch: HostFetchFunction,
) -> napi::Result<()> {
    #[cfg(target_family = "wasm")]
    if !turbo_tasks_fetch::set_host_fetch(host_fetch(std::sync::Arc::new(fetch))) {
        return Err(napi::Error::from_reason("A fetch was already registered"));
    }
    #[cfg(not(target_family = "wasm"))]
    let _ = fetch;
    Ok(())
}

#[cfg(target_family = "wasm")]
fn host_fetch(fetch: std::sync::Arc<HostFetchFunction>) -> turbo_tasks_fetch::HostFetch {
    std::sync::Arc::new(move |request: turbo_tasks_fetch::HostRequest| {
        let fetch = fetch.clone();
        let future: std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<turbo_tasks_fetch::HostResponse, String>,
                    > + Send,
            >,
        > = Box::pin(async move {
            let request = NapiFetchRequest {
                url: request.url,
                headers: request
                    .headers
                    .into_iter()
                    .map(|(name, value)| NapiFetchHeader { name, value })
                    .collect(),
                timeout_ms: request
                    .timeout
                    .map(|timeout| timeout.as_millis().min(u32::MAX as u128) as u32),
            };
            let response = fetch
                .call_async(request)
                .await
                .map_err(|error| error.to_string())?
                .await
                .map_err(|error| error.to_string())?;
            Ok(turbo_tasks_fetch::HostResponse {
                status: response.status as u16,
                headers: response
                    .headers
                    .into_iter()
                    .map(|header| (header.name, header.value))
                    .collect(),
                body: response.body.to_vec(),
            })
        });
        future
    })
}
