use crate::client_messages::{ClientResponseData, UpdateStatusResponse};
use derive_more::{From, TryInto};
use serde::Serialize;
use std::num::NonZeroU64;

pub trait ClientMethod: Into<ServerRequestData> + Serialize {
    type Output: TryFrom<ClientResponseData, Error = &'static str>;
}

#[derive(Serialize)]
#[serde(tag = "status")]
pub enum CheckStatus {
    Updated,
    Update { url: String },
    Error { message: String },
}

#[derive(Serialize)]
pub struct UpdateStatusRequest {
    pub index: usize,
    #[serde(flatten)]
    pub status: CheckStatus,
}

impl ClientMethod for UpdateStatusRequest {
    type Output = UpdateStatusResponse;
}

#[derive(From, Serialize)]
#[serde(tag = "method", content = "data")]
pub enum ServerRequestData {
    UpdateStatus(UpdateStatusRequest),
}

#[derive(Serialize)]
pub struct ServerRequest {
    pub task_id: NonZeroU64,
    #[serde(flatten)]
    pub data: ServerRequestData,
}

#[derive(Serialize)]
pub struct CheckResponse;

#[derive(Serialize, TryInto)]
#[serde(untagged)]
pub enum ServerResponseData {
    Check(CheckResponse),
}

#[derive(Serialize)]
pub struct ServerResponse {
    pub task_id: u64,
    pub data: ServerResponseData,
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    Request(Box<ServerRequest>),
    Response(Box<ServerResponse>),
}

#[cfg(test)]
mod tests {
    use super::{
        CheckResponse, CheckStatus, ServerMessage, ServerRequest, ServerRequestData,
        ServerResponse, ServerResponseData, UpdateStatusRequest,
    };
    use std::num::NonZeroU64;

    #[test]
    fn test_serialize_check_status() {
        assert_eq!(
            serde_json::to_value(&CheckStatus::Updated).unwrap(),
            serde_json::json!({
                "status": "Updated"
            })
        );

        assert_eq!(
            serde_json::to_value(&CheckStatus::Update {
                url: String::from("abc")
            })
            .unwrap(),
            serde_json::json!({
                "status": "Update",
                "url": "abc",
            })
        );
    }

    #[test]
    fn test_serialize_server_request_message() {
        assert_eq!(
            serde_json::to_value(&ServerMessage::Request(Box::new(ServerRequest {
                task_id: NonZeroU64::new(2).unwrap(),
                data: ServerRequestData::UpdateStatus(UpdateStatusRequest {
                    index: 3,
                    status: CheckStatus::Updated,
                })
            })))
            .unwrap(),
            serde_json::json!({
                "type": "Request",
                "task_id": 2,
                "method": "UpdateStatus",
                "data": {
                    "index": 3,
                    "status": "Updated",
                }
            })
        );
    }

    #[test]
    fn test_serialize_server_response_message() {
        assert_eq!(
            serde_json::to_value(&ServerMessage::Response(Box::new(ServerResponse {
                task_id: 2,
                data: ServerResponseData::Check(CheckResponse),
            })))
            .unwrap(),
            serde_json::json!({
                "type": "Response",
                "task_id": 2,
                "data": null,
            })
        );
    }
}
