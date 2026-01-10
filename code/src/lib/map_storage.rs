use defmt::{Format, warn};
use embedded_storage_async::nor_flash::ReadNorFlash;
use esp_storage::FlashStorage;
use sequential_storage::map::{Key, SerializationError, Value};
use trouble_host::{LongTermKey, prelude::*};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::EmbeddedStorageAsyncWrapper;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    IntoBytes,
    FromBytes,
    Immutable,
    KnownLayout,
    Format,
)]
#[repr(C)]
pub struct MapStorageKey([u8; 6]);

impl From<BdAddr> for MapStorageKey {
    fn from(value: BdAddr) -> Self {
        Self(value.into_inner())
    }
}

impl From<MapStorageKey> for BdAddr {
    fn from(value: MapStorageKey) -> Self {
        Self::new(value.0)
    }
}

impl Key for MapStorageKey {
    fn serialize_into(
        &self,
        buffer: &mut [u8],
    ) -> Result<usize, sequential_storage::map::SerializationError> {
        warn!(
            "MapStorageKey serializing buffer len {}. returning {}",
            buffer.len(),
            size_of::<Self>()
        );
        self.write_to_prefix(buffer)
            .map_err(|_| SerializationError::BufferTooSmall)?;
        Ok(size_of::<Self>())
    }

    fn deserialize_from(
        buffer: &[u8],
    ) -> Result<(Self, usize), sequential_storage::map::SerializationError> {
        warn!(
            "MapStorageKey deserializing buffer len {}. returning {}",
            buffer.len(),
            size_of::<Self>()
        );
        if buffer.len() < size_of::<BdAddr>() {
            return Err(SerializationError::BufferTooSmall);
        }
        Ok((
            Self::read_from_prefix(buffer)
                .map_err(|_| SerializationError::BufferTooSmall)?
                .0,
            size_of::<Self>(),
        ))
    }
}

#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
#[repr(C, packed)]
pub struct MapStorageValue {
    ltk: u128,
    security_level: u8,
}

impl<'a> Value<'a> for &'a MapStorageValue {
    fn serialize_into(&self, buffer: &mut [u8]) -> Result<usize, SerializationError> {
        warn!(
            "MapStorageValue serializing buffer len {}. returning {}",
            buffer.len(),
            size_of::<MapStorageValue>()
        );
        self.write_to_prefix(buffer)
            .map_err(|_| SerializationError::BufferTooSmall)?;
        Ok(size_of::<MapStorageValue>())
    }

    fn deserialize_from(buffer: &'a [u8]) -> Result<(Self, usize), SerializationError>
    where
        Self: Sized,
    {
        warn!(
            "MapStorageValue deserializing buffer len {}. returning {}",
            buffer.len(),
            size_of::<MapStorageValue>()
        );
        Ok((
            MapStorageValue::ref_from_prefix(buffer)
                .map_err(|_| SerializationError::BufferTooSmall)?
                .0,
            size_of::<MapStorageValue>(),
        ))
    }
}

pub struct MapStorageKeyValue {
    pub key: MapStorageKey,
    pub value: MapStorageValue,
}

impl From<BondInformation> for MapStorageKeyValue {
    fn from(value: BondInformation) -> Self {
        Self {
            key: value.identity.bd_addr.into(),
            value: MapStorageValue {
                ltk: value.ltk.0,
                security_level: match value.security_level {
                    SecurityLevel::NoEncryption => 0,
                    SecurityLevel::Encrypted => 1,
                    SecurityLevel::EncryptedAuthenticated => 2,
                },
            },
        }
    }
}

impl From<MapStorageKeyValue> for BondInformation {
    fn from(value: MapStorageKeyValue) -> Self {
        Self {
            identity: Identity {
                bd_addr: value.key.into(),
                irk: None,
            },
            is_bonded: true,
            ltk: LongTermKey(value.value.ltk),
            security_level: match value.value.security_level {
                0 => SecurityLevel::NoEncryption,
                1 => SecurityLevel::Encrypted,
                2 => SecurityLevel::EncryptedAuthenticated,
                _ => unreachable!(),
            },
        }
    }
}
// Round up to READ_SIZE
// Since max is not a const fn, just add, it's okay to have extra
pub const DATA_BUFFER_LEN: usize = size_of::<MapStorageKey>()
    + size_of::<MapStorageValue>()
    + EmbeddedStorageAsyncWrapper::<FlashStorage>::READ_SIZE;
