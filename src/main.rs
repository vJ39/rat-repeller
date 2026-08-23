use std::collections::VecDeque;
use std::error::Error;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Sparkline};
use ratatui::{Frame, Terminal};

use rat_repeller::{Waveform, WaveformGenerator};

mod pixel_canvas;
mod splash;

const LOGO_PNG: &[u8] = include_bytes!("../docs/logo.png");
const SPLASH_DURATION: Duration = Duration::from_secs(2);

const MIN_UI_FREQUENCY: u32 = 20;
const FREQUENCY_STEP: u32 = 10;
const FREQUENCY_STEP_LARGE: u32 = 1_000;
const DEFAULT_FREQUENCY: u32 = 20_000;
const DEFAULT_MODULATION_DEPTH_HZ: u32 = 1_000;
const MIN_MODULATION_DEPTH_HZ: u32 = 100;
const MAX_MODULATION_DEPTH_HZ: u32 = 5_000;
const MODULATION_DEPTH_STEP_HZ: u32 = 10;
const DEFAULT_MODULATION_RATE_MILLIHZ: u32 = 300;
const MIN_MODULATION_RATE_MILLIHZ: u32 = 50;
const MAX_MODULATION_RATE_MILLIHZ: u32 = 100_000;
const MODULATION_RATE_STEP_MILLIHZ: u32 = 50;
const MODULATION_RATE_STEP_LARGE_MILLIHZ: u32 = 1_000;
const FREQUENCY_HISTORY_LEN: usize = 200;
const HISTORY_UPDATE_INTERVAL: Duration = Duration::from_millis(100);
/// コールバック呼び出し間隔(≒状態反映のレイテンシ)を抑えるための目標バッファサイズ。
/// デバイスが対応する範囲内であれば適用し、対応範囲外ならデバイスのデフォルトのままにする。
const PREFERRED_BUFFER_FRAMES: u32 = 512;

fn waveform_to_u8(w: Waveform) -> u8 {
    match w {
        Waveform::Sine => 0,
        Waveform::Square => 1,
        Waveform::Sawtooth => 2,
        Waveform::Triangle => 3,
    }
}

fn waveform_from_u8(v: u8) -> Waveform {
    match v {
        1 => Waveform::Square,
        2 => Waveform::Sawtooth,
        3 => Waveform::Triangle,
        _ => Waveform::Sine,
    }
}

fn debug_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("rat-repeller-debug.log")
}

fn debug_log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(debug_log_path())
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// TUIループ(メインスレッド)とオーディオコールバック(cpalの別スレッド)の間で共有する状態。
/// ロックを取らず、それぞれAtomicで読み書きする。
struct SharedState {
    is_on: AtomicBool,
    frequency: AtomicU32,
    sweep_enabled: AtomicBool,
    waveform: AtomicU8,
    modulation_depth_hz: AtomicU32,
    modulation_rate_millihz: AtomicU32,
    effective_frequency_millihz: AtomicU32,
}

impl SharedState {
    fn new() -> Self {
        Self {
            is_on: AtomicBool::new(false),
            frequency: AtomicU32::new(DEFAULT_FREQUENCY),
            sweep_enabled: AtomicBool::new(false),
            waveform: AtomicU8::new(waveform_to_u8(Waveform::Sine)),
            modulation_depth_hz: AtomicU32::new(DEFAULT_MODULATION_DEPTH_HZ),
            modulation_rate_millihz: AtomicU32::new(DEFAULT_MODULATION_RATE_MILLIHZ),
            effective_frequency_millihz: AtomicU32::new(DEFAULT_FREQUENCY * 1000),
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let _ = std::fs::remove_file(debug_log_path());
    let state = Arc::new(SharedState::new());

    let (_stream, sample_rate) = start_audio_stream(Arc::clone(&state))?;

    // 出力デバイスのナイキスト周波数未満に上限を抑える(それ以上を指定してもエイリアシングし音にならない)
    let max_ui_frequency = sample_rate / 2 - 1;
    state
        .frequency
        .store(DEFAULT_FREQUENCY.min(max_ui_frequency), Ordering::Relaxed);

    run_tui(&state, max_ui_frequency)
}

/// デバイスがF32出力で対応する設定のうち、最大サンプルレートのものを選ぶ。
/// サンプルレートが高いほど、出せる周波数の上限(ナイキスト周波数)も上がる。
fn select_output_config(
    device: &cpal::Device,
) -> Result<cpal::SupportedStreamConfig, Box<dyn Error>> {
    let best = device
        .supported_output_configs()?
        .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
        .max_by_key(|c| c.max_sample_rate())
        .ok_or("F32出力に対応する設定が見つかりません")?;
    Ok(best.with_max_sample_rate())
}

fn start_audio_stream(state: Arc<SharedState>) -> Result<(cpal::Stream, u32), Box<dyn Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("出力デバイスが見つかりません")?;
    let config = select_output_config(&device)?;
    let buffer_range = *config.buffer_size();
    let mut stream_config: cpal::StreamConfig = config.into();
    let sample_rate = stream_config.sample_rate;
    let channels = stream_config.channels as usize;

    if let cpal::SupportedBufferSize::Range { min, max } = buffer_range
        && (min..=max).contains(&PREFERRED_BUFFER_FRAMES) {
            stream_config.buffer_size = cpal::BufferSize::Fixed(PREFERRED_BUFFER_FRAMES);
        }
    debug_log(&format!("stream_config: {stream_config:?}"));

    let mut generator = WaveformGenerator::new(sample_rate, DEFAULT_FREQUENCY as f32);

    let err_fn = |err| eprintln!("オーディオストリームエラー: {err}");

    let stream = device.build_output_stream(
        stream_config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            generator.set_on(state.is_on.load(Ordering::Relaxed));
            generator.set_frequency(state.frequency.load(Ordering::Relaxed) as f32);
            generator.set_sweep_enabled(state.sweep_enabled.load(Ordering::Relaxed));
            generator.set_waveform(waveform_from_u8(state.waveform.load(Ordering::Relaxed)));
            let depth_hz = state.modulation_depth_hz.load(Ordering::Relaxed) as f32;
            let rate_hz = state.modulation_rate_millihz.load(Ordering::Relaxed) as f32 / 1000.0;
            generator.set_modulation(depth_hz, rate_hz);
            for frame in data.chunks_mut(channels) {
                let sample = generator.next_sample();
                for value in frame.iter_mut() {
                    *value = sample;
                }
            }
            let effective_millihz = (generator.effective_frequency() * 1000.0) as u32;
            state
                .effective_frequency_millihz
                .store(effective_millihz, Ordering::Relaxed);
        },
        err_fn,
        None,
    )?;

    stream.play()?;
    Ok((stream, sample_rate))
}

fn run_tui(state: &Arc<SharedState>, max_ui_frequency: u32) -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = show_splash(&mut terminal).and_then(|_| ui_loop(&mut terminal, state, max_ui_frequency));

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// 起動直後にロゴをピクセルアート表示する。何らかのキー入力、または`SPLASH_DURATION`
/// の経過で閉じる。ロゴのデコードに失敗した場合は何も表示せず即座に抜ける。
fn show_splash(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<(), Box<dyn Error>> {
    let Ok(image) = image::load_from_memory(LOGO_PNG) else {
        return Ok(());
    };

    let deadline = Instant::now() + SPLASH_DURATION;
    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            let canvas = splash::build_canvas(&image, area.width, area.height);
            frame.render_widget(Paragraph::new(canvas.to_lines(1.0)), area);
        })?;

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if event::poll(remaining.min(Duration::from_millis(100)))?
            && matches!(event::read()?, Event::Key(_))
        {
            break;
        }
    }
    Ok(())
}

fn ui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &Arc<SharedState>,
    max_ui_frequency: u32,
) -> Result<(), Box<dyn Error>> {
    let mut history: VecDeque<u64> = VecDeque::with_capacity(FREQUENCY_HISTORY_LEN);
    let mut last_history_update = Instant::now();

    loop {
        let current_effective_hz = state.effective_frequency_millihz.load(Ordering::Relaxed) / 1000;

        // キーリピート等でループが高速に回っても、グラフの更新はループ回数でなく実時間間隔で行う
        if last_history_update.elapsed() >= HISTORY_UPDATE_INTERVAL {
            history.push_back(current_effective_hz as u64);
            if history.len() > FREQUENCY_HISTORY_LEN {
                history.pop_front();
            }
            last_history_update = Instant::now();
        }

        terminal.draw(|frame| {
            draw(
                frame,
                state,
                current_effective_hz,
                &history,
                max_ui_frequency,
            )
        })?;

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()? {
                let freq_step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    FREQUENCY_STEP_LARGE
                } else {
                    FREQUENCY_STEP
                };
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char(' ') => {
                        let current = state.is_on.load(Ordering::Relaxed);
                        state.is_on.store(!current, Ordering::Relaxed);
                    }
                    KeyCode::Char('s') => {
                        let current = state.sweep_enabled.load(Ordering::Relaxed);
                        state.sweep_enabled.store(!current, Ordering::Relaxed);
                    }
                    KeyCode::Char('w') => {
                        let current = waveform_from_u8(state.waveform.load(Ordering::Relaxed));
                        state
                            .waveform
                            .store(waveform_to_u8(current.next()), Ordering::Relaxed);
                    }
                    KeyCode::Up => {
                        let current = state.frequency.load(Ordering::Relaxed);
                        let next = (current + freq_step).min(max_ui_frequency);
                        state.frequency.store(next, Ordering::Relaxed);
                    }
                    KeyCode::Down => {
                        let current = state.frequency.load(Ordering::Relaxed);
                        let next = current.saturating_sub(freq_step).max(MIN_UI_FREQUENCY);
                        state.frequency.store(next, Ordering::Relaxed);
                    }
                    KeyCode::Right => {
                        let rate_step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                            MODULATION_RATE_STEP_LARGE_MILLIHZ
                        } else {
                            MODULATION_RATE_STEP_MILLIHZ
                        };
                        let current = state.modulation_rate_millihz.load(Ordering::Relaxed);
                        let next = (current + rate_step).min(MAX_MODULATION_RATE_MILLIHZ);
                        state
                            .modulation_rate_millihz
                            .store(next, Ordering::Relaxed);
                    }
                    KeyCode::Left => {
                        let rate_step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                            MODULATION_RATE_STEP_LARGE_MILLIHZ
                        } else {
                            MODULATION_RATE_STEP_MILLIHZ
                        };
                        let current = state.modulation_rate_millihz.load(Ordering::Relaxed);
                        let next = current
                            .saturating_sub(rate_step)
                            .max(MIN_MODULATION_RATE_MILLIHZ);
                        state
                            .modulation_rate_millihz
                            .store(next, Ordering::Relaxed);
                    }
                    KeyCode::Char(']') => {
                        let current = state.modulation_depth_hz.load(Ordering::Relaxed);
                        let next =
                            (current + MODULATION_DEPTH_STEP_HZ).min(MAX_MODULATION_DEPTH_HZ);
                        state.modulation_depth_hz.store(next, Ordering::Relaxed);
                    }
                    KeyCode::Char('[') => {
                        let current = state.modulation_depth_hz.load(Ordering::Relaxed);
                        let next = current
                            .saturating_sub(MODULATION_DEPTH_STEP_HZ)
                            .max(MIN_MODULATION_DEPTH_HZ);
                        state.modulation_depth_hz.store(next, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
    }
    Ok(())
}

fn draw(
    frame: &mut Frame,
    state: &Arc<SharedState>,
    current_effective_hz: u32,
    history: &VecDeque<u64>,
    max_ui_frequency: u32,
) {
    let on = state.is_on.load(Ordering::Relaxed);
    let freq = state.frequency.load(Ordering::Relaxed);
    let sweeping = state.sweep_enabled.load(Ordering::Relaxed);
    let waveform = waveform_from_u8(state.waveform.load(Ordering::Relaxed));
    let depth_hz = state.modulation_depth_hz.load(Ordering::Relaxed);
    let rate_hz = state.modulation_rate_millihz.load(Ordering::Relaxed) as f32 / 1000.0;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let title = Paragraph::new(vec![
        Line::from(Span::styled(
            "((( rat-repeller )))",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "High-frequency sound. Peace of mind.",
            Style::default().fg(Color::Gray),
        )),
    ])
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let status_text = format!("{} / {:?}波", if on { "ON" } else { "OFF" }, waveform);
    let status_color = if on { Color::Green } else { Color::Red };
    let sweep_text = if sweeping {
        let period_secs = 1.0 / rate_hz;
        format!("スイープ ON (±{depth_hz}Hz, 変調速度{rate_hz:.2}Hz=周期{period_secs:.1}秒)")
    } else {
        "スイープ OFF".to_string()
    };
    let status = Paragraph::new(vec![
        Line::from(Span::styled(
            status_text,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(sweep_text, Style::default().fg(Color::Cyan))),
    ])
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL).title("状態"));
    frame.render_widget(status, chunks[1]);

    let freq_widget = Paragraph::new(format!("中心 {freq}Hz   瞬時 {current_effective_hz}Hz"))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).title("周波数"));
    frame.render_widget(freq_widget, chunks[2]);

    let history_data: Vec<u64> = history.iter().copied().collect();
    let graph = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("瞬時周波数の推移(0〜{max_ui_frequency}Hz)")),
        )
        .data(&history_data)
        .max(max_ui_frequency as u64)
        .style(Style::default().fg(Color::Green));
    frame.render_widget(graph, chunks[3]);

    let help = Paragraph::new(format!(
        "Space: ON/OFF  w: 波形切替  s: スイープ切替  ↑/↓: {FREQUENCY_STEP}Hz  Shift+↑/↓: {FREQUENCY_STEP_LARGE}Hz  ←/→: 速度0.05Hz  Shift+←/→: 速度1Hz  [/]: スイープ範囲  q: 終了"
    ))
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL).title("操作"));
    frame.render_widget(help, chunks[4]);
}
