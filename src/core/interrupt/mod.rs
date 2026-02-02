/*
  I_STAT bits
  0     IRQ0 VBLANK (PAL=50Hz, NTSC=60Hz)
  1     IRQ1 GPU   Can be requested via GP0(1Fh) command (rarely used)
  2     IRQ2 CDROM
  3     IRQ3 DMA
  4     IRQ4 TMR0  Timer 0 aka Root Counter 0 (Sysclk or Dotclk)
  5     IRQ5 TMR1  Timer 1 aka Root Counter 1 (Sysclk or H-blank)
  6     IRQ6 TMR2  Timer 2 aka Root Counter 2 (Sysclk or Sysclk/8)
  7     IRQ7 Controller and Memory Card - Byte Received Interrupt
  8     IRQ8 SIO
  9     IRQ9 SPU
  10    IRQ10 Controller - Lightpen Interrupt. Also shared by PIO and DTL cards.
  11-15 Not used (always zero)
  16-31 Garbage
 */

pub trait InterruptController {
    fn raise_hw_interrupts(&mut self,irqs:u16);
    
    fn raise_interrupt(&mut self,irq_type: InterruptType) {
        let bit = (1 << irq_type as usize) as u16;
        self.raise_hw_interrupts(bit);
    }
}

#[derive(Debug)]
pub enum InterruptType {
    VBlank,
    GPU,
    CDROM,
    DMA,
    TIMER0,
    TIMER1,
    TIMER2,
    ControllerMemoryCard,
    SIO,
    SPU,
    LightPen,
}

pub struct IrqHandler {
    irqs: u16,
    changed: bool,
}

impl IrqHandler {
    pub fn new() -> Self {
        Self {
            irqs: 0,
            changed: false,
        }
    }
    pub fn set_irq(&mut self, irq_type: InterruptType) {
        let bit = (1 << irq_type as usize) as u16;
        if (self.irqs ^ bit) != 0 {
            self.changed = true;
            self.irqs |= bit;
        }
    }

    pub fn forward_to_controller<T : InterruptController>(&mut self, controller:&mut T) {
        if self.changed {
            self.changed = false;
            controller.raise_hw_interrupts(self.irqs);
            self.irqs = 0;
        }
    }
}