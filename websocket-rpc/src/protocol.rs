use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub trait ClientApi: Serialize {}

pub trait ClientMethod<T>: Into<T>
where
    T: ClientApi,
{
    type Response: DeserializeOwned;
}

pub trait ServerApi: DeserializeOwned {
    type Response: Serialize;
}

pub trait ServerMethod<T>: DeserializeOwned
where
    T: ServerApi,
{
    type Response: Into<T::Response>;
}

pub struct MessageData {
    pub task_id: u64,
    pub data: Box<RawValue>,
}

#[cfg_attr(test, derive(Debug, Eq, PartialEq))]
#[derive(Deserialize, Serialize)]
pub enum MessageType {
    Request,
    Response,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Deserialize, Serialize)]
pub struct Message {
    pub r#type: MessageType,
    pub task_id: u64,
    pub data: Box<RawValue>,
}

impl Message {
    pub fn request(message_data: MessageData) -> Self {
        Self {
            r#type: MessageType::Request,
            task_id: message_data.task_id,
            data: message_data.data,
        }
    }

    pub fn response(message_data: MessageData) -> Self {
        Self {
            r#type: MessageType::Response,
            task_id: message_data.task_id,
            data: message_data.data,
        }
    }

    pub fn split(self) -> (MessageType, MessageData) {
        (
            self.r#type,
            MessageData {
                task_id: self.task_id,
                data: self.data,
            },
        )
    }
}

#[cfg(test)]
impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.r#type == other.r#type && self.task_id == other.task_id && self.data.get() == other.data.get()
    }
}

#[cfg(test)]
impl Eq for Message {}

#[cfg(test)]
mod tests {
    use super::{Message, MessageData};
    use serde_json::value::RawValue;

    #[test]
    fn test_deserialize_request_message() {
        assert_eq!(
            serde_json::from_str::<Message>(
                r#"{
                    "type": "Request",
                    "task_id": 2,
                    "data": { "method": "Foo", "argument": ["abc"] }
                }"#,
            )
            .unwrap(),
            Message::request(MessageData {
                task_id: 2,
                data: RawValue::from_string(String::from(r#"{ "method": "Foo", "argument": ["abc"] }"#)).unwrap(),
            }),
        );
    }

    #[test]
    fn test_deserialize_response_message() {
        assert_eq!(
            serde_json::from_str::<Message>(
                r#"{
                    "type": "Response",
                    "task_id": 2,
                    "data": null
                }"#,
            )
            .unwrap(),
            Message::response(MessageData {
                task_id: 2,
                data: RawValue::from_string(String::from(r#"null"#)).unwrap(),
            }),
        );
    }

    #[test]
    fn test_serialize_request_message() {
        assert_eq!(
            serde_json::to_value(&Message::request(MessageData {
                task_id: 2,
                data: RawValue::from_string(String::from(r#"{ "method": "Foo", "argument": ["abc"] }"#)).unwrap(),
            }))
            .unwrap(),
            serde_json::json!({
                "type": "Request",
                "task_id": 2,
                "data": {
                    "method": "Foo",
                    "argument": ["abc"],
                },
            }),
        );
    }

    #[test]
    fn test_serialize_response_message() {
        assert_eq!(
            serde_json::to_value(&Message::response(MessageData {
                task_id: 2,
                data: RawValue::from_string(String::from(r#"{ "foo": "bar" }"#)).unwrap(),
            }))
            .unwrap(),
            serde_json::json!({
                "type": "Response",
                "task_id": 2,
                "data": {
                    "foo": "bar",
                },
            }),
        );
    }
}
