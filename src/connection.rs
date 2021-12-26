use crate::check;
use crate::client_messages::{
    ClientMessage, ClientRequest, ClientRequestData, ClientResponse, ClientResponseData,
};
use crate::server_messages::{
    ClientMethod, ServerMessage, ServerRequest, ServerRequestData, ServerResponse,
    ServerResponseData,
};
use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::channel::oneshot;
use futures::future::Either;
use futures::{Future, FutureExt, SinkExt, StreamExt};
use std::collections::HashMap;
use std::future;
use std::num::NonZeroU64;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

enum ServerMessage2 {
    Request {
        data: Box<ServerRequestData>,
        sender: oneshot::Sender<anyhow::Result<ClientResponseData>>,
    },
    Response(ServerResponse),
}

pub struct Connection<T>
where
    T: AsyncRead + AsyncWrite,
{
    client: WebSocketStream<T>,
    server_receiver: UnboundedReceiver<ServerMessage2>,
    server_sender: UnboundedSender<ServerMessage2>,
    send_buffer: Option<ServerMessage2>,
    task_id: NonZeroU64,
    pending_requests: HashMap<NonZeroU64, oneshot::Sender<anyhow::Result<ClientResponseData>>>,
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
            Ok(()) => Either::Left(receiver.map(|result| {
                result??
                    .try_into()
                    .map_err(|message| anyhow::anyhow!("{}", message))
            })),
            Err(error) => Either::Right(future::ready(Err(error.into()))),
        }
    }
}

async fn handle_client_request(
    client_api: ClientApi,
    request_data: ClientRequestData,
) -> ServerResponseData {
    match request_data {
        ClientRequestData::Check(request) => {
            ServerResponseData::Check(check::check(client_api, request).await)
        }
    }
}

impl<T> Connection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn new(inner: T) -> Self {
        let (server_sender, server_receiver) = mpsc::unbounded();

        Self {
            client: WebSocketStream::from_raw_socket(inner, Role::Server, None).await,
            server_receiver,
            server_sender,
            send_buffer: None,
            task_id: NonZeroU64::new(1).unwrap(),
            pending_requests: HashMap::new(),
        }
    }

    fn handle_client_request(&mut self, request: ClientRequest) {
        let sender_1 = self.server_sender.clone();
        let sender_2 = self.server_sender.clone();
        let task_id = request.task_id;
        let request_data = request.data;

        tokio::spawn(
            handle_client_request(ClientApi { sender: sender_1 }, request_data).map(
                move |response| {
                    if sender_2
                        .unbounded_send(ServerMessage2::Response(ServerResponse {
                            task_id,
                            data: response,
                        }))
                        .is_err()
                    {
                        tracing::warn!("Task has completed for task {}.", task_id);
                    }
                },
            ),
        );
    }

    fn handle_client_response(&mut self, response: ClientResponse) {
        if let Some(sender) = self.pending_requests.remove(&response.task_id) {
            if sender.send(Ok(response.data)).is_err() {
                tracing::warn!("Future has been canceled for task {}.", response.task_id);
            }
        } else {
            tracing::warn!("Unexpected ID {}.", response.task_id);
        }
    }

    fn handle_client_message(&mut self, message: Message) -> anyhow::Result<()> {
        tracing::info!("WebSocket message: {:?}.", message);

        match message {
            Message::Text(message) => {
                let client_message = serde_json::from_str::<ClientMessage>(&message)?;

                match client_message {
                    ClientMessage::Request(request) => self.handle_client_request(*request),
                    ClientMessage::Response(response) => self.handle_client_response(*response),
                }

                Ok(())
            }
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => {
                anyhow::bail!("Bad request.")
            }
            Message::Close(_) => Ok(()),
        }
    }

    fn next_task_id(&mut self) -> NonZeroU64 {
        let result = self.task_id;

        self.task_id = NonZeroU64::new(self.task_id.get().wrapping_add(1))
            .unwrap_or_else(|| NonZeroU64::new(1).unwrap());

        result
    }

    fn poll_receive_client(&mut self, cx: &mut Context) -> Poll<Option<anyhow::Result<()>>> {
        self.client
            .poll_next_unpin(cx)?
            .map(|message| message.map(|message| self.handle_client_message(message)))
    }

    fn poll_receive_server(&mut self, cx: &mut Context) -> Poll<anyhow::Result<()>> {
        if self.send_buffer.is_none() {
            self.send_buffer =
                Some(futures::ready!(self.server_receiver.poll_next_unpin(cx)).unwrap());
        }

        futures::ready!(self.client.poll_ready_unpin(cx)?);

        match self.send_buffer.take().unwrap() {
            ServerMessage2::Request { data, sender } => {
                let task_id = self.next_task_id();

                match serde_json::to_string(&ServerMessage::Request(Box::new(ServerRequest {
                    task_id,
                    data: *data,
                }))) {
                    Ok(message) => match self.client.start_send_unpin(Message::Text(message)) {
                        Ok(()) => drop(self.pending_requests.insert(task_id, sender)),
                        Err(error) => drop(sender.send(Err(error.into()))),
                    },
                    Err(error) => drop(sender.send(Err(error.into()))),
                }
            }
            ServerMessage2::Response(response) => {
                match serde_json::to_string(&ServerMessage::Response(Box::new(response))) {
                    Ok(message) => match self.client.start_send_unpin(Message::Text(message)) {
                        Ok(()) => {}
                        Err(error) => tracing::error!("{}", error),
                    },
                    Err(error) => tracing::error!("{}", error),
                }
            }
        }

        Poll::Ready(Ok(()))
    }
}

macro_rules! do_poll {
    ($poll_1:expr) => {
        loop {
            match $poll_1? {
                Poll::Ready(()) => {}
                Poll::Pending => return Poll::Pending,
            }
        }
    };
    ($poll_1:expr, $poll_2:expr,) => {
        loop {
            match $poll_1? {
                Poll::Ready(()) => match $poll_2? {
                    Poll::Ready(()) => {}
                    Poll::Pending => do_poll!($poll_1),
                },
                Poll::Pending => do_poll!($poll_2),
            }
        }
    };
}

impl<T> Future for Connection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    type Output = anyhow::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        do_poll![
            match self.poll_receive_client(cx) {
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(result)) => Poll::Ready(result),
                Poll::Pending => Poll::Pending,
            },
            self.poll_receive_server(cx),
        ]
    }
}
