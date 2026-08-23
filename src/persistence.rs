//! 起動時の設定と瞬時周波数の推移グラフを終了時に保存し、次回起動時に復元する。
//!
//! 保存/読み込みはパスを引数で受け取り、実ファイルシステム上の固定パスに
//! 依存しないテストを可能にする。ファイルが存在しない・壊れている場合は
//! `None`を返しデフォルト値で起動を続けられるようにする(エラーにしない)。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedState {
    pub is_on: bool,
    pub frequency: u32,
    pub waveform: u8,
    pub sweep_enabled: bool,
    pub modulation_depth_hz: u32,
    pub modulation_rate_millihz: u32,
    pub lfo_rise_percent: u32,
    pub history: Vec<u64>,
}

/// `dirs::config_dir()`配下の`rat-repeller/state.json`。ホームディレクトリが
/// 特定できない環境では`None`を返す(その場合は永続化をスキップする)。
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("rat-repeller").join("state.json"))
}

pub fn save_to(path: &Path, state: &PersistedState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

pub fn load_from(path: &Path) -> Option<PersistedState> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> PersistedState {
        PersistedState {
            is_on: true,
            frequency: 21000,
            waveform: 2,
            sweep_enabled: true,
            modulation_depth_hz: 1500,
            modulation_rate_millihz: 400,
            lfo_rise_percent: 30,
            history: vec![20000, 20100, 20200],
        }
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rat-repeller-test-{label}-{}", std::process::id()))
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = unique_temp_dir("roundtrip");
        let path = dir.join("state.json");
        let state = sample_state();

        save_to(&path, &state).expect("save should succeed");
        let loaded = load_from(&path).expect("load should succeed");

        assert_eq!(loaded, state);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_from_missing_path_returns_none() {
        let path = unique_temp_dir("missing").join("state.json");
        assert!(load_from(&path).is_none());
    }

    #[test]
    fn load_from_corrupted_file_returns_none() {
        let dir = unique_temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(&path, "not valid json{{{").unwrap();

        assert!(load_from(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_creates_missing_parent_directory() {
        let dir = unique_temp_dir("newdir");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("state.json");

        save_to(&path, &sample_state()).expect("save should create parent dirs");

        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
