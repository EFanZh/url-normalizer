use crate::server_messages::CheckResponse;
use serde::Deserialize;
use websocket_rpc::ServerMethod;

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
#[serde(tag = "method", content = "argument")]
pub enum ClientRequestData {
    Check(CheckRequest),
}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub struct UpdateStatusResponse;

#[cfg(test)]
mod tests {
    use super::{CheckRequest, ClientRequestData, UpdateStatusResponse};

    #[test]
    fn test_deserialize_client_request_data() {
        assert_eq!(
            serde_json::from_str::<ClientRequestData>(
                r#"{
                "method": "Check",
                "data": {
                    "urls": ["abc", "def"]
                }
            }"#
            )
            .unwrap(),
            ClientRequestData::Check(CheckRequest {
                urls: ["abc", "def"].into_iter().map(str::to_string).collect()
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
