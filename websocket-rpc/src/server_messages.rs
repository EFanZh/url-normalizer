use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::value::RawValue;

pub trait ClientMethod<T>: Into<T>
where
    T: Serialize,
{
    type Output: DeserializeOwned;
}

#[derive(Serialize)]
pub struct ServerRequest {
    pub task_id: u64,
    pub data: Box<RawValue>,
}

#[derive(Serialize)]
pub struct ServerResponse {
    pub task_id: u64,
    pub data: Box<RawValue>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    Request(ServerRequest),
    Response(ServerResponse),
}

#[cfg(test)]
mod tests {
    use super::{ServerMessage, ServerRequest, ServerResponse};
    use serde_json::value::RawValue;

    #[test]
    fn test_serialize_server_request_message() {
        assert_eq!(
            serde_json::to_value(&ServerMessage::Request(ServerRequest {
                task_id: 2,
                data: RawValue::from_string(String::from("456")).unwrap(),
            }))
            .unwrap(),
            serde_json::json!({
                "type": "Request",
                "task_id": 2,
                "data": 456
            })
        );
    }

    #[test]
    fn test_serialize_server_response_message() {
        assert_eq!(
            serde_json::to_value(&ServerMessage::Response(ServerResponse {
                task_id: 2,
                data: RawValue::from_string(String::from("789")).unwrap(),
            }))
            .unwrap(),
            serde_json::json!({
                "type": "Response",
                "task_id": 2,
                "data": 789,
            })
        );
    }
}
