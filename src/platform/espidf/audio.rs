use esp_idf_svc::hal::{
    delay::{FreeRtos, TickType},
    gpio::{AnyIOPin, Input, PinDriver, Pull},
    i2s::{
        I2S0, I2sBiDir, I2sDriver,
        config::{
            Config, DataBitWidth, SlotMode, StdClkConfig, StdConfig, StdGpioConfig, StdSlotConfig,
        },
    },
};
use goodwin_error::{Result, ensure};

use crate::{
    builtin_assistant::SPEECH_HZ,
    voice_core::{CAPTURE_HZ, RECORDING_LIMIT, VoiceActivity, VoiceRequest},
};

pub(super) const FRAME_PAIRS: usize = CAPTURE_HZ as usize / 100;
const DMA_BUFFERS: usize = 6;
const IO_TIMEOUT_MS: u64 = 500;

pub(crate) struct Wiring<'d> {
    pub button: AnyIOPin<'d>,
    pub bit_clock: AnyIOPin<'d>,
    pub word_select: AnyIOPin<'d>,
    pub microphone: AnyIOPin<'d>,
    pub speaker: AnyIOPin<'d>,
    pub mic_slot: usize,
}

pub(crate) struct Audio<'d> {
    i2s: I2sDriver<'d, I2sBiDir>,
    button: PinDriver<'d, Input>,
    mic_slot: usize,
    was_pressed: bool,
    receiving: bool,
}

pub(super) enum Recording {
    Button,
    ManualTest,
    #[cfg(feature = "wake-tflite")]
    Wake {
        captured: Vec<i16>,
        post_trigger: usize,
    },
}

impl<'d> Audio<'d> {
    pub(crate) fn new(i2s: I2S0<'d>, wiring: Wiring<'d>) -> Result<Self> {
        ensure!(wiring.mic_slot < 2, "Invalid I2S microphone slot");
        let config = StdConfig::new(
            Config::new()
                .dma_buffer_count(DMA_BUFFERS as u32)
                .frames_per_buffer(FRAME_PAIRS as u32)
                .auto_clear(true),
            StdClkConfig::from_sample_rate_hz(CAPTURE_HZ),
            StdSlotConfig::philips_slot_default(DataBitWidth::Bits32, SlotMode::Stereo),
            StdGpioConfig::default(),
        );
        let button = PinDriver::input(wiring.button, Pull::Up)?;
        let i2s = I2sDriver::new_std_bidir(
            i2s,
            &config,
            wiring.bit_clock,
            wiring.microphone,
            wiring.speaker,
            AnyIOPin::none(),
            wiring.word_select,
        )?;
        let was_pressed = button.is_low();
        Ok(Self {
            i2s,
            button,
            mic_slot: wiring.mic_slot,
            was_pressed,
            receiving: false,
        })
    }

    pub(super) fn button_pressed(&mut self) -> bool {
        let mut pressed = self.button.is_low();
        if pressed != self.was_pressed {
            FreeRtos::delay_ms(20);
            pressed = self.button.is_low();
        }
        let previous = std::mem::replace(&mut self.was_pressed, pressed);
        pressed && !previous
    }

    pub(super) fn set_receiving(&mut self, receive: bool) -> Result<()> {
        if self.receiving != receive {
            if receive {
                self.i2s.rx_enable()?;
            } else {
                self.i2s.rx_disable()?;
            }
            self.receiving = receive;
        }
        Ok(())
    }

    pub(super) fn record(
        &mut self,
        mode: Recording,
        gain: f32,
        muted: impl Fn() -> bool,
    ) -> Result<Vec<i16>> {
        self.set_receiving(true)?;
        let result = self.capture(mode, gain, muted);
        let stopped = self.set_receiving(false);
        let recording = result?;
        stopped?;
        Ok(recording)
    }

    fn capture(
        &mut self,
        mode: Recording,
        gain: f32,
        muted: impl Fn() -> bool,
    ) -> Result<Vec<i16>> {
        let hold = matches!(mode, Recording::Button);
        let (mut recording, pre_roll) = match mode {
            #[cfg(feature = "wake-tflite")]
            Recording::Wake {
                captured,
                post_trigger,
            } => {
                let pre_roll = captured.len() - post_trigger;
                (captured, pre_roll)
            }
            _ => (Vec::new(), 0),
        };
        let mut activity = VoiceActivity::default();
        let mut captured = pre_roll;
        for frame in recording[pre_roll..].chunks(FRAME_PAIRS) {
            captured += frame.len();
            if activity.push(frame, gain, pre_roll != 0).is_some() {
                recording.truncate(captured);
                return Ok(recording);
            }
        }
        let mut mono = [0; FRAME_PAIRS];
        while recording.len() < RECORDING_LIMIT && (!hold || self.button.is_low()) {
            if muted() {
                return Ok(Vec::new());
            }
            let samples = self.read_mono(&mut mono)?;
            let samples = &samples[..samples.len().min(RECORDING_LIMIT - recording.len())];
            recording.extend_from_slice(samples);
            if !hold && let Some(heard) = activity.push(samples, gain, pre_roll != 0) {
                return Ok(if heard { recording } else { Vec::new() });
            }
        }
        Ok(recording)
    }

    pub(super) fn read_mono<'a>(&mut self, mono: &'a mut [i16; FRAME_PAIRS]) -> Result<&'a [i16]> {
        let mut frame = [0_u8; FRAME_PAIRS * 8];
        let bytes = self
            .i2s
            .read(&mut frame, TickType::new_millis(IO_TIMEOUT_MS).ticks())?;
        ensure!(bytes != 0 && bytes % 8 == 0, "Incomplete I2S capture frame");
        let high = self.mic_slot * 4 + 2;
        for (sample, pair) in mono.iter_mut().zip(frame[..bytes].chunks_exact(8)) {
            *sample = i16::from_le_bytes([pair[high], pair[high + 1]]);
        }
        Ok(&mono[..bytes / 8])
    }

    #[cfg(feature = "wasm-plugins")]
    pub(super) fn play_timer_signal(&mut self) -> Result<()> {
        let mut playback = self.playback(None);
        playback.push(&crate::voice_core::timer_signal_pcm(SPEECH_HZ))?;
        playback.finish()
    }

    pub(super) fn playback<'a>(
        &'a mut self,
        request: Option<&'a VoiceRequest>,
    ) -> Playback<'a, 'd> {
        Playback {
            i2s: &mut self.i2s,
            request,
            input: [0; 6],
            pending: 0,
            frame: [0; FRAME_PAIRS * 8],
            used: 0,
            enabled: false,
        }
    }
}

pub(super) struct Playback<'a, 'd> {
    i2s: &'a mut I2sDriver<'d, I2sBiDir>,
    request: Option<&'a VoiceRequest>,
    input: [u8; 6],
    pending: usize,
    frame: [u8; FRAME_PAIRS * 8],
    used: usize,
    enabled: bool,
}

impl Playback<'_, '_> {
    fn check(&self) -> Result<()> {
        ensure!(
            self.request
                .is_none_or(|request| request.failure().is_none()),
            "Voice request cancelled"
        );
        Ok(())
    }

    pub(super) fn push(&mut self, mut pcm: &[u8]) -> Result<()> {
        self.check()?;
        // HTTP chunk boundaries may fall inside a little-endian sample.
        while !pcm.is_empty() {
            let count = pcm.len().min(self.input.len() - self.pending);
            self.input[self.pending..self.pending + count].copy_from_slice(&pcm[..count]);
            self.pending += count;
            pcm = &pcm[count..];
            if self.pending == self.input.len() {
                self.resample()?;
                self.pending = 0;
            }
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<()> {
        self.check()?;
        ensure!(self.pending % 2 == 0, "Incomplete 16-bit PCM sample");
        if self.pending != 0 {
            self.resample()?;
        }
        self.flush()?;
        ensure!(self.enabled, "Empty PCM response");
        self.frame.fill(0);
        let timeout = TickType::new_millis(IO_TIMEOUT_MS).ticks();
        for _ in 0..DMA_BUFFERS {
            self.check()?;
            self.i2s.write_all(&self.frame, timeout)?;
        }
        self.i2s.tx_disable()?;
        self.enabled = false;
        Ok(())
    }

    fn resample(&mut self) -> Result<()> {
        const _: () = assert!(CAPTURE_HZ * 3 == SPEECH_HZ * 2);
        let sample = |offset| {
            i32::from(i16::from_le_bytes([
                self.input[offset],
                self.input[offset + 1],
            ]))
        };
        let a = sample(0);
        let b = if self.pending >= 4 { sample(2) } else { a };
        let c = if self.pending >= 6 { sample(4) } else { b };
        let resampled = [(2 * a + b) / 3, (b + 2 * c) / 3];
        for value in resampled.into_iter().take(self.pending.min(4) / 2) {
            let slot = (value << 16).to_le_bytes();
            self.frame[self.used..self.used + 4].copy_from_slice(&slot);
            self.frame[self.used + 4..self.used + 8].copy_from_slice(&slot);
            self.used += 8;
            if self.used == self.frame.len() {
                self.flush()?;
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.check()?;
        if self.used != 0 {
            if !self.enabled {
                self.i2s.tx_enable()?;
                self.enabled = true;
            }
            self.i2s.write_all(
                &self.frame[..self.used],
                TickType::new_millis(IO_TIMEOUT_MS).ticks(),
            )?;
            self.used = 0;
        }
        Ok(())
    }
}

impl Drop for Playback<'_, '_> {
    fn drop(&mut self) {
        if self.enabled
            && let Err(error) = self.i2s.tx_disable()
        {
            log::warn!("Stop I2S output: {error}");
        }
    }
}
