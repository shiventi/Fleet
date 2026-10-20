use super::*;
use rustls::{ClientConfig, ClientConnection, Connection, ServerConfig, ServerConnection};
use std::io::{Read, Write};

fn peer(identity: &DeviceIdentity) -> TrustedPeer {
    TrustedPeer {
        name: "server".into(),
        public_key: identity.public_key_pem(),
    }
}

fn approved_clients(identities: &[&DeviceIdentity]) -> TrustStore {
    let mut store = TrustStore::new();
    for (index, identity) in identities.iter().enumerate() {
        store
            .add(TrustedPeer {
                name: format!("client-{index}"),
                public_key: identity.public_key_pem(),
            })
            .expect("approve test client");
    }
    store
}

fn configured(client: ClientConfig, server: ServerConfig) -> (Connection, Connection) {
    let name = ServerName::try_from("localhost").expect("test server name");
    (
        Connection::Client(ClientConnection::new(Arc::new(client), name).expect("test client")),
        Connection::Server(ServerConnection::new(Arc::new(server)).expect("test server")),
    )
}

#[derive(Debug)]
struct MissingClientKey;

impl rustls::client::ResolvesClientCert for MissingClientKey {
    fn resolve(&self, _hints: &[&[u8]], _schemes: &[SignatureScheme]) -> Option<Arc<CertifiedKey>> {
        None
    }

    fn only_raw_public_keys(&self) -> bool {
        true
    }

    fn has_certs(&self) -> bool {
        false
    }
}

fn connections(expected: &DeviceIdentity, bundle: Arc<CertifiedKey>) -> (Connection, Connection) {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = PinnedServerVerifier::new(&peer(expected)).expect("pin test server");
    let client = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 client")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    // check server keys on their own
    let server = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("TLS 1.3 server")
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(
            rustls::server::AlwaysResolvesServerRawPublicKeys::new(bundle),
        ));
    let name = ServerName::try_from("localhost").expect("test server name");
    (
        Connection::Client(ClientConnection::new(Arc::new(client), name).expect("test client")),
        Connection::Server(ServerConnection::new(Arc::new(server)).expect("test server")),
    )
}

fn transfer(sender: &mut Connection, receiver: &mut Connection) -> Result<bool, rustls::Error> {
    let mut bytes = Vec::new();
    while sender.wants_write() {
        assert!(sender.write_tls(&mut bytes).expect("write TLS records") > 0);
    }
    let sent = !bytes.is_empty();
    let mut input = bytes.as_slice();
    while !input.is_empty() {
        assert!(receiver.read_tls(&mut input).expect("read TLS records") > 0);
        receiver.process_new_packets()?;
    }
    Ok(sent)
}

fn handshake(client: &mut Connection, server: &mut Connection) -> Result<(), rustls::Error> {
    for _ in 0..16 {
        let sent = transfer(client, server)?;
        let replied = transfer(server, client)?;
        if !client.is_handshaking() && !server.is_handshaking() {
            return Ok(());
        }
        assert!(sent || replied, "TLS handshake stopped making progress");
    }
    panic!("TLS handshake exceeded the test round limit");
}

#[test]
fn mutual_handshake_accepts_each_approved_client_and_exchanges_data() {
    let server = DeviceIdentity::generate().expect("server identity");
    let first = DeviceIdentity::generate().expect("first client");
    let second = DeviceIdentity::generate().expect("second client");
    let store = approved_clients(&[&first, &second]);
    for identity in [&first, &second] {
        let client = client_config(identity, &peer(&server)).expect("device-key client config");
        let server = server_config(&server, &store).expect("device-key server config");
        let (mut client, mut server) = configured(client, server);
        handshake(&mut client, &mut server).expect("mutually approved handshake");
        assert_eq!(
            client.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );
        assert_eq!(
            server.protocol_version(),
            Some(rustls::ProtocolVersion::TLSv1_3)
        );
        assert_eq!(
            server
                .peer_certificates()
                .expect("authenticated client key")[0],
            local_key(identity).unwrap().cert[0]
        );

        client
            .writer()
            .write_all(b"request")
            .expect("send authenticated request");
        transfer(&mut client, &mut server).expect("transfer authenticated request");
        let mut request = [0; 7];
        server
            .reader()
            .read_exact(&mut request)
            .expect("read authenticated request");
        assert_eq!(&request, b"request");
        server
            .writer()
            .write_all(b"reply")
            .expect("send authenticated reply");
        transfer(&mut server, &mut client).expect("transfer authenticated reply");
        let mut reply = [0; 5];
        client
            .reader()
            .read_exact(&mut reply)
            .expect("read authenticated reply");
        assert_eq!(&reply, b"reply");
    }
}

#[test]
fn mutual_handshake_rejects_an_unknown_client_key() {
    let server = DeviceIdentity::generate().unwrap();
    let approved = DeviceIdentity::generate().unwrap();
    let unknown = DeviceIdentity::generate().unwrap();
    let store = approved_clients(&[&approved]);
    let client = client_config(&unknown, &peer(&server)).unwrap();
    let server = server_config(&server, &store).unwrap();
    let (mut client, mut server) = configured(client, server);
    let error = handshake(&mut client, &mut server).expect_err("reject unknown client");
    assert!(
        matches!(error, rustls::Error::General(ref message) if message == "Client key is not approved")
    );
    assert!(server.is_handshaking());
}

#[test]
fn mutual_handshake_requires_a_client_identity() {
    let server = DeviceIdentity::generate().unwrap();
    let approved = DeviceIdentity::generate().unwrap();
    let store = approved_clients(&[&approved]);
    let mut client = client_config(&approved, &peer(&server)).unwrap();
    client.client_auth_cert_resolver = Arc::new(MissingClientKey);
    let server = server_config(&server, &store).unwrap();
    let (mut client, mut server) = configured(client, server);
    let error = handshake(&mut client, &mut server).expect_err("reject missing client identity");
    assert!(matches!(error, rustls::Error::NoCertificatesPresented));
    assert!(server.is_handshaking());
}

#[test]
fn mutual_handshake_rejects_an_approved_client_key_with_an_attackers_signer() {
    let server = DeviceIdentity::generate().unwrap();
    let approved = DeviceIdentity::generate().unwrap();
    let attacker = DeviceIdentity::generate().unwrap();
    let store = approved_clients(&[&approved]);
    let approved_key = local_key(&approved).unwrap();
    let forged = Arc::new(CertifiedKey::new(
        approved_key.cert.clone(),
        attacker.signing_key().unwrap(),
    ));
    TrustedClientVerifier::new(&store)
        .unwrap()
        .check_key(&forged.cert[0])
        .expect("copied client key matches");
    let mut client = client_config(&approved, &peer(&server)).unwrap();
    client.client_auth_cert_resolver = Arc::new(AlwaysResolvesClientRawPublicKeys::new(forged));
    let server = server_config(&server, &store).unwrap();
    let (mut client, mut server) = configured(client, server);
    let error = handshake(&mut client, &mut server).expect_err("reject forged client signature");
    assert!(matches!(
        error,
        rustls::Error::InvalidCertificate(rustls::CertificateError::BadSignature)
    ));
    assert!(server.is_handshaking());
}

#[test]
fn mutual_handshake_still_rejects_an_unexpected_server_key() {
    let expected = DeviceIdentity::generate().unwrap();
    let wrong = DeviceIdentity::generate().unwrap();
    let client = DeviceIdentity::generate().unwrap();
    let store = approved_clients(&[&client]);
    let client = client_config(&client, &peer(&expected)).unwrap();
    let server = server_config(&wrong, &store).unwrap();
    let (mut client, mut server) = configured(client, server);
    let error = handshake(&mut client, &mut server).expect_err("reject wrong server");
    assert!(matches!(error, rustls::Error::General(ref message)
        if message == "Server key does not match the approved device"));
}

#[test]
fn server_config_rejects_an_empty_approved_client_list() {
    let identity = DeviceIdentity::generate().unwrap();
    let error =
        server_config(&identity, &TrustStore::new()).expect_err("reject empty approval list");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn client_verifier_rejects_extra_certificate_chain_entries() {
    let identity = DeviceIdentity::generate().unwrap();
    let store = approved_clients(&[&identity]);
    let verifier = TrustedClientVerifier::new(&store).unwrap();
    assert!(verifier.offer_client_auth());
    assert!(verifier.client_auth_mandatory());
    assert!(verifier.requires_raw_public_keys());
    assert!(verifier.root_hint_subjects().is_empty());
    let bundle = local_key(&identity).unwrap();
    assert!(
        verifier
            .verify_client_cert(&bundle.cert[0], &bundle.cert, UnixTime::now())
            .is_err()
    );
}

#[test]
fn rebuilding_server_config_rejects_a_removed_client() {
    let server = DeviceIdentity::generate().unwrap();
    let removed = DeviceIdentity::generate().unwrap();
    let remaining = DeviceIdentity::generate().unwrap();
    let mut store = approved_clients(&[&removed, &remaining]);
    assert!(store.remove("client-0"));
    let client = client_config(&removed, &peer(&server)).unwrap();
    let server = server_config(&server, &store).unwrap();
    let (mut client, mut server) = configured(client, server);
    let error = handshake(&mut client, &mut server).expect_err("reject removed client");
    assert!(
        matches!(error, rustls::Error::General(ref message) if message == "Client key is not approved")
    );
}

#[test]
fn approved_server_completes_tls13_handshake_and_exchanges_data() {
    let identity = DeviceIdentity::generate().expect("approved test identity");
    let (mut client, mut server) = connections(&identity, local_key(&identity).unwrap());
    handshake(&mut client, &mut server).expect("approved server handshake");
    assert_eq!(
        client.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
    assert_eq!(
        server.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );

    client.writer().write_all(b"request").expect("send request");
    transfer(&mut client, &mut server).expect("transfer request");
    let mut request = [0; 7];
    server
        .reader()
        .read_exact(&mut request)
        .expect("read request");
    assert_eq!(&request, b"request");
    server.writer().write_all(b"reply").expect("send reply");
    transfer(&mut server, &mut client).expect("transfer reply");
    let mut reply = [0; 5];
    client.reader().read_exact(&mut reply).expect("read reply");
    assert_eq!(&reply, b"reply");
}

#[test]
fn handshake_rejects_a_different_server_key() {
    let expected = DeviceIdentity::generate().expect("approved test identity");
    let other = DeviceIdentity::generate().expect("different test identity");
    let (mut client, mut server) = connections(&expected, local_key(&other).unwrap());
    let error = handshake(&mut client, &mut server).expect_err("reject wrong server key");
    assert!(matches!(error, rustls::Error::General(ref message)
        if message == "Server key does not match the approved device"));
    assert!(client.is_handshaking());
}

#[test]
fn handshake_rejects_the_approved_public_key_with_an_attackers_signer() {
    let expected = DeviceIdentity::generate().expect("approved test identity");
    let attacker = DeviceIdentity::generate().expect("attacker test identity");
    let approved = local_key(&expected).expect("approved public key");
    let forged = Arc::new(CertifiedKey::new(
        approved.cert.clone(),
        attacker.signing_key().unwrap(),
    ));
    let verifier = PinnedServerVerifier::new(&peer(&expected)).unwrap();
    verifier
        .check_key(&forged.cert[0])
        .expect("copied public key alone matches");

    let (mut client, mut server) = connections(&expected, forged);
    let error =
        handshake(&mut client, &mut server).expect_err("reject invalid handshake signature");
    assert!(matches!(
        error,
        rustls::Error::InvalidCertificate(rustls::CertificateError::BadSignature)
    ));
    assert!(client.is_handshaking());
}

#[test]
fn verifier_rejects_certificate_chain_entries_with_raw_keys() {
    let identity = DeviceIdentity::generate().expect("test identity");
    let verifier = PinnedServerVerifier::new(&peer(&identity)).unwrap();
    let bundle = local_key(&identity).unwrap();
    let name = ServerName::try_from("localhost").unwrap();
    let result =
        verifier.verify_server_cert(&bundle.cert[0], &bundle.cert, &name, &[], UnixTime::now());
    assert!(matches!(result, Err(rustls::Error::General(_))));
}

#[test]
fn local_bundle_contains_the_identitys_public_key_and_signer() {
    let identity = DeviceIdentity::generate().expect("test identity");
    let bundle = local_key(&identity).expect("local key bundle");
    let public = p256::PublicKey::from_public_key_pem(&identity.public_key_pem())
        .expect("public key")
        .to_public_key_der()
        .expect("public DER");

    assert_eq!(bundle.cert.len(), 1);
    assert_eq!(bundle.cert[0].as_ref(), public.as_bytes());
    assert_eq!(
        bundle.key.public_key().expect("signer public key").as_ref(),
        public.as_bytes()
    );
}

#[test]
fn pinned_server_accepts_only_the_expected_devices_key() {
    let identity = DeviceIdentity::generate().expect("expected identity");
    let other = DeviceIdentity::generate().expect("different identity");
    let verifier = PinnedServerVerifier::new(&peer(&identity)).expect("pin expected key");

    assert!(
        verifier
            .check_key(&local_key(&identity).unwrap().cert[0])
            .is_ok()
    );
    assert!(
        verifier
            .check_key(&local_key(&other).unwrap().cert[0])
            .is_err()
    );
    assert!(
        verifier
            .check_key(&CertificateDer::from(Vec::new()))
            .is_err()
    );
    assert!(
        verifier
            .check_key(&CertificateDer::from(b"not a key".to_vec()))
            .is_err()
    );
}

#[test]
fn pin_ignores_pem_formatting_and_keeps_its_own_copy() {
    let identity = DeviceIdentity::generate().expect("expected identity");
    let mut saved_peer = peer(&identity);
    saved_peer.public_key = format!("\n{}\n", saved_peer.public_key.replace('\n', "\r\n"));
    let verifier = PinnedServerVerifier::new(&saved_peer).expect("pin formatted key");
    saved_peer.public_key.clear();
    drop(saved_peer);

    assert!(
        verifier
            .check_key(&local_key(&identity).unwrap().cert[0])
            .is_ok()
    );
}

#[test]
fn pin_rejects_malformed_private_and_unsupported_keys() {
    let private = rcgen::KeyPair::generate()
        .expect("test private key")
        .serialize_pem();
    let unsupported = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .expect("unsupported test key")
        .public_key_pem();
    for public_key in [String::new(), "not a key".into(), private, unsupported] {
        let peer = TrustedPeer {
            name: "bad".into(),
            public_key,
        };
        assert!(PinnedServerVerifier::new(&peer).is_err());
    }
}
