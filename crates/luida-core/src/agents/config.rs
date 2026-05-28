use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `~/.luida/agents.json` 기본 경로. `LUIDA_AGENTS_PATH`로 override.
pub fn default_agents_path() -> PathBuf {
    if let Ok(p) = std::env::var("LUIDA_AGENTS_PATH") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".luida").join("agents.json")
}

pub const TIERS: &[&str] = &["complex", "simple"];
pub const MODES: &[&str] = &["headless", "interactive"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentsConfig {
    pub version: u32,
    pub defaults: Defaults,
    pub runtimes: HashMap<String, RuntimeDef>,
    #[serde(default)]
    pub actions: HashMap<String, ActionConfig>,
    #[serde(default, rename = "projectOverrides")]
    pub project_overrides: HashMap<String, HashMap<String, ActionConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Defaults {
    pub runtime: String,
    pub tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDef {
    pub kind: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default, rename = "baseUrl")]
    pub base_url: Option<String>,
    #[serde(default, rename = "apiKeyEnv")]
    pub api_key_env: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub models: RuntimeModels,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeModels {
    pub complex: String,
    pub simple: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActionConfig {
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
}

fn default_true() -> bool {
    true
}

impl AgentsConfig {
    /// JSON 문자열에서 로드. `_`로 시작하는 키(주석)는 무시.
    pub fn from_json(s: &str) -> Result<Self> {
        let mut v: Value = serde_json::from_str(s).context("agents.json 파싱 실패")?;
        strip_comment_keys(&mut v);
        let cfg: Self = serde_json::from_value(v).context("agents.json 구조 불일치")?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path)
            .with_context(|| format!("agents.json 읽기 실패: {path:?}"))?;
        Self::from_json(&s)
    }

    /// 파일이 있으면 로드, 없으면 기본 설정.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::from_file(path)
        } else {
            Ok(Self::default_config())
        }
    }

    /// pretty JSON으로 저장.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).ok();
            }
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
            .with_context(|| format!("agents.json 쓰기 실패: {path:?}"))?;
        Ok(())
    }

    /// 기본 설정 (claude, complex→opus-4-7 / simple→sonnet-4-6).
    pub fn default_config() -> Self {
        let mut runtimes = HashMap::new();
        runtimes.insert(
            "claude".to_string(),
            RuntimeDef {
                kind: "claude-cli".to_string(),
                command: Some("claude".to_string()),
                base_url: None,
                api_key_env: None,
                enabled: true,
                models: RuntimeModels {
                    complex: "claude-opus-4-7".to_string(),
                    simple: "claude-sonnet-4-6".to_string(),
                },
            },
        );
        runtimes.insert(
            "codex".to_string(),
            RuntimeDef {
                kind: "codex-cli".to_string(),
                command: Some("codex".to_string()),
                base_url: None,
                api_key_env: None,
                enabled: true,
                models: RuntimeModels {
                    complex: "gpt-5.1-codex-max".to_string(),
                    simple: "gpt-5.1-codex-mini".to_string(),
                },
            },
        );
        // 행위별 tier 기본 매핑 (스펙 §3.2). complex→opus, simple→sonnet.
        let complex = ["campaign.plan", "quest.review", "escalation.triage", "learning.reflect", "merge.resolve"];
        let simple = ["quest.execute", "campaign.report", "project.ingest", "pr.describe", "inmail.summarize", "handoff.bundle"];
        let mut actions = HashMap::new();
        for a in complex {
            actions.insert(
                a.to_string(),
                ActionConfig {
                    runtime: Some("claude".to_string()),
                    tier: Some("complex".to_string()),
                    model: None,
                    mode: Some("headless".to_string()),
                },
            );
        }
        for a in simple {
            actions.insert(
                a.to_string(),
                ActionConfig {
                    runtime: Some("claude".to_string()),
                    tier: Some("simple".to_string()),
                    model: None,
                    mode: Some("headless".to_string()),
                },
            );
        }

        Self {
            version: 1,
            defaults: Defaults {
                runtime: "claude".to_string(),
                tier: "simple".to_string(),
            },
            runtimes,
            actions,
            project_overrides: HashMap::new(),
        }
    }

    /// 스키마 무결성 검증.
    pub fn validate(&self) -> Result<()> {
        if !TIERS.contains(&self.defaults.tier.as_str()) {
            bail!("defaults.tier 잘못됨: {} (complex|simple)", self.defaults.tier);
        }
        if !self.runtimes.contains_key(&self.defaults.runtime) {
            bail!("defaults.runtime '{}' 가 runtimes에 없음", self.defaults.runtime);
        }
        // 각 action / projectOverride의 runtime·tier·mode 참조 무결성
        for (name, ac) in &self.actions {
            self.validate_action(name, ac)?;
        }
        for (proj, map) in &self.project_overrides {
            for (name, ac) in map {
                self.validate_action(&format!("{proj}:{name}"), ac)?;
            }
        }
        Ok(())
    }

    fn validate_action(&self, name: &str, ac: &ActionConfig) -> Result<()> {
        if let Some(rt) = &ac.runtime {
            if !self.runtimes.contains_key(rt) {
                bail!("action '{name}'의 runtime '{rt}' 가 runtimes에 없음");
            }
        }
        if let Some(t) = &ac.tier {
            if !TIERS.contains(&t.as_str()) {
                bail!("action '{name}'의 tier '{t}' 잘못됨");
            }
        }
        if let Some(m) = &ac.mode {
            if !MODES.contains(&m.as_str()) {
                bail!("action '{name}'의 mode '{m}' 잘못됨 (headless|interactive)");
            }
        }
        Ok(())
    }
}

/// `_`로 시작하는 객체 키(주석)를 재귀적으로 제거.
fn strip_comment_keys(v: &mut Value) {
    if let Value::Object(map) = v {
        map.retain(|k, _| !k.starts_with('_'));
        for val in map.values_mut() {
            strip_comment_keys(val);
        }
    } else if let Value::Array(arr) = v {
        for item in arr.iter_mut() {
            strip_comment_keys(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "version": 1,
      "defaults": { "runtime": "claude", "tier": "simple" },
      "runtimes": {
        "_comment": "주석 무시되어야 함",
        "claude": { "kind": "claude-cli", "command": "claude",
          "models": { "complex": "claude-opus-4-7", "simple": "claude-sonnet-4-6" } }
      },
      "actions": {
        "campaign.plan": { "runtime": "claude", "tier": "complex", "mode": "headless" }
      }
    }"#;

    #[test]
    fn parse_strips_comments() {
        let cfg = AgentsConfig::from_json(SAMPLE).unwrap();
        assert_eq!(cfg.version, 1);
        assert!(cfg.runtimes.contains_key("claude"));
        assert!(!cfg.runtimes.contains_key("_comment"));
    }

    #[test]
    fn default_config_valid() {
        let cfg = AgentsConfig::default_config();
        cfg.validate().unwrap();
        assert_eq!(cfg.defaults.runtime, "claude");
    }

    #[test]
    fn default_config_maps_actions_to_tiers() {
        let cfg = AgentsConfig::default_config();
        // complex 행위
        assert_eq!(cfg.actions["campaign.plan"].tier.as_deref(), Some("complex"));
        assert_eq!(cfg.actions["learning.reflect"].tier.as_deref(), Some("complex"));
        // simple 행위
        assert_eq!(cfg.actions["quest.execute"].tier.as_deref(), Some("simple"));
        assert_eq!(cfg.actions["campaign.report"].tier.as_deref(), Some("simple"));
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let dir = std::env::temp_dir().join(format!("luida-agents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agents.json");
        let cfg = AgentsConfig::default_config();
        cfg.save(&path).unwrap();
        let loaded = AgentsConfig::from_file(&path).unwrap();
        assert_eq!(loaded.defaults.runtime, "claude");
        assert_eq!(loaded.actions.len(), cfg.actions.len());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_rejects_unknown_default_runtime() {
        let mut cfg = AgentsConfig::default_config();
        cfg.defaults.runtime = "ghost".to_string();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_bad_tier() {
        let mut cfg = AgentsConfig::default_config();
        cfg.defaults.tier = "medium".to_string();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_unknown_action_runtime() {
        let json = r#"{
          "version":1,"defaults":{"runtime":"claude","tier":"simple"},
          "runtimes":{"claude":{"kind":"claude-cli","models":{"complex":"o","simple":"s"}}},
          "actions":{"x":{"runtime":"ghost"}}
        }"#;
        assert!(AgentsConfig::from_json(json).is_err());
    }

    #[test]
    fn validate_rejects_bad_mode() {
        let json = r#"{
          "version":1,"defaults":{"runtime":"claude","tier":"simple"},
          "runtimes":{"claude":{"kind":"claude-cli","models":{"complex":"o","simple":"s"}}},
          "actions":{"x":{"mode":"telepathy"}}
        }"#;
        assert!(AgentsConfig::from_json(json).is_err());
    }

    #[test]
    fn enabled_defaults_true() {
        let cfg = AgentsConfig::from_json(SAMPLE).unwrap();
        assert!(cfg.runtimes["claude"].enabled);
    }
}
