use std::{
    convert::Infallible,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct FortyTwo;

#[async_trait]
impl CacheManager for FortyTwo {
    type Value = i32;
    type Error = Infallible;

    #[tracing::instrument(skip(self), level = "trace")]
    async fn create(self) -> Result<i32, Infallible> {
        std::thread::sleep(Duration::from_secs(3));
        tracing::trace!("created new object");
        Ok(42)
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_thread_ids(true)
        .with_writer(std::io::stdout)
        .init();

    let manager = FortyTwo;
    let value = Cache::new(manager, Duration::from_secs(5));

    for _ in 0..20 {
        let value = value.clone();
        tokio::spawn(
            async move {
                let x = value.get().await;
                tracing::trace!(value = ?x, "get a new value");
            }
            .instrument(tracing::trace_span!("sending request")),
        );
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[async_trait]
trait CacheManager: Clone + Send + 'static {
    type Value: Clone + Send + 'static;
    type Error: std::error::Error + Clone + Send + 'static;

    /// Create a new instance of [`CacheManager::Value`].
    async fn create(self) -> Result<Self::Value, Self::Error>;
}

#[derive(Clone)]
struct Cache<M: CacheManager> {
    inner: Arc<Mutex<CacheInner<M>>>,
    lifetime: Duration,
}

#[derive(Debug)]
enum CacheError<E> {
    RefreshDied,
    Refresh(E),
}

struct CacheInner<M: CacheManager> {
    manager: M,
    last_fetched: Option<(Instant, M::Value)>,
    incoming: Option<Weak<broadcast::Sender<Result<M::Value, M::Error>>>>,
}

impl<M: CacheManager> CacheInner<M> {
    fn new(manager: M) -> Self {
        Self {
            manager,
            last_fetched: None,
            incoming: None,
        }
    }
}

impl<M: CacheManager> Cache<M> {
    fn new(manager: M, lifetime: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CacheInner::new(manager))),
            lifetime,
        }
    }
}

impl<M: CacheManager> Cache<M> {
    #[tracing::instrument(skip(self), level = "trace")]
    async fn get(&self) -> Result<M::Value, CacheError<M::Error>> {
        let mut rx = {
            let mut inner = self.inner.lock();

            if let Some((fetched_at, value)) = inner.last_fetched.as_ref() {
                if fetched_at.elapsed() < self.lifetime {
                    return Ok(value.clone());
                } else {
                    tracing::trace!("cached value needs to be refreshed");
                }
            }

            if let Some(incoming) = inner.incoming.as_ref().and_then(Weak::upgrade) {
                tracing::trace!("waiting for a refreshed value");
                incoming.subscribe()
            } else {
                let (tx, rx) = broadcast::channel::<_>(1);

                let tx = Arc::new(tx);
                inner.incoming.replace(Arc::downgrade(&tx));

                let result = inner.manager.clone().create();
                let inner = self.inner.clone();

                tracing::trace!("refreshing cached value");
                tokio::spawn(
                    async move {
                        let result = result.await;
                        {
                            let mut inner = inner.lock();
                            inner.incoming = None;

                            let msg = match result {
                                Ok(value) => {
                                    tracing::trace!("cached value refreshed");
                                    inner.last_fetched.replace((Instant::now(), value.clone()));
                                    Ok(value)
                                }
                                Err(e) => {
                                    tracing::error!("cache value refreshing failed: {}", e);
                                    Err(e)
                                }
                            };
                            let _ = tx.send(msg);
                        }
                    }
                    .in_current_span(),
                );

                rx
            }
        };

        rx.recv()
            .await
            .map_err(|e| {
                tracing::error!("cache value refreshing died: {}", e);
                CacheError::RefreshDied
            })?
            .map_err(CacheError::Refresh)
    }
}
