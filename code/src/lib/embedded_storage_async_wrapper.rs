use core::ops::{Deref, DerefMut};

pub struct EmbeddedStorageAsyncWrapper<T>(pub T);

impl<T> Deref for EmbeddedStorageAsyncWrapper<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for EmbeddedStorageAsyncWrapper<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: embedded_storage::nor_flash::NorFlash> embedded_storage_async::nor_flash::NorFlash
    for EmbeddedStorageAsyncWrapper<T>
{
    const WRITE_SIZE: usize = T::WRITE_SIZE;

    const ERASE_SIZE: usize = T::ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        self.deref_mut().erase(from, to)
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.deref_mut().write(offset, bytes)
    }
}

impl<T: embedded_storage::nor_flash::ReadNorFlash> embedded_storage_async::nor_flash::ReadNorFlash
    for EmbeddedStorageAsyncWrapper<T>
{
    const READ_SIZE: usize = T::READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.deref_mut().read(offset, bytes)
    }

    fn capacity(&self) -> usize {
        self.deref().capacity()
    }
}

impl<T: embedded_storage::nor_flash::ErrorType> embedded_storage_async::nor_flash::ErrorType
    for EmbeddedStorageAsyncWrapper<T>
{
    type Error = T::Error;
}
