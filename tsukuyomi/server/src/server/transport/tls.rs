#![cfg(feature = "tls")]

use std::io;

use futures::{Future, Stream};
use rustls::ServerSession;
use tokio_rustls::{TlsAcceptor, TlsStream};

use super::imp::TransportImpl;
use super::Transport;
use crate::server::CritError;

pub fn tls<T, A>(raw_transport: T, acceptor: A) -> TlsConfig<T>
where
    T: Transport,
    T::Error: 'static,
    A: Into<TlsAcceptor>,
{
    TlsConfig::new(raw_transport, acceptor)
}

#[allow(missing_debug_implementations)]
pub struct TlsConfig<T> {
    raw_transport: T,
    acceptor: TlsAcceptor,
}

impl<T> TlsConfig<T>
where
    T: Transport,
    T::Error: 'static,
{
    pub fn new<A>(raw_transport: T, acceptor: A) -> TlsConfig<T>
    where
        A: Into<TlsAcceptor>,
    {
        TlsConfig {
            raw_transport,
            acceptor: acceptor.into(),
        }
    }
}

impl<T> Transport for TlsConfig<T>
where
    T: Transport,
    T::Error: 'static,
{
}

impl<T> TransportImpl for TlsConfig<T>
where
    T: Transport,
    T::Error: 'static,
{
    type Item = TlsStream<T::Item, ServerSession>;
    type Error = CritError;
    type Incoming = Box<dyn Stream<Item = Self::Item, Error = Self::Error> + Send + 'static>;

    fn incoming(self) -> io::Result<Self::Incoming> {
        let Self {
            acceptor,
            raw_transport,
        } = self;

        let incoming = raw_transport.incoming()?;
        let incoming = Box::new(
            incoming
                .map(move |stream| acceptor.accept(stream).map_err(Into::into).into_stream())
                .map_err(Into::into)
                .flatten(),
        );

        Ok(incoming)
    }
}
