//! CPAL 0.16 owner-thread capture; callbacks only convert into a bounded ring.
use crate::{audio::Pipeline, protocol::VoiceError};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat,
};
use ringbuf::{traits::*, HeapCons, HeapProd, HeapRb};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub struct Capture {
    stream: Option<cpal::Stream>,
    consumer: HeapCons<f32>,
    overflow: Arc<AtomicBool>,
    disconnected: Arc<AtomicBool>,
    pipeline: Pipeline,
    samples: usize,
    limit: usize,
}

pub fn devices() -> Result<Vec<String>, VoiceError> {
    let host = cpal::default_host();
    host.input_devices()
        .map_err(|_| VoiceError::MicrophoneUnavailable)?
        .map(|d| d.name().map_err(|_| VoiceError::MicrophoneUnavailable))
        .collect()
}

fn push<T: Copy>(
    data: &[T],
    producer: &mut HeapProd<f32>,
    overflow: &AtomicBool,
    convert: fn(T) -> f32,
) {
    if overflow.load(Ordering::Relaxed) {
        return;
    }
    for sample in data {
        if producer.try_push(convert(*sample)).is_err() {
            overflow.store(true, Ordering::Release);
            break;
        }
    }
}

impl Capture {
    pub fn start(selected: Option<&str>) -> Result<Self, VoiceError> {
        let host = cpal::default_host();
        let device = if let Some(name) = selected {
            let mut matches = host
                .input_devices()
                .map_err(|_| VoiceError::MicrophoneUnavailable)?
                .filter(|d| d.name().ok().as_deref() == Some(name));
            let device = matches.next().ok_or(VoiceError::MicrophoneUnavailable)?;
            if matches.next().is_some() {
                return Err(VoiceError::MicrophoneUnavailable);
            }
            device
        } else {
            host.default_input_device()
                .ok_or(VoiceError::MicrophoneUnavailable)?
        };
        let supported = device
            .supported_input_configs()
            .map_err(|_| VoiceError::MicrophoneUnavailable)?
            .find(|c| {
                (1..=8).contains(&c.channels())
                    && c.min_sample_rate().0 <= 192000
                    && c.max_sample_rate().0 >= 8000
                    && matches!(
                        c.sample_format(),
                        SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
                    )
            })
            .ok_or(VoiceError::UnsupportedBackend)?;
        let rate = 48000u32.clamp(
            supported.min_sample_rate().0.max(8000),
            supported.max_sample_rate().0.min(192000),
        );
        let config = supported.with_sample_rate(cpal::SampleRate(rate));
        let channels = config.channels();
        let capacity = rate as usize * channels as usize * 2;
        if capacity * 4 > 16 * 1024 * 1024 {
            return Err(VoiceError::UnsupportedBackend);
        }
        let (mut producer, consumer) = HeapRb::<f32>::new(capacity).split();
        let overflow = Arc::new(AtomicBool::new(false));
        let disconnected = Arc::new(AtomicBool::new(false));
        let callback_overflow = overflow.clone();
        let callback_disconnected = disconnected.clone();
        let error = move |_| {
            callback_disconnected.store(true, Ordering::Release);
        };
        let stream = match config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                &config.config(),
                move |data: &[f32], _| push(data, &mut producer, &callback_overflow, |s| s),
                error,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &config.config(),
                move |data: &[i16], _| {
                    push(data, &mut producer, &callback_overflow, |s| {
                        s as f32 / 32768.0
                    })
                },
                error,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &config.config(),
                move |data: &[u16], _| {
                    push(data, &mut producer, &callback_overflow, |s| {
                        (s as f32 - 32768.0) / 32768.0
                    })
                },
                error,
                None,
            ),
            _ => return Err(VoiceError::UnsupportedBackend),
        }
        .map_err(|_| VoiceError::MicrophoneUnavailable)?;
        let pipeline = Pipeline::new(rate, channels)?;
        stream
            .play()
            .map_err(|_| VoiceError::MicrophoneUnavailable)?;
        Ok(Self {
            stream: Some(stream),
            consumer,
            overflow,
            disconnected,
            pipeline,
            samples: 0,
            limit: rate as usize * channels as usize * 120,
        })
    }

    pub fn drain(&mut self) -> Result<bool, VoiceError> {
        if self.overflow.load(Ordering::Acquire) {
            return Err(VoiceError::CaptureOverflow);
        }
        if self.disconnected.load(Ordering::Acquire) {
            return Err(VoiceError::DeviceDisconnected);
        }
        // Bounded by the ring capacity, even if the producer runs concurrently.
        for _ in 0..self.consumer.occupied_len() {
            let Some(sample) = self.consumer.try_pop() else {
                break;
            };
            if self.samples >= self.limit {
                return Ok(true);
            }
            self.pipeline.push(sample)?;
            self.samples += 1;
        }
        Ok(self.samples >= self.limit)
    }

    pub fn stop(mut self) -> Result<zeroize::Zeroizing<Vec<f32>>, VoiceError> {
        self.stream.take();
        self.drain()?;
        self.pipeline.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_overflow_is_explicit() {
        let (mut producer, mut consumer) = HeapRb::<f32>::new(2).split();
        let overflow = AtomicBool::new(false);
        push(&[i16::MIN, 0, i16::MAX], &mut producer, &overflow, |s| {
            s as f32 / 32768.0
        });
        assert!(overflow.load(Ordering::Acquire));
        assert_eq!(consumer.try_pop(), Some(-1.0));
        assert_eq!(consumer.try_pop(), Some(0.0));
    }
}
