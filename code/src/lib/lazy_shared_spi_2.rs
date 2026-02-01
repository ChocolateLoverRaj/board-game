use core::{borrow::BorrowMut, fmt::Debug, marker::PhantomData, ops::DerefMut};

use defmt::{Format, info};
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
struct ActiveCs {
    index: usize,
    state: CsState,
}

struct Inner<SpiBus, CsPins> {
    spi_bus: SpiBus,
    cs_pins: CsPins,
    active_cs: Option<ActiveCs>,
}
pub struct LazySharedSpi2<SpiBus, M: RawMutex, CsPins> {
    inner: Mutex<M, Inner<SpiBus, CsPins>>,
}
impl<'a, SpiBus, M: RawMutex, CsPins> LazySharedSpi2<SpiBus, M, CsPins> {
    pub fn new(spi_bus: SpiBus, cs_pins: CsPins) -> Self {
        Self {
            inner: Mutex::new(Inner {
                spi_bus,
                cs_pins,
                active_cs: None,
            }),
        }
    }
}

pub struct SpiDeviceWithConfig2<'a, SpiBus: SetConfig, M: RawMutex, CsPins, CsPin, D> {
    inner: &'a Mutex<M, Inner<SpiBus, CsPins>>,
    index: usize,
    config: SpiBus::Config,
    delay: D,
    _cs_pin: PhantomData<CsPin>,
}
impl<'a, S: SetConfig, M: RawMutex, CsPins, CsPin, D>
    SpiDeviceWithConfig2<'a, S, M, CsPins, CsPin, D>
{
    pub fn new(
        spi_bus: &'a LazySharedSpi2<S, M, CsPins>,
        cs_index: usize,
        config: S::Config,
        delay: D,
    ) -> Self {
        Self {
            inner: &spi_bus.inner,
            index: cs_index,
            config,
            delay,
            _cs_pin: PhantomData,
        }
    }
}

#[derive(Format)]
pub enum Error2<SpiBus, CsPin>
where
    SpiBus: embedded_hal_async::spi::SpiBus,
    SpiBus: SetConfig,
    <SpiBus as SetConfig>::ConfigError: Debug,
    CsPin: OutputPin,
{
    Spi(SpiBus::Error),
    SpiConfig(<SpiBus as SetConfig>::ConfigError),
    Cs(CsPin::Error),
}
impl<S, C> Debug for Error2<S, C>
where
    S: SpiBus + SetConfig,
    <S as SetConfig>::ConfigError: Debug,
    C: OutputPin,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.kind().fmt(f)
    }
}

impl<S, C> embedded_hal::spi::Error for Error2<S, C>
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

impl<S, M: RawMutex, C, CsPin, D> ErrorType for SpiDeviceWithConfig2<'_, S, M, C, CsPin, D>
where
    S: SpiBus + SetConfig,
    <S as SetConfig>::ConfigError: Debug,
    CsPin: OutputPin,
{
    type Error = Error2<S, CsPin>;
}

impl<S, M, C, CsPin, D> SpiDevice for SpiDeviceWithConfig2<'_, S, M, C, CsPin, D>
where
    S: SpiBus + SetConfig,
    <S as SetConfig>::ConfigError: Debug,
    M: RawMutex,
    C: BorrowMut<[CsPin]>,
    CsPin: OutputPin,
    D: DelayNs,
{
    async fn transaction(
        &mut self,
        operations: &mut [embedded_hal::spi::Operation<'_, u8>],
    ) -> Result<(), Self::Error> {
        let mut inner = self.inner.lock().await;
        inner
            .spi_bus
            .set_config(&self.config)
            .map_err(Error2::SpiConfig)?;
        let Inner {
            spi_bus,
            cs_pins,
            active_cs,
        } = inner.deref_mut();
        match active_cs {
            Some(active_cs) => {
                if active_cs.index == self.index && false {
                    match active_cs.state {
                        CsState::Low => {
                            // Already low, no need to do anything
                        }
                        CsState::Undefined => {
                            cs_pins.borrow_mut()[self.index]
                                .set_low()
                                .await
                                .map_err(Error2::Cs)?;
                        }
                    }
                } else {
                    // Set the other CS to high and then  set our CS to low
                    active_cs.state = CsState::Undefined;
                    info!("setting CS {} high", active_cs.index);
                    cs_pins.borrow_mut()[active_cs.index]
                        .set_high()
                        .await
                        .map_err(Error2::Cs)?;
                    *active_cs = ActiveCs {
                        state: CsState::Undefined,
                        index: self.index,
                    };
                    info!("setting CS {} low", self.index);
                    cs_pins.borrow_mut()[self.index]
                        .set_low()
                        .await
                        .map_err(Error2::Cs)?;
                    active_cs.state = CsState::Low;
                }
            }
            None => {
                let active_cs = active_cs.insert(ActiveCs {
                    state: CsState::Undefined,
                    index: self.index,
                });
                cs_pins.borrow_mut()[self.index]
                    .set_low()
                    .await
                    .map_err(Error2::Cs)?;
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
                        spi_bus.read(words).await.map_err(Error2::Spi)?;
                    }
                    Operation::Write(words) => {
                        spi_bus.write(words).await.map_err(Error2::Spi)?;
                    }
                    Operation::Transfer(read, write) => {
                        spi_bus.transfer(read, write).await.map_err(Error2::Spi)?;
                    }
                    Operation::TransferInPlace(words) => {
                        spi_bus
                            .transfer_in_place(words)
                            .await
                            .map_err(Error2::Spi)?;
                    }
                }
            }
            Ok(())
        };

        let flush_res = inner.spi_bus.flush().await;

        op_res.map_err(Error2::Spi)?;
        flush_res.map_err(Error2::Spi)?;

        Ok(())
    }
}
