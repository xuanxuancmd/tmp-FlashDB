# Rust Evidence Collection Implementation

本文档提供 Rust (cucumber-rs) 环境中证据收集的具体实现代码。

**依赖**：Evidence 类型定义和失败分类规则见 SKILL.md Step 4。

---

## 方案1：自定义 Writer（推荐）

### 完整实现

```rust
use cucumber::Writer;
use serde_json::json;
use chrono::Utc;

pub struct EvidenceWriter {
    current_scenario: Option<String>,
    step_history: Vec<String>,
    assertions: Vec<AssertionRecord>,
}

#[derive(serde::Serialize)]
struct AssertionRecord {
    step: String,
    expected: String,
    actual: String,
}

impl Writer for EvidenceWriter {
    fn handle_scenario_start(&mut self, scenario: &cucumber::Scenario) {
        self.current_scenario = Some(scenario.name.clone());
        self.step_history.clear();
        self.assertions.clear();
    }
    
    fn handle_step_start(&mut self, step: &cucumber::Step) {
        self.step_history.push(step.name.clone());
    }
    
    fn handle_step_failed(&mut self, ev: cucumber::StepFailed) {
        // 收集所有证据
        let manifest = json!({
            "schema_version": "harness-bdd-1.0",
            "scenario_id": self.current_scenario.unwrap(),
            "timestamp": Utc::now().to_rfc3339(),
            "artifacts": [
                json!({
                    "type": "world_state",
                    "format": "json",
                    "content": serde_json::to_value(&*ev.world).unwrap()
                }),
                json!({
                    "type": "stack_trace",
                    "format": "text",
                    "content": ev.error.to_string()
                }),
                json!({
                    "type": "step_history",
                    "format": "json",
                    "content": self.step_history
                }),
                json!({
                    "type": "assertion_diff",
                    "format": "json",
                    "content": self.extract_assertion_diff(&ev)
                }),
            ],
            "chain_of_custody": {
                "test_run_id": std::env::var("CI_JOB_ID").unwrap_or("local"),
                "commit_sha": std::env::var("GIT_COMMIT_SHA").unwrap_or_else(|_| {
                    std::process::Command::new("git")
                        .args(["rev-parse", "HEAD"])
                        .output()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or("unknown")
                }),
            }
        });
        
        // 写入证据文件
        std::fs::write(
            format!("evidence/{}/manifest.json", self.current_scenario.unwrap()),
            serde_json::to_string_pretty(&manifest).unwrap()
        );
    }
    
    fn extract_assertion_diff(&self, ev: &cucumber::StepFailed) -> serde_json::Value {
        let msg = ev.error.to_string();
        
        // 解析 expected/actual
        if msg.contains("expected:") && msg.contains("got:") {
            let parts: Vec<&str> = msg.split("expected:").collect();
            if parts.len() > 1 {
                let exp_actual: Vec<&str> = parts[1].split("got:").collect();
                if exp_actual.len() == 2 {
                    return json!({
                        "step": ev.step.name,
                        "expected": exp_actual[0].trim(),
                        "actual": exp_actual[1].trim()
                    });
                }
            }
        }
        
        json!({
            "step": ev.step.name,
            "error": msg
        })
    }
}

// 注册 Writer
ConnectRuntimeWorld::cucumber()
    .with_writer(EvidenceWriter::new())
    .run_and_exit("./tests/resources/features")
    .await;
```

---

## 方案2：After Hook + World（简单）

```rust
#[after]
fn collect_evidence(world: &mut ConnectRuntimeWorld, scenario: &cucumber::Scenario) {
    if scenario.failed() {
        let world_json = serde_json::to_value(&world).unwrap();
        
        let evidence = json!({
            "scenario": scenario.name,
            "world_state": world_json,
            "timestamp": Utc::now().to_rfc3339(),
        });
        
        std::fs::write(
            format!("evidence/{}/world.json", scenario.name),
            serde_json::to_string_pretty(&evidence).unwrap()
        );
        
        // 保存日志
        if let Some(logs) = &world.logs {
            let content = String::from_utf8_lossy(&logs.lock().unwrap().clone());
            std::fs::write(format!("evidence/{}/logs.txt", scenario.name), content.as_ref());
        }
    }
}
```

---

## 失败分类器实现

```rust
pub fn classify_failure(error: &str, world: &ConnectRuntimeWorld) -> Classification {
    // 规则1：环境问题
    if error.contains("container") 
        || error.contains("env variable")
        || error.contains("docker") {
        return Classification {
            category: "environment_issue",
            confidence: 0.95,
            reasoning: "Error message indicates environment/configuration problem",
        };
    }
    
    // 规则2：Flaky特征
    if error.contains("timeout")
        || error.contains("race condition")
        || error.contains("intermittent") {
        return Classification {
            category: "flaky_test",
            confidence: 0.85,
            reasoning: "Error pattern suggests timing/race condition",
        };
    }
    
    // 规则3：逻辑bug（默认）
    Classification {
        category: "product_bug",
        confidence: 0.90,
        reasoning: format!(
            "World state mismatch: poll_count={}, committed_seqno={}",
            world.poll_count, world.committed_seqno
        ),
    }
}
```

---

## 运行汇总保存

```rust
use std::path::PathBuf;

pub fn save_evidence(run_id: &str, scenario: &str, manifest: &serde_json::Value) {
    let base_dir = PathBuf::from(".opencode/harness/evidence")
        .join(run_id)
        .join(scenario);
    
    std::fs::create_dir_all(&base_dir).unwrap();
    
    std::fs::write(
        base_dir.join("manifest.json"),
        serde_json::to_string_pretty(manifest).unwrap()
    );
}

pub fn save_run_summary(run_id: &str, passed: usize, failed: usize) {
    let summary = json!({
        "run_id": run_id,
        "timestamp": Utc::now().to_rfc3339(),
        "total": passed + failed,
        "passed": passed,
        "failed": failed,
    });
    
    let path = PathBuf::from(".opencode/harness/evidence")
        .join(run_id)
        .join("summary.json");
    
    std::fs::write(path, serde_json::to_string_pretty(&summary).unwrap());
}
```

---

## AI Diagnostic Report 结构定义

```rust
pub struct AiDiagnosticReport {
    schema_version: String,
    test_run: TestRunMeta,
    summary: TestSummary,
    failures: Vec<FailureAnalysis>,
}

#[derive(serde::Serialize)]
struct TestRunMeta {
    id: String,
    timestamp: String,
    environment: Environment,
}

#[derive(serde::Serialize)]
struct Environment {
    commit_sha: String,
    branch: String,
    ci_job: String,
}

#[derive(serde::Serialize)]
struct TestSummary {
    total: usize,
    passed: usize,
    failed: usize,
    flaky: usize,
}

#[derive(serde::Serialize)]
struct FailureAnalysis {
    test: TestInfo,
    status: String,
    error: ErrorInfo,
    world_state_at_failure: serde_json::Value,
    classification: Classification,
    ai_diagnosis: AiDiagnosis,
    history: FailureHistory,
}

#[derive(serde::Serialize)]
struct TestInfo {
    name: String,
    feature_file: String,
    feature_line: usize,
}

#[derive(serde::Serialize)]
struct ErrorInfo {
    message: String,
    error_type: String,
    stack_frames: Vec<StackFrame>,
    code_snippet: Vec<String>,
}

#[derive(serde::Serialize)]
struct StackFrame {
    file: String,
    line: usize,
    function: String,
    in_business_logic: bool,
}

#[derive(serde::Serialize)]
struct Classification {
    category: String,
    confidence: f32,
    reasoning: String,
}

#[derive(serde::Serialize)]
struct AiDiagnosis {
    root_cause_hypothesis: String,
    suspected_location: LocationRef,
    fix_suggestions: Vec<FixSuggestion>,
}

#[derive(serde::Serialize)]
struct LocationRef {
    file: String,
    function: String,
    reason: String,
}

#[derive(serde::Serialize)]
struct FixSuggestion {
    priority: String,
    action: String,
    code_location: String,
    expected_fix: String,
}

#[derive(serde::Serialize)]
struct FailureHistory {
    previous_failures: usize,
    last_passed_commit: String,
    recent_code_changes: Vec<CodeChange>,
}
```

---

## 参考实现路径

- `.opencode/harness/evidence/*.json` - 证据输出目录
- `connect-runtime/tests/e2e_phase1/harness/evidence.rs` - 已有实现