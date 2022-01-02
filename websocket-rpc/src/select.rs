use futures::future::Either;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pin_project_lite::pin_project! {
    pub struct Select<A, B> {
        #[pin]
        future1: A,
        #[pin]
        future2: B,
    }
}

impl<A, B> Future for Select<A, B>
where
    A: Future,
    B: Future,
{
    type Output = Either<A::Output, B::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.project();

        match this.future1.poll(cx) {
            Poll::Ready(result) => Poll::Ready(Either::Left(result)),
            Poll::Pending => this.future2.poll(cx).map(Either::Right),
        }
    }
}

pub fn select<A, B>(future1: A, future2: B) -> Select<A, B>
where
    A: Future,
    B: Future,
{
    Select { future1, future2 }
}
