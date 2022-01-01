#![warn(
    explicit_outlives_requirements,
    macro_use_extern_crate,
    meta_variable_misuse,
    missing_abi,
    // missing_docs,
    noop_method_call,
    pointer_structural_match,
    single_use_lifetimes,
    trivial_casts,
    trivial_numeric_casts,
    unsafe_code,
    unsafe_op_in_unsafe_fn,
    unused_crate_dependencies,
    unused_extern_crates,
    unused_import_braces,
    unused_lifetimes,
    unused_qualifications,
    variant_size_differences,
    // clippy::cargo_common_metadata,
    clippy::clone_on_ref_ptr,
    clippy::cognitive_complexity,
    clippy::create_dir,
    clippy::dbg_macro,
    clippy::debug_assert_with_mut_call,
    clippy::empty_line_after_outer_attr,
    clippy::fallible_impl_from,
    clippy::filetype_is_file,
    clippy::float_cmp_const,
    clippy::get_unwrap,
    clippy::if_then_some_else_none,
    clippy::imprecise_flops,
    clippy::let_underscore_must_use,
    clippy::lossy_float_literal,
    clippy::multiple_inherent_impl,
    clippy::mutex_integer,
    clippy::nonstandard_macro_braces,
    clippy::panic_in_result_fn,
    clippy::path_buf_push_overwrite,
    clippy::pedantic,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::rc_buffer,
    clippy::rc_mutex,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::string_lit_as_bytes,
    clippy::string_to_string,
    clippy::suboptimal_flops,
    clippy::suspicious_operation_groupings,
    clippy::todo,
    clippy::trivial_regex,
    clippy::unimplemented,
    clippy::unnecessary_self_imports,
    clippy::unneeded_field_pattern,
    clippy::use_debug,
    clippy::use_self,
    clippy::useless_let_if_seq,
    clippy::useless_transmute,
    clippy::verbose_file_reads,
    // clippy::wildcard_dependencies,
)]
#![allow(clippy::non_ascii_literal, clippy::wildcard_imports)] // https://github.com/tokio-rs/tracing/pull/1806.

use crate::check::ServerImpl;
use hyper::header::HeaderValue;
use hyper::{header, service, upgrade, Body, Method, Request, Response, Server, StatusCode};
use std::convert::Infallible;
use std::future;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio_tungstenite::tungstenite::handshake;
use tracing_subscriber::util::SubscriberInitExt;
use websocket_rpc::Connection;

mod check;
mod client_api;
mod server_api;

async fn handle_request(mut request: Request<Body>) -> anyhow::Result<Response<Body>> {
    let uri = request.uri();

    tracing::info!(method = %request.method(), uri = %uri, "Received HTTP request.");

    match (request.method(), uri.path()) {
        (&Method::GET, "/") => {
            let mut response = Response::new(Body::from(include_str!("../ui/index.html")));

            *response.status_mut() = StatusCode::OK;

            Ok(response)
        }
        (&Method::GET, "/api") => {
            let mut response = Response::new(Body::empty());

            if let (Some(connection), Some(upgrade), Some(key)) = (
                request.headers_mut().remove(header::CONNECTION),
                request.headers_mut().remove(header::UPGRADE),
                request.headers_mut().remove(header::SEC_WEBSOCKET_KEY),
            ) {
                tokio::spawn(async {
                    match upgrade::on(request).await {
                        Ok(upgraded) => {
                            tracing::info!("Session started.");

                            match Connection::new(upgraded, ServerImpl).await.await {
                                Ok(()) => tracing::info!("Session ended."),
                                Err(error) => tracing::warn!(%error, "Session ended with error."),
                            }
                        }
                        Err(error) => tracing::error!(%error, "Failed to upgrade connection."),
                    }
                });

                *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;

                response.headers_mut().extend([
                    (header::CONNECTION, connection),
                    (header::UPGRADE, upgrade),
                    (
                        header::SEC_WEBSOCKET_ACCEPT,
                        HeaderValue::from_str(&handshake::derive_accept_key(key.as_bytes())).unwrap(),
                    ),
                ]);
            } else {
                *response.status_mut() = StatusCode::BAD_REQUEST;
            }

            Ok(response)
        }
        _ => {
            let mut response = Response::new(Body::empty());

            *response.status_mut() = StatusCode::NOT_FOUND;

            Ok(response)
        }
    }
}

async fn main_inner() -> anyhow::Result<()> {
    let server = Server::bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))).serve(
        service::make_service_fn(|_| future::ready(Ok::<_, Infallible>(service::service_fn(handle_request)))),
    );

    let server_url = format!("http://{}", server.local_addr());

    tracing::info!(server_url = %server_url.as_str(), "Server started.",);

    open::that(server_url)?;

    server.await?;

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().finish().init();

    main_inner().await
}
