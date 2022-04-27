use std::{convert::Infallible, fmt, future::Future, net::SocketAddr, pin::Pin};

use hyper::{
    header,
    server::conn::AddrStream,
    service::{make_service_fn, service_fn},
    Body, Request, Response, StatusCode,
};
use uuid::Uuid;

pub struct Server {
    address: SocketAddr,
    server: Pin<Box<dyn Future<Output = hyper::Result<()>>>>,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("address", &self.addr())
            .finish()
    }
}

impl Server {
    /// Create a new server that binds the provided address.
    pub fn bind(addr: &SocketAddr) -> Self {
        tracing::info!("listening on {}", addr);
        let make_service = make_service_fn(|conn: &AddrStream| {
            let remote_addr = conn.remote_addr();
            let service = service_fn(move |request| handler(remote_addr, request));
            async move { Ok::<_, Infallible>(service) }
        });
        let server = hyper::server::Server::bind(&addr).serve(make_service);

        Server {
            address: addr.clone(),
            server: Box::pin(server),
        }
    }

    /// The address that this server is bound to.
    pub fn addr(&self) -> SocketAddr {
        self.address
    }
}

impl Future for Server {
    type Output = hyper::Result<()>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        Pin::new(&mut self.server).poll(cx)
    }
}

#[tracing::instrument(level = "info", skip_all, fields(id = %Uuid::new_v4()), name = "request")]
async fn handler(addr: SocketAddr, request: Request<Body>) -> Result<Response<Body>, Infallible> {
    tracing::debug!("request from {}", addr);

    let (mut head, body) = request.into_parts();

    let mut response = Response::new(body);
    *response.version_mut() = head.version;
    *response.status_mut() = StatusCode::OK;

    if let Some(value) = head.headers.remove(header::CONTENT_TYPE) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    if let Some(value) = head.headers.remove(header::CONTENT_LENGTH) {
        response.headers_mut().insert(header::CONTENT_LENGTH, value);
    }

    Ok(response)
}
