use crate::client_messages::UpdateStatusResponse;
use serde::Serialize;
use websocket_rpc::ClientMethod;

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

#[derive(Serialize, derive_more::From)]
#[serde(tag = "method", content = "argument")]
pub enum ServerRequestData {
    UpdateStatus(UpdateStatusRequest),
}

impl ClientMethod<ServerRequestData> for UpdateStatusRequest {
    type Output = UpdateStatusResponse;
}

#[derive(Serialize)]
pub struct CheckResponse;

#[derive(Serialize)]
pub enum ServerResponseData {
    Check(CheckResponse),
}

#[cfg(test)]
mod tests {
    use super::{CheckResponse, CheckStatus, ServerRequestData, UpdateStatusRequest};

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
    fn test_serialize_server_request_data() {
        assert_eq!(
            serde_json::to_value(ServerRequestData::UpdateStatus(UpdateStatusRequest {
                index: 3,
                status: CheckStatus::Updated,
            }))
            .unwrap(),
            serde_json::json!({
                "method": "UpdateStatus",
                "data": {
                    "index": 3,
                    "status": "Updated",
                }
            })
        );
    }

    #[test]
    fn test_serialize_check_response() {
        assert_eq!(serde_json::to_value(CheckResponse).unwrap(), serde_json::json!(null));
    }
}
