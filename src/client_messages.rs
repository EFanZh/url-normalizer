use crate::server_messages::CheckResponse;
use derive_more::TryInto;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

#[allow(single_use_lifetimes)]
pub trait ServerMethod: for<'a> Deserialize<'a> {
    type Output: Serialize;
}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub struct CheckRequest {
    pub urls: Vec<String>,
}

impl ServerMethod for CheckRequest {
    type Output = CheckResponse;
}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
#[serde(tag = "method", content = "data")]
pub enum ClientRequestData {
    Check(CheckRequest),
}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub struct ClientRequest {
    pub task_id: u64,
    #[serde(flatten)]
    pub data: ClientRequestData,
}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub struct UpdateStatusResponse;

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize, TryInto)]
#[serde(untagged)]
pub enum ClientResponseData {
    UpdateStatus(UpdateStatusResponse),
}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub struct ClientResponse {
    pub task_id: NonZeroU64,
    pub data: ClientResponseData,
}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    Request(Box<ClientRequest>),
    Response(Box<ClientResponse>),
}

#[cfg(test)]
mod tests {
    use super::{
        CheckRequest, ClientMessage, ClientRequest, ClientRequestData, ClientResponse, ClientResponseData,
        UpdateStatusResponse,
    };
    use std::num::NonZeroU64;

    #[test]
    fn test_deserialize_client_request_message() {
        assert_eq!(
            serde_json::from_value::<ClientMessage>(serde_json::json!({
                "type": "Request",
                "task_id": 2,
                "method": "Check",
                "data": {
                    "urls": ["abc", "def"]
                }
            }))
            .unwrap(),
            ClientMessage::Request(Box::new(ClientRequest {
                task_id: 2,
                data: ClientRequestData::Check(CheckRequest {
                    urls: ["abc", "def"].into_iter().map(str::to_string).collect()
                })
            }))
        );
    }

    #[test]
    fn test_deserialize_client_response_message() {
        assert_eq!(
            serde_json::from_value::<ClientMessage>(serde_json::json!({
                "type": "Response",
                "task_id": 2,
                "data": null
            }))
            .unwrap(),
            ClientMessage::Response(Box::new(ClientResponse {
                task_id: NonZeroU64::new(2).unwrap(),
                data: ClientResponseData::UpdateStatus(UpdateStatusResponse)
            }))
        );
    }
}
