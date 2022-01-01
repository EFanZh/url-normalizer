use serde::{Deserialize, Serialize};
use websocket_rpc::{ServerApi, ServerMethod};

// Server API.

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
#[serde(tag = "method", content = "argument")]
pub enum BackendApi {
    Check(CheckRequest),
}

#[derive(Serialize, derive_more::From)]
#[serde(untagged)]
pub enum BackendApiResponse {
    Check(CheckResponse),
}

impl ServerApi for BackendApi {
    type Response = BackendApiResponse;
}

// Check.

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub struct CheckRequest {
    pub urls: Vec<String>,
}

#[derive(Serialize)]
pub struct CheckResponse;

impl ServerMethod<BackendApi> for CheckRequest {
    type Response = CheckResponse;
}

#[cfg(test)]
mod tests {
    use super::{BackendApi, CheckRequest, CheckResponse};
    use crate::server_api::BackendApiResponse;

    #[test]
    fn test_deserialize_backend_api() {
        assert_eq!(
            serde_json::from_str::<BackendApi>(
                r#"{
                    "method": "Check",
                    "argument": {
                        "urls": ["abc", "def"]
                    }
                }"#
            )
            .unwrap(),
            BackendApi::Check(CheckRequest {
                urls: ["abc", "def"].into_iter().map(str::to_string).collect()
            })
        );
    }

    #[test]
    fn test_serialize_backend_api_response() {
        assert_eq!(
            serde_json::to_value(BackendApiResponse::Check(CheckResponse)).unwrap(),
            serde_json::json!(null)
        );
    }

    #[test]
    fn test_serialize_check_response() {
        assert_eq!(serde_json::to_value(CheckResponse).unwrap(), serde_json::json!(null));
    }
}
