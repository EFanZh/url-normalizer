use crate::client_messages::{ClientMessage, ClientMessageType, ClientRequest, ClientResponse};
use crate::server_messages::{ClientMethod, ServerMessage, ServerRequest, ServerResponse};
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use futures::future::Either;
use futures::{Future, FutureExt, SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::value::{self, RawValue};
use std::collections::HashMap;
use std::future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::WebSocketStream;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to deserialize response: {0}.")]
    Deserialization(serde_json::Error),
    #[error("Failed to serialize request: {0}.")]
    Serialization(serde_json::Error),
    #[error("Client connection closed.")]
    ConnectionClosed,
}

struct ServerRequest2 {
    data: Box<RawValue>,
    sender: oneshot::Sender<serde_json::Result<Box<RawValue>>>,
}

enum ServerMessage2 {
    Request(ServerRequest2),
    Response(ServerResponse),
}

pub struct ClientApi<U>
where
    U: Serialize,
{
    sender: UnboundedSender<ServerMessage2>,
    _phantom: PhantomData<U>,
}

impl<T> ClientApi<T>
where
    T: Serialize,
{
    pub fn call<R>(&self, request: R) -> impl Future<Output = Result<R::Output, Error>>
    where
        R: ClientMethod<T>,
    {
        match value::to_raw_value(&<R as Into<T>>::into(request)).map_err(Error::Serialization) {
            Ok(raw_value) => {
                let (sender, receiver) = oneshot::channel();

                match self.sender.unbounded_send(ServerMessage2::Request(ServerRequest2 {
                    data: raw_value,
                    sender,
                })) {
                    Ok(()) => Either::Left(receiver.map(|result| {
                        match result {
                            Ok(result) => result
                                .map_err(Error::Serialization)
                                .and_then(|data| serde_json::from_str(data.get()).map_err(Error::Deserialization)),
                            Err(oneshot::Canceled) => Err(Error::ConnectionClosed),
                        }
                    })),
                    Err(_) => Either::Right(future::ready(Err(Error::ConnectionClosed))),
                }
            }
            Err(_) => Either::Right(future::ready(Err(Error::ConnectionClosed))),
        }
    }
}

pub trait Handler: Unpin {
    type ClientRequest: Serialize;
    type Request: DeserializeOwned;
    type Response: Serialize + Send;
    type ResponseFuture: Future<Output = Self::Response> + Send + 'static;

    fn handle(&mut self, client_api: ClientApi<Self::ClientRequest>, request: Self::Request) -> Self::ResponseFuture;
}

fn handle_client_response(
    requests: &mut HashMap<u64, oneshot::Sender<serde_json::Result<Box<RawValue>>>>,
    response: ClientResponse,
) {
    if let Some(sender) = requests.remove(&response.task_id) {
        match sender.send(Ok(response.data)) {
            Ok(()) => {}
            Err(_) => tracing::warn!(response.task_id, "Future has been canceled for task."),
        }
    } else {
        tracing::warn!(response.task_id, "Unexpected client response task ID.");
    }
}

fn next_task_id(task_id: &mut u64) -> u64 {
    let result = *task_id;

    *task_id = task_id.wrapping_add(1);

    result
}

struct ClientToServer<'a, T> {
    client: &'a mut WebSocketStream<T>,
    requests: &'a mut HashMap<u64, oneshot::Sender<serde_json::Result<Box<RawValue>>>>,
}

impl<T> ClientToServer<'_, T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_client_to_server_partial(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Option<tungstenite::Result<Option<ClientRequest>>>> {
        self.client.poll_next_unpin(cx).map_ok(|message| {
            tracing::info!(content = ?message, "Got client message.");

            match message {
                Message::Text(message) => match serde_json::from_str(&message) {
                    Ok(ClientMessage {
                        r#type: ClientMessageType::Request,
                        task_id,
                        data,
                    }) => return Some(ClientRequest { task_id, data }),
                    Ok(ClientMessage {
                        r#type: ClientMessageType::Response,
                        task_id,
                        data,
                    }) => handle_client_response(self.requests, ClientResponse { task_id, data }),
                    Err(error) => tracing::warn!(%error, "Failed to deserialize message."),
                },
                Message::Binary(_) => tracing::warn!("Unexpected binary message."),
                Message::Ping(_) | Message::Pong(_) | Message::Close(_) => {}
            }

            None
        })
    }

    fn poll_client_to_server(&mut self, cx: &mut Context) -> Poll<Option<tungstenite::Result<()>>> {
        self.poll_client_to_server_partial(cx).map_ok(drop)
    }
}

struct ServerToClient<'a, T> {
    client: &'a mut WebSocketStream<T>,
    receiver: &'a mut UnboundedReceiver<ServerMessage2>,
}

impl<T> ServerToClient<'_, T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn handle_server_response(&mut self, response: ServerResponse) -> tungstenite::Result<()> {
        match serde_json::to_string(&ServerMessage::Response(response)) {
            Ok(message) => {
                tracing::info!(content = message.as_str(), "Sending request to request.");

                self.client.start_send_unpin(Message::Text(message))?;
            }
            Err(error) => tracing::error!(%error, "Failed to serialize response message."),
        }

        Ok(())
    }

    fn poll_server_to_client_partial(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Option<tungstenite::Result<Option<ServerRequest2>>>> {
        match futures::ready!(self.client.poll_ready_unpin(cx)) {
            Ok(()) => match futures::ready!(self.receiver.poll_next_unpin(cx)) {
                None => self.client.poll_close_unpin(cx)?.map(|()| None),
                Some(message) => Poll::Ready(Some(Ok(match message {
                    ServerMessage2::Request(request) => Some(request),
                    ServerMessage2::Response(response) => {
                        self.handle_server_response(response)?;

                        None
                    }
                }))),
            },
            Err(tungstenite::Error::ConnectionClosed) => Poll::Ready(None),
            Err(error) => Poll::Ready(Some(Err(error))),
        }
    }

    fn poll_server_to_client(&mut self, cx: &mut Context) -> Poll<Option<tungstenite::Result<()>>> {
        self.poll_server_to_client_partial(cx).map_ok(drop)
    }
}

struct Connected<'a, T, H> {
    client: &'a mut WebSocketStream<T>,
    handler: &'a mut H,
    sender: &'a mut UnboundedSender<ServerMessage2>,
    receiver: &'a mut UnboundedReceiver<ServerMessage2>,
    requests: &'a mut HashMap<u64, oneshot::Sender<serde_json::Result<Box<RawValue>>>>,
    task_id: &'a mut u64,
}

impl<T, H> Connected<'_, T, H>
where
    T: AsyncRead + AsyncWrite + Unpin,
    H: Handler,
{
    fn handle_client_request(&mut self, client_request: &ClientRequest) {
        fn send_response(sender: &UnboundedSender<ServerMessage2>, task_id: u64, response: &impl Serialize) {
            let data =
                value::to_raw_value(response).unwrap_or_else(|error| value::to_raw_value(&error.to_string()).unwrap());

            let response = ServerMessage2::Response(ServerResponse { task_id, data });

            drop(sender.unbounded_send(response)); // Ignore send error;
        }

        let client_api_sender = self.sender.clone();

        match serde_json::from_str(client_request.data.get()) {
            Ok(request) => {
                let server_response_sender = self.sender.clone();
                let task_id = client_request.task_id;

                tokio::spawn(
                    self.handler
                        .handle(
                            ClientApi {
                                sender: client_api_sender,
                                _phantom: PhantomData,
                            },
                            request,
                        )
                        .map(move |response| send_response(&server_response_sender, task_id, &response)),
                );
            }
            Err(error) => send_response(self.sender, client_request.task_id, &error.to_string()),
        }
    }

    fn poll_client_to_server(&mut self, cx: &mut Context) -> Poll<Option<tungstenite::Result<()>>> {
        self.as_client_to_server()
            .poll_client_to_server_partial(cx)
            .map_ok(|request| match request {
                None => {}
                Some(request) => self.handle_client_request(&request),
            })
    }

    fn handle_server_request(&mut self, request: ServerRequest2) -> tungstenite::Result<()> {
        let task_id = next_task_id(self.task_id);

        match serde_json::to_string(&ServerMessage::Request(ServerRequest {
            task_id,
            data: request.data,
        })) {
            Ok(message) => {
                tracing::info!(content = message.as_str(), "Sending request to request.");

                self.client.start_send_unpin(Message::Text(message)).map(|()| {
                    self.requests.insert(task_id, request.sender);
                })
            }
            Err(error) => {
                drop(request.sender.send(Err(error))); // Ignore send error.

                Ok(())
            }
        }
    }

    fn poll_server_to_client(&mut self, cx: &mut Context) -> Poll<Option<tungstenite::Result<()>>> {
        self.as_server_to_client()
            .poll_server_to_client_partial(cx)
            .map(|request| {
                request.map(|result| match result? {
                    None => Ok(()),
                    Some(request) => self.handle_server_request(request),
                })
            })
    }

    fn as_client_to_server(&mut self) -> ClientToServer<T> {
        ClientToServer {
            client: self.client,
            requests: self.requests,
        }
    }

    fn as_server_to_client(&mut self) -> ServerToClient<T> {
        ServerToClient {
            client: self.client,
            receiver: self.receiver,
        }
    }
}

enum State<'a, T, H> {
    Connected(Connected<'a, T, H>),
    ClientToServer(ClientToServer<'a, T>),
    ServerToClient(ServerToClient<'a, T>),
    Done,
}

struct ClientToServerData<H> {
    sender: UnboundedSender<ServerMessage2>,
    requests: HashMap<u64, oneshot::Sender<serde_json::Result<Box<RawValue>>>>,
    task_id: u64,
    handler: H,
}

struct ServerToClientData {
    receiver: UnboundedReceiver<ServerMessage2>,
}

pub struct Connection<T, H>
where
    T: AsyncRead + AsyncWrite + Unpin,
    H: Handler,
{
    client: WebSocketStream<T>,
    client_to_server: Option<ClientToServerData<H>>,
    server_to_client: Option<ServerToClientData>,
}

impl<T, H> Connection<T, H>
where
    T: AsyncRead + AsyncWrite + Unpin,
    H: Handler,
{
    pub async fn new(inner: T, handler: H) -> Self {
        let (sender, receiver) = mpsc::unbounded();

        Self {
            client: WebSocketStream::from_raw_socket(inner, Role::Server, None).await,
            client_to_server: Some(ClientToServerData {
                sender,
                requests: HashMap::new(),
                task_id: 0,
                handler,
            }),
            server_to_client: Some(ServerToClientData { receiver }),
        }
    }

    fn state(&mut self) -> State<T, H> {
        match (&mut self.client_to_server, &mut self.server_to_client) {
            (None, None) => State::Done,
            (None, Some(ServerToClientData { receiver })) => State::ServerToClient(ServerToClient {
                client: &mut self.client,
                receiver,
            }),
            (Some(ClientToServerData { requests, .. }), None) => State::ClientToServer(ClientToServer {
                client: &mut self.client,
                requests,
            }),
            (
                Some(ClientToServerData {
                    sender,
                    requests,
                    task_id,
                    handler,
                }),
                Some(ServerToClientData { receiver }),
            ) => State::Connected(Connected {
                client: &mut self.client,
                sender,
                receiver,
                requests,
                task_id,
                handler,
            }),
        }
    }

    fn inner_poll(&mut self, cx: &mut Context) -> Poll<tungstenite::Result<()>> {
        match self.state() {
            State::Connected(mut state) => loop {
                // Both direction in progress.

                match state.poll_client_to_server(cx) {
                    Poll::Ready(None) => {
                        // Client to server is done.

                        let mut server_to_client = state.as_server_to_client();
                        let poll_result = poll_to_end(|| server_to_client.poll_server_to_client(cx));

                        self.client_to_server = None;

                        if poll_result.is_ready() {
                            self.server_to_client = None;
                        }

                        return poll_result;
                    }
                    Poll::Ready(Some(Ok(()))) => {
                        // Client to server has progress.

                        match state.poll_server_to_client(cx) {
                            Poll::Ready(None) => {
                                // Server to client is done.

                                let mut client_to_server = state.as_client_to_server();
                                let poll_result = poll_to_end(|| client_to_server.poll_client_to_server(cx));

                                if poll_result.is_ready() {
                                    self.client_to_server = None;
                                }

                                self.server_to_client = None;

                                return poll_result;
                            }
                            Poll::Ready(Some(Ok(()))) => {
                                // Both direction has progress.
                            }
                            Poll::Ready(Some(Err(error))) => {
                                self.client_to_server = None;
                                self.server_to_client = None;

                                return Poll::Ready(Err(error));
                            }
                            Poll::Pending => {
                                return match poll_to_end(|| state.poll_client_to_server(cx)) {
                                    Poll::Ready(Ok(())) => {
                                        self.client_to_server = None;

                                        Poll::Pending
                                    }
                                    Poll::Ready(Err(error)) => {
                                        self.client_to_server = None;
                                        self.server_to_client = None;

                                        Poll::Ready(Err(error))
                                    }
                                    Poll::Pending => Poll::Pending,
                                }
                            }
                        }
                    }
                    Poll::Ready(Some(Err(error))) => {
                        self.client_to_server = None;
                        self.server_to_client = None;

                        return Poll::Ready(Err(error));
                    }
                    Poll::Pending => {
                        return match poll_to_end(|| state.poll_server_to_client(cx)) {
                            Poll::Ready(Ok(())) => {
                                self.server_to_client = None;

                                Poll::Pending
                            }
                            Poll::Ready(Err(error)) => {
                                self.client_to_server = None;
                                self.server_to_client = None;

                                Poll::Ready(Err(error))
                            }
                            Poll::Pending => Poll::Pending,
                        };
                    }
                }
            },
            State::ClientToServer(mut state) => {
                let poll_result = poll_to_end(|| state.poll_client_to_server(cx));

                if poll_result.is_ready() {
                    self.client_to_server = None;
                }

                poll_result
            }
            State::ServerToClient(mut state) => {
                let poll_result = poll_to_end(|| state.poll_server_to_client(cx));

                if poll_result.is_ready() {
                    self.server_to_client = None;
                }

                poll_result
            }
            State::Done => panic!("The connection is closed."),
        }
    }
}

fn poll_to_end<E>(mut f: impl FnMut() -> Poll<Option<Result<(), E>>>) -> Poll<Result<(), E>> {
    while let Some(()) = futures::ready!(f()?) {}

    Poll::Ready(Ok(()))
}

impl<T, H> Future for Connection<T, H>
where
    T: AsyncRead + AsyncWrite + Unpin,
    H: Handler,
{
    type Output = tungstenite::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        self.inner_poll(cx).map_err(Into::into)
    }
}
