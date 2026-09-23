use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! uuid_identity {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::from_str(value).map(Self)
            }
        }
    };
}

uuid_identity!(OperationId);
uuid_identity!(AttemptId);
uuid_identity!(JournalId);
uuid_identity!(RootNamespaceId);
uuid_identity!(WorkspaceId);
uuid_identity!(ExecutionOwnerId);
uuid_identity!(ResultId);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("idempotency key must not be empty")]
    EmptyIdempotencyKey,
    #[error("idempotency key exceeds the maximum length of 512 bytes")]
    IdempotencyKeyTooLong,
    #[error("payload digest must be exactly 64 hexadecimal characters")]
    InvalidPayloadDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PayloadDigest([u8; 32]);

impl PayloadDigest {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let mut value = [0; 32];
        value.copy_from_slice(&digest);
        Self(value)
    }

    pub fn of_json(value: &serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::to_vec(value).map(|bytes| Self::of_bytes(&bytes))
    }

    pub fn from_hex(value: &str) -> Result<Self, IdentityError> {
        if value.len() != 64 || !value.is_ascii() {
            return Err(IdentityError::InvalidPayloadDigest);
        }

        let mut bytes = [0; 32];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            let pair =
                std::str::from_utf8(chunk).map_err(|_| IdentityError::InvalidPayloadDigest)?;
            bytes[index] =
                u8::from_str_radix(pair, 16).map_err(|_| IdentityError::InvalidPayloadDigest)?;
        }
        Ok(Self(bytes))
    }

    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 0x0f) as usize] as char);
        }
        output
    }
}

impl fmt::Display for PayloadDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for PayloadDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for PayloadDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_hex(&value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyScope {
    Root,
    Session,
    ProviderCall,
    BatchChild,
    HostControl,
    WorkerLaunch,
    CallerDefined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedIdempotencyKey {
    pub scope: IdempotencyScope,
    pub key: String,
}

impl ScopedIdempotencyKey {
    pub fn new(scope: IdempotencyScope, key: impl Into<String>) -> Result<Self, IdentityError> {
        let value = Self {
            scope,
            key: key.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), IdentityError> {
        if self.key.trim().is_empty() {
            return Err(IdentityError::EmptyIdempotencyKey);
        }
        if self.key.len() > 512 {
            return Err(IdentityError::IdempotencyKeyTooLong);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopedIdempotencyKeyWire {
    scope: IdempotencyScope,
    key: String,
}

impl<'de> Deserialize<'de> for ScopedIdempotencyKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ScopedIdempotencyKeyWire::deserialize(deserializer)?;
        Self::new(wire.scope, wire.key).map_err(D::Error::custom)
    }
}
