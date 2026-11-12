//! GoldLink's release signatures (dev/docs/unified-client/UC10_UPDATER.md): the maintainer signs
//! each release's `SHA256SUMS` with an Ed25519 key only they hold; the launcher checks the
//! signature against the public key built into it before it installs an update.
//!
//! What's signed is `"GoldLink release\n" + tag + "\n" + SHA256SUMS`: the tag inside stops an
//! older release's files being offered as a newer one.

use ring::rand::SystemRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};

/// The bytes a release's signature covers.
pub fn signed_message(tag: &str, sums: &[u8]) -> Vec<u8> {
    let mut m = format!("GoldLink release\n{tag}\n").into_bytes();
    m.extend_from_slice(sums);
    m
}

/// A new key: its PKCS#8 document (the private key, to keep) and its public key.
pub fn generate() -> Result<(Vec<u8>, [u8; 32]), String> {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| "no random numbers for a key".to_owned())?;
    let public = public_key(pkcs8.as_ref())?;
    Ok((pkcs8.as_ref().to_vec(), public))
}

/// The public key of a PKCS#8 key.
pub fn public_key(pkcs8: &[u8]) -> Result<[u8; 32], String> {
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| "not an Ed25519 key".to_owned())?;
    pair.public_key()
        .as_ref()
        .try_into()
        .map_err(|_| "unexpected public key size".to_owned())
}

/// The signature (hex, one line) of release `tag`'s `SHA256SUMS`.
pub fn sign(pkcs8: &[u8], tag: &str, sums: &[u8]) -> Result<String, String> {
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| "not an Ed25519 key".to_owned())?;
    Ok(hex::encode(pair.sign(&signed_message(tag, sums)).as_ref()))
}

/// Whether `signature` (hex, surrounding whitespace allowed) is `key`'s for release `tag`'s
/// `SHA256SUMS`.
pub fn verify(key: &[u8; 32], tag: &str, sums: &[u8], signature: &str) -> Result<(), String> {
    let sig =
        hex::decode(signature.trim()).map_err(|_| "the signature isn't readable".to_owned())?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&signed_message(tag, sums), &sig)
        .map_err(|_| "the signature doesn't match GoldLink's key".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_and_checks() {
        let (key, public) = generate().unwrap();
        assert_eq!(public_key(&key).unwrap(), public);
        let sums = b"abc  GoldLink-v0.6.1-linux.tar.gz\n";
        let sig = sign(&key, "v0.6.1", sums).unwrap();
        assert_eq!(verify(&public, "v0.6.1", sums, &format!("{sig}\n")), Ok(()));
        // Another tag, other sums, another key, a damaged signature: refused.
        assert!(verify(&public, "v0.6.2", sums, &sig).is_err());
        assert!(
            verify(
                &public,
                "v0.6.1",
                b"abd  GoldLink-v0.6.1-linux.tar.gz\n",
                &sig
            )
            .is_err()
        );
        let (_, other) = generate().unwrap();
        assert!(verify(&other, "v0.6.1", sums, &sig).is_err());
        assert!(verify(&public, "v0.6.1", sums, "zz").is_err());
        assert!(verify(&public, "v0.6.1", sums, &sig[2..]).is_err());
    }
}
