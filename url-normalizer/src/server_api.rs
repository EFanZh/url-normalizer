use serde::{Deserialize, Serialize};
use websocket_rpc::{ServerApi, ServerMethod};

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub struct CheckRequest {
    pub urls: Vec<String>,
}

#[derive(Serialize)]
pub struct CheckResponse;

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
#[serde(tag = "method", content = "argument")]
pub enum ClientRequestData {
    Check(CheckRequest),
}

impl ServerApi for ClientRequestData {
    type Response = ServerResponseData;
}

impl ServerMethod<ClientRequestData> for CheckRequest {
    type Output = CheckResponse;
}

#[derive(Serialize, derive_more::From)]
pub enum ServerResponseData {
    Check(CheckResponse),
}

#[cfg(test)]
mod tests {
    use super::{CheckRequest, CheckResponse, ClientRequestData};

    #[test]
    fn test_serialize_check_response() {
        assert_eq!(serde_json::to_value(CheckResponse).unwrap(), serde_json::json!(null));
    }

    #[test]
    fn test_deserialize_client_request_data() {
        assert_eq!(
            serde_json::from_str::<ClientRequestData>(
                r#"{
                    "method": "Check",
                    "argument": {
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
}
