//! embassy-net's TCP socket over the simulator's in-memory connections: the host plays the browser.

use core::marker::PhantomData;

#[derive(Clone, Copy)]
pub struct Stack<'d> {
    _p: PhantomData<&'d ()>,
}

impl Stack<'static> {
    /// The station is associated and has its DHCP lease.
    pub fn simulated() -> Self {
        Self { _p: PhantomData }
    }
}

impl Stack<'_> {
    pub async fn wait_link_up(&self) {}

    /// The simulated station never loses its WiFi link.
    pub async fn wait_link_down(&self) {
        core::future::pending::<()>().await
    }

    pub async fn wait_config_up(&self) {}

    pub fn config_v4(&self) -> Option<StaticConfigV4> {
        Some(StaticConfigV4 {
            address: Ipv4Cidr(core::net::Ipv4Addr::new(192, 168, 4, 2)),
        })
    }
}

/// The DHCP lease `board::net::start` logs.
#[derive(Clone, Copy, Debug)]
pub struct StaticConfigV4 {
    pub address: Ipv4Cidr,
}

#[derive(Clone, Copy, Debug)]
pub struct Ipv4Cidr(core::net::Ipv4Addr);

impl Ipv4Cidr {
    pub fn address(&self) -> core::net::Ipv4Addr {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpListenEndpoint {
    pub port: u16,
}

impl From<u16> for IpListenEndpoint {
    fn from(port: u16) -> Self {
        Self { port }
    }
}

pub mod tcp {
    use core::future::poll_fn;
    use core::marker::PhantomData;
    use core::task::Poll;

    use embassy_time::Duration;

    use super::{IpListenEndpoint, Stack};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Error {
        ConnectionReset,
    }

    impl core::fmt::Display for Error {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "{self:?}")
        }
    }

    impl core::error::Error for Error {}

    impl embedded_io_async::Error for Error {
        fn kind(&self) -> embedded_io_async::ErrorKind {
            embedded_io_async::ErrorKind::ConnectionReset
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AcceptError {
        /// The socket already holds a connection.
        InvalidState,
    }

    pub struct TcpSocket<'a> {
        conn: Option<u32>,
        timeout: Option<Duration>,
        last_activity: u64,
        _p: PhantomData<&'a mut [u8]>,
    }

    impl<'a> TcpSocket<'a> {
        pub fn new(_stack: Stack<'a>, _rx: &'a mut [u8], _tx: &'a mut [u8]) -> Self {
            Self {
                conn: None,
                timeout: None,
                last_activity: 0,
                _p: PhantomData,
            }
        }

        pub fn set_timeout(&mut self, timeout: Option<Duration>) {
            self.timeout = timeout;
        }

        /// Only port 80 has clients.
        pub async fn accept<T: Into<IpListenEndpoint>>(
            &mut self,
            local: T,
        ) -> Result<(), AcceptError> {
            if self.conn.is_some() {
                return Err(AcceptError::InvalidState);
            }
            let port = local.into().port;
            if port != 80 {
                return poll_fn(|_| Poll::<Result<(), AcceptError>>::Pending).await;
            }
            let id = poll_fn(|cx| match omnimotion_sim::net::accept(cx.waker()) {
                Some(id) => Poll::Ready(id),
                None => Poll::Pending,
            })
            .await;
            self.conn = Some(id);
            self.last_activity = omnimotion_sim::clock::now();
            Ok(())
        }

        pub fn close(&mut self) {
            if let Some(id) = self.conn.take() {
                omnimotion_sim::net::close(id);
            }
        }

        pub async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            let id = self.conn.ok_or(Error::ConnectionReset)?;
            let deadline = self.timeout.map(|t| self.last_activity + t.as_micros());
            let n = poll_fn(|cx| {
                if let Some(n) = omnimotion_sim::net::read(id, buf, cx.waker()) {
                    return Poll::Ready(Ok(n));
                }
                match deadline {
                    Some(d) if omnimotion_sim::clock::now() >= d => Poll::Ready(Err(Error::ConnectionReset)),
                    Some(d) => {
                        omnimotion_sim::clock::wake_at(d, cx.waker());
                        Poll::Pending
                    }
                    None => Poll::Pending,
                }
            })
            .await?;
            self.last_activity = omnimotion_sim::clock::now();
            Ok(n)
        }

        pub async fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
            let id = self.conn.ok_or(Error::ConnectionReset)?;
            omnimotion_sim::net::write(id, buf);
            self.last_activity = omnimotion_sim::clock::now();
            Ok(buf.len())
        }

        pub async fn flush(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }

    impl Drop for TcpSocket<'_> {
        fn drop(&mut self) {
            self.close();
        }
    }

    impl embedded_io_async::ErrorType for TcpSocket<'_> {
        type Error = Error;
    }

    impl embedded_io_async::Read for TcpSocket<'_> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            TcpSocket::read(self, buf).await
        }
    }

    impl embedded_io_async::Write for TcpSocket<'_> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
            TcpSocket::write(self, buf).await
        }

        async fn flush(&mut self) -> Result<(), Error> {
            TcpSocket::flush(self).await
        }
    }
}
