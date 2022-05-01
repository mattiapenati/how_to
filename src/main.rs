use std::net::SocketAddr;

use clap::Parser;
use http_mirror::Server;
use hyper::Result;
use opentelemetry::runtime;
use tracing::{Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{filter::Targets, fmt::format::FmtSpan, layer::SubscriberExt, Registry};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Log level.
    #[clap(long, default_value_t = Level::INFO)]
    level: Level,

    /// The address to assign to the listening socket.
    address: SocketAddr,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let (subscriber, _guard) = build_subscriber(args.level);
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let result = Server::bind(&args.address).await;
    opentelemetry::global::shutdown_tracer_provider();
    result
}

fn build_subscriber(level: Level) -> (impl Subscriber, WorkerGuard) {
    let (writer, guard) = tracing_appender::non_blocking(std::io::stdout());

    let tracer = opentelemetry_jaeger::new_pipeline()
        .with_agent_endpoint("127.0.0.1:6831")
        .with_max_packet_size(9216)
        .with_auto_split_batch(true)
        .with_service_name("http_mirror")
        .install_batch(runtime::Tokio)
        .expect("failed to create telemetry tracer");
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    let level_filter = Targets::new().with_target("http_mirror", level);
    let fmt_layer = tracing_subscriber::fmt::Layer::default()
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_writer(writer);

    (
        Registry::default()
            .with(level_filter)
            .with(fmt_layer)
            .with(telemetry),
        guard,
    )
}
