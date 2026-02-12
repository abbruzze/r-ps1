use tracing::{debug, info};
use crate::core::clock::{Clock, EventType};
use crate::core::interrupt::{InterruptType, IrqHandler};

#[derive(Debug,PartialEq)]
pub enum TimerClockSource {
    SystemClock,
    DotClock,
    HBlank,
    SystemClockDiv8
}

#[derive(Debug,PartialEq)]
enum TimerSyncMode {
    NoSync,
    // Timer 0 & 1 modes
    PauseDuringBlank,
    ResetTo0AtBlank,
    ResetTo0AtBlankPauseOutside,
    PauseUntilBlankThenFreeRun,
    // Timer 2 modes
    StopAtCurrentValue,
    FreeRun,
}

#[derive(Debug,PartialEq)]
enum TimerIRQRepeatMode {
    OneShot,
    Repeatedly,
}

impl TimerIRQRepeatMode {
    fn from_counter_mode(mode:u16) -> TimerIRQRepeatMode {
        if (mode & (1 << 6)) == 0 {
            TimerIRQRepeatMode::OneShot
        }
        else {
            TimerIRQRepeatMode::Repeatedly
        }
    }
}

#[derive(Debug,PartialEq)]
enum TimerIRQPulseMode {
    Pulse,
    Toggle,
}

impl TimerIRQPulseMode {
    fn from_counter_mode(mode:u16) -> TimerIRQPulseMode {
        if (mode & (1 << 7)) == 0 {
            TimerIRQPulseMode::Pulse
        }
        else {
            TimerIRQPulseMode::Toggle
        }
    }
}

impl TimerSyncMode {
    fn from_counter_mode<const N: usize>(mode:u16) -> TimerSyncMode {
        if (mode & 1) == 0 {
            return TimerSyncMode::NoSync
        }
        let sync_mode = (mode >> 1) & 3;
        match N {
            0|1 => {
                match sync_mode {
                    0 => TimerSyncMode::PauseDuringBlank,
                    1 => TimerSyncMode::ResetTo0AtBlank,
                    2 => TimerSyncMode::ResetTo0AtBlankPauseOutside,
                    3 => TimerSyncMode::PauseUntilBlankThenFreeRun,
                    _ => unreachable!()
                }
            },
            2 => {
                match sync_mode {
                    0|3 => TimerSyncMode::StopAtCurrentValue,
                    1|2 => TimerSyncMode::FreeRun,
                    _ => unreachable!()
                }
            },
            _ => unreachable!()
        }
    }
}

impl TimerClockSource {
    fn from_counter_mode<const N : usize>(counter_mode:u16) -> TimerClockSource {
        const { assert!(N < 3) }
        let source = (counter_mode >> 8) & 3;
        match N {
            0 => {
                match source {
                    0 | 2 => TimerClockSource::SystemClock,
                    1 | 3 => TimerClockSource::DotClock,
                    _ => unreachable!()
                }
            },
            1 => {
                match source {
                    0 | 2 => TimerClockSource::SystemClock,
                    1 | 3 => TimerClockSource::HBlank,
                    _ => unreachable!()
                }
            },
            2 => {
                match source {
                    0 | 1 => TimerClockSource::SystemClock,
                    2 | 3 => TimerClockSource::SystemClockDiv8,
                    _ => unreachable!()
                }
            },
            _ => unreachable!()
        }
    }
}

// in one-shot mode, delay IRQ for this many cycles after reaching target
const ONE_SHOT_IRQ_CYCLE_DELAY: usize = 4;
const COUNTER_PAUSE_CYCLES: usize = 3;

/*
Counter Mode:
  0     Synchronization Enable (0=Free Run, 1=Synchronize via Bit1-2)
  1-2   Synchronization Mode   (0-3, see lists below)
         Synchronization Modes for Counter 0:
           0 = Pause counter during Hblank(s)
           1 = Reset counter to 0000h at Hblank(s)
           2 = Reset counter to 0000h at Hblank(s) and pause outside of Hblank
           3 = Pause until Hblank occurs once, then switch to Free Run
         Synchronization Modes for Counter 1:
           Same as above, but using Vblank instead of Hblank
         Synchronization Modes for Counter 2:
           0 or 3 = Stop counter at current value (forever, no h/v-blank start)
           1 or 2 = Free Run (same as when Synchronization Disabled)
  3     Reset counter to 0000h  (0=After Counter=FFFFh, 1=After Counter=Target)
  4     IRQ when Counter=Target (0=Disable, 1=Enable)
  5     IRQ when Counter=FFFFh  (0=Disable, 1=Enable)
  6     IRQ Once/Repeat Mode    (0=One-shot, 1=Repeatedly)
  7     IRQ Pulse/Toggle Mode   (0=Short Bit10=0 Pulse, 1=Toggle Bit10 on/off)
  8-9   Clock Source (0-3, see list below)
         Counter 0:  0 or 2 = System Clock,  1 or 3 = Dotclock
         Counter 1:  0 or 2 = System Clock,  1 or 3 = Hblank
         Counter 2:  0 or 1 = System Clock,  2 or 3 = System Clock/8
  10    Interrupt Request       (0=Yes, 1=No) (Set after Writing)    (W=1) (R)
  11    Reached Target Value    (0=No, 1=Yes) (Reset after Reading)        (R)
  12    Reached FFFFh Value     (0=No, 1=Yes) (Reset after Reading)        (R)
  13-15 Unknown (seems to be always zero)
  16-31 Garbage (next opcode)
 */
pub struct Timer<const N: usize> {
    counter: u16,
    counter_mode: u16,
    counter_target: u16,
    clock_source: TimerClockSource,
    sync_mode: TimerSyncMode,
    inside_video_blank: bool,
    video_blank_occurred_once: bool,
    irq_repeat_mode: TimerIRQRepeatMode,
    irq_pulse_mode: TimerIRQPulseMode,
    irq_one_shot_fired: bool,
    timer_start_timestamp: u64,
    timer_target_timestamp: u64,
    blank_start_timestamp: u64,
    blank_end_timestamp: u64,
    dot_clock_divider: usize,
    blank_paused_cycles: u64,
    blank_pending_cycles: u64,
}

impl<const N: usize> Timer<N> {
    pub fn new() -> Self {
        const { assert!(N < 3) }
        Self {
            counter: 0,
            counter_mode: 0,
            counter_target: 0,
            clock_source: TimerClockSource::SystemClock,
            sync_mode: TimerSyncMode::NoSync,
            inside_video_blank: false,
            video_blank_occurred_once: false,
            irq_repeat_mode: TimerIRQRepeatMode::OneShot,
            irq_pulse_mode: TimerIRQPulseMode::Pulse,
            irq_one_shot_fired: false,
            timer_start_timestamp: 0,
            timer_target_timestamp: 0,
            blank_start_timestamp: 0,
            blank_end_timestamp: 0,
            dot_clock_divider: 8,
            blank_paused_cycles: 0,
            blank_pending_cycles: 0,
        }
    }

    pub fn initial_scheduling(&mut self,clock:&mut Clock) {
        self.reschedule_timer(clock,0);
    }

    fn adjust_elapsed(&self, elapsed:u32) -> u32 {
        match self.clock_source {
            TimerClockSource::SystemClock => elapsed,
            TimerClockSource::DotClock => elapsed / self.dot_clock_divider as u32,
            TimerClockSource::HBlank => elapsed, // not used
            TimerClockSource::SystemClockDiv8 => elapsed >> 3,
        }
    }

    pub fn read_counter(&self,clock:&Clock) -> u32 {
        if matches!(self.clock_source, TimerClockSource::HBlank) {
            return self.counter as u32
        }

        match self.sync_mode {
            TimerSyncMode::FreeRun | TimerSyncMode::NoSync | TimerSyncMode::ResetTo0AtBlank => {
                let elapsed = (clock.current_time() - self.timer_start_timestamp) as u32;
                self.adjust_elapsed(elapsed)
            }
            TimerSyncMode::PauseDuringBlank => {
                let elapsed = if self.inside_video_blank {
                    (self.blank_start_timestamp - self.timer_start_timestamp - self.blank_paused_cycles) as u32
                }
                else {
                    (clock.current_time() - self.timer_start_timestamp - self.blank_paused_cycles) as u32
                };
                self.adjust_elapsed(elapsed)
            }
            TimerSyncMode::StopAtCurrentValue => {
                self.counter as u32
            }
            TimerSyncMode::ResetTo0AtBlankPauseOutside => {
                // TODO
                0
            }
            TimerSyncMode::PauseUntilBlankThenFreeRun => {
                0
            }
        }
    }

    pub fn read_counter_target(&self) -> u32 {
        self.counter_target as u32
    }

    pub fn write_counter(&mut self, value: u32,clock:&mut Clock) {
        debug!("Writing counter #{N} = {:04X}",value);
        match self.sync_mode {
            TimerSyncMode::PauseUntilBlankThenFreeRun | TimerSyncMode::StopAtCurrentValue => {
                self.counter = value as u16;
                return;
            }
            _ => {}
        }
        match self.clock_source {
            TimerClockSource::HBlank => {
                self.counter = value as u16;
            }
            TimerClockSource::SystemClock => {
                let offset = value as u64;
                self.reschedule_timer(clock,offset);
            }
            TimerClockSource::DotClock => {
                let offset = value as u64 * self.dot_clock_divider as u64;
                self.reschedule_timer(clock,offset);
            }
            TimerClockSource::SystemClockDiv8 => {
                let offset = value as u64 * 8;
                self.reschedule_timer(clock,offset);
            }
        }
    }

    pub fn write_counter_target(&mut self, value: u32,_clock:&mut Clock) {
        debug!("Writing counter target #{N} = {:04X}",value);
        self.counter_target = value as u16;
    }

    pub fn read_counter_mode(&mut self) -> u32 {
        let mode = self.counter_mode as u32;
        // 11    Reached Target Value    (0=No, 1=Yes) (Reset after Reading)        (R)
        // 12    Reached FFFFh Value     (0=No, 1=Yes) (Reset after Reading)        (R)
        self.counter_mode &= !(3 << 11);

        mode
    }

    pub fn peek_counter_mode(&self) -> u32 {
        self.counter_mode as u32
    }

    pub fn write_counter_mode(&mut self, value: u32,clock:&mut Clock) {
        self.counter_mode = value as u16;
        self.clock_source = TimerClockSource::from_counter_mode::<N>(self.counter_mode);
        self.sync_mode = TimerSyncMode::from_counter_mode::<N>(self.counter_mode);
        self.irq_repeat_mode = TimerIRQRepeatMode::from_counter_mode(self.counter_mode);
        self.irq_pulse_mode = TimerIRQPulseMode::from_counter_mode(self.counter_mode);
        debug!("Writing counter mode #{N} = {:04X} source={:?} sync_mode={:?} irq_repeat_mode={:?} irq_pulse_mode:{:?}",value,self.clock_source,self.sync_mode,self.irq_repeat_mode,self.irq_pulse_mode);
        // When resetting the Counter by writing the Mode register, it will stay at 0000h for 2 clock cycles before counting up.
        self.video_blank_occurred_once = false;
        self.irq_one_shot_fired = false;
        self.blank_paused_cycles = 0;
        // 10    Interrupt Request       (0=Yes, 1=No) (Set after Writing)    (W=1) (R)
        self.counter_mode |= 1 << 10;

        if matches!(self.sync_mode,TimerSyncMode::PauseUntilBlankThenFreeRun) {
            self.counter = 0; // stay at 0 until blank
        }
        // check Timer 2
        else if !matches!(self.sync_mode,TimerSyncMode::StopAtCurrentValue) {
            self.counter = 0;
            self.reschedule_timer(clock,0);
        }
        else {
            // freeze counter
            let elapsed = (clock.current_time() - self.timer_start_timestamp) as u32;
            self.counter = self.adjust_elapsed(elapsed) as u16;
        }

    }

    fn cancel_timer_events(&mut self,clock:&mut Clock) {
        match N {
            0 => clock.cancel(EventType::Timer0),
            1 => clock.cancel(EventType::Timer1),
            2 => clock.cancel(EventType::Timer2),
            _ => unreachable!()
        }
    }

    fn get_timer_event_type(&self) -> EventType {
        match N {
            0 => EventType::Timer0,
            1 => EventType::Timer1,
            2 => EventType::Timer2,
            _ => unreachable!()
        }
    }

    fn reschedule_timer(&mut self,clock:&mut Clock,offset_cycles:u64) {
        // cancel previous events
        self.cancel_timer_events(clock);
        let compare_with_ffff = (self.counter_mode & (1 << 3)) == 0;
        let target = if compare_with_ffff {
            0xFFFF
        }
        else {
            self.counter_target
        };
        let ticks = target - self.counter;

        match self.sync_mode {
            TimerSyncMode::PauseDuringBlank => {
                // check if the timer is starting inside blank
                if self.inside_video_blank {
                    self.blank_start_timestamp = clock.current_time();
                }
            }
            _ => {}
        }

        self.timer_start_timestamp = clock.current_time().saturating_sub(offset_cycles);
        match self.clock_source {
            TimerClockSource::HBlank => {/* do nothing */}
            TimerClockSource::SystemClock => {
                self.timer_target_timestamp = clock.schedule(self.get_timer_event_type(), 1 + ticks as u64);
            }
            TimerClockSource::DotClock => {
                self.timer_target_timestamp = clock.schedule_gpu_dot_clock(self.get_timer_event_type(), 1 + ticks as u64, self.dot_clock_divider);
            }
            TimerClockSource::SystemClockDiv8 => {
                self.timer_target_timestamp = clock.schedule(self.get_timer_event_type(), (1 + ticks as u64) << 3);
            }
        }
    }

    pub fn on_timer_expired(&mut self,clock:&mut Clock,irq_handler:&mut IrqHandler) {
        let compare_with_ffff = (self.counter_mode & (1 << 3)) == 0;
        if compare_with_ffff {
            self.counter = 0xFFFF;
        }
        else {
            self.counter = self.counter_target;
        };
        self.count_and_compare(irq_handler);

        self.reschedule_timer(clock,0);
        self.blank_pending_cycles = 0;
        self.blank_paused_cycles = 0;
    }

    pub fn on_blank_start(&mut self,clock:&mut Clock,dot_clock_divider:usize) {
        const { assert!(N < 2) }
        self.inside_video_blank = true;
        self.dot_clock_divider = dot_clock_divider;
        self.blank_start_timestamp = clock.current_time();
        match self.sync_mode {
            TimerSyncMode::PauseDuringBlank => {
                self.cancel_timer_events(clock);
                self.blank_pending_cycles = self.timer_target_timestamp - clock.current_time();
            }
            TimerSyncMode::ResetTo0AtBlank => {
                self.cancel_timer_events(clock);
                self.counter = 0;
                self.reschedule_timer(clock,0);
            }
            TimerSyncMode::ResetTo0AtBlankPauseOutside => {
                // TODO
            }
            TimerSyncMode::PauseUntilBlankThenFreeRun => {
                self.sync_mode = TimerSyncMode::FreeRun;
                self.reschedule_timer(clock,0);
            }
            _ => {}
        }
    }

    pub fn on_blank_end(&mut self,clock:&mut Clock) {
        const { assert!(N < 2) }
        self.inside_video_blank = false;
        self.blank_end_timestamp = clock.current_time();
        match self.sync_mode {
            TimerSyncMode::PauseDuringBlank => {
                // increase pause cycles
                self.blank_paused_cycles += self.blank_end_timestamp - self.blank_start_timestamp;
                // move ahead the expiration time
                self.timer_target_timestamp = clock.schedule(self.get_timer_event_type(), self.blank_pending_cycles);
            }
            TimerSyncMode::ResetTo0AtBlank => {

            }
            TimerSyncMode::ResetTo0AtBlankPauseOutside => {

            }
            _ => {}
        }
    }

    pub fn cycle_hblank_clock(&mut self,irq_handler:&mut IrqHandler) {
        const { assert!(N == 1) }
        if matches!(self.clock_source,TimerClockSource::HBlank) {
            self.tick(irq_handler);
        }
    }

    #[inline]
    fn check_irq_on_match(&mut self,irq_handler:&mut IrqHandler) {
        let raise_irq = match self.irq_repeat_mode {
            TimerIRQRepeatMode::OneShot => {
                if !self.irq_one_shot_fired {
                    self.irq_one_shot_fired = true;
                    true
                }
                else {
                    false
                }
            }
            TimerIRQRepeatMode::Repeatedly => true
        };

        if raise_irq {
            match self.irq_pulse_mode {
                TimerIRQPulseMode::Pulse => {
                    self.counter_mode &= !(1 << 10); // set IRQ
                }
                TimerIRQPulseMode::Toggle => {
                    self.counter_mode ^= 1 << 10; // toggle bit 10
                }
            }
            self.check_irq(irq_handler);
        }
    }

    fn check_irq(&mut self,irq_handler:&mut IrqHandler) {
        let irq = self.counter_mode & (1 << 10) == 0;
        if irq {
            match N {
                0 => irq_handler.set_irq(InterruptType::TIMER0),
                1 => irq_handler.set_irq(InterruptType::TIMER1),
                2 => irq_handler.set_irq(InterruptType::TIMER2),
                _ => unreachable!()
            }
        }
    }

    #[inline]
    fn count_and_compare(&mut self,irq_handler:&mut IrqHandler) {
        // check compare with target or FFFFh
        let compare_with_ffff = (self.counter_mode & (1 << 3)) == 0;
        let reset = if compare_with_ffff {
            self.counter == 0xFFFF
        }
        else {
            self.counter == self.counter_target
        };
        if reset {
            debug!("Timer #{N} reached reset condition at {:04X}",self.counter);
            self.counter = 0x0000;
            if compare_with_ffff {
                // 12    Reached FFFFh Value     (0=No, 1=Yes) (Reset after Reading)        (R)
                self.counter_mode |= 1 << 12;
                // 5     IRQ when Counter=FFFFh  (0=Disable, 1=Enable)
                if (self.counter_mode & (1 << 5)) != 0 {
                    self.check_irq_on_match(irq_handler);
                }
            }
            else {
                // When being reset to 0000h by reaching the Target value(Mode Bit3 set), it will stay at 0000h for 2 clock cycles.
                // 11    Reached Target Value    (0=No, 1=Yes) (Reset after Reading)        (R)
                self.counter_mode |= 1 << 11;
                // 4     IRQ when Counter=Target (0=Disable, 1=Enable)
                if (self.counter_mode & (1 << 4)) != 0 {
                    self.check_irq_on_match(irq_handler);
                }
            }
        }
        else {
            self.counter += 1;
        }
    }

    #[inline]
    fn tick(&mut self,irq_handler:&mut IrqHandler) {
        self.check_sync(irq_handler);
    }

    #[inline]
    fn check_sync(&mut self,irq_handler:&mut IrqHandler) {
        use TimerSyncMode::*;
        // Sync logic
        match self.sync_mode {
            NoSync => {
                self.count_and_compare(irq_handler)
            }
            PauseDuringBlank => { // Pause counter during H/Vblank(s)
                if !self.inside_video_blank {
                    self.count_and_compare(irq_handler)
                }
            }
            // ResetTo0AtBlank => { // Reset counter to 0000h at H/Vblank(s)
            //     if self.at_video_blank {
            //         debug!("Timer #{N} resetting to 0 at video blank");
            //         self.counter = 0x0000;
            //         self.at_video_blank = false;
            //     } else {
            //         self.count_and_compare(irq_handler)
            //     }
            // }
            // ResetTo0AtBlankPauseOutside => { // Reset counter to 0000h at H/Vblank(s) and pause outside of H/Vblank
            //     if self.at_video_blank {
            //         debug!("Timer #{N} resetting to 0 at video blank and pausing outside");
            //         self.counter = 0x0000;
            //         self.at_video_blank = false;
            //     } else if self.inside_video_blank {
            //         self.count_and_compare(irq_handler)
            //     }
            // }
            // PauseUntilBlankThenFreeRun => {
            //     if !self.video_blank_occurred_once {
            //         if self.at_video_blank {
            //             self.video_blank_occurred_once = true;
            //             debug!("Timer #{N} detected first video blank, switching to Free Run",);
            //             self.at_video_blank = false;
            //         }
            //         // else paused
            //     }
            //     else {
            //         self.count_and_compare(irq_handler); // free run
            //     }
            // }
            StopAtCurrentValue => { // Stop counter at current value (forever, no h/v-blank start)
                // do nothing, stopped
            }
            FreeRun => { // Free Run (same as when Synchronization Disabled)
                self.count_and_compare(irq_handler);
            }
            _ => {}
        }
    }
}