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

/// Como `maker_with_forgejo`, mas também devolve o `Recorder` — só os testes de
/// `review_pr` precisam conferir o aviso que vai pros canais humanos quando o limite
/// de rodadas estoura.
async fn maker_with_forgejo_and_recorder(api: &str, agents_toml: &str) -> (Arc<Maker>, Arc<Recorder>) {
    std::env::set_var("OREIA_TEST_MCPAPI_OWNER_TOKEN", "own");
    std::env::set_var("OREIA_TEST_MCPAPI_AGENT_TOKEN", "tok");
    let dir = std::env::temp_dir().join(format!("oreia-mcpapi-rec-{}", uuid_like()));
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
    router.add_model(
        "fake",
        ModelSpec::local("http://localhost:1/v1", "fake"),
        Arc::new(crate::maker::tests::ScriptedLlm),
    );
    router.set_role("default", vec!["fake".into()]);
    let rec = Arc::new(Recorder::default());
    let maker =
        Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();
    (maker, rec)
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
        handoff_from: None,
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

// ── checkout_issue / run_checks / submit_issue / release_issue (etapa 5) ─────────────

mod checkout_flow {
    use super::*;
    use oreia_core::{Phase, PhaseStatus, Task, WorkItem, WorkMode};

    fn git_missing() -> bool {
        std::process::Command::new("git").arg("--version").output().is_err()
    }

    fn git_sync(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// Repositório "remoto" (bare local) com `main` semeado — devolve o caminho do `.git`.
    fn setup_bare_remote(root: &std::path::Path) -> std::path::PathBuf {
        let remote_dir = root.join("remote/acme");
        let seed = root.join("seed");
        std::fs::create_dir_all(&remote_dir).unwrap();
        std::fs::create_dir_all(&seed).unwrap();
        let bare = remote_dir.join("app.git");
        git_sync(root, &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
        git_sync(&seed, &["init", "-b", "main"]);
        std::fs::write(seed.join("README.md"), "app").unwrap();
        git_sync(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
        git_sync(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "init"]);
        git_sync(&seed, &["push", bare.to_str().unwrap(), "main"]);
        bare
    }

    /// Empurra um branch de agente com um arquivo a mais, simulando uma tentativa
    /// anterior que chegou a commitar mas nunca abriu PR (falhou antes disso).
    fn push_agent_branch(root: &std::path::Path, bare: &std::path::Path, branch: &str, extra_file: &str) {
        let seed = root.join("seed");
        git_sync(&seed, &["checkout", "-b", branch]);
        std::fs::write(seed.join(extra_file), "conteúdo da tentativa anterior").unwrap();
        git_sync(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
        git_sync(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "feat: tentativa"]);
        git_sync(&seed, &["push", bare.to_str().unwrap(), branch]);
        git_sync(&seed, &["checkout", "main"]);
    }

    /// `Maker` com Forgejo (dono + `dev-back`) E `[mcp]` configurados, git real
    /// (`[forgejo].git_url` aponta pro bare local) — único helper deste módulo; os
    /// outros testes do arquivo (`list_stories` etc.) não precisam de git de verdade.
    async fn maker_for_checkout(
        api: &str,
        root: &std::path::Path,
        extra_toml: &str,
        llm: Arc<dyn LlmProvider>,
    ) -> Arc<Maker> {
        std::env::set_var("OREIA_TEST_CHECKOUT_OWNER_TOKEN", "own");
        std::env::set_var("OREIA_TEST_CHECKOUT_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_CHECKOUT_MCP_TOKEN", "mcptok");
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
git_url = "file://{d}/remote"
owner_token_env = "OREIA_TEST_CHECKOUT_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_CHECKOUT_AGENT_TOKEN"

[mcp]
enabled = true
agent_user = "agent-mcp"
agent_token_env = "OREIA_TEST_CHECKOUT_MCP_TOKEN"
workdir = "{d}/mcpwork"

{extra_toml}
"#,
            d = root.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();
        assert!(forge.as_ref().unwrap().mcp.is_some(), "token do MCP deveria ter sido carregado");

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), llm);
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        Maker::new(Arc::new(cfg), Arc::new(router), vec![rec as Arc<dyn Channel>], forge).await.unwrap()
    }

    fn issue_json(n: u64, labels: Value, body: &str) -> Value {
        json!({ "number": n, "title": "Somar números", "body": body, "labels": labels, "state": "open" })
    }

    const ACCEPTANCE_BODY: &str =
        "## Objetivo\nsomar\n\n## Critérios de aceite\n- [ ] soma(2,3) == 5\n- [ ] soma(0,0) == 0\n";

    #[tokio::test]
    async fn checkout_issue_refuses_when_already_claimed() {
        let (api, _calls) = recording_forgejo(vec![]).await;
        let maker = maker_with_forgejo(&api, agent_back_toml()).await;
        maker.claim_mcp("acme/app", 5, None);

        let err = maker.checkout_issue(RepoIssueReq { repo: "acme/app".into(), n: 5 }).await.unwrap_err();
        assert!(err.to_string().contains("já está com"), "{err}");

        std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn checkout_issue_clones_a_free_issue_and_parses_acceptance_criteria() {
        if git_missing() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-checkout-free-{}", uuid_like()));
        setup_bare_remote(&root);

        let routes = vec![
            ("/issues/10", issue_json(10, json!([]), ACCEPTANCE_BODY)),
            ("/pulls", json!([])),
            ("/labels", json!([{"id": 9, "name": "claimed:mcp"}, {"id": 3, "name": "agent:back"}])),
        ];
        let (api, calls) = recording_forgejo(routes).await;
        let maker = maker_for_checkout(&api, &root, "", Arc::new(DummyLlm)).await;

        let resp = maker.checkout_issue(RepoIssueReq { repo: "acme/app".into(), n: 10 }).await.unwrap();
        assert_eq!(resp.branch, "agent/mcp/issue-10");
        assert_eq!(resp.acceptance_criteria, vec!["soma(2,3) == 5", "soma(0,0) == 0"]);
        assert!(resp.failure_context.is_none());
        let workdir = std::path::PathBuf::from(&resp.workdir);
        assert!(workdir.join("README.md").exists(), "deveria ter clonado o conteúdo da base");
        assert!(maker.mcp_claim("acme/app", 10).is_some());

        let calls_now = calls.lock().unwrap().clone();
        let labeled =
            calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues/10/labels")).expect("label aplicada");
        assert_eq!(labeled.2["labels"], json!([9]));
        assert!(calls_now.iter().any(|(m, p, _)| m == "POST" && p.ends_with("/issues/10/comments")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn checkout_issue_continues_the_failed_agents_branch_and_reports_the_failure() {
        if git_missing() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-checkout-fail-{}", uuid_like()));
        let bare = setup_bare_remote(&root);
        push_agent_branch(&root, &bare, "agent/dev-back/issue-7", "soma.py");

        let routes = vec![
            ("/issues/7", issue_json(7, json!([{"name": "agent:back"}]), "")),
            ("/pulls", json!([])),
            ("/branches", json!([{"name": "agent/dev-back/issue-7"}])),
            ("/labels", json!([{"id": 9, "name": "claimed:mcp"}])),
        ];
        let (api, _calls) = recording_forgejo(routes).await;
        let maker = maker_for_checkout(&api, &root, "", Arc::new(DummyLlm)).await;

        let mut failed = Task::new(oreia_core::Source::Forgejo, "dono", "somar números");
        failed.model_role = Some("dev-back".into());
        failed.set_status(TaskStatus::Failed);
        failed.phases = vec![Phase {
            id: "p1".into(),
            name: "Write soma.py".into(),
            instruction: "x".into(),
            status: PhaseStatus::Failed,
            summary: Some("deu erro: sintaxe inválida".into()),
            resolved_step: None,
        }];
        failed.work_item = Some(WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue: 7,
            agent: "dev-back".into(),
            title: String::new(),
            mode: WorkMode::Code,
            review_branch: None,
            base_branch: None,
        });
        maker.store.save(&failed).await.unwrap();

        let resp = maker.checkout_issue(RepoIssueReq { repo: "acme/app".into(), n: 7 }).await.unwrap();
        assert_eq!(resp.branch, "agent/dev-back/issue-7");
        let failure = resp.failure_context.expect("deveria ter contexto de falha");
        assert!(failure.contains("deu erro: sintaxe inválida"), "{failure}");
        let workdir = std::path::PathBuf::from(&resp.workdir);
        assert!(workdir.join("soma.py").exists(), "deveria ter continuado do branch do agente");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn run_checks_fails_with_the_role_test_cmd_and_submit_issue_refuses() {
        if git_missing() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-checks-{}", uuid_like()));
        let bare = setup_bare_remote(&root);
        push_agent_branch(&root, &bare, "agent/dev-back/issue-7", "soma.py");

        let routes = vec![
            ("/issues/7", issue_json(7, json!([{"name": "agent:back"}]), "")),
            ("/pulls", json!([])),
            ("/branches", json!([{"name": "agent/dev-back/issue-7"}])),
            ("/labels", json!([{"id": 9, "name": "claimed:mcp"}])),
        ];
        let (api, _calls) = recording_forgejo(routes).await;
        let extra =
            "[escalation.dev-back]\nsteps = [{ executor = \"builtin\", model = \"fake\" }]\ntest_cmd = \"false\"\n";
        let maker = maker_for_checkout(&api, &root, extra, Arc::new(DummyLlm)).await;

        let mut failed = Task::new(oreia_core::Source::Forgejo, "dono", "somar números");
        failed.model_role = Some("dev-back".into());
        failed.set_status(TaskStatus::Failed);
        failed.work_item = Some(WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue: 7,
            agent: "dev-back".into(),
            title: String::new(),
            mode: WorkMode::Code,
            review_branch: None,
            base_branch: None,
        });
        maker.store.save(&failed).await.unwrap();

        maker.checkout_issue(RepoIssueReq { repo: "acme/app".into(), n: 7 }).await.unwrap();

        let checks = maker.run_checks(RepoIssueReq { repo: "acme/app".into(), n: 7 }).await.unwrap();
        assert!(!checks.passed, "{checks:?}");
        assert!(checks.reason.as_deref().unwrap_or_default().contains("test_cmd"), "{checks:?}");

        let submit = maker
            .submit_issue(SubmitIssueReq { repo: "acme/app".into(), n: 7, summary: "tentei consertar".into() })
            .await
            .unwrap();
        assert!(!submit.checks.passed);
        assert!(submit.pull_request.is_none());

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn submit_issue_commits_as_agent_mcp_opens_a_pr_and_updates_it_on_a_second_call() {
        if git_missing() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-submit-{}", uuid_like()));
        let bare = setup_bare_remote(&root);

        // Mock com estado: a 1ª POST /pulls cria (200); a 2ª simula "já existe" (422),
        // caindo no fallback que acha o PR aberto pelo branch e só comenta.
        let pull_attempts = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let attempts = pull_attempts.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, _body: Bytes| {
            let attempts = attempts.clone();
            async move {
                let path = uri.path().to_string();
                if method == Method::GET && path.ends_with("/issues/10") {
                    return Json(issue_json(10, json!([]), "")).into_response();
                }
                if method == Method::GET && path.ends_with("/pulls") {
                    return Json(json!([{
                        "number": 99, "title": "t", "html_url": "http://forgejo.test/acme/app/pulls/99",
                        "state": "open", "head": {"ref": "agent/mcp/issue-10"}
                    }]))
                    .into_response();
                }
                if method == Method::POST && path.ends_with("/pulls") {
                    let n = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n == 0 {
                        return Json(json!({
                            "number": 99, "title": "t", "html_url": "http://forgejo.test/acme/app/pulls/99", "state": "open"
                        }))
                        .into_response();
                    }
                    return (StatusCode::UNPROCESSABLE_ENTITY, "já existe PR pra este branch").into_response();
                }
                Json(json!({})).into_response()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        let maker = maker_for_checkout(&api, &root, "", Arc::new(DummyLlm)).await;
        let checkout = maker.checkout_issue(RepoIssueReq { repo: "acme/app".into(), n: 10 }).await.unwrap();
        let workdir = std::path::PathBuf::from(&checkout.workdir);

        std::fs::write(workdir.join("soma.py"), "def soma(a, b): return a + b").unwrap();
        let first = maker
            .submit_issue(SubmitIssueReq { repo: "acme/app".into(), n: 10, summary: "implementei a soma".into() })
            .await
            .unwrap();
        assert!(first.checks.passed, "{:?}", first.checks);
        assert_eq!(first.pull_request.as_deref(), Some("http://forgejo.test/acme/app/pulls/99"));

        let author =
            git_sync(&root, &["--git-dir", bare.to_str().unwrap(), "log", "-1", "--format=%an", "agent/mcp/issue-10"]);
        assert_eq!(author.trim(), "agent-mcp");

        // Issue livre (nunca teve agente) — a resolução via MCP não é um handoff de ninguém.
        let stats = maker.mcp_stats(Some("mcp".into())).await;
        let mcp_stats = stats.iter().find(|s| s.role == "mcp").expect("entrada \"mcp\" em /stats");
        assert_eq!(mcp_stats.resolved_phases, 1);
        assert!(mcp_stats.handoff_by_agent.is_empty(), "{:?}", mcp_stats.handoff_by_agent);

        std::fs::write(workdir.join("soma.py"), "def soma(a, b):\n    return a + b\n").unwrap();
        let second = maker
            .submit_issue(SubmitIssueReq { repo: "acme/app".into(), n: 10, summary: "ajustei a formatação".into() })
            .await
            .unwrap();
        assert_eq!(
            second.pull_request.as_deref(),
            Some("http://forgejo.test/acme/app/pulls/99"),
            "segunda chamada deveria apontar pro MESMO PR, não criar um novo"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn submit_issue_after_an_agent_failure_logs_a_handoff_in_stats() {
        if git_missing() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-handoff-{}", uuid_like()));
        let bare = setup_bare_remote(&root);
        push_agent_branch(&root, &bare, "agent/dev-back/issue-7", "soma.py");

        // `recording_forgejo` não distingue método (GET lista PRs, POST cria um) — aqui
        // precisamos dos dois comportamentos reais no mesmo caminho `/pulls`.
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, _body: Bytes| async move {
            let path = uri.path().to_string();
            if method == Method::GET && path.ends_with("/issues/7") {
                return Json(issue_json(7, json!([{"name": "agent:back"}]), "")).into_response();
            }
            if method == Method::GET && path.ends_with("/branches") {
                return Json(json!([{"name": "agent/dev-back/issue-7"}])).into_response();
            }
            if method == Method::GET && path.ends_with("/labels") {
                return Json(json!([{"id": 9, "name": "claimed:mcp"}])).into_response();
            }
            if method == Method::GET && path.ends_with("/pulls") {
                return Json(json!([])).into_response();
            }
            if method == Method::POST && path.ends_with("/pulls") {
                return Json(json!({
                    "number": 55, "title": "t", "html_url": "http://forgejo.test/acme/app/pulls/55", "state": "open"
                }))
                .into_response();
            }
            Json(json!({})).into_response()
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");
        let maker = maker_for_checkout(&api, &root, "", Arc::new(DummyLlm)).await;

        let mut failed = Task::new(oreia_core::Source::Forgejo, "dono", "somar números");
        failed.model_role = Some("dev-back".into());
        failed.set_status(TaskStatus::Failed);
        failed.work_item = Some(WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue: 7,
            agent: "dev-back".into(),
            title: String::new(),
            mode: WorkMode::Code,
            review_branch: None,
            base_branch: None,
        });
        maker.store.save(&failed).await.unwrap();

        let checkout = maker.checkout_issue(RepoIssueReq { repo: "acme/app".into(), n: 7 }).await.unwrap();
        std::fs::write(std::path::PathBuf::from(&checkout.workdir).join("soma.py"), "def soma(a, b): return a + b")
            .unwrap();
        let resp = maker
            .submit_issue(SubmitIssueReq { repo: "acme/app".into(), n: 7, summary: "consertei o bug".into() })
            .await
            .unwrap();
        assert!(resp.checks.passed, "{:?}", resp.checks);

        let stats = maker.mcp_stats(Some("mcp".into())).await;
        let mcp_stats = stats.iter().find(|s| s.role == "mcp").expect("entrada \"mcp\" em /stats");
        assert_eq!(mcp_stats.handoff_by_agent.get("dev-back"), Some(&1), "{:?}", mcp_stats.handoff_by_agent);

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn release_issue_reapplies_the_agent_label_and_the_webhook_dispatches_again() {
        if git_missing() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-release-{}", uuid_like()));
        let bare = setup_bare_remote(&root);
        push_agent_branch(&root, &bare, "agent/dev-back/issue-7", "soma.py");

        let routes = vec![
            ("/issues/7", issue_json(7, json!([{"name": "agent:back"}]), "")),
            ("/pulls", json!([])),
            ("/branches", json!([{"name": "agent/dev-back/issue-7"}])),
            ("/labels", json!([{"id": 9, "name": "claimed:mcp"}, {"id": 3, "name": "agent:back"}])),
        ];
        let (api, calls) = recording_forgejo(routes).await;
        // ScriptedLlm (não DummyLlm): a parte final deste teste dispara a via automática
        // de verdade (planner + executor), não só confere a posse.
        let maker = maker_for_checkout(&api, &root, "", Arc::new(crate::maker::tests::ScriptedLlm)).await;

        maker.checkout_issue(RepoIssueReq { repo: "acme/app".into(), n: 7 }).await.unwrap();
        assert!(maker.mcp_claim("acme/app", 7).is_some());

        let released = maker
            .release_issue(ReleaseIssueReq { repo: "acme/app".into(), n: 7, return_to_agent: true })
            .await
            .unwrap();
        assert_eq!(released.returned_to_agent.as_deref(), Some("dev-back"));
        assert!(maker.mcp_claim("acme/app", 7).is_none(), "a posse do MCP deveria ter sido liberada");

        let calls_now = calls.lock().unwrap().clone();
        let relabel = calls_now
            .iter()
            .rfind(|(m, p, _)| m == "POST" && p.ends_with("/issues/7/labels"))
            .expect("label do agente reaplicada");
        assert_eq!(relabel.2["labels"], json!([3]));

        // O webhook de "labeled" que o Forgejo mandaria de verdade agora acha a issue
        // livre de novo (sem posse do MCP) e despacha o dev-back normalmente.
        let event_body = json!({
            "action": "labeled",
            "issue": {"number": 7, "title": "Somar números", "body": "", "labels": [{"name": "agent:back"}]},
            "repository": {"name": "app", "full_name": "acme/app", "owner": {"login": "acme"}, "default_branch": "main"},
            "sender": {"login": "dono"}
        });
        let event = oreia_forgejo::webhook::parse_event("issues", event_body.to_string().as_bytes()).unwrap();
        maker.clone().handle_forgejo(event).await;

        // `is_active` é transitório (a tarefa libera a posse assim que termina, com
        // sucesso ou erro — ver `Maker::execute`), então o sinal confiável de que a via
        // automática disparou de novo é uma `Task` nova ter sido criada pro dev-back,
        // não a posse continuar ativa num instante arbitrário depois.
        let mut dispatched = false;
        // 480×25ms = 12s: visto falhar com 200 (5s) numa suíte cheia rodando em paralelo
        // sob carga pesada (o dispatch em si é rápido; o flake era tempo de CPU, não lógica).
        for _ in 0..480 {
            let tasks = maker.store.list().await.unwrap_or_default();
            if tasks.iter().any(|t| t.work_item.as_ref().map(|w| (w.issue, w.agent.as_str())) == Some((7, "dev-back")))
            {
                dispatched = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(dispatched, "deveria ter despachado uma tarefa pro dev-back depois do release + relabel");

        std::fs::remove_dir_all(&root).ok();
    }

    // ── dispatch_issue (etapa 7) ──────────────────────────────────────────────────

    #[tokio::test]
    async fn dispatch_issue_without_step_behaves_like_assign() {
        if git_missing() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-dispatch-{}", uuid_like()));
        setup_bare_remote(&root);

        let routes =
            vec![("/issues/10", issue_json(10, json!([]), "")), ("/labels", json!([{"id": 3, "name": "agent:back"}]))];
        let (api, calls) = recording_forgejo(routes).await;
        let maker = maker_for_checkout(&api, &root, "", Arc::new(crate::maker::tests::ScriptedLlm)).await;

        let resp = maker
            .clone()
            .dispatch_issue(DispatchIssueReq { repo: "acme/app".into(), n: 10, agent: "dev-back".into(), step: None })
            .await
            .unwrap();
        assert_eq!(resp.agent, "dev-back");
        assert!(resp.step.is_none());

        let calls_now = calls.lock().unwrap().clone();
        let labeled =
            calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues/10/labels")).expect("label aplicada");
        assert_eq!(labeled.2["labels"], json!([3]));

        let mut dispatched = false;
        // 480×25ms = 12s: visto falhar com 200 (5s) numa suíte cheia rodando em paralelo
        // sob carga pesada (o dispatch em si é rápido; o flake era tempo de CPU, não lógica).
        for _ in 0..480 {
            let tasks = maker.store.list().await.unwrap_or_default();
            if tasks.iter().any(|t| t.work_item.as_ref().map(|w| (w.issue, w.agent.as_str())) == Some((10, "dev-back")))
            {
                dispatched = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(dispatched, "deveria ter despachado uma tarefa pro dev-back");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn dispatch_issue_rejects_a_step_when_the_role_has_no_escalation_configured() {
        let (api, calls) = recording_forgejo(vec![]).await;
        let maker = maker_with_forgejo(&api, agent_back_toml()).await;

        let err = maker
            .clone()
            .dispatch_issue(DispatchIssueReq {
                repo: "acme/app".into(),
                n: 10,
                agent: "dev-back".into(),
                step: Some(2),
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("escalation.dev-back"), "{err}");
        assert!(calls.lock().unwrap().is_empty(), "não deveria ter chamado o Forgejo");

        std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn dispatch_issue_rejects_a_step_out_of_the_ladders_range() {
        let root = std::env::temp_dir().join(format!("oreia-dispatch-range-{}", uuid_like()));
        let (api, _calls) = recording_forgejo(vec![]).await;
        let extra = "[escalation.dev-back]\nsteps = [{ executor = \"builtin\", model = \"fake\" }]\n";
        let maker = maker_for_checkout(&api, &root, extra, Arc::new(DummyLlm)).await;

        let err = maker
            .clone()
            .dispatch_issue(DispatchIssueReq {
                repo: "acme/app".into(),
                n: 10,
                agent: "dev-back".into(),
                step: Some(5),
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("fora do intervalo"), "{err}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn dispatch_issue_with_step_skips_straight_to_that_rung() {
        if git_missing() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-dispatch-step-{}", uuid_like()));
        setup_bare_remote(&root);

        let routes =
            vec![("/issues/11", issue_json(11, json!([]), "")), ("/labels", json!([{"id": 3, "name": "agent:back"}]))];
        let (api, _calls) = recording_forgejo(routes).await;
        let extra = "[escalation.dev-back]\nsteps = [\n  { executor = \"builtin\", model = \"fake\" },\n  { executor = \"builtin\", model = \"fake\" },\n]\ntest_cmd = \"true\"\n";
        let maker = maker_for_checkout(&api, &root, extra, Arc::new(crate::maker::tests::ScriptedLlm)).await;

        maker
            .clone()
            .dispatch_issue(DispatchIssueReq {
                repo: "acme/app".into(),
                n: 11,
                agent: "dev-back".into(),
                step: Some(2),
            })
            .await
            .unwrap();

        let path = maker.cfg.paths.data.join("escalation.jsonl");
        let mut found_step_2 = false;
        // 480×25ms = 12s: visto falhar com 200 (5s) numa suíte cheia rodando em paralelo
        // sob carga pesada (o dispatch em si é rápido; o flake era tempo de CPU, não lógica).
        for _ in 0..480 {
            let entries = crate::executor::log::load(&path).await.unwrap_or_default();
            if entries.iter().any(|e| e.step == 2 && e.passed) {
                found_step_2 = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(found_step_2, "deveria ter resolvido a fase no degrau 2 (forçado), sem tentar o degrau 1");

        std::fs::remove_dir_all(&root).ok();
    }
}

// ── review_pr com limite de rodadas (etapa 6) ────────────────────────────────────────

mod review_flow {
    use super::*;

    fn pr_with_agent_branch(n: u64) -> Value {
        json!({
            "number": n, "title": "Somar números", "html_url": format!("http://forgejo.test/acme/app/pulls/{n}"),
            "state": "open", "head": {"ref": "agent/dev-back/issue-7"}, "base": {"ref": "main"}
        })
    }

    async fn write_round_entries(maker: &Maker, repo: &str, n: u64, count: usize) {
        let path = maker.cfg.paths.data.join("mcp.jsonl");
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        let mut buf = String::new();
        for _ in 0..count {
            buf.push_str(&format!(
                "{{\"tool\":\"review_pr\",\"repo\":\"{repo}\",\"issue\":{n},\"result\":\"request_changes\",\"at\":\"2026-01-01T00:00:00Z\"}}\n"
            ));
        }
        tokio::fs::write(&path, buf).await.unwrap();
    }

    #[tokio::test]
    async fn review_pr_comment_only_comments_never_dispatches() {
        let (api, calls) = recording_forgejo(vec![]).await;
        let (maker, _rec) = maker_with_forgejo_and_recorder(&api, agent_back_toml()).await;

        let resp = maker
            .clone()
            .review_pr(ReviewPrReq {
                repo: "acme/app".into(),
                n: 6,
                verdict: "comment".into(),
                comment: "boa ideia".into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.verdict, "comment");
        assert!(resp.round.is_none());

        let calls_now = calls.lock().unwrap().clone();
        assert!(calls_now
            .iter()
            .any(|(m, p, v)| { m == "POST" && p.ends_with("/pulls/6/reviews") && v["event"] == "COMMENT" }));
        assert!(!calls_now.iter().any(|(_, p, _)| p.ends_with("/pulls/6")), "comment não precisa buscar o PR");

        std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn review_pr_request_changes_dispatches_the_branchs_agent() {
        let routes = vec![("/pulls/7", pr_with_agent_branch(7)), ("/reviews", json!({}))];
        let (api, calls) = recording_forgejo(routes).await;
        let (maker, _rec) = maker_with_forgejo_and_recorder(&api, agent_back_toml()).await;

        let resp = maker
            .clone()
            .review_pr(ReviewPrReq {
                repo: "acme/app".into(),
                n: 7,
                verdict: "request_changes".into(),
                comment: "faltou tratar o caso negativo".into(),
            })
            .await
            .unwrap();
        assert_eq!(resp.verdict, "request_changes");
        assert_eq!(resp.round, Some(1));

        let calls_now = calls.lock().unwrap().clone();
        assert!(calls_now
            .iter()
            .any(|(m, p, v)| { m == "POST" && p.ends_with("/pulls/7/reviews") && v["event"] == "REQUEST_CHANGES" }));

        let mut dispatched = false;
        // 480×25ms = 12s: visto falhar com 200 (5s) numa suíte cheia rodando em paralelo
        // sob carga pesada (o dispatch em si é rápido; o flake era tempo de CPU, não lógica).
        for _ in 0..480 {
            let tasks = maker.store.list().await.unwrap_or_default();
            if tasks.iter().any(|t| t.work_item.as_ref().map(|w| (w.issue, w.agent.as_str())) == Some((7, "dev-back")))
            {
                dispatched = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(dispatched, "request_changes deveria ter acionado o dev-back (dono do branch do PR)");

        let audit = tokio::fs::read_to_string(maker.cfg.paths.data.join("mcp.jsonl")).await.unwrap();
        assert!(
            audit.contains("\"tool\":\"review_pr\"") && audit.contains("\"result\":\"request_changes\""),
            "{audit}"
        );

        std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn review_pr_refuses_after_max_rounds_and_warns_the_humans() {
        let routes = vec![("/pulls/8", pr_with_agent_branch(8)), ("/reviews", json!({}))];
        let (api, _calls) = recording_forgejo(routes).await;
        let (maker, rec) = maker_with_forgejo_and_recorder(&api, agent_back_toml()).await;
        write_round_entries(&maker, "acme/app", 8, 3).await; // já no limite padrão (3)

        let err = maker
            .clone()
            .review_pr(ReviewPrReq {
                repo: "acme/app".into(),
                n: 8,
                verdict: "request_changes".into(),
                comment: "mais uma rodada".into(),
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("limite"), "{err}");
        assert!(rec.contains("limite") || rec.contains("atingiu"), "{}", rec.all());

        std::fs::remove_dir_all(maker.cfg.paths.data.parent().unwrap()).ok();
    }
}
