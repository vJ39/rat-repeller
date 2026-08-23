use std::f32::consts::PI;

/// サイン波を1サンプルずつ生成する。ON/OFF状態と周波数を保持する。
/// スイープモードを有効にすると、中心周波数を軸に実効周波数自体を低周波オシレータ(LFO)で正弦波状に揺らす。
pub struct SineWaveGenerator {
    sample_rate: u32,
    center_frequency: f32,
    phase: f32,
    on: bool,
    sweep_enabled: bool,
    modulation_depth: f32,
    modulation_rate: f32,
    lfo_phase: f32,
}

impl SineWaveGenerator {
    pub const MIN_FREQUENCY: f32 = 1.0;

    pub fn new(sample_rate: u32, frequency: f32) -> Self {
        let mut generator = Self {
            sample_rate,
            center_frequency: Self::MIN_FREQUENCY,
            phase: 0.0,
            on: false,
            sweep_enabled: false,
            modulation_depth: 0.0,
            modulation_rate: 0.0,
            lfo_phase: 0.0,
        };
        generator.set_frequency(frequency);
        generator
    }

    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn set_frequency(&mut self, frequency: f32) {
        let nyquist = self.sample_rate as f32 / 2.0;
        self.center_frequency = frequency.clamp(Self::MIN_FREQUENCY, nyquist - 1.0);
    }

    pub fn frequency(&self) -> f32 {
        self.center_frequency
    }

    pub fn set_sweep_enabled(&mut self, enabled: bool) {
        self.sweep_enabled = enabled;
    }

    pub fn is_sweep_enabled(&self) -> bool {
        self.sweep_enabled
    }

    /// depth: 中心周波数からの最大変位(Hz)。rate: 変調の周期(Hz)。負値は0にクランプする。
    pub fn set_modulation(&mut self, depth: f32, rate: f32) {
        self.modulation_depth = depth.max(0.0);
        self.modulation_rate = rate.max(0.0);
    }

    pub fn effective_frequency(&self) -> f32 {
        let nyquist = self.sample_rate as f32 / 2.0;
        if !self.sweep_enabled {
            return self.center_frequency;
        }
        let lfo = (2.0 * PI * self.lfo_phase).sin();
        (self.center_frequency + self.modulation_depth * lfo)
            .clamp(Self::MIN_FREQUENCY, nyquist - 1.0)
    }

    pub fn next_sample(&mut self) -> f32 {
        if !self.on {
            return 0.0;
        }
        let frequency = self.effective_frequency();
        let sample = (2.0 * PI * self.phase).sin();

        self.phase += frequency / self.sample_rate as f32;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        if self.sweep_enabled && self.modulation_rate > 0.0 {
            self.lfo_phase += self.modulation_rate / self.sample_rate as f32;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
        }

        sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_state_always_returns_zero() {
        let mut generator = SineWaveGenerator::new(48000, 20000.0);
        generator.set_on(false);
        for _ in 0..1000 {
            assert_eq!(generator.next_sample(), 0.0);
        }
    }

    #[test]
    fn on_state_amplitude_within_range() {
        let mut generator = SineWaveGenerator::new(48000, 20000.0);
        generator.set_on(true);
        for _ in 0..10000 {
            let sample = generator.next_sample();
            assert!((-1.0..=1.0).contains(&sample));
        }
    }

    #[test]
    fn matches_sine_formula() {
        let sample_rate = 48000u32;
        let frequency = 1000.0f32;
        let mut generator = SineWaveGenerator::new(sample_rate, frequency);
        generator.set_on(true);
        for n in 0..48 {
            let expected = (2.0 * PI * frequency * n as f32 / sample_rate as f32).sin();
            let actual = generator.next_sample();
            assert!(
                (actual - expected).abs() < 1e-4,
                "n={n} expected={expected} actual={actual}"
            );
        }
    }

    #[test]
    fn frequency_clamped_to_lower_bound() {
        let mut generator = SineWaveGenerator::new(48000, 20000.0);
        generator.set_frequency(-100.0);
        assert_eq!(generator.frequency(), SineWaveGenerator::MIN_FREQUENCY);
        generator.set_frequency(0.0);
        assert_eq!(generator.frequency(), SineWaveGenerator::MIN_FREQUENCY);
    }

    #[test]
    fn frequency_clamped_to_nyquist() {
        let sample_rate = 48000u32;
        let mut generator = SineWaveGenerator::new(sample_rate, 20000.0);
        generator.set_frequency(sample_rate as f32);
        assert!(generator.frequency() < sample_rate as f32 / 2.0);
    }

    #[test]
    fn frequency_change_takes_effect_immediately() {
        let mut generator = SineWaveGenerator::new(48000, 15000.0);
        assert_eq!(generator.frequency(), 15000.0);
        generator.set_frequency(20000.0);
        assert_eq!(generator.frequency(), 20000.0);
    }

    #[test]
    fn sweep_disabled_by_default() {
        let generator = SineWaveGenerator::new(48000, 20000.0);
        assert!(!generator.is_sweep_enabled());
    }

    #[test]
    fn sweep_toggle() {
        let mut generator = SineWaveGenerator::new(48000, 20000.0);
        generator.set_sweep_enabled(true);
        assert!(generator.is_sweep_enabled());
        generator.set_sweep_enabled(false);
        assert!(!generator.is_sweep_enabled());
    }

    #[test]
    fn sweep_disabled_matches_fixed_frequency() {
        let sample_rate = 48000u32;
        let frequency = 1000.0f32;
        let mut generator = SineWaveGenerator::new(sample_rate, frequency);
        generator.set_on(true);
        generator.set_modulation(2000.0, 0.5);
        // sweep_enabled=falseのままなら変調パラメータを設定しても固定周波数と同じ波形になる
        for n in 0..48 {
            let expected = (2.0 * PI * frequency * n as f32 / sample_rate as f32).sin();
            let actual = generator.next_sample();
            assert!(
                (actual - expected).abs() < 1e-4,
                "n={n} expected={expected} actual={actual}"
            );
        }
    }

    #[test]
    fn sweep_enabled_amplitude_within_range() {
        let mut generator = SineWaveGenerator::new(48000, 20000.0);
        generator.set_on(true);
        generator.set_sweep_enabled(true);
        generator.set_modulation(2000.0, 0.5);
        for _ in 0..48000 {
            let sample = generator.next_sample();
            assert!((-1.0..=1.0).contains(&sample));
        }
    }

    #[test]
    fn sweep_effective_frequency_respects_nyquist() {
        let sample_rate = 48000u32;
        let nyquist = sample_rate as f32 / 2.0;
        let mut generator = SineWaveGenerator::new(sample_rate, nyquist - 100.0);
        generator.set_sweep_enabled(true);
        generator.set_modulation(10_000.0, 1.0);
        for _ in 0..sample_rate {
            let freq = generator.effective_frequency();
            assert!(freq < nyquist, "freq={freq} nyquist={nyquist}");
            assert!(freq >= SineWaveGenerator::MIN_FREQUENCY);
            generator.next_sample();
        }
    }

    #[test]
    fn set_modulation_clamps_negative_values() {
        let mut generator = SineWaveGenerator::new(48000, 20000.0);
        generator.set_modulation(-500.0, -1.0);
        assert_eq!(generator.modulation_depth, 0.0);
        assert_eq!(generator.modulation_rate, 0.0);
    }
}
