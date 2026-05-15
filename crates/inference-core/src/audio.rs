use std::io::Cursor;

use rubato::{FftFixedInOut, Resampler};

use crate::backend::SttError;

const TARGET_RATE: u32 = 16_000;
const MIN_RATE: u32 = 8_000;
const MAX_RATE: u32 = 96_000;
const MAX_CHANNELS: u16 = 2;

/// Decodes a WAV byte slice and returns mono f32 samples at 16 kHz.
pub fn process_wav(bytes: &[u8]) -> Result<Vec<f32>, SttError> {
    let cursor = Cursor::new(bytes);
    let reader = hound::WavReader::new(cursor)
        .map_err(|e| SttError::AudioDecode(e.to_string()))?;
    let spec = reader.spec();

    if spec.sample_rate < MIN_RATE || spec.sample_rate > MAX_RATE {
        return Err(SttError::AudioUnsupported(format!(
            "sample rate {} outside [{MIN_RATE}, {MAX_RATE}]",
            spec.sample_rate
        )));
    }
    if spec.channels == 0 || spec.channels > MAX_CHANNELS {
        return Err(SttError::AudioUnsupported(format!(
            "channels {} not in [1, {MAX_CHANNELS}]",
            spec.channels
        )));
    }

    let samples_f32 = decode_samples(reader)?;
    if samples_f32.is_empty() {
        return Err(SttError::AudioUnsupported("zero samples".to_string()));
    }

    let mono = to_mono(samples_f32, spec.channels);
    if spec.sample_rate == TARGET_RATE {
        Ok(mono)
    } else {
        resample_to_16k(&mono, spec.sample_rate)
    }
}

fn decode_samples(reader: hound::WavReader<Cursor<&[u8]>>) -> Result<Vec<f32>, SttError> {
    let spec = reader.spec();
    match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<f32>, _>>()
            .map_err(|e| SttError::AudioDecode(e.to_string())),
        hound::SampleFormat::Int => {
            let max = match spec.bits_per_sample {
                8 => f32::from(i8::MAX),
                16 => f32::from(i16::MAX),
                24 => 8_388_607.0,
                32 => {
                    #[allow(clippy::cast_precision_loss)]
                    let m = i32::MAX as f32;
                    m
                }
                bits => {
                    return Err(SttError::AudioUnsupported(format!(
                        "unsupported int bits: {bits}"
                    )))
                }
            };
            reader
                .into_samples::<i32>()
                .map(|s| {
                    s.map(|v| {
                        #[allow(clippy::cast_precision_loss)]
                        let sample = v as f32 / max;
                        sample
                    })
                })
                .collect::<Result<Vec<f32>, _>>()
                .map_err(|e| SttError::AudioDecode(e.to_string()))
        }
    }
}

fn to_mono(interleaved: Vec<f32>, channels: u16) -> Vec<f32> {
    if channels == 1 {
        return interleaved;
    }
    let ch = usize::from(channels);
    let mut out = Vec::with_capacity(interleaved.len() / ch);
    let mut i = 0;
    while i + ch <= interleaved.len() {
        let mut sum = 0.0_f32;
        for k in 0..ch {
            sum += interleaved[i + k];
        }
        #[allow(clippy::cast_precision_loss)]
        out.push(sum / ch as f32);
        i += ch;
    }
    out
}

fn resample_to_16k(samples: &[f32], src_rate: u32) -> Result<Vec<f32>, SttError> {
    let chunk = 1024_usize;
    let mut resampler = FftFixedInOut::<f32>::new(
        src_rate as usize,
        TARGET_RATE as usize,
        chunk,
        1,
    )
    .map_err(|e| SttError::Resample(e.to_string()))?;

    #[allow(clippy::cast_precision_loss)]
    let capacity = samples.len() * TARGET_RATE as usize / src_rate as usize;
    let mut out = Vec::with_capacity(capacity);
    let in_chunk = resampler.input_frames_next();
    let mut idx = 0;
    let mut input: Vec<Vec<f32>> = vec![vec![0.0_f32; in_chunk]];
    while idx + in_chunk <= samples.len() {
        input[0].copy_from_slice(&samples[idx..idx + in_chunk]);
        let result = resampler
            .process(&input, None)
            .map_err(|e| SttError::Resample(e.to_string()))?;
        out.extend_from_slice(&result[0]);
        idx += in_chunk;
    }
    // Pad+process the tail if needed.
    if idx < samples.len() {
        for (i, v) in samples[idx..].iter().enumerate() {
            input[0][i] = *v;
        }
        for i in (samples.len() - idx)..in_chunk {
            input[0][i] = 0.0;
        }
        let result = resampler
            .process(&input, None)
            .map_err(|e| SttError::Resample(e.to_string()))?;
        out.extend_from_slice(&result[0]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};
    use std::io::Cursor;

    fn synth_wav_i16(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
        let spec = WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut buf = Vec::new();
        {
            let mut writer = WavWriter::new(Cursor::new(&mut buf), spec).unwrap();
            for s in samples {
                writer.write_sample(*s).unwrap();
            }
            writer.finalize().unwrap();
        }
        buf
    }

    #[test]
    fn decodes_mono_16k_i16_without_resampling() {
        // 1000 samples at 16k = 62.5 ms
        #[allow(clippy::cast_possible_truncation)]
        let s: Vec<i16> = (0..1000).map(|i| (i * 32) as i16).collect();
        let wav = synth_wav_i16(&s, 1, 16_000);
        let out = process_wav(&wav).unwrap();
        assert_eq!(out.len(), 1000);
        // First sample should be ~0.0 (i16 0 → f32 0/32767)
        assert!(out[0].abs() < 0.001);
    }

    #[test]
    fn decodes_stereo_16k_and_mixes_to_mono() {
        // 100 stereo frames = 200 interleaved samples
        let s: Vec<i16> = (0..200).map(|i| if i % 2 == 0 { 32767 } else { -32768 }).collect();
        let wav = synth_wav_i16(&s, 2, 16_000);
        let out = process_wav(&wav).unwrap();
        assert_eq!(out.len(), 100);
        // L=+1.0, R=-1.0 mean = 0
        for v in &out {
            assert!(v.abs() < 0.01, "got {v}");
        }
    }

    #[test]
    fn resamples_44100_to_16000() {
        // 44100 samples at 44.1k = 1 s -> should produce ~16000 at 16k
        let s: Vec<i16> = vec![0; 44_100];
        let wav = synth_wav_i16(&s, 1, 44_100);
        let out = process_wav(&wav).unwrap();
        // Allow ±5% tolerance because of chunked resampling padding.
        let expected = 16_000;
        let lo = expected * 95 / 100;
        let hi = expected * 105 / 100;
        assert!(out.len() >= lo && out.len() <= hi, "got {} samples, expected ~{expected}", out.len());
    }

    #[test]
    fn rejects_zero_channel_wav_at_header_level() {
        // hound rejects channels=0 at write time; instead test sample-rate out of range:
        let s: Vec<i16> = vec![0; 10];
        let wav = synth_wav_i16(&s, 1, 4_000); // below MIN_RATE
        let err = process_wav(&wav).unwrap_err();
        match err {
            SttError::AudioUnsupported(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_garbage_bytes_as_bad_audio() {
        let err = process_wav(b"not a wav file").unwrap_err();
        match err {
            SttError::AudioDecode(_) => {}
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
