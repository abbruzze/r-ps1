use std::collections::BinaryHeap;
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CDROMEventType {
    //RaiseIRQ(u8,Option<(u8,u64)>),
    CdRomRaiseIrq { irq: u8 },
    CdRomRaiseIrqFor2ndResponse { irq: u8, cmd_to_complete: u8, delay: Option<u64> },
}

// Tipo di evento
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    HBlankStart,
    HBlankEnd,
    RasterLineEnd,
    Timer0,
    Timer1,
    Timer2,
    SIO0,
    DoThrottle,
    GPUCommandCompleted,
    CDROM(CDROMEventType)
}

#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: EventType,
    pub over_cycles: usize,
}

#[derive(Debug, Clone)]
struct ClockEvent {
    pub event_type: EventType,
    pub timestamp: u64,
}

impl PartialEq for ClockEvent {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp
    }
}

impl Eq for ClockEvent {}

impl PartialOrd for ClockEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ClockEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        // for min-heap
        other.timestamp.cmp(&self.timestamp)
    }
}

#[derive(Debug, Clone)]
pub struct ClockConfig {
    pub cpu_hz: u64,
    pub gpu_hz: u64,
    pub gpu_to_cpu_ratio: f64,
}

impl ClockConfig {
    pub const NTSC_CPU_CLOCK : u64 = 33_868_800;
    pub const NTSC_GPU_CLOCK : u64 = 53_693_175;
    pub const PAL_CPU_CLOCK : u64 = 33_868_800;
    pub const PAL_GPU_CLOCK : u64 = 53_203_425;
    pub const NTSC : ClockConfig = ClockConfig {
        cpu_hz : ClockConfig::NTSC_CPU_CLOCK,
        gpu_hz: ClockConfig::NTSC_GPU_CLOCK,
        gpu_to_cpu_ratio: ClockConfig::NTSC_GPU_CLOCK as f64 / ClockConfig::NTSC_CPU_CLOCK as f64,
    };
    pub const PAL : ClockConfig = ClockConfig {
        cpu_hz : ClockConfig::PAL_CPU_CLOCK,
        gpu_hz: ClockConfig::PAL_GPU_CLOCK,
        gpu_to_cpu_ratio: ClockConfig::PAL_GPU_CLOCK as f64 / ClockConfig::PAL_CPU_CLOCK as f64,
    };

    pub fn gpu_to_cpu_cycles(&self, gpu_cycles: u64) -> u64 {
        (gpu_cycles as f64 / self.gpu_to_cpu_ratio) as u64
    }

    pub fn cpu_to_gpu_cycles(&self, cpu_cycles: u64) -> u64 {
        (cpu_cycles as f64 * self.gpu_to_cpu_ratio) as u64
    }
}

pub struct Clock {
    events: BinaryHeap<ClockEvent>,
    current_time: u64,
    clock_config: ClockConfig,
}

impl Clock {
    pub fn new(clock_config: ClockConfig) -> Self {
        Self {
            events: BinaryHeap::new(),
            current_time: 0,
            clock_config,
        }
    }

    pub fn get_clock_config(&self) -> &ClockConfig {
        &self.clock_config
    }

    pub fn advance_time(&mut self, cpu_cycles: u64) {
        self.current_time += cpu_cycles;
    }

    pub fn schedule(&mut self, event_type: EventType, cpu_cycles_ahead: u64) -> u64 {
        let target = self.current_time + cpu_cycles_ahead;
        let event = ClockEvent {
            event_type,
            timestamp: target,
        };
        self.events.push(event);
        target
    }

    pub fn schedule_gpu(&mut self, event_type: EventType, gpu_cycles_ahead: u64) -> u64 {
        let cpu_cycles = self.clock_config.gpu_to_cpu_cycles(gpu_cycles_ahead);
        let target = self.current_time + cpu_cycles;
        let event = ClockEvent {
            event_type,
            timestamp: target,
        };
        self.events.push(event);
        target
    }

    pub fn schedule_gpu_dot_clock(&mut self, event_type: EventType, gpu_cycles_ahead: u64,dot_clock_divider:usize) -> u64 {
        let cpu_cycles = self.clock_config.gpu_to_cpu_cycles(gpu_cycles_ahead) * dot_clock_divider as u64;
        let target = self.current_time + cpu_cycles;
        let event = ClockEvent {
            event_type,
            timestamp: target,
        };
        self.events.push(event);
        target
    }

    pub fn schedule_absolute(&mut self, event_type: EventType, timestamp: u64) {
        let event = ClockEvent {
            event_type,
            timestamp,
        };
        self.events.push(event);
    }

    pub fn cancel(&mut self, event_type: EventType) {
        self.events.retain(|e| e.event_type != event_type);
    }

    pub fn cancel_where<F>(&mut self, mut predicate: F)
    where
        F: FnMut(&EventType) -> bool,
    {
        self.events.retain(|e| !predicate(&e.event_type));
    }

    pub fn next_event(&mut self) -> Option<Event> {
        self.events.pop().map(|e| Event {
            event_type: e.event_type, over_cycles: (self.current_time - e.timestamp) as usize
        })
    }

    pub fn next_events(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Some(event) = self.events.peek() && event.timestamp <= self.current_time {
            let event = self.events.pop().unwrap();
            events.push(Event { event_type: event.event_type, over_cycles: (self.current_time - event.timestamp) as usize });
        }
        events
    }

    pub fn has_ready_event(&self) -> bool {
        self.events.peek().map_or(false, |e| self.current_time >= e.timestamp)
    }

    pub fn next_event_time(&self) -> Option<u64> {
        self.events.peek().map(|e| e.timestamp)
    }

    pub fn current_time(&self) -> u64 {
        self.current_time
    }
}