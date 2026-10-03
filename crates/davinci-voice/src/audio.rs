//! Bounded off-callback downmix and band-limited 16 kHz conversion.

use crate::protocol::VoiceError;
use rubato::{FftFixedIn, Resampler};
use zeroize::Zeroizing;

pub const MAX_SAMPLES: usize = 120 * 16_000;
const CHUNK: usize = 1024;

pub struct Pipeline {
    rate: usize,
    channels: usize,
    channel: usize,
    sum: f32,
    frames: usize,
    input: Zeroizing<Vec<f32>>,
    output: Zeroizing<Vec<f32>>,
    delay: usize,
    resampler: FftFixedIn<f32>,
}

impl Pipeline {
    pub fn new(rate: u32, channels: u16) -> Result<Self, VoiceError> {
        if !(8000..=192000).contains(&rate) || !(1..=8).contains(&channels) {
            return Err(VoiceError::UnsupportedBackend);
        }
        let resampler = FftFixedIn::new(rate as usize, 16000, CHUNK, 2, 1)
            .map_err(|_| VoiceError::UnsupportedBackend)?;
        Ok(Self {
            rate: rate as usize,
            channels: channels as usize,
            channel: 0,
            sum: 0.0,
            frames: 0,
            input: Zeroizing::new(Vec::with_capacity(CHUNK)),
            output: Zeroizing::new(Vec::with_capacity(MAX_SAMPLES)),
            delay: resampler.output_delay(),
            resampler,
        })
    }

    pub fn push(&mut self, sample: f32) -> Result<(), VoiceError> {
        if self.frames >= self.rate * 120 {
            return Err(VoiceError::CaptureOverflow);
        }
        self.sum += if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.channel += 1;
        if self.channel == self.channels {
            self.input.push(self.sum / self.channels as f32);
            self.frames += 1;
            self.channel = 0;
            self.sum = 0.0;
            if self.input.len() == CHUNK {
                self.process()?;
            }
        }
        Ok(())
    }

    fn process(&mut self) -> Result<(), VoiceError> {
        let block = self
            .resampler
            .process(&[&self.input[..]], None)
            .map_err(|_| VoiceError::InferenceFailed)?;
        let skip = self.delay.min(block[0].len());
        self.delay -= skip;
        let remaining = MAX_SAMPLES.saturating_sub(self.output.len());
        self.output.extend(block[0][skip..].iter().take(remaining));
        self.input.clear();
        Ok(())
    }

    pub fn finish(mut self) -> Result<Zeroizing<Vec<f32>>, VoiceError> {
        let expected = (self.frames * 16000 / self.rate).min(MAX_SAMPLES);
        // At most one partial input block plus the filter's delayed tail.
        for _ in 0..16 {
            if self.output.len() >= expected {
                break;
            }
            self.input.resize(CHUNK, 0.0);
            self.process()?;
        }
        if self.output.len() < expected {
            return Err(VoiceError::InferenceFailed);
        }
        self.output.truncate(expected);
        Ok(self.output)
    }
}

pub fn no_speech(pcm: &[f32]) -> bool {
    pcm.len() < 4800
        || pcm.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / (pcm.len().max(1) as f64) < 1e-8
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rates_preserve_duration_and_flush_tail() {
        for rate in [16_000, 44_100, 48_000] {
            let mut p = Pipeline::new(rate, 2).unwrap();
            for n in 0..rate {
                let x = (n as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.2;
                p.push(x).unwrap();
                p.push(x).unwrap();
            }
            let pcm = p.finish().unwrap();
            assert_eq!(pcm.len(), 16_000);
            assert!(pcm[15_000..].iter().any(|s| s.abs() > 0.1));
            assert!(!no_speech(&pcm));
        }
    }
    #[test]
    fn silence_nonfinite_and_configuration_bounds() {
        assert!(Pipeline::new(4000, 1).is_err());
        assert!(Pipeline::new(48000, 9).is_err());
        let mut p = Pipeline::new(16000, 1).unwrap();
        for _ in 0..16000 {
            p.push(f32::NAN).unwrap();
        }
        assert!(no_speech(&p.finish().unwrap()));
        assert!(no_speech(&[0.5; 100]));
    }
}
