use std::collections::VecDeque;
use std::error::Error;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
use ratatui_image::picker::Picker;
use ratatui_image::{protocol::StatefulProtocol, StatefulImage};

use rat_repeller::SineWaveGenerator;

const LOGO_PNG: &[u8] = include_bytes!("../docs/logo.png");

const MIN_UI_FREQUENCY: u32 = 20;
const FREQUENCY_STEP: u32 = 10;
const FREQUENCY_STEP_LARGE: u32 = 1_000;
const DEFAULT_FREQUENCY: u32 = 20_000;
const MODULATION_DEPTH_HZ: f32 = 1_000.0;
const DEFAULT_MODULATION_RATE_MILLIHZ: u32 = 300;
const MIN_MODULATION_RATE_MILLIHZ: u32 = 50;
const MAX_MODULATION_RATE_MILLIHZ: u32 = 3_000;
const MODULATION_RATE_STEP_MILLIHZ: u32 = 50;
const FREQUENCY_HISTORY_LEN: usize = 200;

fn main() -> Result<(), Box<dyn Error>> {
    let is_on = Arc::new(AtomicBool::new(false));
    let frequency = Arc::new(AtomicU32::new(DEFAULT_FREQUENCY));
    let sweep_enabled = Arc::new(AtomicBool::new(false));
    let modulation_rate_millihz = Arc::new(AtomicU32::new(DEFAULT_MODULATION_RATE_MILLIHZ));
    let effective_frequency_millihz = Arc::new(AtomicU32::new(DEFAULT_FREQUENCY * 1000));

    let (_stream, sample_rate) = start_audio_stream(
        Arc::clone(&is_on),
        Arc::clone(&frequency),
        Arc::clone(&sweep_enabled),
        Arc::clone(&modulation_rate_millihz),
        Arc::clone(&effective_frequency_millihz),
    )?;

    // 出力デバイスのナイキスト周波数未満に上限を抑える(それ以上を指定してもエイリアシングし音にならない)
    let max_ui_frequency = sample_rate / 2 - 1;
    frequency.store(DEFAULT_FREQUENCY.min(max_ui_frequency), Ordering::Relaxed);

    let logo = load_logo_protocol();

    run_tui(
        &is_on,
        &frequency,
        &sweep_enabled,
        &modulation_rate_millihz,
        &effective_frequency_millihz,
        max_ui_frequency,
        logo,
    )
}

/// 端末が画像プロトコル(Sixel/Kitty/iTerm2等)に対応していればロゴを読み込む。
/// 対応していない/検出に失敗した場合はNoneを返し、呼び出し側はテキストタイトルにフォールバックする。
fn load_logo_protocol() -> Option<StatefulProtocol> {
    let debug_log = std::env::temp_dir().join("rat-repeller-debug.log");
    let picker = match Picker::from_query_stdio() {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::write(&debug_log, format!("Picker::from_query_stdio: {e:?}\n"));
            return None;
        }
    };
    let dyn_img = match image::load_from_memory(LOGO_PNG) {
        Ok(img) => img,
        Err(e) => {
            let _ = std::fs::write(&debug_log, format!("image::load_from_memory: {e:?}\n"));
            return None;
        }
    };
    let _ = std::fs::write(&debug_log, "load_logo_protocol: OK\n");
    Some(picker.new_resize_protocol(dyn_img))
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

fn start_audio_stream(
    is_on: Arc<AtomicBool>,
    frequency: Arc<AtomicU32>,
    sweep_enabled: Arc<AtomicBool>,
    modulation_rate_millihz: Arc<AtomicU32>,
    effective_frequency_millihz: Arc<AtomicU32>,
) -> Result<(cpal::Stream, u32), Box<dyn Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("出力デバイスが見つかりません")?;
    let config = select_output_config(&device)?;
    let stream_config: cpal::StreamConfig = config.into();
    let sample_rate = stream_config.sample_rate;
    let channels = stream_config.channels as usize;

    let mut generator = SineWaveGenerator::new(sample_rate, DEFAULT_FREQUENCY as f32);

    let err_fn = |err| eprintln!("オーディオストリームエラー: {err}");

    let stream = device.build_output_stream(
        stream_config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            generator.set_on(is_on.load(Ordering::Relaxed));
            generator.set_frequency(frequency.load(Ordering::Relaxed) as f32);
            generator.set_sweep_enabled(sweep_enabled.load(Ordering::Relaxed));
            let rate_hz = modulation_rate_millihz.load(Ordering::Relaxed) as f32 / 1000.0;
            generator.set_modulation(MODULATION_DEPTH_HZ, rate_hz);
            for frame in data.chunks_mut(channels) {
                let sample = generator.next_sample();
                for value in frame.iter_mut() {
                    *value = sample;
                }
            }
            let effective_millihz = (generator.effective_frequency() * 1000.0) as u32;
            effective_frequency_millihz.store(effective_millihz, Ordering::Relaxed);
        },
        err_fn,
        None,
    )?;

    stream.play()?;
    Ok((stream, sample_rate))
}

fn run_tui(
    is_on: &Arc<AtomicBool>,
    frequency: &Arc<AtomicU32>,
    sweep_enabled: &Arc<AtomicBool>,
    modulation_rate_millihz: &Arc<AtomicU32>,
    effective_frequency_millihz: &Arc<AtomicU32>,
    max_ui_frequency: u32,
    logo: Option<StatefulProtocol>,
) -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = ui_loop(
        &mut terminal,
        is_on,
        frequency,
        sweep_enabled,
        modulation_rate_millihz,
        effective_frequency_millihz,
        max_ui_frequency,
        logo,
    );

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn ui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    is_on: &Arc<AtomicBool>,
    frequency: &Arc<AtomicU32>,
    sweep_enabled: &Arc<AtomicBool>,
    modulation_rate_millihz: &Arc<AtomicU32>,
    effective_frequency_millihz: &Arc<AtomicU32>,
    max_ui_frequency: u32,
    mut logo: Option<StatefulProtocol>,
) -> Result<(), Box<dyn Error>> {
    let mut history: VecDeque<u64> = VecDeque::with_capacity(FREQUENCY_HISTORY_LEN);

    loop {
        let current_effective_hz = effective_frequency_millihz.load(Ordering::Relaxed) / 1000;
        history.push_back(current_effective_hz as u64);
        if history.len() > FREQUENCY_HISTORY_LEN {
            history.pop_front();
        }

        terminal.draw(|frame| {
            draw(
                frame,
                is_on,
                frequency,
                sweep_enabled,
                modulation_rate_millihz,
                current_effective_hz,
                &history,
                max_ui_frequency,
                logo.as_mut(),
            )
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                let step = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    FREQUENCY_STEP_LARGE
                } else {
                    FREQUENCY_STEP
                };
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char(' ') => {
                        let current = is_on.load(Ordering::Relaxed);
                        is_on.store(!current, Ordering::Relaxed);
                    }
                    KeyCode::Char('s') => {
                        let current = sweep_enabled.load(Ordering::Relaxed);
                        sweep_enabled.store(!current, Ordering::Relaxed);
                    }
                    KeyCode::Up => {
                        let current = frequency.load(Ordering::Relaxed);
                        let next = (current + step).min(max_ui_frequency);
                        frequency.store(next, Ordering::Relaxed);
                    }
                    KeyCode::Down => {
                        let current = frequency.load(Ordering::Relaxed);
                        let next = current.saturating_sub(step).max(MIN_UI_FREQUENCY);
                        frequency.store(next, Ordering::Relaxed);
                    }
                    KeyCode::Char(']') => {
                        let current = modulation_rate_millihz.load(Ordering::Relaxed);
                        let next = (current + MODULATION_RATE_STEP_MILLIHZ)
                            .min(MAX_MODULATION_RATE_MILLIHZ);
                        modulation_rate_millihz.store(next, Ordering::Relaxed);
                    }
                    KeyCode::Char('[') => {
                        let current = modulation_rate_millihz.load(Ordering::Relaxed);
                        let next = current
                            .saturating_sub(MODULATION_RATE_STEP_MILLIHZ)
                            .max(MIN_MODULATION_RATE_MILLIHZ);
                        modulation_rate_millihz.store(next, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn draw(
    frame: &mut Frame,
    is_on: &Arc<AtomicBool>,
    frequency: &Arc<AtomicU32>,
    sweep_enabled: &Arc<AtomicBool>,
    modulation_rate_millihz: &Arc<AtomicU32>,
    current_effective_hz: u32,
    history: &VecDeque<u64>,
    max_ui_frequency: u32,
    logo: Option<&mut StatefulProtocol>,
) {
    let on = is_on.load(Ordering::Relaxed);
    let freq = frequency.load(Ordering::Relaxed);
    let sweeping = sweep_enabled.load(Ordering::Relaxed);
    let rate_hz = modulation_rate_millihz.load(Ordering::Relaxed) as f32 / 1000.0;

    let title_height = if logo.is_some() { 12 } else { 4 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(title_height),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());

    if let Some(protocol) = logo {
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(chunks[0]);
        frame.render_widget(block, chunks[0]);
        frame.render_stateful_widget(StatefulImage::default(), inner, protocol);
    } else {
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
    }

    let status_text = if on { "ON" } else { "OFF" };
    let status_color = if on { Color::Green } else { Color::Red };
    let sweep_text = if sweeping {
        format!("スイープ ON (±{MODULATION_DEPTH_HZ:.0}Hz, {rate_hz:.2}Hz周期)")
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
        "Space: ON/OFF  s: スイープ切替  ↑/↓: {FREQUENCY_STEP}Hz  Shift+↑/↓: {FREQUENCY_STEP_LARGE}Hz  [/]: スイープ速度  q: 終了"
    ))
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL).title("操作"));
    frame.render_widget(help, chunks[4]);
}
