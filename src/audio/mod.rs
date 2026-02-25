pub mod cpal;

#[derive(Debug,Copy,Clone)]
pub struct AudioSample {
    left: i16,
    right: i16,
}

impl AudioSample {
    pub fn new(left: i16, right: i16) -> Self {
        Self { left, right }
    }
    pub fn new_lr((left, right): (i16, i16)) -> Self {
        Self {left, right}
    }
    pub fn get_sample_lr(&self) -> (i16, i16) {
        (self.left, self.right)
    }
}

pub trait AudioDevice {
    fn play_sample(&mut self, sample: AudioSample);
}