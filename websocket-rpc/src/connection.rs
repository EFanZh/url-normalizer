use crate::cancellation_token::CancellationToken;
use crate::protocol::{ClientApi, ClientMethod, Message as RpcMessage, MessageData, MessageType};
use crate::{select, ServerApi};
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot::{self, Canceled, Sender};
use futures::future::Either;
use futures::{Future, FutureExt, SinkExt, StreamExt};
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

pub trait Handler {
    type ClientApi: ClientApi;
    type ServerApi: ServerApi;
    type ServerResponseFuture: Future<Output = <Self::ServerApi as ServerApi>::Response> + Send + 'static;

    fn handle(
        &mut self,
        rpc_client: RpcClient<Self::ClientApi>,
        request: Self::ServerApi,
    ) -> Self::ServerResponseFuture;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to deserialize response: {0}.")]
    Deserialization(serde_json::Error),
    #[error("Failed to serialize request: {0}.")]
    Serialization(serde_json::Error),
    #[error("Client connection is closed.")]
    ConnectionClosed,
}

struct ServerRequest {
    data: Box<RawValue>,
    sender: Sender<Box<RawValue>>,
}

enum ServerMessage {
    Request(ServerRequest),
    Response(MessageData),
}

pub struct RpcClient<C>
where
    C: ClientApi,
{
    sender: UnboundedSender<ServerMessage>,
    _phantom: PhantomData<C>,
}

impl<C> RpcClient<C>
where
    C: ClientApi,
{
    pub fn call<R>(&self, request: R) -> impl Future<Output = Result<R::Response, Error>>
    where
        R: ClientMethod<C>,
    {
        match value::to_raw_value::<C>(&request.into()).map_err(Error::Serialization) {
            Ok(request_data) => {
                let (sender, receiver) = oneshot::channel();

                match self.sender.unbounded_send(ServerMessage::Request(ServerRequest {
                    data: request_data,
                    sender,
                })) {
                    Ok(()) => Either::Left(receiver.map(|result| match result {
                        Ok(response_data) => serde_json::from_str(response_data.get()).map_err(Error::Deserialization),
                        Err(Canceled) => Err(Error::ConnectionClosed),
                    })),
                    Err(_) => Either::Right(future::ready(Err(Error::ConnectionClosed))),
                }
            }
            Err(_) => Either::Right(future::ready(Err(Error::ConnectionClosed))),
        }
    }
}

struct ClientToServer<'a, T> {
    client: &'a mut WebSocketStream<T>,
    requests: &'a mut HashMap<u64, Sender<Box<RawValue>>>,
}

impl<T> ClientToServer<'_, T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn handle_client_response(&mut self, response: MessageData) {
        if let Some(sender) = self.requests.remove(&response.task_id) {
            match sender.send(response.data) {
                Ok(()) => {}
                Err(_) => tracing::warn!(
                    response.task_id,
                    "Ignoring client response, the task has been canceled."
                ),
            }
        } else {
            tracing::warn!(response.task_id, "Ignoring client response, the task ID is invalid.");
        }
    }

    fn poll_client_to_server_partial(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Option<tungstenite::Result<Option<MessageData>>>> {
        self.client.poll_next_unpin(cx).map_ok(|message| {
            tracing::info!(content = %message, "Received WebSocket message.");

            match message {
                Message::Text(message) => match serde_json::from_str::<RpcMessage>(&message) {
                    Ok(message) => match message.split() {
                        (MessageType::Request, request) => return Some(request),
                        (MessageType::Response, response) => self.handle_client_response(response),
                    },
                    Err(error) => tracing::warn!(%error, "Ignoring unrecognized client WebSocket message."),
                },
                Message::Binary(_) => tracing::warn!("Ignoring client WebSocket binary message."),
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
    receiver: &'a mut UnboundedReceiver<ServerMessage>,
}

impl<T> ServerToClient<'_, T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn handle_server_response(&mut self, response: MessageData) -> tungstenite::Result<()> {
        let message = RpcMessage::response(response).serialize_to_json_string();

        tracing::info!(content = %message.as_str(), "Sending response to client.");

        self.client.start_send_unpin(Message::Text(message))?;

        Ok(())
    }

    fn poll_server_to_client_partial(
        &mut self,
        cx: &mut Context,
    ) -> Poll<Option<tungstenite::Result<Option<ServerRequest>>>> {
        match futures::ready!(self.client.poll_ready_unpin(cx)) {
            Ok(()) => match futures::ready!(self.receiver.poll_next_unpin(cx)) {
                None => self.client.poll_close_unpin(cx)?.map(|()| None),
                Some(message) => Poll::Ready(Some(Ok(match message {
                    ServerMessage::Request(request) => Some(request),
                    ServerMessage::Response(response) => {
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
    sender: &'a mut UnboundedSender<ServerMessage>,
    receiver: &'a mut UnboundedReceiver<ServerMessage>,
    requests: &'a mut HashMap<u64, Sender<Box<RawValue>>>,
    task_id: &'a mut u64,
    cancellation_token: &'a CancellationToken,
}

impl<T, H> Connected<'_, T, H>
where
    T: AsyncRead + AsyncWrite + Unpin,
    H: Handler,
{
    fn handle_client_request(&mut self, client_request: &MessageData) {
        fn send_response(sender: &UnboundedSender<ServerMessage>, task_id: u64, response: &impl Serialize) {
            let data = value::to_raw_value(response)
                .unwrap_or_else(|error| value::to_raw_value(error.to_string().as_str()).unwrap());

            let response = ServerMessage::Response(MessageData { task_id, data });

            drop(sender.unbounded_send(response)); // Ignore send error;
        }

        let client_api_sender = self.sender.clone();

        match serde_json::from_str(client_request.data.get()) {
            Ok(request) => {
                let server_response_sender = self.sender.clone();
                let task_id = client_request.task_id;

                let handler_task = self
                    .handler
                    .handle(
                        RpcClient {
                            sender: client_api_sender,
                            _phantom: PhantomData,
                        },
                        request,
                    )
                    .map(move |response| send_response(&server_response_sender, task_id, &response));

                tokio::spawn(select::select(handler_task, self.cancellation_token.cancelled()).map(drop));
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

    fn next_task_id(&mut self) -> u64 {
        let result = *self.task_id;

        *self.task_id = self.task_id.wrapping_add(1);

        result
    }

    fn handle_server_request(&mut self, request: ServerRequest) -> tungstenite::Result<()> {
        let task_id = self.next_task_id();
        let ServerRequest { data, sender } = request;
        let message = RpcMessage::request(MessageData { task_id, data }).serialize_to_json_string();

        tracing::info!(content = %message.as_str(), "Sending request to client.");

        self.client.start_send_unpin(Message::Text(message)).map(|()| {
            self.requests.insert(task_id, sender);
        })
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

#[allow(variant_size_differences)]
enum State<'a, T, H> {
    Connected(Connected<'a, T, H>),
    ClientToServer(ClientToServer<'a, T>),
    ServerToClient(ServerToClient<'a, T>),
    Done,
}

struct ClientToServerData<H> {
    sender: UnboundedSender<ServerMessage>,
    requests: HashMap<u64, Sender<Box<RawValue>>>,
    task_id: u64,
    handler: H,
}

struct ServerToClientData {
    receiver: UnboundedReceiver<ServerMessage>,
}

pin_project_lite::pin_project! {
    pub struct Connection<T, H>
    where
        // See <https://github.com/taiki-e/pin-project-lite/issues/2>.
        T: AsyncRead,
        T: AsyncWrite,
        T: Unpin,
        H: Handler,
    {
        #[pin]
        client: WebSocketStream<T>,
        client_to_server: Option<ClientToServerData<H>>,
        server_to_client: Option<ServerToClientData>,
        cancellation_token: CancellationToken,
    }
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
            cancellation_token: CancellationToken::new(),
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
                cancellation_token: &self.cancellation_token,
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
        self.inner_poll(cx)
    }
}
