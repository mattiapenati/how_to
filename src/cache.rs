use std::{
    error::Error,
    sync::{Arc, Weak},
    time::Instant,
};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::broadcast;
use tracing::Instrument;

#[async_trait]
pub trait CacheManager: Send + Sync + 'static {
    type Value: Clone + Send + Sync + 'static;

    /// Return `true` if a value is still valid.
    fn valid(&self, value: &Self::Value, last_update: Instant) -> bool;

    /// Create a new instance of [`CacheManager::Value`].
    async fn refresh(&self) -> Result<Self::Value, CacheError>;
}

#[derive(Clone)]
pub struct CacheError(CacheErrorInner);

impl CacheError {
    pub fn new<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        CacheError(CacheErrorInner::Refresh(Arc::new(err)))
    }

    #[inline]
    fn panic<E>(err: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        CacheError(CacheErrorInner::Panic(Arc::new(err)))
    }
}

#[derive(Clone)]
enum CacheErrorInner {
    Refresh(Arc<dyn Error + Send + Sync>),
    Panic(Arc<dyn Error + Send + Sync>),
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            CacheErrorInner::Refresh(_) => f.write_str("during refresh something goes wrong"),
            CacheErrorInner::Panic(_) => f.write_str("panic during value refresh"),
        }
    }
}

impl std::fmt::Debug for CacheError {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for CacheError {
    fn cause(&self) -> Option<&dyn Error> {
        match self.0 {
            CacheErrorInner::Refresh(ref e) => Some(e),
            CacheErrorInner::Panic(ref e) => Some(e),
        }
    }
}

#[derive(Clone)]
pub struct Cache<T>(Arc<Mutex<CacheInner<T>>>)
where
    T: Send + Sync;

struct CacheInner<T>
where
    T: Send + Sync,
{
    manager: Arc<dyn CacheManager<Value = T> + Send + Sync>,
    last_update: Option<(T, Instant)>,
    incoming: Option<Weak<broadcast::Sender<Result<T, CacheError>>>>,
}

impl<T> Cache<T>
where
    T: Clone + Send + Sync + 'static,
{
    pub fn new<M>(manager: M) -> Self
    where
        M: CacheManager<Value = T>,
    {
        Cache(Arc::new(Mutex::new(CacheInner {
            manager: Arc::new(manager),
            last_update: None,
            incoming: None,
        })))
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub async fn get(&self) -> Result<T, CacheError> {
        let mut rx = {
            let mut inner = self.0.lock();

            if let Some((value, last_update)) = inner.last_update.as_ref() {
                if inner.manager.valid(value, *last_update) {
                    // stored valued is still valid. then it is returned
                    return Ok(value.clone());
                } else {
                    tracing::trace!("cached value needs to be refreshed");
                }
            }

            if let Some(incoming) = inner.incoming.as_ref().and_then(Weak::upgrade) {
                // someone else is refreshing the contained value
                tracing::trace!("waiting for a refreshed value");
                incoming.subscribe()
            } else {
                // broadcast channel to send the value to the other waiting threads
                let (tx, rx) = broadcast::channel(1);

                let tx = Arc::new(tx);
                inner.incoming.replace(Arc::downgrade(&tx));

                tracing::trace!("refreshing cached value");
                tokio::spawn({
                    let manager = inner.manager.clone();
                    let inner = self.0.clone();

                    async move {
                        let result = manager.refresh().await;
                        {
                            let mut inner = inner.lock();
                            inner.incoming = None;

                            let msg = match result {
                                Ok(value) => {
                                    tracing::trace!("cached value refreshed");
                                    inner.last_update.replace((value.clone(), Instant::now()));
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
                    .in_current_span()
                });

                rx
            }
        };

        rx.recv()
            .await
            .map_err(|e| {
                tracing::error!("cache value refreshing died: {}", e);
                CacheError::panic(e)
            })?
            .map_err(CacheError::new)
    }
}
