use std::{error::Error, future::Future, net::UdpSocket, sync::Arc, time::Duration};

use oinq::{
    ApplicationCode, CloseInfo, Connection, Endpoint, TransportCloseKind, TransportCode,
    TransportError, TransportErrorKind, TransportOptions,
    frame::{FrameError, FrameLimit, FramedReader, FramedWriter},
};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::{
    AlertDescription, ClientConfig, DigitallySignedStruct, DistinguishedName, RootCertStore,
    ServerConfig, SignatureScheme,
    client::danger::HandshakeSignatureValid,
    crypto::aws_lc_rs::{self, kx_group::X25519MLKEM768},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, UnixTime},
    server::{
        WebPkiClientVerifier,
        danger::{ClientCertVerified, ClientCertVerifier},
    },
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug)]
struct RejectClientCertificates(Arc<dyn ClientCertVerifier>);

impl ClientCertVerifier for RejectClientCertificates {
    fn offer_client_auth(&self) -> bool {
        self.0.offer_client_auth()
    }

    fn client_auth_mandatory(&self) -> bool {
        self.0.client_auth_mandatory()
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.0.root_hint_subjects()
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Err(rustls::Error::General(
            "client certificate rejected by test policy".into(),
        ))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

struct Identity {
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

struct Pki {
    server: Identity,
    client: Identity,
    roots: Arc<RootCertStore>,
}

impl Pki {
    fn new() -> Self {
        let key = KeyPair::generate().expect("test operation succeeds");
        let mut ca =
            CertificateParams::new(vec!["test-ca".into()]).expect("test operation succeeds");
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let root = ca
            .self_signed(&key)
            .expect("test operation succeeds")
            .der()
            .clone();
        let issuer = Issuer::from_params(&ca, &key);
        let identity = |name: &str| {
            let key = KeyPair::generate().expect("test operation succeeds");
            let leaf = CertificateParams::new(vec![name.into()])
                .expect("test operation succeeds")
                .signed_by(&key, &issuer)
                .expect("test operation succeeds");
            Identity {
                chain: vec![leaf.der().clone(), root.clone()],
                key: PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
            }
        };
        let mut roots = RootCertStore::empty();
        roots.add(root.clone()).expect("test operation succeeds");
        Self {
            server: identity("localhost"),
            client: identity("client"),
            roots: Arc::new(roots),
        }
    }

    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::CryptoProvider {
            kx_groups: vec![X25519MLKEM768],
            ..aws_lc_rs::default_provider()
        })
    }

    fn server_config(&self, client_roots: Arc<RootCertStore>) -> Arc<ServerConfig> {
        let verifier = WebPkiClientVerifier::builder_with_provider(client_roots, Self::provider())
            .build()
            .expect("test operation succeeds");
        Arc::new(
            ServerConfig::builder_with_provider(Self::provider())
                .with_protocol_versions(&[&rustls::version::TLS13])
                .expect("test operation succeeds")
                .with_client_cert_verifier(verifier)
                .with_single_cert(self.server.chain.clone(), self.server.key.clone_key())
                .expect("test operation succeeds"),
        )
    }

    fn rejecting_server_config(&self) -> Arc<ServerConfig> {
        let verifier =
            WebPkiClientVerifier::builder_with_provider(self.roots.clone(), Self::provider())
                .build()
                .expect("test operation succeeds");
        Arc::new(
            ServerConfig::builder_with_provider(Self::provider())
                .with_protocol_versions(&[&rustls::version::TLS13])
                .expect("test operation succeeds")
                .with_client_cert_verifier(Arc::new(RejectClientCertificates(verifier)))
                .with_single_cert(self.server.chain.clone(), self.server.key.clone_key())
                .expect("test operation succeeds"),
        )
    }

    fn client_config(&self, auth: bool) -> Arc<ClientConfig> {
        let builder = ClientConfig::builder_with_provider(Self::provider())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .expect("test operation succeeds")
            .with_root_certificates(self.roots.clone());
        Arc::new(if auth {
            builder
                .with_client_auth_cert(self.client.chain.clone(), self.client.key.clone_key())
                .expect("test operation succeeds")
        } else {
            builder.with_no_client_auth()
        })
    }

    fn endpoints(&self) -> (Endpoint, Endpoint) {
        (
            Endpoint::server_with_socket(
                self.server_config(self.roots.clone()),
                socket(),
                TransportOptions::default(),
            )
            .expect("test operation succeeds"),
            Endpoint::client_with_socket(
                self.client_config(true),
                socket(),
                TransportOptions::default(),
            )
            .expect("test operation succeeds"),
        )
    }
}

fn socket() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").expect("test operation succeeds")
}

async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), f)
        .await
        .expect("transport test timed out")
}

async fn pair(server: &Endpoint, client: &Endpoint) -> (Connection, Connection) {
    let connect = client.connect(
        server.local_addr().expect("test operation succeeds"),
        "localhost",
    );
    let accept = async {
        server
            .accept()
            .await
            .expect("test operation succeeds")
            .await
            .expect("test operation succeeds")
    };
    let (client, server) = tokio::join!(connect, accept);
    (server, client.expect("test operation succeeds"))
}

async fn attempt(
    server: &Endpoint,
    client: &Endpoint,
    name: &str,
) -> (
    Result<Connection, oinq::TransportError>,
    Result<Connection, oinq::TransportError>,
) {
    tokio::join!(
        client.connect(server.local_addr().expect("bound socket address"), name),
        async { server.accept().await.expect("listener is open").await }
    )
}

#[tokio::test]
async fn mtls_streams_framing_and_completion() {
    bounded(async {
        let pki = Pki::new();
        let (server, client) = pki.endpoints();
        let (s, c) = pair(&server, &client).await;
        assert_eq!(s.peer_certificates().expect("test operation succeeds"), pki.client.chain);
        assert_eq!(c.peer_certificates().expect("test operation succeeds"), pki.server.chain);
        assert_eq!(c.remote_addr(), server.local_addr().expect("test operation succeeds"));
        assert_eq!(c.local_addr().expect("test operation succeeds"), client.local_addr().expect("test operation succeeds"));
        let limit = FrameLimit::new(1024).expect("test operation succeeds");
        let (send, recv) = c.open_bi().await.expect("test operation succeeds");
        let mut writer = FramedWriter::new(send, limit);
        writer.start_raw(b"request").expect("test operation succeeds");
        writer.flush().await.expect("test operation succeeds");
        let mut send = writer.into_inner();
        send.finish().expect("test operation succeeds");
        let (reply, request) = s.accept_bi().await.expect("test operation succeeds");
        let mut reader = FramedReader::new(request, limit);
        assert_eq!(reader.read().await.expect("test operation succeeds").expect("test operation succeeds").as_bytes(), b"request");
        assert!(reader.read().await.expect("test operation succeeds").is_none());
        assert_eq!(send.stopped().await.expect("test operation succeeds"), None);
        let mut writer = FramedWriter::new(reply, limit);
        writer.start_raw(b"response").expect("test operation succeeds");
        writer.flush().await.expect("test operation succeeds");
        writer.into_inner().finish().expect("test operation succeeds");
        assert_eq!(FramedReader::new(recv, limit).read().await.expect("test operation succeeds").expect("test operation succeeds").as_bytes(), b"response");

        for (sender, receiver) in [(&s, &c), (&c, &s)] {
            let mut send = sender.open_uni().await.expect("test operation succeeds");
            send.write_all(b"uni").await.expect("test operation succeeds");
            send.finish().expect("test operation succeeds");
            let mut recv = receiver.accept_uni().await.expect("test operation succeeds");
            let mut data = Vec::new();
            recv.read_to_end(&mut data).await.expect("test operation succeeds");
            assert_eq!(data, b"uni");
            assert_eq!(send.stopped().await.expect("test operation succeeds"), None);
        }
        c.close(ApplicationCode::new(17).expect("test operation succeeds"), b"done");
        assert!(matches!(s.closed().await, CloseInfo::Application { code, reason } if code.get() == 17 && reason == b"done"));
        server.close(ApplicationCode::default(), b"");
        client.close(ApplicationCode::default(), b"");
        tokio::join!(server.wait_idle(), client.wait_idle());
    }).await;
}

#[tokio::test]
async fn mtls_rejection_reports_crypto_alert() {
    bounded(async {
        let pki = Pki::new();
        let server = Endpoint::server_with_socket(
            pki.server_config(pki.roots.clone()),
            socket(),
            TransportOptions::default(),
        )
        .expect("test operation succeeds");
        let anonymous = Endpoint::client(
            pki.client_config(false),
            "127.0.0.1:0".parse().expect("test operation succeeds"),
            TransportOptions::default(),
        )
        .expect("test operation succeeds");
        let (connected, accepted) = attempt(&server, &anonymous, "localhost").await;
        let error = accepted.expect_err("server rejects missing client certificate");
        assert_eq!(error.kind(), TransportErrorKind::Connection);
        assert_crypto_close(&error, AlertDescription::CertificateRequired);
        assert!(error.source().is_some());
        match connected {
            Ok(c) => {
                let info = c.closed().await;
                let close = info
                    .transport_close()
                    .expect("peer close carries transport detail");
                assert_eq!(close.kind(), TransportCloseKind::Crypto);
                assert_eq!(
                    close.tls_alert(),
                    Some(AlertDescription::CertificateRequired)
                );
                assert_eq!(
                    close.code().map(TransportCode::get),
                    Some(0x100 + u64::from(u8::from(AlertDescription::CertificateRequired)))
                );
            }
            Err(error) => assert_crypto_close(&error, AlertDescription::CertificateRequired),
        }
        anonymous.close(ApplicationCode::default(), b"");
        server.close(ApplicationCode::default(), b"");
    })
    .await;
}

#[tokio::test]
async fn rejected_client_certificate_reports_handshake_failure() {
    bounded(async {
        let pki = Pki::new();
        let server = Endpoint::server_with_socket(
            pki.rejecting_server_config(),
            socket(),
            TransportOptions::default(),
        )
        .expect("test operation succeeds");
        let client = Endpoint::client_with_socket(
            pki.client_config(true),
            socket(),
            TransportOptions::default(),
        )
        .expect("test operation succeeds");
        let (connected, accepted) = attempt(&server, &client, "localhost").await;
        let error = accepted.expect_err("server rejects the client certificate");
        assert_crypto_close(&error, AlertDescription::HandshakeFailure);
        match connected {
            Ok(c) => {
                let info = c.closed().await;
                let close = info
                    .transport_close()
                    .expect("peer close carries transport detail");
                assert_eq!(close.kind(), TransportCloseKind::Crypto);
                assert_eq!(close.tls_alert(), Some(AlertDescription::HandshakeFailure));
            }
            Err(error) => assert_crypto_close(&error, AlertDescription::HandshakeFailure),
        }
        server.close(ApplicationCode::default(), b"");
        client.close(ApplicationCode::default(), b"");
        tokio::join!(server.wait_idle(), client.wait_idle());
    })
    .await;
}

#[tokio::test]
async fn incoming_reports_the_peer_address_before_the_handshake() {
    bounded(async {
        let pki = Pki::new();
        let server = Endpoint::server_with_socket(
            pki.rejecting_server_config(),
            socket(),
            TransportOptions::default(),
        )
        .expect("test operation succeeds");
        let client = Endpoint::client_with_socket(
            pki.client_config(true),
            socket(),
            TransportOptions::default(),
        )
        .expect("test operation succeeds");
        let client_addr = client.local_addr().expect("bound socket address");
        let server_addr = server.local_addr().expect("bound socket address");
        let (connected, accepted) = tokio::join!(client.connect(server_addr, "localhost"), async {
            let incoming = server.accept().await.expect("listener is open");
            assert_eq!(incoming.remote_addr(), client_addr);
            incoming.await
        });
        let error = accepted.expect_err("server rejects the client certificate");
        assert_crypto_close(&error, AlertDescription::HandshakeFailure);
        drop(connected);
        server.close(ApplicationCode::default(), b"");
        client.close(ApplicationCode::default(), b"");
        tokio::join!(server.wait_idle(), client.wait_idle());

        let (server, client) = pki.endpoints();
        let client_addr = client.local_addr().expect("bound socket address");
        let server_addr = server.local_addr().expect("bound socket address");
        let (connected, accepted) = tokio::join!(client.connect(server_addr, "localhost"), async {
            let incoming = server.accept().await.expect("listener is open");
            let addr = incoming.remote_addr();
            assert_eq!(addr, client_addr);
            (addr, incoming.await)
        });
        let (addr, accepted) = accepted;
        let accepted = accepted.expect("handshake succeeds");
        assert_eq!(accepted.remote_addr(), addr);
        let connected = connected.expect("handshake succeeds");
        connected.close(ApplicationCode::default(), b"");
        accepted.close(ApplicationCode::default(), b"");
        server.close(ApplicationCode::default(), b"");
        client.close(ApplicationCode::default(), b"");
        tokio::join!(server.wait_idle(), client.wait_idle());
    })
    .await;
}

#[tokio::test]
async fn idle_timeout_reports_non_crypto_close() {
    bounded(async {
        let pki = Pki::new();
        let options = TransportOptions {
            idle_timeout: Some(Duration::from_millis(250)),
            ..TransportOptions::default()
        };
        let server = Endpoint::server_with_socket(
            pki.server_config(pki.roots.clone()),
            socket(),
            options.clone(),
        )
        .expect("test operation succeeds");
        let client = Endpoint::client_with_socket(pki.client_config(true), socket(), options)
            .expect("test operation succeeds");
        let (s, c) = pair(&server, &client).await;
        for connection in [&s, &c] {
            let info = connection.closed().await;
            let close = info
                .transport_close()
                .expect("idle timeout carries transport detail");
            assert_eq!(close.kind(), TransportCloseKind::TimedOut);
            assert_eq!(close.code(), None);
            assert_eq!(close.tls_alert(), None);
        }
        server.close(ApplicationCode::default(), b"");
        client.close(ApplicationCode::default(), b"");
        tokio::join!(server.wait_idle(), client.wait_idle());
    })
    .await;
}

#[tokio::test]
async fn application_and_local_closes_remain_distinct() {
    bounded(async {
        let pki = Pki::new();
        let (server, client) = pki.endpoints();
        let (s, c) = pair(&server, &client).await;
        s.close(
            ApplicationCode::new(9).expect("test operation succeeds"),
            b"local",
        );
        let local = s.closed().await;
        assert!(local.transport_close().is_none());
        assert!(matches!(local, CloseInfo::Local));
        let peer = c.closed().await;
        assert!(peer.transport_close().is_none());
        assert!(
            matches!(peer, CloseInfo::Application { code, reason } if code.get() == 9 && reason == b"local")
        );
        server.close(ApplicationCode::default(), b"");
        client.close(ApplicationCode::default(), b"");
        tokio::join!(server.wait_idle(), client.wait_idle());
    })
    .await;
}

fn assert_crypto_close(error: &TransportError, alert: AlertDescription) {
    let close = error
        .transport_close()
        .expect("connection failure carries a transport close");
    assert_eq!(close.kind(), TransportCloseKind::Crypto);
    assert_eq!(close.tls_alert(), Some(alert));
    assert_eq!(
        close.code().expect("crypto close carries a code").get(),
        0x100 + u64::from(u8::from(alert))
    );
}

#[tokio::test]
async fn invalid_credentials_and_reload_do_not_end_listener() {
    bounded(async {
        let pki = Pki::new();
        let other = Pki::new();
        let (server, client) = pki.endpoints();
        let (s, c) = pair(&server, &client).await;
        let anonymous = Endpoint::client(
            pki.client_config(false),
            "127.0.0.1:0".parse().expect("test operation succeeds"),
            TransportOptions::default(),
        )
        .expect("test operation succeeds");
        let (connected, accepted) = attempt(&server, &anonymous, "localhost").await;
        assert!(accepted.is_err());
        if let Ok(c) = connected {
            assert!(matches!(c.closed().await, CloseInfo::Transport(_)));
        }
        let (connected, accepted) = attempt(&server, &client, "wrong-name").await;
        assert!(connected.is_err());
        assert!(accepted.is_err());
        let untrusted = Endpoint::client(
            other.client_config(true),
            "127.0.0.1:0".parse().expect("test operation succeeds"),
            TransportOptions::default(),
        )
        .expect("test operation succeeds");
        let (connected, accepted) = attempt(&server, &untrusted, "localhost").await;
        assert!(connected.is_err());
        assert!(accepted.is_err());

        server
            .set_server_config(pki.server_config(other.roots.clone()))
            .expect("test operation succeeds");
        let (connected, accepted) = attempt(&server, &client, "localhost").await;
        assert!(accepted.is_err());
        if let Ok(c) = connected {
            c.closed().await;
        }
        // The old session still has its identity and can exchange streams.
        assert_eq!(
            s.peer_certificates().expect("test operation succeeds"),
            pki.client.chain
        );
        let mut send = c.open_uni().await.expect("test operation succeeds");
        send.write_all(b"old session")
            .await
            .expect("test operation succeeds");
        send.finish().expect("test operation succeeds");
        let mut data = Vec::new();
        s.accept_uni()
            .await
            .expect("test operation succeeds")
            .read_to_end(&mut data)
            .await
            .expect("test operation succeeds");
        assert_eq!(data, b"old session");
        server
            .set_server_config(pki.server_config(pki.roots.clone()))
            .expect("test operation succeeds");
        let _new = pair(&server, &client).await;
        server.close(ApplicationCode::default(), b"");
    })
    .await;
}

#[tokio::test]
async fn abort_stop_cancellation_and_partial_frame() {
    bounded(async {
        let pki = Pki::new();
        let (server, client) = pki.endpoints();
        let (s, c) = pair(&server, &client).await;
        // Cancel an accept before any stream is available.
        assert!(futures::poll!(Box::pin(s.accept_uni())).is_pending());
        let mut send = c.open_uni().await.expect("test operation succeeds");
        send.write_all(&[0, 0])
            .await
            .expect("test operation succeeds");
        let recv = s.accept_uni().await.expect("test operation succeeds");
        let mut reader =
            FramedReader::new(recv, FrameLimit::new(64).expect("test operation succeeds"));
        // Allow the prefix to be consumed, then cancel the incomplete frame read.
        assert!(
            tokio::time::timeout(Duration::from_millis(20), reader.read())
                .await
                .is_err()
        );
        send.write_all(&[0, 3, b'a'])
            .await
            .expect("test operation succeeds");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), reader.read())
                .await
                .is_err()
        );
        send.write_all(b"bc")
            .await
            .expect("test operation succeeds");
        assert_eq!(
            reader
                .read()
                .await
                .expect("test operation succeeds")
                .expect("test operation succeeds")
                .as_bytes(),
            b"abc"
        );
        send.reset(ApplicationCode::new(42).expect("test operation succeeds"))
            .expect("test operation succeeds");
        assert!(send.stopped().await.is_err());
        let error = reader.read().await.unwrap_err();
        assert!(matches!(error, FrameError::Io(_)));
        assert!(error.source().is_some());

        let mut send = s.open_uni().await.expect("test operation succeeds");
        send.write_all(b"stop me")
            .await
            .expect("test operation succeeds");
        let mut recv = c.accept_uni().await.expect("test operation succeeds");
        recv.stop(ApplicationCode::new(71).expect("test operation succeeds"))
            .expect("test operation succeeds");
        assert_eq!(
            send.stopped()
                .await
                .expect("test operation succeeds")
                .expect("test operation succeeds")
                .get(),
            71
        );
        server.close(ApplicationCode::default(), b"");
    })
    .await;
}

#[tokio::test]
async fn owned_incoming_streams_and_clones_retain_endpoint() {
    bounded(async {
        let pki = Pki::new();
        let (server, client) = pki.endpoints();
        let server_clone = server.clone();
        drop(server);
        let server = server_clone;
        let server_addr = server.local_addr().expect("bound server address");
        let accept = async {
            let incoming = server.accept().await.expect("test operation succeeds");
            drop(server);
            incoming.await.expect("test operation succeeds")
        };
        let (s, c) = tokio::join!(
            accept,
            client.connect(server_addr, "localhost")
        );
        let c = c.expect("test operation succeeds");
        let c_clone = c.clone();
        drop(c);
        drop(client);
        let mut send = c_clone.open_uni().await.expect("test operation succeeds");
        send.write_all(b"kept").await.expect("test operation succeeds");
        let mut recv = s.accept_uni().await.expect("test operation succeeds");
        drop(s);
        drop(c_clone);
        send.write_all(b" alive").await.expect("test operation succeeds");
        send.finish().expect("test operation succeeds");
        let mut bytes = Vec::new();
        recv.read_to_end(&mut bytes).await.expect("test operation succeeds");
        assert_eq!(bytes, b"kept alive");
        let (server, client) = pki.endpoints();
        let server_clone = server.clone();
        drop(server);
        let (_s, c) = pair(&server_clone, &client).await;
        server_clone.close(ApplicationCode::new(5).expect("test operation succeeds"), b"endpoint");
        assert!(matches!(c.closed().await, CloseInfo::Application { code, reason } if code.get() == 5 && reason == b"endpoint"));
        drop(recv);
        drop(send);
        server_clone.wait_idle().await;
    })
    .await;
}

#[test]
fn checked_options_and_codes() {
    let default = TransportOptions::default();
    default.validate().expect("test operation succeeds");
    assert_eq!(default.idle_timeout, Some(Duration::from_secs(30)));
    assert_eq!(default.keep_alive_interval, None);
    assert_eq!(default.max_concurrent_bidi_streams, 100);
    assert_eq!(
        ApplicationCode::new(ApplicationCode::MAX)
            .expect("test operation succeeds")
            .get(),
        ApplicationCode::MAX
    );
    assert!(ApplicationCode::new(u64::MAX).is_err());
    assert_eq!(
        TransportCode::new(TransportCode::MAX)
            .expect("test operation succeeds")
            .get(),
        TransportCode::MAX
    );
    assert!(TransportCode::new(TransportCode::MAX + 1).is_err());
    assert_eq!(
        TransportCode::new(0x100 + u64::from(u8::from(AlertDescription::HandshakeFailure)))
            .expect("test operation succeeds")
            .tls_alert(),
        Some(AlertDescription::HandshakeFailure)
    );
    assert_eq!(
        TransportCode::new(42)
            .expect("test operation succeeds")
            .tls_alert(),
        None
    );
    for options in [
        TransportOptions {
            max_concurrent_bidi_streams: (1 << 60) + 1,
            ..default.clone()
        },
        TransportOptions {
            max_concurrent_uni_streams: (1 << 60) + 1,
            ..default.clone()
        },
        TransportOptions {
            keep_alive_interval: Some(Duration::MAX),
            idle_timeout: None,
            ..default.clone()
        },
        TransportOptions {
            keep_alive_interval: Some(Duration::ZERO),
            ..default.clone()
        },
        TransportOptions {
            keep_alive_interval: Some(Duration::from_secs(30)),
            ..default.clone()
        },
        TransportOptions {
            idle_timeout: Some(Duration::MAX),
            ..default.clone()
        },
        TransportOptions {
            max_concurrent_bidi_streams: u64::MAX,
            ..default.clone()
        },
        TransportOptions {
            max_concurrent_uni_streams: u64::MAX,
            ..default.clone()
        },
        TransportOptions {
            stream_receive_window: u64::MAX,
            ..default.clone()
        },
        TransportOptions {
            receive_window: u64::MAX,
            ..default
        },
    ] {
        let error = options.validate().unwrap_err();
        assert_eq!(error.kind(), TransportErrorKind::InvalidOptions);
        assert!(error.source().is_some());
    }
    TransportOptions {
        idle_timeout: None,
        keep_alive_interval: Some(Duration::from_secs(1)),
        ..TransportOptions::default()
    }
    .validate()
    .expect("test operation succeeds");
}

#[test]
fn keepalive_with_disabled_idle_timeout() {
    for idle_timeout in [
        None,
        Some(Duration::ZERO),
        Some(Duration::from_nanos(1)),
        Some(Duration::from_nanos(999_999)),
    ] {
        let mut options = TransportOptions {
            idle_timeout,
            keep_alive_interval: Some(Duration::from_secs(1)),
            ..TransportOptions::default()
        };
        options
            .validate()
            .expect("positive keepalive is valid with a disabled idle timeout");

        options.keep_alive_interval = Some(Duration::ZERO);
        assert_eq!(
            options.validate().unwrap_err().kind(),
            TransportErrorKind::InvalidOptions
        );
    }
}

#[test]
fn keepalive_must_be_shorter_than_enabled_idle_timeout() {
    let mut options = TransportOptions {
        idle_timeout: Some(Duration::from_millis(1)),
        keep_alive_interval: Some(Duration::from_micros(500)),
        ..TransportOptions::default()
    };
    options.validate().expect("shorter keepalive is valid");
    for interval in [Duration::from_millis(1), Duration::from_secs(1)] {
        options.keep_alive_interval = Some(interval);
        assert_eq!(
            options.validate().unwrap_err().kind(),
            TransportErrorKind::InvalidOptions
        );
    }
}

#[tokio::test]
async fn framed_write_resumes_after_flow_control_cancellation() {
    bounded(async {
        let pki = Pki::new();
        let options = TransportOptions {
            stream_receive_window: 1024,
            receive_window: 1024,
            send_window: 1024,
            ..TransportOptions::default()
        };
        let server = Endpoint::server(
            pki.server_config(pki.roots.clone()),
            "127.0.0.1:0".parse().expect("test operation succeeds"),
            options.clone(),
        )
        .expect("test operation succeeds");
        let client = Endpoint::client(
            pki.client_config(true),
            "127.0.0.1:0".parse().expect("test operation succeeds"),
            options,
        )
        .expect("test operation succeeds");
        let (s, c) = pair(&server, &client).await;
        let send = c.open_uni().await.expect("test operation succeeds");
        let limit = FrameLimit::new(256 * 1024).expect("test operation succeeds");
        let payload = vec![0x5a; 256 * 1024];
        let mut writer = FramedWriter::new(send, limit);
        writer.start_raw(&payload).expect("test operation succeeds");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), writer.flush())
                .await
                .is_err()
        );
        assert!(writer.is_pending());
        let mut reader = FramedReader::new(
            s.accept_uni().await.expect("test operation succeeds"),
            limit,
        );
        let (flushed, frame) = tokio::join!(writer.flush(), reader.read());
        flushed.expect("test operation succeeds");
        assert_eq!(
            frame
                .expect("test operation succeeds")
                .expect("test operation succeeds")
                .as_bytes(),
            payload
        );
        assert!(!writer.is_pending());
        writer
            .into_inner()
            .finish()
            .expect("test operation succeeds");
        assert!(
            reader
                .read()
                .await
                .expect("test operation succeeds")
                .is_none()
        );
        server.close(ApplicationCode::default(), b"");
    })
    .await;
}

#[test]
fn keepalive_is_compared_with_encoded_idle_timeout() {
    let mut options = TransportOptions {
        idle_timeout: Some(Duration::from_micros(1_000_900)),
        keep_alive_interval: Some(Duration::from_micros(999_999)),
        ..TransportOptions::default()
    };
    options
        .validate()
        .expect("keepalive below encoded timeout is valid");
    for interval in [
        Duration::from_millis(1000),
        Duration::from_micros(1_000_500),
        Duration::from_micros(1_000_900),
    ] {
        options.keep_alive_interval = Some(interval);
        assert_eq!(
            options.validate().unwrap_err().kind(),
            TransportErrorKind::InvalidOptions
        );
    }
}

#[tokio::test]
async fn framed_stream_reads_reuse_recycled_storage() {
    bounded(async {
        let pki = Pki::new();
        let (server, client) = pki.endpoints();
        let (s, c) = pair(&server, &client).await;
        let limit = FrameLimit::new(1024).expect("valid frame limit");
        let send = c.open_uni().await.expect("open stream");
        let mut writer = FramedWriter::new(send, limit);
        // Start with the largest payload so every later frame fits its allocation.
        let payloads: &[&[u8]] = &[
            b"first and largest payload",
            b"second",
            b"",
            b"fourth payload",
            b"last",
        ];
        for payload in payloads {
            writer.start_raw(payload).expect("prepare frame");
            writer.flush().await.expect("send frame");
        }
        let mut send = writer.into_inner();
        send.finish().expect("finish stream");
        let recv = s.accept_uni().await.expect("accept stream");
        let mut reader = FramedReader::new(recv, limit);
        let mut allocation = None;
        for payload in payloads {
            let frame = reader
                .read()
                .await
                .expect("read frame")
                .expect("frame before EOF");
            assert_eq!(frame.as_bytes(), *payload);
            let buffer = frame.into_bytes();
            let storage = (buffer.as_ptr(), buffer.capacity());
            if let Some(expected) = allocation {
                assert_eq!(storage, expected, "recycled allocation must be reused");
            } else {
                allocation = Some(storage);
            }
            reader.recycle(buffer).expect("recycle frame buffer");
        }
        assert!(reader.read().await.expect("read EOF").is_none());
        assert_eq!(send.stopped().await.expect("acknowledged FIN"), None);
        server.close(ApplicationCode::default(), b"");
    })
    .await;
}
