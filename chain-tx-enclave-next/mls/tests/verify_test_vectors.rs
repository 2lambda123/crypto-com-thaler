use mls::ciphersuite::P256;
use mls::error::KeyPackageError;
use mls::Codec;
use mls::{keypackage::DEFAULT_LIFE_TIME, KeyPackage};
use ra_client::ENCLAVE_CERT_VERIFIER;

/// FIXME enable this test after golang version catch up with spec
#[test]
#[ignore]
fn verify_keypackage_test_vector_mock() {
    // keypackage_mock.bin is generated by golang implementation
    static VECTOR: &[u8] = include_bytes!("test_vectors/keypackage_mock.bin");
    let kp = KeyPackage::<P256>::read_bytes(VECTOR).expect("decode");
    assert!(matches!(
        kp.verify(&*ENCLAVE_CERT_VERIFIER, 0),
        Err(KeyPackageError::InvalidCredential)
    ));
}

#[test]
fn verify_keypackage_test_vector() {
    static VECTOR: &[u8] = include_bytes!("test_vectors/keypackage.bin");

    let kp = KeyPackage::<P256>::read_bytes(VECTOR).expect("decode");
    let now = 1596448111;
    let expire = now + DEFAULT_LIFE_TIME;
    kp.verify(&*ENCLAVE_CERT_VERIFIER, now).unwrap();
    assert!(matches!(
        kp.verify(&*ENCLAVE_CERT_VERIFIER, expire + 1),
        Err(KeyPackageError::NotAfter(_))
    ));
}
