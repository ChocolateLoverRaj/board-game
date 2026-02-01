use core::{
    cell::{RefCell, RefMut},
    fmt::Debug,
};

use defmt::Format;
use embassy_embedded_hal::SetConfig;
use embassy_sync::{blocking_mutex::raw::RawMutex, mutex::Mutex};
use embedded_hal::spi::{Error as EmbeddedHalSpiError, ErrorKind, ErrorType, Operation};
use embedded_hal_async::{
    delay::DelayNs,
    digital::OutputPin,
    spi::{SpiBus, SpiDevice},
};

#[derive(Debug, Format, PartialEq, Eq)]
enum CsState {
    Low,
    Undefined,
}

#[derive(Debug, Format)]
struct ActiveCs<'a, C> {
    // id: usize,
    state: CsState,
    cs: RefMut<'a, C>,
    cs_cell: &'a RefCell<C>,
}

struct Inner<'a, S, C> {
    spi: S,
    active_cs: Option<ActiveCs<'a, C>>,
}
pub struct LazySharedSpi<'a, S, M: RawMutex, C> {
    inner: Mutex<M, Inner<'a, S, C>>,
    // next_id: portable_atomic::AtomicUsize,
}
impl<'a, S, M: RawMutex, C> LazySharedSpi<'a, S, M, C> {
    pub fn new(spi_bus: S) -> Self {
        Self {
            inner: Mutex::new(Inner {
                spi: spi_bus,
                active_cs: None,
            }),
        }
    }
}

pub struct SpiDeviceWithConfig<'a, S: SetConfig, M: RawMutex, C, D> {
    inner: &'a Mutex<M, Inner<'a, S, C>>,
    cs: &'a RefCell<C>,
    // id: usize,
    config: S::Config,
    delay: D,
}
impl<'a, S: SetConfig, M: RawMutex, C, D> SpiDeviceWithConfig<'a, S, M, C, D> {
    pub fn new(
        spi_bus: &'a LazySharedSpi<'a, S, M, C>,
        cs: &'a RefCell<C>,
        config: S::Config,
        delay: D,
    ) -> Self {
        Self {
            inner: &spi_bus.inner,
            cs,
            config,
            delay,
        }
    }
}

#[derive(Format)]
pub enum Error<S, C>
where
    S: SpiBus,
    S: SetConfig,
    <S as SetConfig>::ConfigError: Debug,
    C: OutputPin,
{
    Spi(S::Error),
    SpiConfig(<S as SetConfig>::ConfigError),
    Cs(C::Error),
}
impl<S, C> Debug for Error<S, C>
where
    S: SpiBus + SetConfig,
    <S as SetConfig>::ConfigError: Debug,
    C: OutputPin,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.kind().fmt(f)
    }
}

impl<S, C> embedded_hal::spi::Error for Error<S, C>
where
    S: SpiBus + SetConfig,
    <S as SetConfig>::ConfigError: Debug,
    C: OutputPin,
{
    fn kind(&self) -> embedded_hal::spi::ErrorKind {
        match self {
            Self::Spi(e) => e.kind(),
            Self::SpiConfig(_e) => ErrorKind::Other,
            Self::Cs(_e) => ErrorKind::ChipSelectFault,
        }
    }
}

impl<S, M: RawMutex, C, D> ErrorType for SpiDeviceWithConfig<'_, S, M, C, D>
where
    S: SpiBus + SetConfig,
    <S as SetConfig>::ConfigError: Debug,
    C: OutputPin,
{
    type Error = Error<S, C>;
}

impl<S, M, C, D> SpiDevice for SpiDeviceWithConfig<'_, S, M, C, D>
where
    S: SpiBus + SetConfig,
    <S as SetConfig>::ConfigError: Debug,
    M: RawMutex,
    C: OutputPin,
    D: DelayNs,
{
    async fn transaction(
        &mut self,
        operations: &mut [embedded_hal::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        let mut inner = self.inner.lock().await;
        inner
            .spi
            .set_config(&self.config)
            .map_err(Error::SpiConfig)?;

        match &mut inner.active_cs {
            Some(active_cs) => {
                if active_cs.cs_cell.as_ptr() == self.cs.as_ptr() {
                    match active_cs.state {
                        CsState::Low => {
                            // Already low, no need to do anything
                        }
                        CsState::Undefined => {
                            active_cs.cs.set_low().await.map_err(Error::Cs)?;
                        }
                    }
                } else {
                    // Set the other CS to high and then  set our CS to low
                    active_cs.cs.set_high().await.map_err(Error::Cs)?;
                    *active_cs = ActiveCs {
                        state: CsState::Undefined,
                        cs: self.cs.borrow_mut(),
                        cs_cell: self.cs,
                    };
                    active_cs.cs.set_low().await.map_err(Error::Cs)?;
                    active_cs.state = CsState::Low;
                }
            }
            None => {
                let active_cs = inner.active_cs.insert(ActiveCs {
                    state: CsState::Undefined,
                    cs: self.cs.borrow_mut(),
                    cs_cell: self.cs,
                });
                active_cs.cs.set_low().await.map_err(Error::Cs)?;
                active_cs.state = CsState::Low;
            }
        }

        let op_res = {
            for operation in operations {
                match operation {
                    Operation::DelayNs(ns) => {
                        self.delay.delay_ns(*ns).await;
                    }
                    Operation::Read(words) => {
                        inner.spi.read(words).await.map_err(Error::Spi)?;
                    }
                    Operation::Write(words) => {
                        inner.spi.write(words).await.map_err(Error::Spi)?;
                    }
                    Operation::Transfer(read, write) => {
                        inner.spi.transfer(read, write).await.map_err(Error::Spi)?;
                    }
                    Operation::TransferInPlace(words) => {
                        inner
                            .spi
                            .transfer_in_place(words)
                            .await
                            .map_err(Error::Spi)?;
                    }
                }
            }
            Ok(())
        };

        let flush_res = inner.spi.flush().await;

        op_res.map_err(Error::Spi)?;
        flush_res.map_err(Error::Spi)?;

        Ok(())
    }
}
