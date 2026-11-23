//! BLS combine and verify for the PNP threshold scheme: key versions, partial
//! signature collection, and combination with verification of the result.

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use threshold_bls::{
    group::Element,
    poly::Poly,
    schemes::bls12_377::G2Scheme,
    serialization,
    sig::{BlindScheme, BlindThresholdScheme, Scheme, ThresholdScheme},
};

pub type PublicKey = <G2Scheme as Scheme>::Public;

/// The length of a partial signature: an 8-byte length prefix, a 48-byte G1
/// point and the 4-byte share index.
pub const PARTIAL_SIGNATURE_LEN: usize = 60;

/// One key version's public data, checked for consistency when decoded.
#[derive(Debug, Clone)]
pub struct KeyVersion {
    pub key_version: u32,
    pub threshold: usize,
    polynomial: Poly<PublicKey>,
    public_key: PublicKey,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum KeyVersionError {
    #[error("the polynomial is not the exact encoding of a polynomial")]
    PolynomialEncoding,
    #[error("the public key is not the exact encoding of a public key")]
    PublicKeyEncoding,
    #[error("threshold {threshold} does not fit a polynomial with {coefficients} coefficients")]
    ThresholdMismatch {
        threshold: usize,
        coefficients: usize,
    },
    #[error("the public key is the identity point")]
    PublicKeyIsIdentity,
    #[error("the public key is not the polynomial's constant term")]
    PublicKeyNotConstantTerm,
}

#[derive(Debug, thiserror::Error)]
pub enum CombineError {
    #[error("aggregating the partial signatures failed: {0}")]
    Aggregate(String),
    #[error("verifying the combined signature failed: {0}")]
    Verify(String),
}

/// Decodes `bytes` as a `T` and refuses it unless it encodes back to the same
/// bytes. The library's decoder accepts trailing bytes.
fn decode_exact<T: serde::de::DeserializeOwned + serde::Serialize>(bytes: &[u8]) -> Option<T> {
    let value: T = serialization::deserialize(bytes).ok()?;
    (bincode::serialize(&value).ok()? == bytes).then_some(value)
}

impl KeyVersion {
    pub fn decode(
        key_version: u32,
        threshold: usize,
        polynomial_hex: &str,
        pub_key_base64: &str,
    ) -> Result<KeyVersion, KeyVersionError> {
        let polynomial: Poly<PublicKey> = hex::decode(polynomial_hex)
            .ok()
            .and_then(|bytes| decode_exact(&bytes))
            .ok_or(KeyVersionError::PolynomialEncoding)?;
        let public_key: PublicKey = BASE64
            .decode(pub_key_base64)
            .ok()
            .and_then(|bytes| decode_exact(&bytes))
            .ok_or(KeyVersionError::PublicKeyEncoding)?;

        if public_key == PublicKey::zero() {
            return Err(KeyVersionError::PublicKeyIsIdentity);
        }
        let coefficients = polynomial.degree() + 1;
        if threshold != coefficients {
            return Err(KeyVersionError::ThresholdMismatch {
                threshold,
                coefficients,
            });
        }
        if *polynomial.public_key() != public_key {
            return Err(KeyVersionError::PublicKeyNotConstantTerm);
        }
        Ok(KeyVersion {
            key_version,
            threshold,
            polynomial,
            public_key,
        })
    }

    pub fn verify_partial(&self, blinded_message: &[u8], partial: &[u8]) -> bool {
        G2Scheme::verify_blind_partial(&self.polynomial, blinded_message, partial).is_ok()
    }

    /// Aggregates the partial signatures and verifies the result against the
    /// public key.
    pub fn combine(
        &self,
        blinded_message: &[u8],
        partials: &[&[u8]],
    ) -> Result<Vec<u8>, CombineError> {
        let partials: Vec<Vec<u8>> = partials.iter().map(|p| p.to_vec()).collect();
        let combined = G2Scheme::aggregate(&self.polynomial, &partials)
            .map_err(|e| CombineError::Aggregate(e.to_string()))?;
        G2Scheme::blind_verify(&self.public_key, blinded_message, &combined)
            .map_err(|e| CombineError::Verify(e.to_string()))?;
        Ok(combined)
    }
}

/// The share index a partial signature carries: its last four bytes as a
/// little-endian u32. None unless the partial has the length of one.
pub fn partial_share_index(partial: &[u8]) -> Option<u32> {
    if partial.len() != PARTIAL_SIGNATURE_LEN {
        return None;
    }
    let index = partial[PARTIAL_SIGNATURE_LEN - 4..].try_into().ok()?;
    Some(u32::from_le_bytes(index))
}

/// Why a signer's partial signature was refused before it was counted.
#[derive(Debug, PartialEq)]
pub enum Rejection {
    NotBase64,
    WrongLength { len: usize },
    WrongShareIndex { expected: u32, received: u32 },
    FailedVerification,
}

/// A partial signature and the index of the signer that sent it.
type Collected = (usize, Vec<u8>);

/// The partial signatures collected for one blinded message. Partials that
/// passed verification are kept apart from those that did not, so a failed
/// combination only verifies the latter.
pub struct PartialSignatures<'k> {
    key: &'k KeyVersion,
    blinded_message: Vec<u8>,
    verified: Vec<Collected>,
    unverified: Vec<Collected>,
}

#[derive(Debug, PartialEq)]
pub enum CombineAttempt {
    NotEnough,
    Combined(Vec<u8>),
    /// The combination failed. These signer indices sent partials that failed
    /// verification and were dropped.
    Failed {
        rejected: Vec<usize>,
    },
}

impl<'k> PartialSignatures<'k> {
    pub fn new(key: &'k KeyVersion, blinded_message: Vec<u8>) -> Self {
        Self {
            key,
            blinded_message,
            verified: Vec::new(),
            unverified: Vec::new(),
        }
    }

    /// Checks the encoding, the length and, if given, the share index of a
    /// signer's partial signature, and keeps it for combination.
    pub fn add(
        &mut self,
        signer: usize,
        expected_index: Option<u32>,
        signature_base64: &str,
    ) -> Result<(), Rejection> {
        let partial = BASE64
            .decode(signature_base64)
            .map_err(|_| Rejection::NotBase64)?;
        let Some(received) = partial_share_index(&partial) else {
            return Err(Rejection::WrongLength { len: partial.len() });
        };
        if let Some(expected) = expected_index
            && expected != received
        {
            return Err(Rejection::WrongShareIndex { expected, received });
        }
        self.unverified.push((signer, partial));
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.verified.len() + self.unverified.len()
    }

    /// Combines the verified partials, then the unverified ones, without
    /// verifying each first. If that fails, verifies each unverified partial
    /// on its own, keeps the valid ones as verified and drops the rest. It does
    /// not combine again in the same call.
    pub fn try_combine(&mut self) -> CombineAttempt {
        if self.count() < self.key.threshold {
            return CombineAttempt::NotEnough;
        }
        let all: Vec<&[u8]> = self
            .verified
            .iter()
            .chain(&self.unverified)
            .map(|(_, partial)| partial.as_slice())
            .collect();
        if let Ok(combined) = self.key.combine(&self.blinded_message, &all) {
            return CombineAttempt::Combined(combined);
        }

        let mut rejected = Vec::new();
        for (signer, partial) in std::mem::take(&mut self.unverified) {
            if self.key.verify_partial(&self.blinded_message, &partial) {
                self.verified.push((signer, partial));
            } else {
                rejected.push(signer);
            }
        }
        CombineAttempt::Failed { rejected }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
    use serde::Deserialize;
    use threshold_bls::group::Element;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct KeyVector {
        key_version: u32,
        threshold: usize,
        polynomial: String,
        pub_key: String,
    }

    #[derive(Deserialize)]
    struct PartialVector {
        key: usize,
        share: usize,
        message: usize,
        partial: String,
        valid: bool,
    }

    #[derive(Deserialize)]
    struct CombinedVector {
        key: usize,
        message: usize,
        shares: Vec<usize>,
        signature: String,
    }

    #[derive(Deserialize)]
    struct Vectors {
        keys: Vec<KeyVector>,
        messages: Vec<String>,
        partials: Vec<PartialVector>,
        combined: Vec<CombinedVector>,
    }

    fn vectors() -> Vectors {
        serde_json::from_str(include_str!("../tests/data/combine_vectors.json")).unwrap()
    }

    fn key(vectors: &Vectors, index: usize) -> KeyVersion {
        let k = &vectors.keys[index];
        KeyVersion::decode(k.key_version, k.threshold, &k.polynomial, &k.pub_key).unwrap()
    }

    fn message(vectors: &Vectors, index: usize) -> Vec<u8> {
        hex::decode(&vectors.messages[index]).unwrap()
    }

    /// The honest partial of a share on a message, from the vectors.
    fn partial(vectors: &Vectors, key: usize, share: usize, message: usize) -> Vec<u8> {
        let vector = vectors
            .partials
            .iter()
            .find(|p| p.key == key && p.share == share && p.message == message && p.valid)
            .unwrap();
        hex::decode(&vector.partial).unwrap()
    }

    #[test]
    fn combined_signatures_equal_the_ts_library() {
        let vectors = vectors();
        for combined in &vectors.combined {
            let key = key(&vectors, combined.key);
            let message = message(&vectors, combined.message);
            let partials: Vec<Vec<u8>> = combined
                .shares
                .iter()
                .map(|&s| partial(&vectors, combined.key, s, combined.message))
                .collect();
            let partials: Vec<&[u8]> = partials.iter().map(Vec::as_slice).collect();
            assert_eq!(
                key.combine(&message, &partials).unwrap(),
                hex::decode(&combined.signature).unwrap(),
                "key {} message {} shares {:?}",
                combined.key,
                combined.message,
                combined.shares
            );
        }
    }

    #[test]
    fn partial_verification_agrees_with_the_ts_library() {
        let vectors = vectors();
        for (i, p) in vectors.partials.iter().enumerate() {
            let key = key(&vectors, p.key);
            let message = message(&vectors, p.message);
            assert_eq!(
                key.verify_partial(&message, &hex::decode(&p.partial).unwrap()),
                p.valid,
                "partial vector {i} (key {}, share {}, message {})",
                p.key,
                p.share,
                p.message
            );
        }
        assert_eq!(vectors.partials.iter().filter(|p| !p.valid).count(), 3);
    }

    #[test]
    fn share_index_is_the_last_four_bytes() {
        let vectors = vectors();
        for share in 0..5 {
            assert_eq!(
                partial_share_index(&partial(&vectors, 3, share, 0)),
                Some(share as u32)
            );
        }
        let valid = partial(&vectors, 3, 1, 0);
        assert_eq!(partial_share_index(&valid[..59]), None);
        let mut long = valid.clone();
        long.push(0);
        assert_eq!(partial_share_index(&long), None);
    }

    #[test]
    fn decode_refuses_key_versions_whose_parts_disagree() {
        let vectors = vectors();
        let (v1, v2) = (&vectors.keys[0], &vectors.keys[1]);
        let decode = |threshold: usize, polynomial: &str, pub_key: &str| {
            KeyVersion::decode(1, threshold, polynomial, pub_key).err()
        };

        assert_eq!(decode(2, &v1.polynomial, &v1.pub_key), None);
        assert_eq!(
            decode(3, &v1.polynomial, &v1.pub_key),
            Some(KeyVersionError::ThresholdMismatch {
                threshold: 3,
                coefficients: 2
            })
        );
        assert_eq!(
            decode(2, &v1.polynomial, &v2.pub_key),
            Some(KeyVersionError::PublicKeyNotConstantTerm)
        );
        let identity = BASE64.encode(bincode::serialize(&PublicKey::zero()).unwrap());
        assert_eq!(
            decode(2, &v1.polynomial, &identity),
            Some(KeyVersionError::PublicKeyIsIdentity)
        );
        // The identity is reported before a threshold that does not fit.
        assert_eq!(
            decode(3, &v1.polynomial, &identity),
            Some(KeyVersionError::PublicKeyIsIdentity)
        );
        assert_eq!(
            decode(2, &format!("{}00", v1.polynomial), &v1.pub_key),
            Some(KeyVersionError::PolynomialEncoding)
        );
        assert_eq!(
            decode(2, "not hex", &v1.pub_key),
            Some(KeyVersionError::PolynomialEncoding)
        );
        let mut long_key = BASE64.decode(&v1.pub_key).unwrap();
        long_key.push(0);
        assert_eq!(
            decode(2, &v1.polynomial, &BASE64.encode(long_key)),
            Some(KeyVersionError::PublicKeyEncoding)
        );
        assert_eq!(
            decode(2, &v1.polynomial, "not base64!"),
            Some(KeyVersionError::PublicKeyEncoding)
        );
    }

    #[test]
    fn a_bad_partial_is_dropped_and_collection_continues() {
        let vectors = vectors();
        let key = key(&vectors, 0);
        // The first invalid vector has one value byte flipped.
        let flipped =
            hex::decode(&vectors.partials.iter().find(|p| !p.valid).unwrap().partial).unwrap();

        let mut partials = PartialSignatures::new(&key, message(&vectors, 0));
        partials
            .add(0, None, &BASE64.encode(partial(&vectors, 0, 0, 0)))
            .unwrap();
        partials.add(1, None, &BASE64.encode(&flipped)).unwrap();
        assert_eq!(
            partials.try_combine(),
            CombineAttempt::Failed { rejected: vec![1] }
        );
        assert_eq!(partials.count(), 1);

        partials
            .add(2, None, &BASE64.encode(partial(&vectors, 0, 2, 0)))
            .unwrap();
        let expected = vectors
            .combined
            .iter()
            .find(|c| c.key == 0 && c.message == 0 && c.shares == [0, 2])
            .unwrap();
        assert_eq!(
            partials.try_combine(),
            CombineAttempt::Combined(hex::decode(&expected.signature).unwrap())
        );
    }

    #[test]
    fn wrong_length_and_wrong_index_are_rejected_before_counting() {
        let vectors = vectors();
        let key = key(&vectors, 0);
        let mut partials = PartialSignatures::new(&key, message(&vectors, 0));

        let mut long = partial(&vectors, 0, 0, 0);
        long.push(0);
        assert_eq!(
            partials.add(0, None, &BASE64.encode(long)),
            Err(Rejection::WrongLength { len: 61 })
        );
        assert_eq!(
            partials.add(0, Some(0), &BASE64.encode(partial(&vectors, 0, 1, 0))),
            Err(Rejection::WrongShareIndex {
                expected: 0,
                received: 1
            })
        );
        assert_eq!(
            partials.add(0, None, "not base64!"),
            Err(Rejection::NotBase64)
        );
        assert_eq!(partials.count(), 0);
    }

    #[test]
    fn try_combine_waits_for_the_threshold() {
        let vectors = vectors();
        let key = key(&vectors, 0);
        let mut partials = PartialSignatures::new(&key, message(&vectors, 0));
        partials
            .add(0, Some(0), &BASE64.encode(partial(&vectors, 0, 0, 0)))
            .unwrap();
        assert_eq!(partials.count(), 1);
        assert_eq!(partials.try_combine(), CombineAttempt::NotEnough);
    }
}
