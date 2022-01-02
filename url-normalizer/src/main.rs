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
use hyper::server::conn::AddrStream;
use hyper::{service, upgrade, Body, Method, Request, Response, Server, StatusCode};
use reqwest::Client;
use std::convert::Infallible;
use std::future::{self, Ready};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio_tungstenite::tungstenite;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::handshake::server;
use tracing::{Instrument, Span};
use websocket_rpc::Connection;

mod check;
mod client_api;
mod server_api;

fn make_response<T>(status: StatusCode, body: T) -> Response<T> {
    let mut response = Response::new(body);

    *response.status_mut() = status;

    response
}

async fn handle_websocket_session(client: Client, request: Request<Body>) {
    match upgrade::on(request).await {
        Ok(upgraded) => {
            tracing::info!("WebSocket session started.");

            match Connection::new(upgraded, ServerImpl::new(client)).await.await {
                Ok(()) => tracing::info!("WebSocket session ended."),
                Err(error) => tracing::warn!(%error, "WebSocket session ended with error."),
            }
        }
        Err(error) => tracing::error!(%error, "Failed to upgrade connection."),
    }
}

fn handle_request(client: &Client, request: Request<Body>) -> Response<Body> {
    let uri = request.uri();

    tracing::info!(method = %request.method(), uri = %uri, "Received HTTP request.");

    match uri.path() {
        "/" => match *request.method() {
            Method::GET => make_response(StatusCode::OK, Body::from(include_str!("../ui/index.html"))),
            _ => make_response(StatusCode::METHOD_NOT_ALLOWED, Body::empty()),
        },
        "/api" => match server::create_response_with_body(&request, Body::empty) {
            Ok(response) => {
                tokio::spawn(
                    handle_websocket_session(client.clone(), request).instrument(tracing::info_span!("Session")),
                );

                response
            }
            Err(tungstenite::Error::Protocol(ProtocolError::WrongHttpMethod)) => {
                make_response(StatusCode::METHOD_NOT_ALLOWED, Body::empty())
            }
            Err(tungstenite::Error::Protocol(_)) => make_response(StatusCode::BAD_REQUEST, Body::empty()),
            Err(_) => make_response(StatusCode::INTERNAL_SERVER_ERROR, Body::empty()),
        },
        _ => make_response(StatusCode::NOT_FOUND, Body::empty()),
    }
}

fn make_future<T>(value: T) -> Ready<Result<T, Infallible>> {
    future::ready(Ok(value))
}

struct Counter(usize);

impl Counter {
    fn next(&mut self) -> usize {
        let result = self.0;

        self.0 += 1;

        result
    }
}

async fn main_inner() -> anyhow::Result<()> {
    const CHROME_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/96.0.4664.110 Safari/537.36";

    let mut connection_counter = Counter(0);
    let client = Client::builder().user_agent(CHROME_USER_AGENT).build()?;

    let server = Server::bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))).serve(
        service::make_service_fn(move |connection: &AddrStream| {
            let connection_id = connection_counter.next();

            tracing::info_span!("Connection", id = connection_id, remote = %connection.remote_addr()).in_scope(|| {
                let connection_span = Span::current();
                let mut request_counter = Counter(0);
                let client = client.clone();

                make_future(service::service_fn(move |request| {
                    connection_span.in_scope(|| {
                        let request_id = request_counter.next();

                        tracing::info_span!("Request", id = request_id)
                            .in_scope(|| make_future(handle_request(&client, request)))
                    })
                }))
            })
        }),
    );

    let server_url = format!("http://{}", server.local_addr());

    tracing::info!(server_url = %server_url.as_str(), "Server started.",);

    open::that(server_url)?;

    server.await?;

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    main_inner().await
}
