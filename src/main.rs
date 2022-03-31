use std::time::{Duration, Instant};

use async_trait::async_trait;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;

mod cache;
use cache::{Cache, CacheError, CacheManager};

struct FortyTwo;

#[async_trait]
impl CacheManager for FortyTwo {
    type Value = i32;

    fn valid(&self, _value: &Self::Value, last_update: Instant) -> bool {
        last_update.elapsed() < Duration::from_secs(5)
    }

    #[tracing::instrument(skip(self), level = "trace")]
    async fn refresh(&self) -> Result<i32, CacheError> {
        std::thread::sleep(Duration::from_secs(3));
        tracing::info!("created new object");
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

    let value = Cache::new(FortyTwo);

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
