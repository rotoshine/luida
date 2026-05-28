use anyhow::{Context, Result};

use super::config::{ActionConfig, AgentsConfig};

/// 행위 해소 결과 — 실제로 어떤 런타임·모델·모드로 실행할지.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAgent {
    pub action: String,
    pub runtime: String,
    pub model: String,
    pub kind: String,
    pub tier: String,
    pub mode: String,
    pub command: Option<String>,
}

/// action(+optional project)을 런타임/모델/모드로 해소.
/// 우선순위: projectOverrides > actions > defaults.
pub fn resolve(
    cfg: &AgentsConfig,
    action: &str,
    project: Option<&str>,
) -> Result<ResolvedAgent> {
    let mut runtime = cfg.defaults.runtime.clone();
    let mut tier = cfg.defaults.tier.clone();
    let mut model: Option<String> = None;
    let mut mode = "headless".to_string();

    let mut apply = |ac: &ActionConfig| {
        if let Some(r) = &ac.runtime {
            runtime = r.clone();
        }
        if let Some(t) = &ac.tier {
            tier = t.clone();
        }
        if let Some(m) = &ac.model {
            model = Some(m.clone());
        }
        if let Some(md) = &ac.mode {
            mode = md.clone();
        }
    };

    // actions → projectOverrides 순으로 덮어씀 (project가 최우선)
    if let Some(ac) = cfg.actions.get(action) {
        apply(ac);
    }
    if let Some(proj) = project {
        if let Some(map) = cfg.project_overrides.get(proj) {
            if let Some(ac) = map.get(action) {
                apply(ac);
            }
        }
    }

    let rt = cfg
        .runtimes
        .get(&runtime)
        .with_context(|| format!("action '{action}'의 runtime '{runtime}' 정의 없음"))?;

    if !rt.enabled {
        anyhow::bail!(
            "runtime '{runtime}'은 비활성(enabled=false). action '{action}'에 사용할 수 없음"
        );
    }

    let model = model.unwrap_or_else(|| match tier.as_str() {
        "complex" => rt.models.complex.clone(),
        _ => rt.models.simple.clone(),
    });

    Ok(ResolvedAgent {
        action: action.to_string(),
        runtime,
        model,
        kind: rt.kind.clone(),
        tier,
        mode,
        command: rt.command.clone(),
    })
}

/// 런타임 CLI가 PATH에 있는지 확인 (claude-cli/codex-cli 가용성).
/// command가 없는 런타임(API 기반)은 true 반환(가용성은 호출 시점 판단).
pub fn runtime_available(cfg: &AgentsConfig, runtime: &str) -> bool {
    let Some(rt) = cfg.runtimes.get(runtime) else {
        return false;
    };
    let Some(cmd) = &rt.command else {
        return true; // API 런타임은 여기서 판단 안 함
    };
    which(cmd)
}

fn which(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AgentsConfig {
        let json = r#"{
          "version": 1,
          "defaults": { "runtime": "claude", "tier": "simple" },
          "runtimes": {
            "claude": { "kind": "claude-cli", "command": "claude",
              "models": { "complex": "opus", "simple": "sonnet" } },
            "codex": { "kind": "codex-cli", "command": "codex",
              "models": { "complex": "codex-max", "simple": "codex-mini" } },
            "deepseek": { "kind": "openai-compatible", "enabled": false, "baseUrl": "x",
              "models": { "complex": "r", "simple": "c" } }
          },
          "actions": {
            "campaign.plan": { "runtime": "claude", "tier": "complex" },
            "quest.execute": { "runtime": "claude", "tier": "simple", "mode": "headless" }
          },
          "projectOverrides": {
            "agora": { "quest.execute": { "runtime": "codex", "tier": "complex", "mode": "interactive" } }
          }
        }"#;
        AgentsConfig::from_json(json).unwrap()
    }

    #[test]
    fn resolves_action_complex_to_opus() {
        let r = resolve(&cfg(), "campaign.plan", None).unwrap();
        assert_eq!(r.runtime, "claude");
        assert_eq!(r.tier, "complex");
        assert_eq!(r.model, "opus");
        assert_eq!(r.kind, "claude-cli");
        assert_eq!(r.mode, "headless");
    }

    #[test]
    fn resolves_simple_to_sonnet() {
        let r = resolve(&cfg(), "quest.execute", None).unwrap();
        assert_eq!(r.model, "sonnet");
        assert_eq!(r.mode, "headless");
    }

    #[test]
    fn project_override_wins() {
        let r = resolve(&cfg(), "quest.execute", Some("agora")).unwrap();
        assert_eq!(r.runtime, "codex");
        assert_eq!(r.tier, "complex");
        assert_eq!(r.model, "codex-max");
        assert_eq!(r.mode, "interactive");
    }

    #[test]
    fn project_without_override_uses_action() {
        // admin엔 override 없음 → action 기본
        let r = resolve(&cfg(), "quest.execute", Some("admin")).unwrap();
        assert_eq!(r.runtime, "claude");
        assert_eq!(r.model, "sonnet");
    }

    #[test]
    fn unknown_action_falls_back_to_defaults() {
        let r = resolve(&cfg(), "some.new.action", None).unwrap();
        assert_eq!(r.runtime, "claude"); // defaults
        assert_eq!(r.tier, "simple");
        assert_eq!(r.model, "sonnet");
        assert_eq!(r.mode, "headless");
    }

    #[test]
    fn disabled_runtime_rejected() {
        let json = r#"{
          "version":1,"defaults":{"runtime":"claude","tier":"simple"},
          "runtimes":{
            "claude":{"kind":"claude-cli","command":"claude","models":{"complex":"o","simple":"s"}},
            "deepseek":{"kind":"openai-compatible","enabled":false,"baseUrl":"x","models":{"complex":"r","simple":"c"}}
          },
          "actions":{"x":{"runtime":"deepseek"}}
        }"#;
        let cfg = AgentsConfig::from_json(json).unwrap();
        assert!(resolve(&cfg, "x", None).is_err()); // disabled
    }

    #[test]
    fn explicit_model_overrides_tier() {
        let json = r#"{
          "version":1,"defaults":{"runtime":"claude","tier":"simple"},
          "runtimes":{"claude":{"kind":"claude-cli","command":"claude","models":{"complex":"o","simple":"s"}}},
          "actions":{"x":{"model":"custom-model"}}
        }"#;
        let cfg = AgentsConfig::from_json(json).unwrap();
        assert_eq!(resolve(&cfg, "x", None).unwrap().model, "custom-model");
    }

    #[test]
    fn runtime_available_checks_path() {
        let c = cfg();
        // 'which' 자체는 거의 항상 있지만, 존재하지 않는 command는 false
        let json = r#"{
          "version":1,"defaults":{"runtime":"claude","tier":"simple"},
          "runtimes":{"claude":{"kind":"claude-cli","command":"luida-nonexistent-xyz","models":{"complex":"o","simple":"s"}}}
        }"#;
        let cfg2 = AgentsConfig::from_json(json).unwrap();
        assert!(!runtime_available(&cfg2, "claude"));
        // API 런타임(command 없음)은 true
        assert!(runtime_available(&c, "deepseek"));
        // 모르는 런타임은 false
        assert!(!runtime_available(&c, "ghost"));
    }
}
