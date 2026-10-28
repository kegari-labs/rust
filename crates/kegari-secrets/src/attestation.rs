//! Generic signed records; application policy interprets authenticated payloads.
use hmac::{Hmac, Mac};
use kegari_digest::{canonical_bytes, canonical_digest};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// Key material is borrowed from the host's Keychain-backed secret service; it
/// has no Debug/Serialize implementation and is never written by this module.
pub struct AttestationKey {
    id: String,
    bytes: Vec<u8>,
}

impl AttestationKey {
    pub fn new(id: String, bytes: Vec<u8>) -> Result<Self> {
        if id.trim().is_empty() || bytes.len() < 32 {
            return Err(Error::invalid(
                "attestation key needs a stable ID and at least 256 bits",
            ));
        }
        Ok(Self { id, bytes })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn sign<T: Serialize + Clone>(&self, purpose: &str, payload: &T) -> Result<Signed<T>> {
        if purpose.is_empty() {
            return Err(Error::invalid("signed evidence requires a domain purpose"));
        }
        let mut record = Signed {
            schema_version: 1,
            key_id: self.id.clone(),
            purpose: purpose.into(),
            payload: payload.clone(),
            signature: String::new(),
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.bytes)
            .map_err(|_| Error::invalid("invalid attestation key"))?;
        mac.update(&record.signing_bytes()?);
        record.signature = hex::encode(mac.finalize().into_bytes());
        Ok(record)
    }

    pub fn verify<T: Serialize>(&self, purpose: &str, signed: &Signed<T>) -> Result<()> {
        if signed.schema_version != 1 || signed.key_id != self.id || signed.purpose != purpose {
            return Err(Error::new(
                "invalid_attestation",
                "attestation schema/key/purpose does not match",
            ));
        }
        let signature = hex::decode(&signed.signature)
            .map_err(|_| Error::new("invalid_attestation", "invalid authentication tag"))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.bytes)
            .map_err(|_| Error::invalid("invalid attestation key"))?;
        mac.update(&signed.signing_bytes()?);
        mac.verify_slice(&signature).map_err(|_| {
            Error::new(
                "invalid_attestation",
                "authentication tag verification failed",
            )
        })
    }
}

impl Drop for AttestationKey {
    fn drop(&mut self) {
        // Avoid retaining an unnecessary copy after the host operation. Keychain
        // owns persistence; no security claim depends on optimizer behavior here.
        self.bytes.fill(0);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed<T> {
    pub schema_version: u32,
    pub key_id: String,
    pub purpose: String,
    pub payload: T,
    pub signature: String,
}

impl<T: Serialize> Signed<T> {
    fn signing_bytes(&self) -> Result<Vec<u8>> {
        #[derive(Serialize)]
        struct Envelope<'a, T> {
            schema_version: u32,
            key_id: &'a str,
            purpose: &'a str,
            payload: &'a T,
        }
        canonical_bytes(&Envelope {
            schema_version: self.schema_version,
            key_id: &self.key_id,
            purpose: &self.purpose,
            payload: &self.payload,
        })
    }

    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }
}
