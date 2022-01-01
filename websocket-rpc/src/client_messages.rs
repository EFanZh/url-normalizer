use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub trait ServerMethod: DeserializeOwned {
    type Output: Serialize;
}

#[cfg_attr(test, derive(Debug))]
#[derive(Deserialize)]
pub struct ClientRequest {
    pub task_id: u64,
    pub data: Box<RawValue>,
}

#[cfg(test)]
impl PartialEq for ClientRequest {
    fn eq(&self, other: &Self) -> bool {
        self.task_id == other.task_id && self.data.get() == other.data.get()
    }
}

#[cfg(test)]
impl Eq for ClientRequest {}

#[cfg_attr(test, derive(Debug))]
#[derive(Deserialize)]
pub struct ClientResponse {
    pub task_id: u64,
    pub data: Box<RawValue>,
}

#[cfg(test)]
impl PartialEq for ClientResponse {
    fn eq(&self, other: &Self) -> bool {
        self.task_id == other.task_id && self.data.get() == other.data.get()
    }
}

#[cfg(test)]
impl Eq for ClientResponse {}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize)]
pub enum ClientMessageType {
    Request,
    Response,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Deserialize)]
pub struct ClientMessage {
    pub r#type: ClientMessageType,
    pub task_id: u64,
    pub data: Box<RawValue>,
}

#[cfg(test)]
impl PartialEq for ClientMessage {
    fn eq(&self, other: &Self) -> bool {
        self.task_id == other.task_id && self.data.get() == other.data.get()
    }
}

#[cfg(test)]
impl Eq for ClientMessage {}

#[cfg(test)]
mod tests {
    use super::{ClientMessage, ClientMessageType, ClientRequest};
    use serde_json::value::RawValue;

    #[test]
    fn test_deserialize_client_request() {
        assert_eq!(
            serde_json::from_str::<ClientRequest>(
                r#"{
                    "task_id": 2,
                    "data": { "urls": ["abc", "def"] }
                }"#,
            )
            .unwrap(),
            ClientRequest {
                task_id: 2,
                data: RawValue::from_string(String::from(r#"{ "urls": ["abc", "def"] }"#)).unwrap(),
            }
        );
    }

    #[test]
    fn test_deserialize_client_request_message() {
        assert_eq!(
            serde_json::from_str::<ClientMessage>(
                r#"{
                    "type": "Request",
                    "task_id": 2,
                    "data": { "urls": ["abc", "def"] }
                }"#,
            )
            .unwrap(),
            ClientMessage {
                r#type: ClientMessageType::Request,
                task_id: 2,
                data: RawValue::from_string(String::from(r#"{ "urls": ["abc", "def"] }"#)).unwrap(),
            }
        );
    }

    #[test]
    fn test_deserialize_client_response_message() {
        assert_eq!(
            serde_json::from_str::<ClientMessage>(
                r#"{
                    "type": "Response",
                    "task_id": 2,
                    "data": null
                }"#
            )
            .unwrap(),
            ClientMessage {
                r#type: ClientMessageType::Response,
                task_id: 2,
                data: RawValue::from_string(String::from(r#"null"#)).unwrap(),
            }
        );
    }
}
