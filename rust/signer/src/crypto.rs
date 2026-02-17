use threshold_bls::{
    schemes::bls12_377::G2Scheme as SigScheme,
    sig::{BlindThresholdScheme, Scheme, Share},
};

type PrivateKey = <SigScheme as Scheme>::Private;

#[derive(Debug, thiserror::Error)]
pub enum BlsError {
    #[error("failed to deserialize private key share: {0}")]
    KeyDeserialize(#[from] bincode::Error),
    #[error("failed to compute blinded signature: {0}")]
    SigningFailed(String),
}

/// Compute a partial BLS blind signature over a blinded message
/// using a bincode-serialized private key share.
///
/// Returns the raw partial signature bytes.
pub fn compute_blinded_signature(
    blinded_msg: &[u8],
    private_key: &[u8],
) -> Result<Vec<u8>, BlsError> {
    let share: Share<PrivateKey> = bincode::deserialize(private_key)?;

    let signature = SigScheme::sign_blind_partial(&share, blinded_msg)
        .map_err(|e| BlsError::SigningFailed(format!("{e:?}")))?;

    Ok(signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

    fn hex_key(hex_str: &str) -> Vec<u8> {
        hex::decode(hex_str).unwrap()
    }

    fn b64_msg(b64_str: &str) -> Vec<u8> {
        BASE64.decode(b64_str).unwrap()
    }

    fn b64_encode(bytes: &[u8]) -> String {
        BASE64.encode(bytes)
    }

    // Single-signer dev key (index 0) from TS test values
    const PNP_DEV_SIGNER_PRIVATE_KEY: &str =
        "00000000dd0005bf4de5f2f052174f5cf58dae1af1d556c7f7f85d6fb3656e1d0f10720f";

    // Threshold dev key shares (index 0) for versions 1-3
    const PNP_DEV_KEY_V1: &str =
        "000000000e7e1a2fad3b54deb2b1b32cf4c7b084842d50bbb5c6143b9d9577d16e050f03";
    const PNP_DEV_KEY_V2: &str =
        "0000000087c722e1338395b942d8332328795a46c718baeb8fef9e5c63111d495469c50e";
    const PNP_DEV_KEY_V3: &str =
        "000000005b2c8089ead28a08233b6b16b2341542453523445950cfbd9bd2f1d09c8eee0c";

    // Blinded phone number used in integration tests
    const BLINDED_PHONE_NUMBER: &str =
        "n/I9srniwEHm5o6t3y0tTUB5fn7xjxRrLP1F/i8ORCdqV++WWiaAzUo3GA2UNHiB";

    // Expected signatures for blinded phone number with each key version
    const EXPECTED_SIG_V1: &str =
        "MAAAAAAAAACEVdw1ULDwAiTcZuPnZxHHh38PNa+/g997JgV10QnEq9yeuLxbM9l7vk0EAicV7IAAAAAA";
    const EXPECTED_SIG_V2: &str =
        "MAAAAAAAAAAmUJY0s9p7fMfs7GIoSiGJoObAN8ZpA7kRqeC9j/Q23TBrG3Jtxc8xWibhNVZhbYEAAAAA";
    const EXPECTED_SIG_V3: &str =
        "MAAAAAAAAAC4aBbzhHvt6l/b+8F7cILmWxZZ5Q7S6R4RZ/IgZR7Pfb9B1Wg9fsDybgxVTSv5BYEAAAAA";

    #[test]
    fn sign_blinded_hello_world() {
        // Mirrors apps/signer/test/signing/bls-signature.test.ts:
        // blind("hello world", seed=[0,1,...,30,0]) then partial sign with dev key.
        use rand_chacha::ChaChaRng;
        use rand_core::SeedableRng;
        use threshold_bls::sig::BlindScheme;

        let message = b"hello world";
        let mut seed = [0u8; 32];
        for i in 0..31 {
            seed[i] = i as u8;
        }
        let mut rng = ChaChaRng::from_seed(seed);
        let (_blinding_factor, blinded_msg) = SigScheme::blind_msg(message, &mut rng);

        let expected_signature =
            "MAAAAAAAAADDilSaA/xvbtE4NV3agMzHIf8PGPQ83Cu8gQy5E2mRWyUIges8bjE4EBe1L7pcY4AAAAAA";

        let result =
            compute_blinded_signature(&blinded_msg, &hex_key(PNP_DEV_SIGNER_PRIVATE_KEY)).unwrap();
        assert_eq!(b64_encode(&result), expected_signature);
    }

    #[test]
    fn sign_blinded_phone_number_all_key_versions() {
        let cases = [
            (PNP_DEV_KEY_V1, EXPECTED_SIG_V1),
            (PNP_DEV_KEY_V2, EXPECTED_SIG_V2),
            (PNP_DEV_KEY_V3, EXPECTED_SIG_V3),
        ];
        for (version, (key, expected_sig)) in cases.iter().enumerate() {
            let result =
                compute_blinded_signature(&b64_msg(BLINDED_PHONE_NUMBER), &hex_key(key)).unwrap();
            assert_eq!(
                b64_encode(&result),
                *expected_sig,
                "signature mismatch for key version {}",
                version + 1
            );
        }
    }

    #[test]
    fn invalid_blinded_message() {
        // Valid key but garbage blinded message (not a valid G1 point)
        let result = compute_blinded_signature(&[0u8; 12], &hex_key(PNP_DEV_SIGNER_PRIVATE_KEY));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BlsError::SigningFailed(_)));
    }

    #[test]
    fn invalid_key_bytes() {
        // Too short to be a valid Share<PrivateKey>
        let result = compute_blinded_signature(&b64_msg(BLINDED_PHONE_NUMBER), &[0u8; 4]);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), BlsError::KeyDeserialize(_)));
    }
}
