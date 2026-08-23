use std::f32::consts::PI;

/// 波形の種類。位相(0.0〜1.0)を受け取り`[-1.0, 1.0]`のサンプル値を返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Square,
    Sawtooth,
    Triangle,
}

impl Waveform {
    pub const ALL: [Waveform; 4] = [
        Waveform::Sine,
        Waveform::Square,
        Waveform::Sawtooth,
        Waveform::Triangle,
    ];

    pub fn next(self) -> Waveform {
        match self {
            Waveform::Sine => Waveform::Square,
            Waveform::Square => Waveform::Sawtooth,
            Waveform::Sawtooth => Waveform::Triangle,
            Waveform::Triangle => Waveform::Sine,
        }
    }

    fn sample(self, phase: f32) -> f32 {
        match self {
            Waveform::Sine => (2.0 * PI * phase).sin(),
            Waveform::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Sawtooth => 2.0 * phase - 1.0,
            Waveform::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
        }
    }
}

/// 非対称三角波LFO。位相`phase`(0.0〜1.0)が`rise_ratio`未満の区間で`-1.0`から`1.0`へ
/// 線形に上昇し、それ以降の区間で`1.0`から`-1.0`へ線形に下降する。`rise_ratio`が
/// 0.5なら対称三角波(`Waveform::Triangle`と同じ)になる。
fn asymmetric_triangle_lfo(phase: f32, rise_ratio: f32) -> f32 {
    if phase < rise_ratio {
        -1.0 + 2.0 * (phase / rise_ratio)
    } else {
        1.0 - 2.0 * ((phase - rise_ratio) / (1.0 - rise_ratio))
    }
}

/// 指定した波形を1サンプルずつ生成する。ON/OFF状態と周波数を保持する。
/// スイープモードを有効にすると、中心周波数を軸に実効周波数自体を低周波オシレータ(LFO)で揺らす。
pub struct WaveformGenerator {
    sample_rate: u32,
    center_frequency: f32,
    phase: f32,
    on: bool,
    waveform: Waveform,
    sweep_enabled: bool,
    modulation_depth: f32,
    modulation_rate: f32,
    lfo_phase: f32,
    lfo_rise_ratio: f32,
}

impl WaveformGenerator {
    pub const MIN_FREQUENCY: f32 = 1.0;
    pub const MIN_LFO_RISE_RATIO: f32 = 0.01;
    pub const MAX_LFO_RISE_RATIO: f32 = 0.99;

    pub fn new(sample_rate: u32, frequency: f32) -> Self {
        let mut generator = Self {
            sample_rate,
            center_frequency: Self::MIN_FREQUENCY,
            phase: 0.0,
            on: false,
            waveform: Waveform::Sine,
            sweep_enabled: false,
            modulation_depth: 0.0,
            modulation_rate: 0.0,
            lfo_phase: 0.0,
            lfo_rise_ratio: 0.5,
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

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
    }

    pub fn waveform(&self) -> Waveform {
        self.waveform
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

    /// LFOの上昇区間の比率(0.0〜1.0)。0.5未満だと上げが短く下げが長い、
    /// 0.5より大きいと上げが長く下げが短い非対称な揺れ方になる。
    pub fn set_lfo_rise_ratio(&mut self, ratio: f32) {
        self.lfo_rise_ratio = ratio.clamp(Self::MIN_LFO_RISE_RATIO, Self::MAX_LFO_RISE_RATIO);
    }

    pub fn lfo_rise_ratio(&self) -> f32 {
        self.lfo_rise_ratio
    }

    pub fn effective_frequency(&self) -> f32 {
        let nyquist = self.sample_rate as f32 / 2.0;
        if !self.sweep_enabled {
            return self.center_frequency;
        }
        let lfo = asymmetric_triangle_lfo(self.lfo_phase, self.lfo_rise_ratio);
        (self.center_frequency + self.modulation_depth * lfo)
            .clamp(Self::MIN_FREQUENCY, nyquist - 1.0)
    }

    pub fn next_sample(&mut self) -> f32 {
        if !self.on {
            return 0.0;
        }
        let frequency = self.effective_frequency();
        let sample = self.waveform.sample(self.phase);

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
        let mut generator = WaveformGenerator::new(48000, 20000.0);
        generator.set_on(false);
        for _ in 0..1000 {
            assert_eq!(generator.next_sample(), 0.0);
        }
    }

    #[test]
    fn on_state_amplitude_within_range() {
        let mut generator = WaveformGenerator::new(48000, 20000.0);
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
        let mut generator = WaveformGenerator::new(sample_rate, frequency);
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
        let mut generator = WaveformGenerator::new(48000, 20000.0);
        generator.set_frequency(-100.0);
        assert_eq!(generator.frequency(), WaveformGenerator::MIN_FREQUENCY);
        generator.set_frequency(0.0);
        assert_eq!(generator.frequency(), WaveformGenerator::MIN_FREQUENCY);
    }

    #[test]
    fn frequency_clamped_to_nyquist() {
        let sample_rate = 48000u32;
        let mut generator = WaveformGenerator::new(sample_rate, 20000.0);
        generator.set_frequency(sample_rate as f32);
        assert!(generator.frequency() < sample_rate as f32 / 2.0);
    }

    #[test]
    fn frequency_change_takes_effect_immediately() {
        let mut generator = WaveformGenerator::new(48000, 15000.0);
        assert_eq!(generator.frequency(), 15000.0);
        generator.set_frequency(20000.0);
        assert_eq!(generator.frequency(), 20000.0);
    }

    #[test]
    fn sweep_disabled_by_default() {
        let generator = WaveformGenerator::new(48000, 20000.0);
        assert!(!generator.is_sweep_enabled());
    }

    #[test]
    fn sweep_toggle() {
        let mut generator = WaveformGenerator::new(48000, 20000.0);
        generator.set_sweep_enabled(true);
        assert!(generator.is_sweep_enabled());
        generator.set_sweep_enabled(false);
        assert!(!generator.is_sweep_enabled());
    }

    #[test]
    fn sweep_disabled_matches_fixed_frequency() {
        let sample_rate = 48000u32;
        let frequency = 1000.0f32;
        let mut generator = WaveformGenerator::new(sample_rate, frequency);
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
        let mut generator = WaveformGenerator::new(48000, 20000.0);
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
        let mut generator = WaveformGenerator::new(sample_rate, nyquist - 100.0);
        generator.set_sweep_enabled(true);
        generator.set_modulation(10_000.0, 1.0);
        for _ in 0..sample_rate {
            let freq = generator.effective_frequency();
            assert!(freq < nyquist, "freq={freq} nyquist={nyquist}");
            assert!(freq >= WaveformGenerator::MIN_FREQUENCY);
            generator.next_sample();
        }
    }

    #[test]
    fn set_modulation_clamps_negative_values() {
        let mut generator = WaveformGenerator::new(48000, 20000.0);
        generator.set_modulation(-500.0, -1.0);
        assert_eq!(generator.modulation_depth, 0.0);
        assert_eq!(generator.modulation_rate, 0.0);
    }

    #[test]
    fn default_waveform_is_sine() {
        let generator = WaveformGenerator::new(48000, 20000.0);
        assert_eq!(generator.waveform(), Waveform::Sine);
    }

    #[test]
    fn waveform_can_be_switched() {
        let mut generator = WaveformGenerator::new(48000, 20000.0);
        generator.set_waveform(Waveform::Square);
        assert_eq!(generator.waveform(), Waveform::Square);
    }

    #[test]
    fn waveform_next_cycles_through_all_variants() {
        assert_eq!(Waveform::Sine.next(), Waveform::Square);
        assert_eq!(Waveform::Square.next(), Waveform::Sawtooth);
        assert_eq!(Waveform::Sawtooth.next(), Waveform::Triangle);
        assert_eq!(Waveform::Triangle.next(), Waveform::Sine);
    }

    #[test]
    fn square_wave_matches_expected_pattern() {
        assert_eq!(Waveform::Square.sample(0.0), 1.0);
        assert_eq!(Waveform::Square.sample(0.25), 1.0);
        assert_eq!(Waveform::Square.sample(0.5), -1.0);
        assert_eq!(Waveform::Square.sample(0.75), -1.0);
    }

    #[test]
    fn sawtooth_wave_matches_expected_pattern() {
        assert!((Waveform::Sawtooth.sample(0.0) - (-1.0)).abs() < 1e-6);
        assert!((Waveform::Sawtooth.sample(0.25) - (-0.5)).abs() < 1e-6);
        assert!((Waveform::Sawtooth.sample(0.5) - 0.0).abs() < 1e-6);
        assert!((Waveform::Sawtooth.sample(0.75) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn triangle_wave_matches_expected_pattern() {
        assert!((Waveform::Triangle.sample(0.0) - (-1.0)).abs() < 1e-6);
        assert!((Waveform::Triangle.sample(0.25) - 0.0).abs() < 1e-6);
        assert!((Waveform::Triangle.sample(0.5) - 1.0).abs() < 1e-6);
        assert!((Waveform::Triangle.sample(0.75) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn default_lfo_rise_ratio_is_half() {
        let generator = WaveformGenerator::new(48000, 20000.0);
        assert_eq!(generator.lfo_rise_ratio(), 0.5);
    }

    #[test]
    fn lfo_rise_ratio_half_matches_symmetric_triangle() {
        // rise_ratio=0.5のLFOはWaveform::Triangleと同じ値になるはず
        for phase in [0.0, 0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9] {
            let lfo_value = asymmetric_triangle_lfo(phase, 0.5);
            let triangle_value = Waveform::Triangle.sample(phase);
            assert!(
                (lfo_value - triangle_value).abs() < 1e-5,
                "phase={phase} lfo={lfo_value} triangle={triangle_value}"
            );
        }
    }

    #[test]
    fn lfo_rise_ratio_shifts_peak_position() {
        // ピーク(+1)に達するのはrise_ratioの位置になるはず
        let ratio = 0.2;
        let peak = asymmetric_triangle_lfo(ratio, ratio);
        assert!((peak - 1.0).abs() < 1e-5, "peak={peak}");

        // 位相0の時点は上昇区間の始点(-1付近)
        let start = asymmetric_triangle_lfo(0.0, ratio);
        assert!((start - (-1.0)).abs() < 1e-5, "start={start}");
    }

    #[test]
    fn lfo_rise_ratio_clamped_to_valid_range() {
        let mut generator = WaveformGenerator::new(48000, 20000.0);
        generator.set_lfo_rise_ratio(0.0);
        assert_eq!(
            generator.lfo_rise_ratio(),
            WaveformGenerator::MIN_LFO_RISE_RATIO
        );
        generator.set_lfo_rise_ratio(1.0);
        assert_eq!(
            generator.lfo_rise_ratio(),
            WaveformGenerator::MAX_LFO_RISE_RATIO
        );
    }

    #[test]
    fn sweep_with_asymmetric_lfo_stays_within_amplitude_range() {
        let mut generator = WaveformGenerator::new(48000, 20000.0);
        generator.set_on(true);
        generator.set_sweep_enabled(true);
        generator.set_modulation(2000.0, 0.5);
        generator.set_lfo_rise_ratio(0.2);
        for _ in 0..48000 {
            let sample = generator.next_sample();
            assert!((-1.0..=1.0).contains(&sample));
        }
    }

    #[test]
    fn all_waveforms_stay_within_amplitude_range() {
        for waveform in Waveform::ALL {
            let mut generator = WaveformGenerator::new(48000, 20000.0);
            generator.set_on(true);
            generator.set_waveform(waveform);
            for _ in 0..1000 {
                let sample = generator.next_sample();
                assert!(
                    (-1.0..=1.0).contains(&sample),
                    "waveform={waveform:?} sample={sample}"
                );
            }
        }
    }
}
