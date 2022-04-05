use std::{
    convert::Infallible,
    net::SocketAddr,
    os::unix::prelude::{AsRawFd, FromRawFd},
    time::Duration,
};

use color_eyre::Report;
use futures::stream::unfold;
use hyper::{server::accept::Accept, service::make_service_fn, Body, Request, Response, Server};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::TcpStream;
use tower::ServiceBuilder;

#[tokio::main]
async fn main() -> Result<(), Report> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let listener = Listener::new(addr)?;
    let acc = listener.into_acceptor();

    Server::builder(acc)
        .serve(make_service_fn(move |_stream: &TcpStream| async move {
            Ok::<_, Infallible>(ServiceBuilder::new().service_fn(hello_world))
        }))
        .await?;

    Ok(())
}

async fn hello_world(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    println!("{} {}", req.method(), req.uri());
    tokio::time::sleep(Duration::from_millis(250)).await;
    Ok(Response::builder()
        .body(Body::from("Hello World!\n"))
        .unwrap())
}

enum Listener {
    Waiting { socket: Socket },
    Listening { ln: tokio::net::TcpListener },
}

impl Listener {
    fn new(addr: SocketAddr) -> Result<Self, Report> {
        let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
        println!("Binding...");
        socket.bind(&addr.into())?;

        Ok(Self::Waiting { socket })
    }

    fn into_acceptor(self) -> impl Accept<Conn = TcpStream, Error = Report> {
        hyper::server::accept::from_stream(unfold(self, |mut ln| async move {
            let stream = ln.accept().await;
            Some((stream, ln))
        }))
    }

    async fn accept(&mut self) -> Result<TcpStream, Report> {
        match self {
            Listener::Waiting { socket } => {
                tokio::time::sleep(Duration::from_secs(2)).await;

                println!(
                    "Listening on {}...",
                    socket.local_addr()?.as_socket().unwrap()
                );
                socket.listen(128)?;
                socket.set_nonblocking(true)?;
                let fd = socket.as_raw_fd();
                std::mem::forget(socket);
                let ln = unsafe { std::net::TcpListener::from_raw_fd(fd) };
                let ln = tokio::net::TcpListener::from_std(ln)?;
                let mut state = Self::Listening { ln };

                std::mem::swap(self, &mut state);
                match state {
                    Listener::Waiting { socket } => std::mem::forget(socket),
                    _ => unreachable!(),
                };

                match self {
                    Listener::Listening { ln } => Ok(ln.accept().await?.0),
                    _ => unreachable!(),
                }
            }
            Listener::Listening { ln } => Ok(ln.accept().await?.0),
        }
    }
}
