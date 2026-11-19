#![feature(min_specialization)]
#![feature(arbitrary_self_types)]
#![feature(arbitrary_self_types_pointers)]

mod client;
mod error;
mod response;
// `reqwest`'s API on wasm, served by the host's fetch; see the module docs for why reqwest cannot be
// used there.
#[cfg(target_family = "wasm")]
mod wasm_reqwest;

#[cfg(target_family = "wasm")]
pub use crate::wasm_reqwest::{HostFetch, HostRequest, HostResponse, set_host_fetch};
pub use crate::{
    client::{
        __test_only_reqwest_client_cache_clear, __test_only_reqwest_client_cache_len,
        FetchClientConfig,
    },
    error::{FetchError, FetchErrorKind, FetchIssue},
    response::{FetchResult, HttpResponse, HttpResponseBody},
};
