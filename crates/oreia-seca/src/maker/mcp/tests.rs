use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{Method, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::Json;
use oreia_channels::Channel;
use oreia_core::ChatMessage;
use oreia_llm::{Completion, GenerateOpts, LlmError, LlmProvider, ModelRouter, ModelSpec, Usage};
use serde_json::{json, Value};

use super::*;
use crate::config::Config;
use crate::maker::tests::Recorder;
use crate::maker::ForgejoCtx;

/// Modelo falso: nenhum destes testes chama o LLM de verdade (lista/leitura não planeja
/// nem executa nada), só precisa existir pra `Maker::new` aceitar a config.
struct DummyLlm;
#[async_trait]
impl LlmProvider for DummyLlm {
    fn model_name(&self) -> &str {
        "fake"
    }
    async fn generate(&self, _: &[ChatMessage], _: &GenerateOpts) -> Result<Completion, LlmError> {
        Ok(Completion { text: String::new(), usage: Usage::default(), model: "fake".into() })
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst))
}

/// Forgejo falso: resolve cada rota pelo método+caminho. `routes` mapeia um sufixo de
/// caminho pra um corpo JSON de resposta; o endpoint `.diff` responde texto puro.
async fn mock_forgejo(routes: Vec<(&'static str, Value)>, diff: Option<&'static str>) -> String {
    let routes = Arc::new(routes);
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri, _body: Bytes| {
        let routes = routes.clone();
        async move {
            let path = uri.path().to_string();
            if method == Method::GET && path.ends_with(".diff") {
                return (StatusCode::OK, diff.unwrap_or("")).into_response();
            }
            for (suffix, body) in routes.iter() {
                if path.ends_with(suffix) {
                    return Json(body.clone()).into_response();
                }
            }
            Json(json!({})).into_response()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

type Calls = Arc<Mutex<Vec<(String, String, Value)>>>;

/// Como `mock_forgejo`, mas grava método+caminho+corpo de cada chamada — usado pelos
/// testes de ferramentas de ESCRITA (`create_story`), que precisam conferir o que foi
/// enviado ao Forgejo (labels, corpo da issue), não só a resposta.
async fn recording_forgejo(routes: Vec<(&'static str, Value)>) -> (String, Calls) {
    let routes = Arc::new(routes);
    let calls: Calls = Arc::default();
    let c = calls.clone();
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
        let routes = routes.clone();
        let c = c.clone();
        async move {
            let path = uri.path().to_string();
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            c.lock().unwrap().push((method.to_string(), path.clone(), v));
            for (suffix, resp) in routes.iter() {
                if path.ends_with(suffix) {
                    return Json(resp.clone()).into_response();
                }
            }
            Json(json!({})).into_response()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), calls)
}

async fn maker_with_forgejo(api: &str, agents_toml: &str) -> Arc<Maker> {
    std::env::set_var("OREIA_TEST_MCPAPI_OWNER_TOKEN", "own");
    std::env::set_var("OREIA_TEST_MCPAPI_AGENT_TOKEN", "tok");
    let dir = std::env::temp_dir().join(format!("oreia-mcpapi-{}", uuid_like()));
    let toml = format!(
        r#"
[paths]
data = "{d}/data"
workspace = "{d}/workspace"
skills = "{d}/skills"

[models.fake]
base_url = "http://localhost:1/v1"
model = "fake"

[roles]
default = ["fake"]

[forgejo]
url = "{api}"
owner_token_env = "OREIA_TEST_MCPAPI_OWNER_TOKEN"
repos = ["acme/app"]

{agents_toml}
"#,
        d = dir.display()
    );
    let cfg: Config = toml::from_str(&toml).unwrap();
    cfg.validate().unwrap();
    let forge = ForgejoCtx::build(&cfg).unwrap();

    let mut router = ModelRouter::new(Duration::from_secs(60), None);
    router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(DummyLlm));
    router.set_role("default", vec!["fake".into()]);
    let rec = Arc::new(Recorder::default());
    Maker::new(Arc::new(cfg), Arc::new(router), vec![rec as Arc<dyn Channel>], forge).await.unwrap()
}

fn agent_back_toml() -> &'static str {
    r#"
[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_MCPAPI_AGENT_TOKEN"
"#
}

#[tokio::test]
async fn list_stories_classifies_free_claimed_and_done_issues() {
    let issues = json!([
        {"number": 1, "title": "Livre, sem label", "body": "", "labels": [], "state": "open"},
        {"number": 2, "title": "Com agente, sem posse ativa", "body": "", "labels": [{"name": "agent:back"}], "state": "open"},
        {"number": 3, "title": "Fechada", "body": "", "labels": [{"name": "agent:back"}], "state": "closed"},
    ]);
    let api = mock_forgejo(vec![("/issues", issues), ("/pulls", json!([]))], None).await;
    let maker = maker_with_forgejo(&api, agent_back_toml()).await;

    maker.claim_mcp("acme/app", 2, Some("task-x".into()));

    let stories = maker.list_stories(ListStoriesReq { repo: None, status: None }).await.unwrap();
    assert_eq!(stories.len(), 3);

    let s1 = stories.iter().find(|s| s.number == 1).unwrap();
    assert_eq!(s1.status, "available");
    assert!(s1.owner.is_none());

    let s2 = stories.iter().find(|s| s.number == 2).unwrap();
    assert_eq!(s2.status, "in_progress");
    assert_eq!(s2.owner.as_deref(), Some("mcp"));

    let s3 = stories.iter().find(|s| s.number == 3).unwrap();
    assert_eq!(s3.status, "done");

    let available = maker.list_stories(ListStoriesReq { repo: None, status: Some("available".into()) }).await.unwrap();
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].number, 1);

    std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
}

#[tokio::test]
async fn get_story_reports_issue_and_mcp_owner() {
    let issue = json!({"number": 5, "title": "Somar", "body": "faz a soma", "labels": [], "state": "open"});
    let api = mock_forgejo(vec![("/issues/5", issue), ("/pulls", json!([]))], None).await;
    let maker = maker_with_forgejo(&api, agent_back_toml()).await;
    maker.claim_mcp("acme/app", 5, Some("task-y".into()));

    let story = maker.get_story(RepoIssueReq { repo: "acme/app".into(), n: 5 }).await.unwrap();
    assert_eq!(story.title, "Somar");
    assert_eq!(story.body, "faz a soma");
    assert_eq!(story.owner.as_deref(), Some("mcp"));
    assert!(story.phases.is_empty(), "sem Task registrada, não deveria ter fases");

    std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
}

#[tokio::test]
async fn get_pr_parses_files_from_the_diff_and_truncates() {
    let pr = json!({
        "number": 9, "title": "feat: soma", "html_url": "http://forgejo.test/acme/app/pulls/9",
        "state": "open", "user": {"login": "agent-back"}, "head": {"ref": "agent/dev-back/issue-7"}
    });
    let diff = "diff --git a/soma.py b/soma.py\nindex 000..111 100644\n--- a/soma.py\n+++ b/soma.py\n@@ -0,0 +1 @@\n+def soma(): pass\n";
    let api = mock_forgejo(vec![("/pulls/9", pr)], Some(diff)).await;
    let maker = maker_with_forgejo(&api, agent_back_toml()).await;

    let got = maker.get_pr(RepoIssueReq { repo: "acme/app".into(), n: 9 }).await.unwrap();
    assert_eq!(got.title, "feat: soma");
    assert_eq!(got.author.as_deref(), Some("agent-back"));
    assert_eq!(got.files_changed, vec!["soma.py".to_string()]);
    assert!(got.diff.contains("def soma"));
    assert!(got.review_notes.is_none(), "sem [mcp] configurado, não deveria procurar REVIEW.md");

    std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
}

#[tokio::test]
async fn mcp_stats_reuses_the_same_aggregation_as_the_stats_command() {
    let api = mock_forgejo(vec![], None).await;
    let maker = maker_with_forgejo(&api, agent_back_toml()).await;

    let path = maker.cfg.paths.data.join("escalation.jsonl");
    let entry = crate::executor::log::EscalationLogEntry {
        task_id: "t1".into(),
        role: "dev-back".into(),
        phase: 1,
        step: 1,
        executor: "builtin".into(),
        model: "m".into(),
        passed: true,
        reason: None,
        duration_ms: 10,
        prompt_tokens: 1,
        completion_tokens: 1,
        cost_usd: 0.01,
        at: chrono::Utc::now(),
    };
    crate::executor::log::append(&path, &[entry]).await.unwrap();

    let stats = maker.mcp_stats(None).await;
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].role, "dev-back");
    assert_eq!(stats[0].resolved_phases, 1);

    std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
}

fn story_req(agent: Option<&str>, criteria: Vec<&str>) -> CreateStoryReq {
    CreateStoryReq {
        repo: "acme/app".into(),
        title: "Somar números".into(),
        goal: "somar dois números e devolver o resultado".into(),
        acceptance_criteria: criteria.into_iter().map(String::from).collect(),
        likely_files: vec!["soma.py".into()],
        out_of_scope: Some("não precisa de interface".into()),
        agent: agent.map(String::from),
    }
}

#[tokio::test]
async fn create_story_rejects_without_acceptance_criteria() {
    let (api, calls) = recording_forgejo(vec![]).await;
    let maker = maker_with_forgejo(&api, agent_back_toml()).await;

    let err = maker.create_story(story_req(None, vec![])).await.unwrap_err();
    assert!(err.to_string().contains("critério de aceite"), "{err}");
    assert!(calls.lock().unwrap().is_empty(), "não deveria ter chamado o Forgejo");

    std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
}

#[tokio::test]
async fn create_story_with_agent_applies_the_label_and_logs_audit_without_the_content() {
    let labels = json!([{"id": 3, "name": "agent:back"}]);
    let created = json!({
        "number": 42, "title": "Somar números", "body": "", "labels": [{"id": 3, "name": "agent:back"}],
        "html_url": "http://forgejo.test/acme/app/issues/42"
    });
    let (api, calls) = recording_forgejo(vec![("/labels", labels), ("/issues", created)]).await;
    let maker = maker_with_forgejo(&api, agent_back_toml()).await;

    let resp = maker.create_story(story_req(Some("dev-back"), vec!["soma(2,3) == 5"])).await.unwrap();
    assert_eq!(resp.number, 42);
    assert_eq!(resp.agent.as_deref(), Some("dev-back"));

    let calls_now = calls.lock().unwrap().clone();
    let created_call = calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues")).expect("issue criada");
    assert_eq!(created_call.2["labels"], json!([3]));
    let body = created_call.2["body"].as_str().unwrap();
    assert!(body.contains("## Objetivo"), "{body}");
    assert!(body.contains("soma(2,3) == 5"), "{body}");
    assert!(body.contains("soma.py"), "{body}");
    assert!(body.contains("não precisa de interface"), "{body}");

    let audit = tokio::fs::read_to_string(maker.cfg.paths.data.join("mcp.jsonl")).await.unwrap();
    assert!(audit.contains("\"tool\":\"create_story\""), "{audit}");
    assert!(audit.contains("\"issue\":42"), "{audit}");
    // O log de auditoria nunca grava o conteúdo da história, só ferramenta/repo/issue/resultado.
    assert!(!audit.contains("soma.py") && !audit.contains("Objetivo"), "{audit}");

    std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
}

#[tokio::test]
async fn create_story_without_agent_creates_an_unlabeled_issue_and_skips_label_lookup() {
    let created = json!({
        "number": 7, "title": "Somar números", "body": "", "labels": [],
        "html_url": "http://forgejo.test/acme/app/issues/7"
    });
    let (api, calls) = recording_forgejo(vec![("/issues", created)]).await;
    let maker = maker_with_forgejo(&api, agent_back_toml()).await;

    let resp = maker.create_story(story_req(None, vec!["qualquer critério"])).await.unwrap();
    assert!(resp.agent.is_none());

    let calls_now = calls.lock().unwrap().clone();
    let created_call = calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues")).expect("issue criada");
    assert_eq!(created_call.2["labels"], json!([]));
    assert!(
        !calls_now.iter().any(|(_, p, _)| p.ends_with("/labels")),
        "sem agente, não devia ler labels: {calls_now:?}"
    );

    std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
}

#[tokio::test]
async fn create_story_rejects_unknown_agent_before_calling_the_forgejo() {
    let (api, calls) = recording_forgejo(vec![]).await;
    let maker = maker_with_forgejo(&api, agent_back_toml()).await;

    let err = maker.create_story(story_req(Some("dev-front-que-nao-existe"), vec!["c1"])).await.unwrap_err();
    assert!(err.to_string().contains("não existe"), "{err}");
    assert!(calls.lock().unwrap().is_empty());

    std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
}
