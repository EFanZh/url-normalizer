use serde::{Deserialize, Serialize};
use websocket_rpc::{ClientApi, ClientMethod};

// Client API.

#[derive(Serialize, derive_more::From)]
#[serde(tag = "method", content = "argument")]
pub enum UiApi {
    UpdateStatus(UpdateStatusRequest),
}

impl ClientApi for UiApi {}

// UpdateStatus.

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

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub struct UpdateStatusResponse;

impl ClientMethod<UiApi> for UpdateStatusRequest {
    type Response = UpdateStatusResponse;
}

#[cfg(test)]
mod tests {
    use super::{CheckStatus, UiApi, UpdateStatusRequest, UpdateStatusResponse};

    #[test]
    fn test_serialize_server_request_data() {
        assert_eq!(
            serde_json::to_value(UiApi::UpdateStatus(UpdateStatusRequest {
                index: 3,
                status: CheckStatus::Updated,
            }))
            .unwrap(),
            serde_json::json!({
                "method": "UpdateStatus",
                "argument": {
                    "index": 3,
                    "status": "Updated",
                }
            })
        );
    }

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
    fn test_deserialize_update_status_response() {
        assert_eq!(
            serde_json::from_str::<UpdateStatusResponse>("null").unwrap(),
            UpdateStatusResponse
        );
    }
}
