use std::collections::VecDeque;
use crate::audio::{AudioDevice, AudioSample};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

pub struct CpalAudioDevice {
    stream: Option<cpal::Stream>,
    buffer: Vec<AudioSample>,
    buffer_size: usize,
    audio_queue: Arc<Mutex<VecDeque<AudioSample>>>,
}

impl CpalAudioDevice {
    pub fn new(buffer_capacity_in_millis:usize) -> Self {
        let buffer_capacity = 2 * buffer_capacity_in_millis * 44100 / 1000;
        let dev = CpalAudioDevice {
            stream: None,
            buffer: Vec::with_capacity(buffer_capacity),
            buffer_size: buffer_capacity,
            audio_queue: Arc::new(Mutex::new(VecDeque::new())),
        };

        dev
    }

    pub fn start(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        match host.default_output_device() {
            Some(device) => {
                if let Ok(descr) = device.description() {
                    info!("Audio device selected: {:?}", descr);
                }
                match device.supported_output_configs() {
                    Ok(mut configs) => {
                        match configs.find(|c| c.min_sample_rate() <= 44100 && c.max_sample_rate() >= 44100 && matches!(c.sample_format(),SampleFormat::I16)) {
                            Some(config) => {
                                let config = config.try_with_sample_rate(44100).unwrap();
                                info!("Starting audio device using audio output config: {:?}", config);

                                let queue = self.audio_queue.clone();

                                let stream = device.build_output_stream(
                                    &config.config(),
                                    {
                                        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                                            if let Ok(mut queue) = queue.lock() {
                                                for audio in data.chunks_mut(2) {
                                                    let (left, right) = queue.pop_front().map(|s| (s.left,s.right)).unwrap_or((0,0));
                                                    audio[0] = left;
                                                    audio[1] = right;
                                                }
                                            }
                                            else {
                                                error!("Error getting audio queue lock");
                                            }
                                        }
                                    },
                                    move |err| {
                                        // react to errors here.
                                        error!("Audio stream error: {:?}", err);
                                    },
                                    None // None=blocking, Some(Duration)=timeout
                                ).unwrap();

                                stream.play().unwrap();
                                info!("Audio device started");
                                self.stream = Some(stream);

                                Ok(())
                            }
                            None => {
                                Err("No suitable audio output config found for 44100Hz and i16 sample format".to_string())
                            }
                        }
                    }
                    Err(e) => {
                        Err(format!("Error getting output device configs: {}", e))
                    }
                }
            }
            None => {
                Err("No audio output device found".to_string())
            }
        }
    }
}

impl AudioDevice for CpalAudioDevice {
    fn play_sample(&mut self, sample: AudioSample) {
        self.buffer.push(sample);
        if self.buffer.len() >= self.buffer_size {
            if let Ok(mut queue) = self.audio_queue.lock() {
                for sample in self.buffer.iter() {
                    queue.push_back(*sample);
                }
                self.buffer.clear();
            }
        }
    }
}