use crate::audio::{AudioDevice, AudioSample};
use std::sync::mpsc;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, StreamConfig};
use tracing::{error, info};

pub struct CpalAudioDevice {
    tx_channel: Option<mpsc::Sender<AudioSample>>,
    stream: Option<cpal::Stream>,
}

impl CpalAudioDevice {
    pub fn new() -> Self {
        CpalAudioDevice {
            tx_channel: None,
            stream: None,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        match host.default_output_device() {
            Some(device) => {
                match device.supported_output_configs() {
                    Ok(mut configs) => {
                        match configs.find(|c| c.min_sample_rate() <= 44100 && c.max_sample_rate() >= 44100 && matches!(c.sample_format(),SampleFormat::I16)) {
                            Some(config) => {
                                let config = config.try_with_sample_rate(44100).unwrap();
                                info!("Starting audio device using audio output config: {:?}", config);
                                let (tx_channel, rx_channel) = mpsc::channel::<AudioSample>();
                                self.tx_channel = Some(tx_channel);

                                let stream = device.build_output_stream(
                                    &config.config(),
                                    {
                                        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                                            for audio in data.chunks_mut(2) {
                                                let (left, right) = if let Ok(sample) = rx_channel.try_recv() {
                                                    (sample.left, sample.right)
                                                } else {
                                                    (0, 0)
                                                };
                                                audio[0] = left;
                                                audio[1] = right;
                                            }
                                        }
                                    },
                                    move |err| {
                                        // react to errors here.
                                        error!("Audio stream error: {:?}", err);
                                    },
                                    None // None=blocking, Some(Duration)=timeout
                                ).unwrap();
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
        match self.tx_channel.as_ref().unwrap().send(sample) {
            Ok(_) => {}
            Err(e) => {
                error!("Error sending audio sample to audio device {:?}",e);
            }
        }
    }
}