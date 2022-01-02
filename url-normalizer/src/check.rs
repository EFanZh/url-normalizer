use crate::client_api::{CheckStatus, UiApi, UpdateStatusRequest};
use crate::server_api::{BackendApi, BackendApiResponse, CheckRequest, CheckResponse};
use futures::future::BoxFuture;
use futures::{stream, FutureExt, StreamExt};
use reqwest::{Client, Method, Url};
use std::fmt::Write;
use websocket_rpc::{Handler, RpcClient, ServerApi};

fn normalize_url(url: &Url) -> &str {
    let url_str = url.as_str();
    let path = url.path();

    if path.bytes().any(|c| c != b'/') {
        return url_str;
    }

    let mut trim_length = path.len();

    match url.query() {
        None => {}
        Some("") => trim_length += 1,
        _ => return url_str,
    }

    match url.fragment() {
        None => {}
        Some("") => trim_length += 1,
        _ => return url_str,
    }

    &url_str[..url_str.len() - trim_length]
}

async fn check_url(client: &Client, url: String) -> CheckStatus {
    let mut buffer = String::new();

    if let Some(base) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) {
        let base = base.trim_end_matches('/');
        let mut candidate = String::from("http");

        for method in [Method::HEAD, Method::GET] {
            for s in ["s", ""] {
                for slash in ["", "/"] {
                    candidate.push_str(s);
                    candidate.push_str("://");
                    candidate.push_str(base);
                    candidate.push_str(slash);

                    buffer.clear();

                    match client.request(method.clone(), &candidate).send().await {
                        Ok(response) => {
                            if response.status().is_success() {
                                let normalized_url = normalize_url(response.url());

                                return if normalized_url == url {
                                    CheckStatus::Updated
                                } else {
                                    buffer.push_str(normalized_url);

                                    CheckStatus::Update { url: buffer }
                                };
                            }

                            write!(buffer, "{}", response.status()).unwrap();
                        }
                        Err(error) => {
                            write!(buffer, "{}", error).unwrap();
                        }
                    }

                    candidate.truncate(4);
                }
            }
        }
    } else {
        buffer.push_str("Invalid URL.");
    }

    CheckStatus::Error { message: buffer }
}

async fn check(rpc_client: RpcClient<UiApi>, request: CheckRequest) -> CheckResponse {
    let client = Client::new();

    let mut iter = stream::iter(
        request
            .urls
            .into_iter()
            .enumerate()
            .map(|(i, url)| check_url(&client, url).map(move |status| (i, status))),
    )
    .buffer_unordered(16);

    while let Some((i, status)) = iter.next().await {
        if let Err(error) = rpc_client.call(UpdateStatusRequest { index: i, status }).await {
            tracing::warn!(%error, "Failed to update status.");
        }
    }

    CheckResponse
}

pub struct ServerImpl;

impl Handler for ServerImpl {
    type ClientApi = UiApi;
    type ServerApi = BackendApi;
    type ServerResponseFuture = BoxFuture<'static, <Self::ServerApi as ServerApi>::Response>;

    fn handle(&mut self, rpc_client: RpcClient<UiApi>, request: Self::ServerApi) -> Self::ServerResponseFuture {
        async move {
            match request {
                BackendApi::Check(request) => BackendApiResponse::Check(check(rpc_client, request).await),
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use reqwest::Url;

    #[test]
    fn test_normalize_url() {
        let test_cases = [
            ("https://example.com/", "https://example.com"),
            ("https://example.com/#", "https://example.com"),
            ("https://example.com/#foobar", "https://example.com/#foobar"),
            ("https://example.com/?", "https://example.com"),
            ("https://example.com/?#", "https://example.com"),
            ("https://example.com/?foo=bar", "https://example.com/?foo=bar"),
            ("https://example.com/?foo=bar#", "https://example.com/?foo=bar#"),
            (
                "https://example.com/?foo=bar#foobar",
                "https://example.com/?foo=bar#foobar",
            ),
            ("https://example.com//", "https://example.com"),
            ("https://example.com//#", "https://example.com"),
            ("https://example.com//?", "https://example.com"),
            ("https://example.com//?#", "https://example.com"),
            ("https://example.com//?#foobar", "https://example.com//?#foobar"),
            (
                "https://example.com//?foo=bar#foobar",
                "https://example.com//?foo=bar#foobar",
            ),
            ("https://example.com/a", "https://example.com/a"),
        ];

        for (url, expected) in test_cases {
            let parsed_url = Url::parse(url).unwrap();

            assert_eq!(parsed_url.as_str(), url);
            assert_eq!(super::normalize_url(&parsed_url), expected);
        }
    }
}
