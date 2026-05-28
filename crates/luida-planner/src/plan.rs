//! 원정 계획(plan_json) 스키마 + 파싱 + 검증 + 위상정렬.
//!
//! plan_json은 `campaign.plan` 행위(LLM)의 산출물. 사용자 프롬프트를 다중 프로젝트
//! quest DAG로 분해한 것.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// 원정 계획 — quest DAG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CampaignPlan {
    pub title: String,
    pub quests: Vec<PlannedQuest>,
}

/// 계획된 quest 1건.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedQuest {
    /// plan 내 고유 키 (depends_on 참조용).
    pub key: String,
    pub project: String,
    pub brief: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

impl CampaignPlan {
    /// JSON 문자열 파싱. LLM이 코드펜스/산문으로 감쌌을 수 있어 첫 `{`~마지막 `}`를 추출.
    pub fn parse(raw: &str) -> Result<Self> {
        let json = extract_json_object(raw)
            .ok_or_else(|| anyhow::anyhow!("plan JSON 객체를 찾을 수 없음"))?;
        let plan: CampaignPlan = serde_json::from_str(json)?;
        Ok(plan)
    }

    /// 계획 유효성 검증 + 위상정렬 순서(키) 반환.
    ///
    /// - quests 비어있지 않음
    /// - 키 고유
    /// - 모든 project가 등록된 모험지(`known_projects`)
    /// - depends_on이 존재하는 키만 참조
    /// - 사이클 없음 (Kahn 위상정렬)
    pub fn validate(&self, known_projects: &HashSet<String>) -> Result<Vec<String>> {
        if self.quests.is_empty() {
            bail!("계획에 quest가 없습니다");
        }

        let mut keys = HashSet::new();
        for q in &self.quests {
            if q.key.trim().is_empty() {
                bail!("quest key가 비어있습니다");
            }
            if !keys.insert(q.key.clone()) {
                bail!("중복된 quest key: {}", q.key);
            }
            if q.brief.trim().is_empty() {
                bail!("quest '{}'의 brief가 비어있습니다", q.key);
            }
            if !known_projects.contains(&q.project) {
                bail!("quest '{}'의 프로젝트 '{}'가 미등록", q.key, q.project);
            }
        }

        for q in &self.quests {
            for dep in &q.depends_on {
                if !keys.contains(dep) {
                    bail!("quest '{}'의 의존성 '{}'가 존재하지 않음", q.key, dep);
                }
                if dep == &q.key {
                    bail!("quest '{}'가 자기 자신에 의존", q.key);
                }
            }
        }

        topo_sort(&self.quests)
    }
}

/// Kahn 위상정렬. 사이클이면 Err.
fn topo_sort(quests: &[PlannedQuest]) -> Result<Vec<String>> {
    let mut indegree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
    for q in quests {
        indegree.entry(q.key.as_str()).or_insert(0);
        for dep in &q.depends_on {
            *indegree.entry(q.key.as_str()).or_insert(0) += 1;
            dependents.entry(dep.as_str()).or_default().push(q.key.as_str());
        }
    }

    // 결정성을 위해 입력 순서로 큐 초기화.
    let mut queue: Vec<&str> = quests
        .iter()
        .map(|q| q.key.as_str())
        .filter(|k| indegree.get(k).copied().unwrap_or(0) == 0)
        .collect();

    let mut order = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        let k = queue[i];
        i += 1;
        order.push(k.to_string());
        if let Some(children) = dependents.get(k) {
            for &c in children {
                let e = indegree.get_mut(c).unwrap();
                *e -= 1;
                if *e == 0 {
                    queue.push(c);
                }
            }
        }
    }

    if order.len() != quests.len() {
        bail!("계획에 의존성 사이클이 있습니다");
    }
    Ok(order)
}

/// 문자열에서 첫 `{`부터 짝이 맞는 마지막 `}`까지의 JSON 객체 후보를 추출.
fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projects(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn plan_json() -> &'static str {
        r#"{
          "title": "스키마 동기화 원정",
          "quests": [
            { "key": "a", "project": "agora", "brief": "스키마 변경" },
            { "key": "b", "project": "admin", "brief": "반영", "depends_on": ["a"] }
          ]
        }"#
    }

    #[test]
    fn parse_plain_json() {
        let p = CampaignPlan::parse(plan_json()).unwrap();
        assert_eq!(p.title, "스키마 동기화 원정");
        assert_eq!(p.quests.len(), 2);
        assert_eq!(p.quests[1].depends_on, vec!["a"]);
    }

    #[test]
    fn parse_with_code_fence_and_prose() {
        let raw = format!("계획입니다:\n```json\n{}\n```\n끝.", plan_json());
        let p = CampaignPlan::parse(&raw).unwrap();
        assert_eq!(p.quests.len(), 2);
    }

    #[test]
    fn parse_garbage_fails() {
        assert!(CampaignPlan::parse("no json here").is_err());
    }

    #[test]
    fn validate_ok_returns_topo_order() {
        let p = CampaignPlan::parse(plan_json()).unwrap();
        let order = p.validate(&projects(&["agora", "admin"])).unwrap();
        assert_eq!(order, vec!["a", "b"]);
    }

    #[test]
    fn validate_rejects_unknown_project() {
        let p = CampaignPlan::parse(plan_json()).unwrap();
        assert!(p.validate(&projects(&["agora"])).is_err()); // admin 미등록
    }

    #[test]
    fn validate_rejects_duplicate_key() {
        let raw = r#"{"title":"t","quests":[
          {"key":"a","project":"agora","brief":"x"},
          {"key":"a","project":"agora","brief":"y"}]}"#;
        let p = CampaignPlan::parse(raw).unwrap();
        assert!(p.validate(&projects(&["agora"])).is_err());
    }

    #[test]
    fn validate_rejects_missing_dependency() {
        let raw = r#"{"title":"t","quests":[
          {"key":"a","project":"agora","brief":"x","depends_on":["ghost"]}]}"#;
        let p = CampaignPlan::parse(raw).unwrap();
        assert!(p.validate(&projects(&["agora"])).is_err());
    }

    #[test]
    fn validate_rejects_cycle() {
        let raw = r#"{"title":"t","quests":[
          {"key":"a","project":"agora","brief":"x","depends_on":["b"]},
          {"key":"b","project":"agora","brief":"y","depends_on":["a"]}]}"#;
        let p = CampaignPlan::parse(raw).unwrap();
        assert!(p.validate(&projects(&["agora"])).is_err());
    }

    #[test]
    fn validate_rejects_empty_quests() {
        let raw = r#"{"title":"t","quests":[]}"#;
        let p = CampaignPlan::parse(raw).unwrap();
        assert!(p.validate(&projects(&["agora"])).is_err());
    }

    #[test]
    fn topo_handles_diamond() {
        // a → b,c → d
        let raw = r#"{"title":"t","quests":[
          {"key":"a","project":"p","brief":"x"},
          {"key":"b","project":"p","brief":"x","depends_on":["a"]},
          {"key":"c","project":"p","brief":"x","depends_on":["a"]},
          {"key":"d","project":"p","brief":"x","depends_on":["b","c"]}]}"#;
        let p = CampaignPlan::parse(raw).unwrap();
        let order = p.validate(&projects(&["p"])).unwrap();
        // a가 먼저, d가 마지막
        assert_eq!(order[0], "a");
        assert_eq!(order[3], "d");
    }
}
