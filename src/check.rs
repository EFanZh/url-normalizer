use crate::client_messages::CheckRequest;
use crate::connection::ClientApi;
use crate::server_messages::{CheckResponse, CheckStatus, UpdateStatusRequest};
use futures::{stream, FutureExt, StreamExt};
use reqwest::Client;
use std::fmt::Write;

async fn check_url(client: &Client, url: String) -> CheckStatus {
    let mut buffer = String::new();

    if let Some(base) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) {
        let base = base.trim_end_matches('/');
        let mut candidate = String::from("http");

        for s in ["s", ""] {
            for slash in ["", "/"] {
                candidate.push_str(s);
                candidate.push_str("://");
                candidate.push_str(base);
                candidate.push_str(slash);

                buffer.clear();

                match client.head(&candidate).send().await {
                    Ok(response) => {
                        if response.status().is_success() {
                            let response_url = response.url();
                            let response_url_str = response_url.as_str();

                            let normalized_url = if response_url.path() == "/" {
                                &response_url_str[..response_url_str.len() - 1]
                            } else {
                                response_url_str
                            };

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
    } else {
        buffer.push_str("Invalid URL.");
    }

    CheckStatus::Error { message: buffer }
}

pub async fn check(client_api: ClientApi, request: CheckRequest) -> CheckResponse {
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
        if let Err(error) = client_api.call(UpdateStatusRequest { index: i, status }).await {
            tracing::warn!("{}", error);
        }
    }

    CheckResponse
}
