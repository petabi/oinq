//! Shared test code

use std::sync::LazyLock;

use quinn::{Connection, RecvStream, SendStream};
use tokio::sync::Mutex;

pub(crate) struct Channel {
    pub(crate) server: Endpoint,
    pub(crate) client: Endpoint,
}

pub(crate) struct Endpoint {
    pub(crate) _conn: Connection,
    pub(crate) send: SendStream,
    pub(crate) recv: RecvStream,
}

pub(crate) static TOKEN: LazyLock<Mutex<u32>> = LazyLock::new(|| Mutex::new(0));

/// Creates a bidirectional channel, returning server's send and receive and
/// client's send and receive streams.
pub(crate) async fn channel() -> Channel {
    use std::{
        net::{IpAddr, Ipv6Addr, SocketAddr},
        sync::Arc,
    };

    use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
    use rustls::{
        crypto::aws_lc_rs::{self, kx_group::X25519MLKEM768},
        pki_types::{CertificateDer, PrivatePkcs8KeyDer},
    };

    const TEST_SERVER_NAME: &str = "test-server";

    let cert =
        rcgen::generate_simple_self_signed([TEST_SERVER_NAME.to_string()]).expect("infallible");
    let cert_der = vec![CertificateDer::from(cert.cert)];
    let key_der = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    // A successful handshake must use hybrid key exchange: no classical fallback is allowed.
    let provider = Arc::new(rustls::crypto::CryptoProvider {
        kx_groups: vec![X25519MLKEM768],
        ..aws_lc_rs::default_provider()
    });
    let server_crypto = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 is supported")
        .with_no_client_auth()
        .with_single_cert(cert_der.clone(), key_der.into())
        .expect("infallible");
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(server_crypto).expect("invalid server config"),
    ));
    // Let the OS select an available port to avoid conflicts and reserved ports.
    let server_endpoint = quinn::Endpoint::server(
        server_config,
        SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0),
    )
    .unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        let server_connection = match server_endpoint.accept().await {
            Some(conn) => match conn.await {
                Ok(conn) => conn,
                Err(e) => panic!("{}", e.to_string()),
            },
            None => panic!("connection closed"),
        };
        let (server_send, mut server_recv) = server_connection.accept_bi().await.unwrap();
        let mut server_buf = [0; 5];
        server_recv.read_exact(&mut server_buf).await.unwrap();
        (server_connection, server_send, server_recv)
    });

    let mut root_cert_store = rustls::RootCertStore::empty();
    root_cert_store.add_parsable_certificates(cert_der);
    let client_crypto = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 is supported")
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();
    let client_config = quinn::ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(client_crypto).expect("invalid client config"),
    ));
    let client_endpoint =
        quinn::Endpoint::client(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)).unwrap();
    let client_connecting = client_endpoint
        .connect_with(client_config, server_addr, TEST_SERVER_NAME)
        .unwrap();

    let client_connection = client_connecting.await.unwrap();
    let (mut client_send, client_recv) = client_connection.open_bi().await.unwrap();
    client_send.write_all(b"ready").await.unwrap();

    let (server_connection, server_send, server_recv) = handle.await.unwrap();

    Channel {
        server: self::Endpoint {
            _conn: server_connection,
            send: server_send,
            recv: server_recv,
        },
        client: self::Endpoint {
            _conn: client_connection,
            send: client_send,
            recv: client_recv,
        },
    }
}

#[tokio::test]
async fn handshake_with_x25519mlkem768() {
    // channel() allows only X25519MLKEM768 on both peers and awaits the handshake.
    let _channel = channel().await;
}
