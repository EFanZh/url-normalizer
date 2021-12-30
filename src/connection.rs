use crate::check;
use crate::client_messages::{ClientMessage, ClientRequest, ClientRequestData, ClientResponse, ClientResponseData};
use crate::server_messages::{
    ClientMethod, ServerMessage, ServerRequest, ServerRequestData, ServerResponse, ServerResponseData,
};
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use futures::future::Either;
use futures::stream::FusedStream;
use futures::{Future, FutureExt, SinkExt, StreamExt};
use std::collections::HashMap;
use std::future;
use std::num::NonZeroU64;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::WebSocketStream;

enum ServerMessage2 {
    Request {
        data: Box<ServerRequestData>,
        sender: oneshot::Sender<anyhow::Result<ClientResponseData>>,
    },
    Response(ServerResponse),
}

pub struct ClientApi {
    sender: UnboundedSender<ServerMessage2>,
}

impl ClientApi {
    pub fn call<T>(&self, request: T) -> impl Future<Output = anyhow::Result<T::Output>>
    where
        T: ClientMethod,
    {
        let (sender, receiver) = oneshot::channel();

        match self.sender.unbounded_send(ServerMessage2::Request {
            data: Box::new(request.into()),
            sender,
        }) {
            Ok(()) => Either::Left(
                receiver.map(|result| result??.try_into().map_err(|message| anyhow::anyhow!("{}", message))),
            ),
            Err(error) => Either::Right(future::ready(Err(error.into()))),
        }
    }
}

async fn dispatch_client_request(client_api: ClientApi, request_data: ClientRequestData) -> ServerResponseData {
    match request_data {
        ClientRequestData::Check(request) => ServerResponseData::Check(check::check(client_api, request).await),
    }
}

fn handle_client_request(sender: UnboundedSender<ServerMessage2>, request: ClientRequest) {
    let client_api_sender = sender.clone();
    let server_response_sender = sender;
    let task_id = request.task_id;
    let request_data = request.data;

    tokio::spawn(
        dispatch_client_request(
            ClientApi {
                sender: client_api_sender,
            },
            request_data,
        )
        .map(move |response| {
            match server_response_sender.unbounded_send(ServerMessage2::Response(ServerResponse {
                task_id,
                data: response,
            })) {
                Ok(()) => {}
                Err(_) => tracing::warn!(task_id, "Session is closed."),
            }
        }),
    );
}

fn handle_client_response(
    requests: &mut HashMap<NonZeroU64, oneshot::Sender<anyhow::Result<ClientResponseData>>>,
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

fn poll_client_to_server<T>(
    cx: &mut Context,
    client: &mut WebSocketStream<T>,
    sender: &UnboundedSender<ServerMessage2>,
    requests: &mut HashMap<NonZeroU64, oneshot::Sender<anyhow::Result<ClientResponseData>>>,
) -> Poll<Option<tungstenite::Result<()>>>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    client.poll_next_unpin(cx).map(|result| {
        result.map(|result| {
            let message = result?;

            tracing::info!(content = ?message, "Got client message.");

            match message {
                Message::Text(message) => match serde_json::from_str(&message) {
                    Ok(ClientMessage::Request(request)) => handle_client_request(sender.clone(), *request),
                    Ok(ClientMessage::Response(response)) => handle_client_response(requests, *response),
                    Err(error) => tracing::warn!(%error, "Failed to deserialize message."),
                },
                Message::Binary(_) => tracing::warn!("Unexpected binary request."),
                Message::Ping(_) | Message::Pong(_) | Message::Close(_) => {}
            }

            Ok(())
        })
    })
}

fn next_task_id(task_id: &mut NonZeroU64) -> NonZeroU64 {
    let result = *task_id;

    *task_id = NonZeroU64::new(task_id.get().wrapping_add(1)).unwrap_or_else(|| NonZeroU64::new(1).unwrap());

    result
}

fn handle_server_request<T>(
    client: &mut WebSocketStream<T>,
    requests: &mut HashMap<NonZeroU64, oneshot::Sender<Result<ClientResponseData, anyhow::Error>>>,
    task_id: &mut NonZeroU64,
    data: ServerRequestData,
    sender: oneshot::Sender<Result<ClientResponseData, anyhow::Error>>,
) where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let task_id = next_task_id(task_id);

    match serde_json::to_string(&ServerMessage::Request(Box::new(ServerRequest { task_id, data }))) {
        Ok(message) => match client.start_send_unpin(Message::Text(message)) {
            Ok(()) => {
                requests.insert(task_id, sender);
            }
            Err(error) => drop(sender.send(Err(error.into()))), // Ignore send error.
        },
        Err(error) => drop(sender.send(Err(error.into()))), // Ignore send error.
    }
}

fn handle_server_response<T>(client: &mut WebSocketStream<T>, response: ServerResponse) -> tungstenite::Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    match serde_json::to_string(&ServerMessage::Response(Box::new(response))) {
        Ok(message) => client.start_send_unpin(Message::Text(message))?,
        Err(error) => tracing::error!(%error, "Failed to serialize response message."),
    }

    Ok(())
}

fn poll_server_to_client<T>(
    cx: &mut Context,
    client: &mut WebSocketStream<T>,
    receiver: &mut UnboundedReceiver<ServerMessage2>,
    requests: &mut HashMap<NonZeroU64, oneshot::Sender<anyhow::Result<ClientResponseData>>>,
    task_id: &mut NonZeroU64,
) -> Poll<tungstenite::Result<()>>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    futures::ready!(client.poll_ready_unpin(cx)?);

    match futures::ready!(receiver.poll_next_unpin(cx)).unwrap() {
        ServerMessage2::Request { data, sender } => {
            handle_server_request(client, requests, task_id, *data, sender);
        }
        ServerMessage2::Response(response) => handle_server_response(client, response)?,
    }

    Poll::Ready(Ok(()))
}

fn poll_server_to_client_2<T>(
    cx: &mut Context,
    client: &mut WebSocketStream<T>,
    receiver: &mut UnboundedReceiver<ServerMessage2>,
) -> Poll<Option<tungstenite::Result<()>>>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    if !receiver.is_terminated() {
        futures::ready!(client.poll_ready_unpin(cx)?);

        match futures::ready!(receiver.poll_next_unpin(cx)) {
            None => {}
            Some(message) => {
                match message {
                    ServerMessage2::Request { sender, .. } => drop(sender.send(Err(anyhow::anyhow!(
                        "Abort sending request to client since client response channel is closed."
                    )))), // Ignore send error.
                    ServerMessage2::Response(response) => handle_server_response(client, response)?,
                }

                return Poll::Ready(Some(Ok(())));
            }
        }
    }

    futures::ready!(client.poll_close_unpin(cx)?);

    Poll::Ready(None)
}

struct ClientToServer {
    sender: UnboundedSender<ServerMessage2>,
    requests: HashMap<NonZeroU64, oneshot::Sender<anyhow::Result<ClientResponseData>>>,
    task_id: NonZeroU64,
}

pub struct Connection<T>
where
    T: AsyncRead + AsyncWrite,
{
    client: WebSocketStream<T>,
    receiver: UnboundedReceiver<ServerMessage2>,
    client_to_server: Option<ClientToServer>,
}

impl<T> Connection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn new(inner: T) -> Self {
        let (sender, receiver) = mpsc::unbounded();

        Self {
            client: WebSocketStream::from_raw_socket(inner, Role::Server, None).await,
            receiver,
            client_to_server: Some(ClientToServer {
                sender,
                requests: HashMap::new(),
                task_id: NonZeroU64::new(1).unwrap(),
            }),
        }
    }

    fn inner_poll(&mut self, cx: &mut Context) -> Poll<tungstenite::Result<()>> {
        if let Some(client_to_server) = &mut self.client_to_server {
            loop {
                match (
                    poll_client_to_server(
                        cx,
                        &mut self.client,
                        &client_to_server.sender,
                        &mut client_to_server.requests,
                    )?,
                    poll_server_to_client(
                        cx,
                        &mut self.client,
                        &mut self.receiver,
                        &mut client_to_server.requests,
                        &mut client_to_server.task_id,
                    )?,
                ) {
                    (Poll::Ready(None), Poll::Ready(())) => {
                        self.client_to_server = None;

                        break;
                    }
                    (Poll::Ready(None), Poll::Pending) => {
                        self.client_to_server = None;

                        return Poll::Pending;
                    }
                    (Poll::Ready(Some(())), Poll::Ready(())) => continue,
                    (Poll::Ready(Some(())), Poll::Pending) => {
                        while let Some(()) = futures::ready!(poll_client_to_server(
                            cx,
                            &mut self.client,
                            &client_to_server.sender,
                            &mut client_to_server.requests,
                        )?) {}

                        self.client_to_server = None;

                        return Poll::Pending;
                    }
                    (Poll::Pending, Poll::Ready(())) => loop {
                        futures::ready!(poll_server_to_client(
                            cx,
                            &mut self.client,
                            &mut self.receiver,
                            &mut client_to_server.requests,
                            &mut client_to_server.task_id,
                        )?);
                    },
                    (Poll::Pending, Poll::Pending) => return Poll::Pending,
                };
            }
        }

        while let Some(()) = futures::ready!(poll_server_to_client_2(cx, &mut self.client, &mut self.receiver)?) {}

        Poll::Ready(Ok(()))
    }
}

impl<T> Future for Connection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = anyhow::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        self.inner_poll(cx).map(|result| match result {
            Ok(()) | Err(tungstenite::Error::ConnectionClosed) => Ok(()),
            Err(error) => Err(error.into()),
        })
    }
}
