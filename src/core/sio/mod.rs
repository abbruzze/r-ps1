use std::collections::VecDeque;
use tracing::{debug, info, warn};
use crate::core::clock::{Clock, EventType};
use crate::core::CPU_CLOCK;
use crate::core::controllers::Controller;
use crate::core::interrupt::{InterruptType, IrqHandler};

// some attempts here...
// 8 seems too low for pad.exe
// 16 seems too high for resolution.exe
const TX_RX_DATA_CYCLES : usize = 12 * CPU_CLOCK / 250_000; // 8 bit sent + 8 bit received at 250 kbps

/*
SIO_TX_DATA Notes
The hardware can hold (almost) 2 bytes in the TX direction (one being currently transferred, and, once when the start bit was sent, another byte can be stored in SIO_TX_DATA). When writing to SIO_TX_DATA, both SIO_STAT.0 and SIO_STAT.2 become zero.
As soon as the transfer starts, SIO_STAT.0 becomes set (indicating that one can write a new byte to SIO_TX_DATA; although the transmission is still busy). As soon as the transfer of the most recently written byte ends, SIO_STAT.2 becomes set.

SIO_RX_DATA Notes
The hardware can hold 8 bytes in the RX direction (when receiving further byte(s) while the RX FIFO is full, then the last FIFO entry will by overwritten by the new byte, and SIO_STAT.4 gets set;
the hardware does NOT automatically disable RTS when the FIFO becomes full). The RX FIFO overrun flag is not accessible on SIO0.
Data can be read from SIO_RX_DATA when SIO_STAT.1 is set, that flag gets automatically cleared after reading from SIO_RX_DATA (unless there are still further bytes in the RX FIFO).
Note: The hardware does always store incoming data in RX FIFO (even when Parity or Stop bits are invalid).
Note: A 16bit read allows to read two FIFO entries at once; nethertheless, it removes only ONE entry from the FIFO.
On the contrary, a 32bit read DOES remove FOUR entries (although, there's nothing that'd indicate if the FIFO did actually contain four entries).
Reading from Empty RX FIFO returns either the most recently received byte or zero (the hardware stores incoming data in ALL unused FIFO entries; eg. if five entries are used,
then the data gets stored thrice, after reading 6 bytes, the FIFO empty flag gets set, but nethertheless, the last byte can be read two more times, but doing further reads returns 00h).
 */
pub struct SIO0 {
    baud: u16,
    mode: u16,
    controllers: [Controller;2],
    selected_device: Option<u8>,
    irq: bool,
    ctrl: u16,
    tx_data: VecDeque<u8>,
    rx_fifo: VecDeque<u8>,
    tx_idle:bool,
    ack_asserted: bool,
    start_timer_timestamp: u64,
}

impl SIO0 {
    pub fn new(c1_connected:bool,c2_connected:bool) -> SIO0 {
        SIO0 {
            baud: 0,
            mode: 0,
            controllers: [Controller::new(0,c1_connected),Controller::new(1,c2_connected)],
            selected_device: None,
            irq: false,
            ctrl: 0,
            tx_data: VecDeque::with_capacity(2),
            rx_fifo: VecDeque::with_capacity(8),
            tx_idle: true,
            ack_asserted: false,
            start_timer_timestamp: 0,
        }
    }

    pub fn get_controller_mut(&mut self,index:usize) -> &mut Controller {
        &mut self.controllers[index]
    }
    /*
    1F801048h+N*10h - SIO#_MODE (R/W) (eg. 004Eh --> 8N1 with Factor=MUL16)
      0-1   Baudrate Reload Factor     (1=MUL1, 2=MUL16, 3=MUL64) (or 0=MUL1 on SIO0, STOP on SIO1)
      2-3   Character Length           (0=5 bits, 1=6 bits, 2=7 bits, 3=8 bits)
      4     Parity Enable              (0=No, 1=Enable)
      5     Parity Type                (0=Even, 1=Odd) (seems to be vice-versa...?)
      6-7   SIO1 stop bit length       (0=Reserved/1bit, 1=1bit, 2=1.5bits, 3=2bits)
      8     SIO0 clock polarity (CPOL) (0=High when idle, 1=Low when idle)
      9-15  Not used (always zero)
    Bits 6-7 on SIO0 and bit 8 on SIO1 are always zero. On SIO0 the character length shall be set to 8, the clock polarity should be set to high-when-idle and parity should be disabled, as all controllers and memory cards expect these settings.
     */
    pub fn write_mode(&mut self,value:u16) {
        debug!("SIO0 Setting mode to {:04X}",value);
        self.mode = value;
    }
    pub fn read_mode(&self) -> u16 {
        self.mode
    }
    /*
    1F80104Eh+N*10h - SIO#_BAUD (R/W) (eg. 00DCh --> 9600 bps; when Factor=MUL16)
      0-15  Baudrate Reload value for decrementing Baudrate Timer
    The timer is decremented on every clock cycle and reloaded when writing to this register and when it reaches zero. Upon reload, the 16-bit Reload value is multiplied by the Baudrate Factor (see SIO_MODE.Bit0-1), divided by 2, and then copied to the 21-bit Baudrate Timer (SIO_MODE.Bit11-31). The resulting transfer rate can be calculated as follows:
      SIO0: BitsPerSecond = 33868800 / MIN(((Reload*Factor) AND NOT 1),1)
      SIO1: BitsPerSecond = 33868800 / MIN(((Reload*Factor) AND NOT 1),Factor)
    According to the original nocash page, the way this register works is actually slightly different for SIO0 vs. SIO1:
      SIO0_BAUD is multiplied by Factor, and does then elapse "2" times per bit.
      SIO1_BAUD is NOT multiplied, and, instead, elapses "2*Factor" times per bit.
    The standard baud rate for SIO0 devices, including both controllers and memory cards, is ~250 kHz, with SIO0_BAUD being set to 0088h (serial clock high for 44h cycles then low for 44h cycles).
     */
    pub fn write_baud(&mut self,value:u16) {
        debug!("SIO0 Setting baud to {:04X}",value);
        self.baud = value;
    }
    pub fn read_baud(&self) -> u16 {
        self.baud
    }

    /*
    1F80104Ah+N*10h - SIO#_CTRL (R/W)
      0     TX Enable (TXEN)      (0=Disable, 1=Enable)
      1     DTR Output Level      (0=Off, 1=On)
      2     RX Enable (RXEN)      (SIO1: 0=Disable, 1=Enable)  ;Disable also clears RXFIFO
                                  (SIO0: 0=only receive when /CS low, 1=force receiving single byte)
      3     SIO1 TX Output Level  (0=Normal, 1=Inverted, during Inactivity & Stop bits)
      4     Acknowledge           (0=No change, 1=Reset SIO_STAT.Bits 3,4,5,9)      (W)
      5     SIO1 RTS Output Level (0=Off, 1=On)
      6     Reset                 (0=No change, 1=Reset most registers to zero) (W)
      7     SIO1 unknown?         (read/write-able when FACTOR non-zero) (otherwise always zero)
      8-9   RX Interrupt Mode     (0..3 = IRQ when RX FIFO contains 1,2,4,8 bytes)
      10    TX Interrupt Enable   (0=Disable, 1=Enable) ;when SIO_STAT.0-or-2 ;Ready
      11    RX Interrupt Enable   (0=Disable, 1=Enable) ;when N bytes in RX FIFO
      12    DSR Interrupt Enable  (0=Disable, 1=Enable) ;when SIO_STAT.7  ;DSR high or /ACK low
      13    SIO0 port select      (0=port 1, 1=port 2) (/CS pulled low when bit 1 set)
      14-15 Not used              (always zero)
    On SIO0, DTR is wired to the /CS pin on the controller and memory card ports; bit 1 will pull (assert) /CS low when set. Bit 13 is used to select which port's /CS shall be asserted (all other signals are wired in parallel).
    Bit 2 behaves differently on SIO0: when not set, incoming data will be ignored unless bit 1 is also set. When set, data will be received regardless of whether /CS is asserted, however bit 2 will be automatically cleared after a byte is received.
    Note that some emulators do not implement all SIO0 interrupts, as the kernel's controller driver only ever uses the DSR (/ACK) interrupt.
     */
    pub fn write_ctrl(&mut self,value:u16) {
        //debug!("SIO0 writing ctrl {:04X}",value);
        self.ctrl = value & !0x50;
        if (value & 0x10) != 0 {
            self.irq = false;
        }
        if (value & 0x40) != 0 {
            self.irq = false;
            self.selected_device = None;
            //self.tx_data.clear();
            //self.rx_fifo.clear();
            // self.controllers[0].reset();
            // self.controllers[1].reset();
        }
        if (value & 0x02) != 0 { // DTR on -> CS
            let selected_device = ((value >> 13) & 1) as u8;
            self.selected_device = Some(selected_device);
        }
        else {
            // if let Some(control_index) = self.selected_device {
            //     self.controllers[control_index as usize].reset();
            // }
            self.selected_device = None;
        }
        debug!("SI0 selected device: {:?} ctrl={:02X}",self.selected_device,value);
    }
    pub fn read_ctrl(&self) -> u16 {
        self.ctrl
    }
    /*
    Writing to this register starts a transfer (if, or as soon as, TXEN=1 and CTS=on and SIO_STAT.2=Ready). Writing to this register while SIO_STAT.0=Busy causes the old value to be overwritten.
    The "TXEN=1" condition is a bit more complex: Writing to SIO_TX_DATA latches the current TXEN value, and the transfer DOES start if the current TXEN value OR the latched TXEN value is set
    (ie. if TXEN gets cleared after writing to SIO_TX_DATA, then the transfer may STILL start if the old latched TXEN value was set; this appears for SIO transfers in Wipeout 2097).
     */
    pub fn write_tx_data(&mut self,data:u8,clock:&mut Clock) {
        debug!("SIO0 Writing data {:02X} sel={:?}",data,self.selected_device);
        match self.selected_device {
            Some(_) if (self.ctrl & 0x01) != 0 => {
                if self.tx_data.len() < 2 {
                    self.tx_data.push_back(data);
                    self.reschedule(clock);
                }
                else {
                    debug!("SIO0 TX FIFO overflow, discarding data {:02X}",data);
                }
            }
            _ => {}
        }
    }

    fn reschedule(&mut self,clock:&mut Clock) {
        self.tx_idle = false;
        /*
        let factor = match self.mode & 3 {
            0 | 1 => 1,
            2 => 16,
            3 => 64,
            _ => unreachable!()
        };
        let cycles = (self.baud * factor) << 3;        
         */
        // use fixed reasonable value: the above value works well with "pad" but not with "resolution"
        self.start_timer_timestamp = clock.current_time();
        clock.schedule(EventType::SIO0,TX_RX_DATA_CYCLES as u64);
    }

    pub fn on_tx_transmitted(&mut self,clock:&mut Clock,interrupt_handler:&mut IrqHandler) {
        // complete transfer
        debug!("SIO0 Completing transfer sel={:?}",self.selected_device);
        match self.selected_device {
            Some(dev) => {
                let tx_data = self.tx_data.pop_front().unwrap();
                let rx_data = self.controllers[dev as usize].read_byte_after_command(tx_data);
                debug!("SIO0 Transferred sel={:?} {:02X}, received {:02X}",self.selected_device,tx_data,rx_data);
                self.ack_asserted = self.controllers[dev as usize].ack();
                if self.ack_asserted && (self.ctrl & (1 << 12)) != 0 { // DSR Interrupt Enable
                    self.irq = true;
                    interrupt_handler.set_irq(InterruptType::ControllerMemoryCard);
                }
                self.rx_data(rx_data);

            }
            None => {
                let tx_data = self.tx_data.pop_front().unwrap();
                debug!("SIO0 No device selected, discarding transmitted data {:02X}",tx_data);
            }
        }
        // clear RXEN after receiving a byte
        self.ctrl &= !0x04;
        self.tx_idle = self.tx_data.is_empty();
        if !self.tx_idle {
            self.reschedule(clock);
        }
    }
    /*
    1F801040h+N*10h - SIO#_RX_DATA (R)
      0-7   Received Data      (1st RX FIFO entry) (oldest entry)
      8-15  Preview            (2nd RX FIFO entry)
      16-23 Preview            (3rd RX FIFO entry)
      24-31 Preview            (4th RX FIFO entry) (5th..8th cannot be previewed)
    A data byte can be read when SIO_STAT.1=1. Some emulators behave incorrectly when this register is read using a 16/32-bit memory access, so it should only be accessed as an 8-bit register.
     */
    pub fn read_rx_data(&mut self) -> u8 {
        debug!("Reading from RX FIFO len={}",self.rx_fifo.len());
        self.rx_fifo.pop_front().unwrap_or(0xFF)
    }
    pub fn peek_rx_data(&self) -> u8 {
        *self.rx_fifo.get(0).unwrap_or(&0xFF)
    }
    /*
    1F801044h+N*10h - SIO#_STAT (R)
      0     TX FIFO Not Full       (1=Ready for new byte)  (depends on CTS) (TX requires CTS)
      1     RX FIFO Not Empty      (0=Empty, 1=Data available)
      2     TX Idle                (1=Idle/Finished)       (depends on TXEN and on CTS)
      3     RX Parity Error        (0=No, 1=Error; Wrong Parity, when enabled) (sticky)
      4     SIO1 RX FIFO Overrun   (0=No, 1=Error; received more than 8 bytes) (sticky)
      5     SIO1 RX Bad Stop Bit   (0=No, 1=Error; Bad Stop Bit) (when RXEN)   (sticky)
      6     SIO1 RX Input Level    (0=Normal, 1=Inverted) ;only AFTER receiving Stop Bit
      7     DSR Input Level        (0=Off, 1=On) (remote DTR) ;DSR not required to be on
      8     SIO1 CTS Input Level   (0=Off, 1=On) (remote RTS) ;CTS required for TX
      9     Interrupt Request      (0=None, 1=IRQ) (See SIO_CTRL.Bit4,10-12)   (sticky)
      10    Unknown                (always zero)
      11-31 Baudrate Timer         (15-21 bit timer, decrementing at 33MHz)
    Bit 0 gets set after sending the start bit, bit 2 is set after sending all bits including the stop bit if any.
    On SIO0, DSR is wired to the /ACK pin on the controller and memory card ports; bit 7 is thus set when /ACK is low (asserted) and cleared when it is high. Bits 4-6 and 8 are always zero.
    The number of bits actually used by the baud rate timer is probably affected by the reload factor set in SIO_MODE.
     */
    pub fn read_status(&self,clock:&Clock) -> u32 {
        let mut status = 0u32;
        // bit 0 - TX Busy
        if self.tx_data.len() < 2 {
            status |= 0x01;
        }
        // bit 1 - RX FIFO Empty
        if !self.rx_fifo.is_empty() {
            status |= 0x02;
        }
        // bit 2 - TX Idle
        if self.tx_idle {
            status |= 0x04;
        }
        // bit 7 - DSR (/ACK) Level
        if self.ack_asserted {
            status |= 0x80;
        }
        // bit 9 - IRQ Flag
        if self.irq {
            status |= 0x200;
        }
        // baud rate timer
        status |= ((clock.current_time() - self.start_timer_timestamp) as u32) << 11;
        status
    }
    
    #[inline]
    fn rx_data(&mut self,data:u8) {
        if self.rx_fifo.len() < 8 {
            self.rx_fifo.push_back(data);
        }
    }
}