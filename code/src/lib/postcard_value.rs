use core::ops::{Deref, DerefMut};

use defmt::Format;
use sequential_storage::map::{SerializationError, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Format, Serialize, Deserialize, Default, Clone, Copy)]
pub struct PostcardValue<T>(pub T);

impl<T> Deref for PostcardValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for PostcardValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a, T: Serialize + Deserialize<'a>> Value<'a> for PostcardValue<T> {
    fn serialize_into(
        &self,
        buffer: &mut [u8],
    ) -> Result<usize, sequential_storage::map::SerializationError> {
        Ok(postcard::to_slice(self, buffer)
            .map_err(|e| match e {
                postcard::Error::SerializeBufferFull => SerializationError::BufferTooSmall,
                _ => SerializationError::InvalidData,
            })?
            .len())
    }

    fn deserialize_from(buffer: &'a [u8]) -> Result<(Self, usize), SerializationError>
    where
        Self: Sized,
    {
        let (value, unused_bytes) = postcard::take_from_bytes(buffer).map_err(|e| match e {
            postcard::Error::DeserializeUnexpectedEnd => SerializationError::BufferTooSmall,
            _ => SerializationError::InvalidFormat,
        })?;
        Ok((value, buffer.len() - unused_bytes.len()))
    }
}
