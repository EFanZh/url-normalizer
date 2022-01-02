use futures::{Future, FutureExt};
use std::sync::Arc;
use tokio::sync::Semaphore;

pub struct CancellationToken {
    inner: Arc<Semaphore>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Semaphore::new(0)),
        }
    }

    pub fn cancelled(&self) -> impl Future<Output = ()> {
        Arc::clone(&self.inner).acquire_many_owned(1).map(drop)
    }
}

impl Drop for CancellationToken {
    fn drop(&mut self) {
        self.inner.close();
    }
}
