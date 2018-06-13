//#![deny(missing_docs)]
//#![deny(warnings)]
#![feature(unsize)]
#![no_std]

extern crate embedded_hal as hal;

use hal::blocking::spi::{Transfer, Write};
//use hal::spi::{Mode, Phase, Polarity};
use hal::digital::OutputPin;

const FXOSC: u64 = 26_000_000;

#[macro_use]
mod macros;
mod config;
mod status;
mod traits;

#[derive(Debug)]
pub enum Error<E> {
    RxOverflow,
    Spi(E),
}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Error::Spi(e)
    }
}

pub struct Cc1101<SPI, CS> {
    spi: SPI,
    cs: CS,
    //    gdo0: GDO0,
    //    gdo2: GDO2,
}

impl<SPI, CS, E> Cc1101<SPI, CS>
where
    SPI: Transfer<u8, Error = E> + Write<u8, Error = E>,
    CS: OutputPin,
{
    pub fn new(spi: SPI, cs: CS) -> Result<Self, Error<E>> {
        Ok(Self { spi, cs })
    }

    pub fn set_frequency(&mut self, hz: u64) -> Result<(), Error<E>> {
        let freq = hz * 1_u64.rotate_left(16) / FXOSC;
        self.write_register(config::Register::FREQ2, ((freq >> 16) & 0xff) as u8)?;
        self.write_register(config::Register::FREQ1, ((freq >> 8) & 0xff) as u8)?;
        self.write_register(config::Register::FREQ0, (freq & 0xff) as u8)?;
        Ok(())
    }

    pub fn get_hw_info(&mut self) -> Result<(u8, u8), Error<E>> {
        use status::*;
        let partnum = self.read_status(Register::PARTNUM)?;
        let version = self.read_status(Register::VERSION)?;
        Ok((partnum, version))
    }

    pub fn set_sync_mode(&mut self, sync_mode: SyncMode) -> Result<(), Error<E>> {
        use config::*;

        let reset: u16 = (u16::from(SYNC1::default().bits())) << 8 | (u16::from(SYNC1::default().bits()));

        let (mode, word) = match sync_mode {
            SyncMode::Disabled => (SyncCheck::DISABLED, reset),
            SyncMode::MatchPartial(word) => (SyncCheck::CHECK_15_16, word),
            SyncMode::MatchPartialRepeated(word) => (SyncCheck::CHECK_30_32, word),
            SyncMode::MatchFull(word) => (SyncCheck::CHECK_16_16, word),
        };
        self.modify_register(config::Register::MDMCFG2, |r| {
            MDMCFG2(r).modify().sync_mode(mode.value()).bits()
        })?;
        self.write_register(Register::SYNC1, ((word >> 8) & 0xff) as u8)?;
        self.write_register(Register::SYNC0, (word & 0xff) as u8)
    }

    pub fn set_modulation(&mut self, format: Modulation) -> Result<(), Error<E>> {
        use config::*;
        self.modify_register(Register::MDMCFG2, |r| {
            MDMCFG2(r).modify().mod_format(format.value()).bits()
        })
    }

    pub fn set_address_filter(&mut self, filter: AddressFilter) -> Result<(), Error<E>> {
        use config::*;

        let (mode, addr) = match filter {
            AddressFilter::Disabled => (AddressCheck::DISABLED, ADDR::default().bits()),
            AddressFilter::Device(addr) => (AddressCheck::SELF, addr),
            AddressFilter::DeviceLowBroadcast(addr) => (AddressCheck::SELF_LOW_BROADCAST, addr),
            AddressFilter::DeviceHighLowBroadcast(addr) => {
                (AddressCheck::SELF_HIGH_LOW_BROADCAST, addr)
            }
        };
        self.modify_register(Register::PKTCTRL1, |r| {
            PKTCTRL1(r).modify().adr_chk(mode.value()).bits()
        })?;
        self.write_register(Register::ADDR, addr)
    }

    pub fn set_packet_length(&mut self, length: PacketLength) -> Result<(), Error<E>> {
        use config::*;

        let (format, pktlen) = match length {
            PacketLength::Fixed(limit) => (LengthConfig::FIXED, limit),
            PacketLength::Variable(max_limit) => (LengthConfig::VARIABLE, max_limit),
            PacketLength::Infinite => (LengthConfig::INFINITE, PKTLEN::default().bits()),
        };
        self.modify_register(Register::PKTCTRL0, |r| {
            PKTCTRL0(r).modify().length_config(format.value()).bits()
        })?;
        self.write_register(Register::PKTLEN, pktlen)
    }

    pub fn set_radio_mode(&mut self, radio_mode: RadioMode) -> Result<(), Error<E>> {
        let target = match radio_mode {
            RadioMode::Receive => {
                self.set_radio_mode(RadioMode::Idle)?;
                self.write_strobe(Command::SRX)?;
                MachineState::RX
            }
            RadioMode::Transmit => {
                self.set_radio_mode(RadioMode::Idle)?;
                self.write_strobe(Command::STX)?;
                MachineState::TX
            }
            RadioMode::Idle => {
                self.write_strobe(Command::SIDLE)?;
                MachineState::IDLE
            }
        };
        self.await_machine_state(target)
    }

    #[cfg_attr(rustfmt, rustfmt_skip)]
    pub fn set_defaults(&mut self) -> Result<(), Error<E>> {
        use config::*;

        self.write_strobe(Command::SRES)?;

        self.write_register(Register::PKTCTRL0, PKTCTRL0::default()
            .white_data(0).bits()
        )?;

        self.write_register(Register::FSCTRL1, FSCTRL1::default()
            .freq_if(0x08).bits() // f_if = (f_osc / 2^10) * FREQ_IF
        )?;

        self.write_register(Register::MDMCFG4, MDMCFG4::default()
            .chanbw_e(0x03) // bw_chan = f_osc / (8 * (4 + chanbw_m) * 2^chanbw_e
            .chanbw_m(0x00)
            .drate_e(0x0A).bits()
        )?;

        self.write_register(Register::MDMCFG3, MDMCFG3::default()
            .drate_m(0x83).bits() // r_data = (((256 + drate_m) * 2^drate_e) / 2**38) * f_osc
        )?;

        self.write_register(Register::MDMCFG2, MDMCFG2::default()
            .dem_dcfilt_off(1).bits()
        )?;

        self.write_register(Register::DEVIATN, DEVIATN::default()
            .deviation_e(0x03)
            .deviation_m(0x05).bits()
        )?;

        self.write_register(Register::MCSM0, MCSM0::default()
            .fs_autocal(AutoCalibration::FROM_IDLE.value()).bits()
        )?;

        self.write_register(Register::AGCCTRL2, AGCCTRL2::default()
            .max_lna_gain(0x04).bits()
        )?;

        Ok(())
    }

    fn await_machine_state(&mut self, target: MachineState) -> Result<(), Error<E>> {
        use status::*;
        loop {
            let marcstate = MARCSTATE(self.read_status(Register::MARCSTATE)?);
            if target.value() == marcstate.marc_state() {
                break;
            }
        }
        Ok(())
    }

    pub fn rx_bytes_available(&mut self) -> Result<u8, Error<E>> {
        use status::*;

        let mut last = 0;

        loop {
            let rxbytes = RXBYTES(self.read_status(Register::RXBYTES)?);
            if rxbytes.rxfifo_overflow() == 1 {
                return Err(Error::RxOverflow);
            }

            let nbytes = rxbytes.num_rxbytes();
            if nbytes > 0 && nbytes == last {
                break;
            }

            last = nbytes;
        }
        Ok(last)
    }

    // Should also be able to configure MCSM1.RXOFF_MODE to declare what state
    // to enter after fully receiving a packet.
    // Possible targets: IDLE, FSTON, TX, RX
    pub fn receive(&mut self, buf: &mut [u8], rssi: &mut u8, lqi: &mut u8) -> Result<(), Error<E>> {
        use status::*;

        self.rx_bytes_available()?;

        self.read_burst(Command::FIFO, buf)?;

        *rssi = self.read_status(Register::RSSI)?;
        *lqi = self.read_status(Register::LQI)?;

        self.write_strobe(Command::SFRX)?;

        Ok(())
    }

    fn read_register(&mut self, reg: config::Register) -> Result<u8, Error<E>> {
        self.cs.set_low();

        let mut buffer = [reg.addr() | Access::READ_SINGLE.offset(), 0];
        self.spi.transfer(&mut buffer)?;

        self.cs.set_high();

        Ok(buffer[1])
    }

    fn read_status(&mut self, reg: status::Register) -> Result<u8, Error<E>> {
        self.cs.set_low();

        let mut buffer = [reg.addr() | Access::READ_SINGLE.offset(), 0];
        self.spi.transfer(&mut buffer)?;

        self.cs.set_high();

        Ok(buffer[1])
    }

    fn read_burst(&mut self, com: Command, buf: &mut [u8]) -> Result<(), Error<E>> {
        self.cs.set_low();
        buf[0] = com.addr() | Access::READ_BURST.offset();
        self.spi.transfer(buf)?;
        self.cs.set_high();
        Ok(())
    }

    fn write_strobe(&mut self, com: Command) -> Result<(), Error<E>> {
        self.cs.set_low();
        self.spi.write(&[com.addr()])?;
        self.cs.set_high();
        Ok(())
    }

    fn write_register(&mut self, reg: config::Register, byte: u8) -> Result<(), Error<E>> {
        self.cs.set_low();

        let buffer = [reg.addr() | Access::WRITE_SINGLE.offset(), byte];
        self.spi.write(&buffer)?;

        self.cs.set_high();

        Ok(())
    }

    fn write_burst(&mut self, com: Command, buf: &mut [u8]) -> Result<(), Error<E>> {
        self.cs.set_low();

        // Hopefully the same as writing an array that starts with the command followed by buf
        self.spi
            .write(&[com.addr() | Access::WRITE_BURST.offset()])?;
        self.spi.write(&buf)?;

        self.cs.set_high();

        Ok(())
    }

    fn modify_register<F>(&mut self, reg: config::Register, f: F) -> Result<(), Error<E>>
    where
        F: FnOnce(u8) -> u8,
    {
        let r = self.read_register(reg)?;
        self.write_register(reg, f(r))?;
        Ok(())
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum Access {
    /// Write Single Byte
    WRITE_SINGLE = 0x00,
    /// Write Burst
    WRITE_BURST = 0x40,
    /// Read Single Byte
    READ_SINGLE = 0x80,
    /// Read Burst
    READ_BURST = 0xC0,
}

impl Access {
    fn offset(&self) -> u8 {
        *self as u8
    }
}

impl Command {
    fn addr(self) -> u8 {
        self as u8
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum Command {
    /* STROBE COMMANDS */
    SRES = 0x30,    // Reset chip
    SFSTXON = 0x31, // Enable/calibrate freq synthesizer
    SXOFF = 0x32,   // Turn off crystal oscillator.
    SCAL = 0x33,    // Calibrate freq synthesizer & disable
    SRX = 0x34,     // Enable RX.
    STX = 0x35,     // Enable TX.
    SIDLE = 0x36,   // Exit RX / TX
    SAFC = 0x37,    // AFC adjustment of freq synthesizer
    SWOR = 0x38,    // Start automatic RX polling sequence
    SPWD = 0x39,    // Enter pwr down mode when CSn goes hi
    SFRX = 0x3A,    // Flush the RX FIFO buffer.
    SFTX = 0x3B,    // Flush the TX FIFO buffer.
    SWORRST = 0x3C, // Reset real time clock.
    SNOP = 0x3D,    // No operation.
    PATABLE = 0x3E, // Power Amplifier Table
    FIFO = 0x3F,    // FIFO Access
}

impl Modulation {
    fn value(self) -> u8 {
        self as u8
    }
}

pub enum Modulation {
    BinaryFrequencyShiftKeying = 0b000,
    GaussianFrequencyShiftKeying = 0b001,
    OnOffKeying = 0b011,
    FourFrequencyShiftKeying = 0b100,
    MinimumShiftKeying = 0b111,
}

pub enum PacketLength {
    Fixed(u8),
    Variable(u8),
    Infinite,
}

pub enum AddressFilter {
    Disabled,
    Device(u8),
    DeviceLowBroadcast(u8),
    DeviceHighLowBroadcast(u8),
}

pub enum RadioMode {
    Receive,
    Transmit,
    Idle,
}

impl MachineState {
    fn value(&self) -> u8 {
        *self as u8
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum MachineState {
    SLEEP = 0x00,
    IDLE = 0x01,
    XOFF = 0x02,
    VCOON_MC = 0x03,
    REGON_MC = 0x04,
    MANCAL = 0x05,
    VCOON = 0x06,
    REGON = 0x07,
    STARTCAL = 0x08,
    BWBOOST = 0x09,
    FS_LOCK = 0x0A,
    IFADCON = 0x0B,
    ENDCAL = 0x0C,
    RX = 0x0D,
    RX_END = 0x0E,
    RX_RST = 0x0F,
    TXRX_SWITCH = 0x10,
    RXFIFO_OVERFLOW = 0x11,
    FSTXON = 0x12,
    TX = 0x13,
    TX_END = 0x14,
    RXTX_SWITCH = 0x15,
    TXFIFO_UNDERFLOW = 0x16,
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum GdoCfg {
    RX_FIFO_FILLED = 0x00,
    RX_FIFO_FILLED_END_OF_PKT = 0x01,
    TX_FIFO_FILLED = 0x02,
    TX_FIFO_FULL = 0x03,
    RX_FIFO_OVERFLOW = 0x04,
    TX_FIFO_UNDERFLOW = 0x05,
    SYNC_WORD = 0x06,
    CRC_OK = 0x07,
    PQT_REACHED = 0x08,
    CHANNEL_CLEAR = 0x09,
    PLL_LOCK = 0x0A,
    SERIAL_CLOCK = 0x0B,
    SERIAL_SYNC_DATA_OUT = 0x0C,
    SERIAL_DATA_OUT = 0x0D,
    CARRIER_SENSE = 0x0E,
    LAST_CRC_OK = 0x0F,

    RX_HARD_DATA_1 = 0x16,
    RX_HARD_DATA_0 = 0x17,

    PA_PD = 0x1B,
    LNA_PD = 0x1C,
    RX_SYMBOL_TICK = 0x1D,

    WOR_EVNT0 = 0x24,
    WOR_EVNT1 = 0x25,
    CLK_256 = 0x26,
    CLK_32k = 0x27,

    CHIP_RDYn = 0x29,

    XOSC_STABLE = 0x2B,

    HIGH_IMPEDANCE = 0x2E,
    HARDWIRE_TO_0 = 0x2F,
    CLK_XOSC_1 = 0x30,
    CLK_XOSC_1_5 = 0x31,
    CLK_XOSC_2 = 0x32,
    CLK_XOSC_3 = 0x33,
    CLK_XOSC_4 = 0x34,
    CLK_XOSC_6 = 0x35,
    CLK_XOSC_8 = 0x36,
    CLK_XOSC_12 = 0x37,
    CLK_XOSC_16 = 0x38,
    CLK_XOSC_24 = 0x39,
    CLK_XOSC_32 = 0x3A,
    CLK_XOSC_48 = 0x3B,
    CLK_XOSC_64 = 0x3C,
    CLK_XOSC_96 = 0x3D,
    CLK_XOSC_128 = 0x3E,
    CLK_XOSC_192 = 0x3F,
}

impl GdoCfg {
    fn value(&self) -> u8 {
        *self as u8
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum FifoThreshold {
    TX_61_RX_4 = 0x00,
    TX_57_RX_8 = 0x01,
    TX_53_RX_12 = 0x02,
    TX_49_RX_16 = 0x03,
    TX_45_RX_20 = 0x04,
    TX_41_RX_24 = 0x05,
    TX_37_RX_28 = 0x06,
    TX_33_RX_32 = 0x07,
    TX_29_RX_36 = 0x08,
    TX_25_RX_40 = 0x09,
    TX_21_RX_44 = 0x0A,
    TX_17_RX_48 = 0x0B,
    TX_13_RX_52 = 0x0C,
    TX_9_RX_56 = 0x0D,
    TX_5_RX_60 = 0x0E,
    TX_1_RX_64 = 0x0F,
}

impl FifoThreshold {
    fn value(&self) -> u8 {
        *self as u8
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum AddressCheck {
    DISABLED = 0x00,
    SELF = 0x01,
    SELF_LOW_BROADCAST = 0x02,
    SELF_HIGH_LOW_BROADCAST = 0x03,
}

impl AddressCheck {
    fn value(&self) -> u8 {
        *self as u8
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum LengthConfig {
    FIXED = 0x00,
    VARIABLE = 0x01,
    INFINITE = 0x02,
}

impl LengthConfig {
    fn value(&self) -> u8 {
        *self as u8
    }
}

pub enum SyncMode {
    Disabled,
    MatchPartial(u16),
    MatchPartialRepeated(u16),
    MatchFull(u16),
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum SyncCheck {
    DISABLED = 0x00,
    CHECK_15_16 = 0x01,
    CHECK_16_16 = 0x02,
    CHECK_30_32 = 0x03,
    CHECK_0_0_CS = 0x04,
    CHECK_15_16_CS = 0x05,
    CHECK_16_16_CS = 0x06,
    CHECK_30_32_CS = 0x07,
}

impl SyncCheck {
    fn value(&self) -> u8 {
        *self as u8
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum NumPreamble {
    N_2 = 0x00,
    N_3 = 0x01,
    N_4 = 0x02,
    N_6 = 0x03,
    N_8 = 0x04,
    N_12 = 0x05,
    N_16 = 0x06,
    N_24 = 0x07,
}

#[allow(dead_code)]
impl NumPreamble {
    fn value(&self) -> u8 {
        *self as u8
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum AutoCalibration {
    DISABLED = 0x00,
    FROM_IDLE = 0x01,
    TO_IDLE = 0x02,
    TO_IDLE_EVERY_4TH = 0x03,
}

impl AutoCalibration {
    fn value(&self) -> u8 {
        *self as u8
    }
}

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
enum PoTimeout {
    EXPIRE_COUNT_1 = 0x00,
    EXPIRE_COUNT_16 = 0x01,
    EXPIRE_COUNT_64 = 0x02,
    EXPIRE_COUNT_256 = 0x03,
}

impl PoTimeout {
    fn value(&self) -> u8 {
        *self as u8
    }
}
