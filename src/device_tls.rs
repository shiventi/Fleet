//! Creates TLS 1.3 configs that verify both devices using approved public keys.
//! Used by device-key agent, control, watch, and dashboard commands.
//! All commands still require a shared token; peer approvals are loaded at setup.

use crate::identity::DeviceIdentity;
use crate::trust::{TrustStore, TrustedPeer};
use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
use rustls::sign::CertifiedKey;
use std::io;
use std::sync::Arc;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::DistinguishedName;
use rustls::ServerConfig;
use rustls::server::AlwaysResolvesServerRawPublicKeys;
use rustls::ClientConfig;
use rustls::client::AlwaysResolvesClientRawPublicKeys;

/// Bundles our public key and signer without generating a certificate.
/// The certificate-shaped container holds raw public-key DER bytes, not X.509.
///
/// # Errors
/// Returns an error if the signer cannot be loaded or has no public key.
pub fn local_key(identity: &DeviceIdentity) -> io::Result<Arc<CertifiedKey>> {
    let signing_key = identity.signing_key()?;
    let pub_key = signing_key
        .public_key()
        .ok_or_else(|| io::Error::other("Signing key has no public key"))?;
    let pub_bytes = pub_key.as_ref().to_vec();

    let public_key = CertificateDer::from(pub_bytes);
    let bundle = CertifiedKey::new(vec![public_key], signing_key);
    Ok(Arc::new(bundle))
}

/// Verifies one approved P-256 server key and its TLS 1.3 handshake signature.
/// Identifies the server by its pinned key, not a certificate hostname or expiry.
/// Does not authenticate clients or load the approved peer list itself.
#[derive(Debug)]
pub struct PinnedServerVerifier {
    expected_key: Vec<u8>,
}

impl PinnedServerVerifier {
    /// Copies the expected key from a peer selected from the approved list.
    /// The caller must select the intended server, not an arbitrary approved peer.
    ///
    /// # Errors
    /// Returns an error for malformed, private, or unsupported keys.
    pub fn new(peer: &TrustedPeer) -> io::Result<Self> {
        let public_key = p256::PublicKey::from_public_key_pem(peer.public_key.trim())
            .map_err(io::Error::other)?;
        let der = public_key.to_public_key_der().map_err(io::Error::other)?;
        let expected_key = der.as_bytes().to_vec();
        Ok(Self { expected_key })
    }

    /// Rejects any presented key whose full DER bytes differ from the approved key.
    fn check_key(&self, presented: &CertificateDer<'_>) -> Result<(), rustls::Error> {
        if presented.as_ref() != self.expected_key.as_slice() {
            return Err(rustls::Error::General(
                "Server key does not match the approved device".into(),
            ));
        }
        Ok(())
    }
}

impl ServerCertVerifier for PinnedServerVerifier {
    /// Approves the pinned raw key and rejects extra certificate-chain entries.
    /// Rustls checks proof of private-key ownership in the signature callback.
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>], _server_name: &ServerName<'_>, _ocsp_response: &[u8], _now: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        if !intermediates.is_empty() {
            return Err(rustls::Error::General("Raw keys must not include a certificate chain".into(),));
        }
        self.check_key(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }
    /// Checks the pinned key, then delegates handshake signature checking to Rustls.
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.check_key(cert)?;
        let key = SubjectPublicKeyInfoDer::from(cert.as_ref());
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        rustls::crypto::verify_tls13_signature_with_raw_key(message, &key, dss, &provider.signature_verification_algorithms)
    }
    /// Rejects TLS 1.2. Device-key configurations must also require TLS 1.3.
    fn verify_tls12_signature(&self, _message: &[u8], _cert: &CertificateDer<'_>, _dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("Device-key TLS requires TLS 1.3".into()))
    }
    /// Advertises the P-256/SHA-256 signature scheme used by device identities.
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
    /// Requests raw public keys rather than X.509 certificates.
    fn requires_raw_public_keys(&self) -> bool {
        return true
    }
}

impl TrustedClientVerifier {
    /// Copies the approved client keys. Rebuild after changing the peer list.
    ///
    /// # Errors
    /// Returns an error if the list is empty or a key cannot be parsed.
    pub fn new(store: &TrustStore) -> io::Result<Self> {
        if store.peers().is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "No approved clients"));
        }
        let mut approved_keys = Vec::new();

        for peer in store.peers() {
            let pub_key = p256::PublicKey::from_public_key_pem(peer.public_key.trim()).map_err(io::Error::other)?;
            let der = pub_key.to_public_key_der().map_err(io::Error::other)?;
            approved_keys.push(der.as_bytes().to_vec());
        }
        Ok(Self { approved_keys })
    }

    /// Rejects keys that are not in the copied approval list.
    fn check_key(&self, presented: &CertificateDer<'_>) -> Result<(), rustls::Error> {
        let approved = self.approved_keys.iter().any(|key| {
            key.as_slice() == presented.as_ref()
        });
        if !approved {
            return Err(rustls::Error::General("Client key is not approved".into(),));
        }
        Ok(())
    }
}


impl ClientCertVerifier for TrustedClientVerifier {
    /// Requires one approved raw key, without a certificate chain.
    fn verify_client_cert(&self, end_entity: &CertificateDer<'_>, intermediates: &[CertificateDer<'_>], _now: UnixTime) -> Result<ClientCertVerified, rustls::Error> {
        if !intermediates.is_empty() {
            return Err(rustls::Error::General("Raw keys must not include a certificate chain".into(),));
        }
        self.check_key(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }
    /// Checks that the client owns the private key for its approved public key.
    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.check_key(cert)?;
        let key = SubjectPublicKeyInfoDer::from(cert.as_ref());
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        rustls::crypto::verify_tls13_signature_with_raw_key(message, &key, dss, &provider.signature_verification_algorithms)
    }
    /// Rejects TLS 1.2. Device-key configurations must also require TLS 1.3.
    fn verify_tls12_signature(&self, _message: &[u8], _cert: &CertificateDer<'_>, _dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("Device-key TLS requires TLS 1.3".into()))
    }
    /// Advertises the P-256/SHA-256 signature scheme used by device identities.
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
    /// Requests raw public keys rather than X.509 certificates.
    fn requires_raw_public_keys(&self) -> bool {
        return true
    }
    fn offer_client_auth(&self) -> bool {
        return true;
    }
    fn client_auth_mandatory(&self) -> bool {
        return true;
    }
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        return &[];
    }
}


/// Requires clients to prove ownership of a key in the supplied approval list.
/// Approvals are copied at setup; changing the store does not update this verifier.
#[derive(Debug)]
pub struct TrustedClientVerifier {
    approved_keys: Vec<Vec<u8>>,
}


/// Presents our device key and requires an approved client key using TLS 1.3.
/// Rebuild after changing approvals. Existing connections are not revoked.
///
/// # Errors
/// Returns an error if the keys or TLS configuration cannot be loaded.
pub fn server_config(identity: &DeviceIdentity, store: &TrustStore) -> io::Result<ServerConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = Arc::new(TrustedClientVerifier::new(store)?);
    let key = local_key(identity)?;

    let server_config = ServerConfig::builder_with_provider(provider).with_protocol_versions(&[&rustls::version::TLS13]).map_err(io::Error::other)?.with_client_cert_verifier(verifier).with_cert_resolver(Arc::new(AlwaysResolvesServerRawPublicKeys::new(key)));

    Ok(server_config)
}

/// Presents our device key and accepts only the selected server's key using TLS 1.3.
/// The caller must select the intended peer from its approved list.
///
/// # Errors
/// Returns an error if the keys or TLS configuration cannot be loaded.
pub fn client_config(identity: &DeviceIdentity, peer: &TrustedPeer) -> io::Result<ClientConfig> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let verifier = Arc::new(PinnedServerVerifier::new(peer)?);
    let key = local_key(identity)?;

    let client_config = ClientConfig::builder_with_provider(provider).with_protocol_versions(&[&rustls::version::TLS13]).map_err(io::Error::other)?.dangerous().with_custom_certificate_verifier(verifier).with_client_cert_resolver(Arc::new(AlwaysResolvesClientRawPublicKeys::new(key)));

    Ok(client_config)
}

#[cfg(test)]
mod tests;
