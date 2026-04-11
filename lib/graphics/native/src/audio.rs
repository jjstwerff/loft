// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! G5: Audio playback via rodio.
//! Thread-local state: one OutputStream + a list of loaded clips.

use std::cell::RefCell;
use std::io::Cursor;
use std::time::Duration;

/// A loaded audio clip — the raw bytes kept in memory for replay.
struct Clip {
    data: Vec<u8>,
}

struct AudioState {
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
    clips: Vec<Clip>,
    /// Currently playing sinks (one per active playback).
    sinks: Vec<rodio::Sink>,
}

thread_local! {
    static AUDIO: RefCell<Option<AudioState>> = const { RefCell::new(None) };
}

/// Ensure the audio output stream is initialised.
fn ensure_audio() -> bool {
    AUDIO.with(|cell| {
        if cell.borrow().is_some() {
            return true;
        }
        match rodio::OutputStream::try_default() {
            Ok((stream, handle)) => {
                *cell.borrow_mut() = Some(AudioState {
                    _stream: stream,
                    handle,
                    clips: Vec::new(),
                    sinks: Vec::new(),
                });
                true
            }
            Err(e) => {
                eprintln!("loft_audio: cannot open audio device: {e}");
                false
            }
        }
    })
}

/// Load an audio file (WAV or OGG).  Returns clip index (>= 0) or
/// `i32::MIN` (loft null sentinel) on failure.
#[unsafe(no_mangle)]
pub extern "C" fn loft_audio_load(path_ptr: *const u8, path_len: usize) -> i32 {
    let path = unsafe { loft_ffi::text(path_ptr, path_len) };
    if !ensure_audio() {
        return i32::MIN;
    }
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return i32::MIN,
    };
    AUDIO.with(|cell| {
        let mut st = cell.borrow_mut();
        let st = st.as_mut().unwrap();
        let idx = st.clips.len();
        st.clips.push(Clip { data });
        idx as i32
    })
}

/// Play a loaded clip at the given volume (0.0–1.0).
/// Returns sink index (for stopping) or -1 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn loft_audio_play(clip: i32, volume: f64) -> i32 {
    if clip < 0 {
        return -1;
    }
    AUDIO.with(|cell| {
        let mut st = cell.borrow_mut();
        let Some(st) = st.as_mut() else { return -1 };
        let idx = clip as usize;
        if idx >= st.clips.len() {
            return -1;
        }
        let data = st.clips[idx].data.clone();
        let cursor = Cursor::new(data);
        let source = match rodio::Decoder::new(cursor) {
            Ok(s) => s,
            Err(_) => return -1,
        };
        let sink = match rodio::Sink::try_new(&st.handle) {
            Ok(s) => s,
            Err(_) => return -1,
        };
        sink.set_volume(volume as f32);
        sink.append(source);
        // Reuse finished sink slots.
        for (i, s) in st.sinks.iter().enumerate() {
            if s.empty() {
                st.sinks[i] = sink;
                return i as i32;
            }
        }
        let si = st.sinks.len();
        st.sinks.push(sink);
        si as i32
    })
}

/// Stop a playing clip by sink index.
#[unsafe(no_mangle)]
pub extern "C" fn loft_audio_stop(sink_idx: i32) {
    if sink_idx < 0 {
        return;
    }
    AUDIO.with(|cell| {
        let mut st = cell.borrow_mut();
        let Some(st) = st.as_mut() else { return };
        let idx = sink_idx as usize;
        if idx < st.sinks.len() {
            st.sinks[idx].stop();
        }
    });
}

/// Set volume of a playing clip (0.0–1.0).
#[unsafe(no_mangle)]
pub extern "C" fn loft_audio_set_volume(sink_idx: i32, volume: f64) {
    if sink_idx < 0 {
        return;
    }
    AUDIO.with(|cell| {
        let mut st = cell.borrow_mut();
        let Some(st) = st.as_mut() else { return };
        let idx = sink_idx as usize;
        if idx < st.sinks.len() {
            st.sinks[idx].set_volume(volume as f32);
        }
    });
}

/// A raw PCM source: mono f32 samples at a given sample rate.
struct RawPcmSource {
    samples: Vec<f32>,
    pos: usize,
    sample_rate: u32,
}

impl Iterator for RawPcmSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.pos < self.samples.len() {
            let v = self.samples[self.pos];
            self.pos += 1;
            Some(v)
        } else {
            None
        }
    }
}

impl rodio::Source for RawPcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.samples.len() - self.pos)
    }
    fn channels(&self) -> u16 {
        1
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f64(
            self.samples.len() as f64 / self.sample_rate as f64,
        ))
    }
}

/// Play raw PCM samples (mono f32, values -1.0 to 1.0) at the given sample rate.
/// Returns sink index (for stopping/volume) or -1 on failure.
/// Native compilation path: receives raw pointer + count.
#[unsafe(no_mangle)]
pub extern "C" fn loft_audio_play_raw(
    data_ptr: *const f32,
    data_count: u32,
    sample_rate: i32,
    volume: f64,
) -> i32 {
    if data_ptr.is_null() || data_count == 0 || sample_rate <= 0 {
        return -1;
    }
    if !ensure_audio() {
        return -1;
    }
    let samples = unsafe { std::slice::from_raw_parts(data_ptr, data_count as usize) }.to_vec();
    let source = RawPcmSource {
        samples,
        pos: 0,
        sample_rate: sample_rate as u32,
    };
    AUDIO.with(|cell| {
        let mut st = cell.borrow_mut();
        let Some(st) = st.as_mut() else { return -1 };
        let sink = match rodio::Sink::try_new(&st.handle) {
            Ok(s) => s,
            Err(_) => return -1,
        };
        sink.set_volume(volume as f32);
        sink.append(source);
        for (i, s) in st.sinks.iter().enumerate() {
            if s.empty() {
                st.sinks[i] = sink;
                return i as i32;
            }
        }
        let si = st.sinks.len();
        st.sinks.push(sink);
        si as i32
    })
}

/// Interpreter wrapper: extracts vector<single> via LoftStore + LoftRef.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_audio_play_raw(
    store: loft_ffi::LoftStore,
    data: loft_ffi::LoftRef,
    sample_rate: i32,
    volume: f64,
) -> i32 {
    let count = unsafe { store.vector_len(&data) } as u32;
    let data_ptr = unsafe { store.vector_data_ptr(&data) } as *const f32;
    loft_audio_play_raw(data_ptr, count, sample_rate, volume)
}
