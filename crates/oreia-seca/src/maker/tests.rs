//! Testes de ponta a ponta do orquestrador com um modelo falso e um canal que grava as mensagens.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use oreia_channels::{Channel, Recipient};
use oreia_core::{ChatMessage, Incoming, Source, TaskStatus};
use oreia_llm::{Completion, GenerateOpts, LlmError, LlmProvider, ModelRouter, ModelSpec, Usage};
use tokio::sync::mpsc;

use super::Maker;
use crate::config::Config;

/// Modelo falso que responde conforme o prompt de sistema.
struct ScriptedLlm;

#[async_trait]
impl LlmProvider for ScriptedLlm {
    fn model_name(&self) -> &str {
        "fake"
    }

    async fn generate(&self, messages: &[ChatMessage], opts: &GenerateOpts) -> Result<Completion, LlmError> {
        let system = opts.system.as_deref().unwrap_or("");
        let last = messages.last().map(|m| m.content.as_str()).unwrap_or("");
        let text = if system.contains("task planner") {
            r#"Aqui está: {"phases":[
                {"id":"p1","name":"Write script","instruction":"Create soma.py"},
                {"id":"p2","name":"Write readme","instruction":"Create README.md"}
            ]}"#
            .to_string()
        } else if system.contains("autonomous software developer") {
            if last.starts_with("GOAL:") {
                let file = if last.contains("): Write script") { "soma.py" } else { "README.md" };
                format!(
                    "Criando {file}.\n<action name=\"file_write\"><param name=\"path\">{file}</param><param name=\"content\">ok</param></action>"
                )
            } else {
                "Fase concluída.".to_string()
            }
        } else if system.contains("Resuma a conversa") {
            "resumo curto".to_string()
        } else {
            "Olá! Resposta direta.".to_string()
        };
        Ok(Completion { text, usage: Usage::default(), model: "fake".into() })
    }
}

#[derive(Default)]
pub(crate) struct Recorder {
    pub sent: Mutex<Vec<String>>,
}

impl Recorder {
    pub fn all(&self) -> String {
        self.sent.lock().unwrap().join("\n---\n")
    }
    pub fn contains(&self, s: &str) -> bool {
        self.sent.lock().unwrap().iter().any(|m| m.contains(s))
    }
}

#[async_trait]
impl Channel for Recorder {
    fn name(&self) -> &'static str {
        "telegram"
    }
    fn source(&self) -> Source {
        Source::Telegram
    }
    async fn send(&self, _to: &Recipient, text: &str) -> anyhow::Result<()> {
        self.sent.lock().unwrap().push(text.to_string());
        Ok(())
    }
    async fn run(self: Arc<Self>, _tx: mpsc::Sender<Incoming>) -> anyhow::Result<()> {
        Ok(())
    }
}

pub(crate) async fn test_maker(approval_sources: Vec<&str>) -> (Arc<Maker>, Arc<Recorder>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("oreia-maker-{}", uuid_like()));
    let sources = approval_sources.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ");
    let toml = format!(
        r#"
[paths]
data = "{d}/data"
workspace = "{d}/workspace"
skills = "{d}/skills"

[approval]
sources = [{sources}]
timeout_secs = 5

[models.fake]
base_url = "http://localhost:1/v1"
model = "fake"

[roles]
default = ["fake"]
"#,
        d = dir.display()
    );
    let cfg: Config = toml::from_str(&toml).unwrap();
    cfg.validate().unwrap();

    let mut router = ModelRouter::new(Duration::from_secs(60), None);
    router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
    router.set_role("default", vec!["fake".into()]);

    let rec = Arc::new(Recorder::default());
    let maker = Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], None).await.unwrap();
    (maker, rec, dir)
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst))
}

async fn wait_for(mut cond: impl FnMut() -> bool) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condição não atingida a tempo");
}

fn tg(text: &str) -> Incoming {
    Incoming::new(Source::Telegram, "42", text)
}

#[tokio::test]
async fn direct_reply_keeps_session() {
    let (maker, rec, dir) = test_maker(vec![]).await;
    maker.clone().handle(tg("oi, tudo bem?")).await;
    assert!(rec.contains("Resposta direta"), "{}", rec.all());
    assert_eq!(maker.sessions.get("telegram:42").len(), 2);

    maker.clone().handle(tg("/resumir")).await;
    assert!(rec.contains("Conversa resumida"), "{}", rec.all());
    assert!(maker.sessions.get("telegram:42")[0].content.contains("resumo curto"));

    maker.clone().handle(tg("/limpar")).await;
    assert!(maker.sessions.get("telegram:42").is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn complex_request_plans_and_executes() {
    let (maker, rec, dir) = test_maker(vec![]).await;
    maker.clone().handle(tg("crie um script python completo que soma números")).await;
    wait_for(|| rec.contains("concluída!")).await;

    let tasks = maker.store.list().await.unwrap();
    assert_eq!(tasks.len(), 1);
    let t = &tasks[0];
    assert_eq!(t.status, TaskStatus::Done, "{}", rec.all());
    let wd = dir.join("workspace").join(&t.id);
    assert!(wd.join("soma.py").exists());
    assert!(wd.join("README.md").exists());
    assert!(wd.join("OREIA_SECA_TAREFA.md").exists());
    assert!(rec.contains("Plano (2 fases)"));
    assert!(rec.contains("✓ Write script"));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn rest_task_waits_for_approval() {
    let (maker, rec, dir) = test_maker(vec!["rest"]).await;
    let m = maker.clone();
    let h = tokio::spawn(async move {
        m.handle(Incoming::new(Source::Rest, "cron", "crie um script de backup completo")).await
    });

    wait_for(|| !maker.approvals.list_pending().is_empty()).await;
    let (id, _) = maker.approvals.list_pending().remove(0);
    let t = maker.store.load(&id).await.unwrap().unwrap();
    assert_eq!(t.status, TaskStatus::AwaitingApproval);
    assert!(rec.contains("Aprovação necessária"));

    maker.clone().handle(tg(&format!("/approve {id}"))).await;
    h.await.unwrap();
    wait_for(|| rec.contains("concluída!")).await;
    assert!(rec.contains("aprovada"));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn rejected_task_does_not_run() {
    let (maker, rec, dir) = test_maker(vec!["rest"]).await;
    let m = maker.clone();
    let h = tokio::spawn(async move {
        m.handle(Incoming::new(Source::Rest, "cron", "crie um script de backup completo")).await
    });
    wait_for(|| !maker.approvals.list_pending().is_empty()).await;
    let (id, _) = maker.approvals.list_pending().remove(0);

    maker.clone().handle(tg(&format!("/reject {id}"))).await;
    h.await.unwrap();
    let t = maker.store.load(&id).await.unwrap().unwrap();
    assert_eq!(t.status, TaskStatus::Rejected);
    assert!(!rec.contains("concluída"));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn commands_answer() {
    let (maker, rec, dir) = test_maker(vec![]).await;
    maker.clone().handle(tg("/status")).await;
    assert!(rec.contains("Nenhuma tarefa ativa"));
    assert!(rec.contains("default → fake"));
    maker.clone().handle(tg("/skills")).await;
    assert!(rec.contains("file_write"));
    maker.clone().handle(tg("/prs")).await;
    assert!(rec.contains("Forgejo não configurado"));
    maker.clone().handle(tg("/approve task-nada")).await;
    assert!(rec.contains("Nenhuma aprovação pendente"));
    maker.clone().handle(tg("/xyz")).await;
    assert!(rec.contains("Comando desconhecido"));
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn retry_resumes_failed_phases() {
    let (maker, rec, dir) = test_maker(vec![]).await;
    let mut t = oreia_core::Task::new(Source::Telegram, "42", "crie algo");
    t.phases = vec![oreia_core::Phase {
        id: "p1".into(),
        name: "Write script".into(),
        instruction: "x".into(),
        status: oreia_core::PhaseStatus::Failed,
        summary: None,
        resolved_step: None,
    }];
    t.set_status(TaskStatus::Running);
    maker.store.save(&t).await.unwrap();

    maker.recover_interrupted().await;
    assert!(rec.contains("interrompidas"));

    maker.clone().handle(tg(&format!("/retry {}", t.id))).await;
    wait_for(|| rec.contains("concluída!")).await;
    assert_eq!(maker.store.load(&t.id).await.unwrap().unwrap().status, TaskStatus::Done);
    std::fs::remove_dir_all(dir).ok();
}

// ── Forgejo: issue com label → agente → commit/push → PR ─────────────────────

mod forgejo_flow {
    use super::*;
    use crate::maker::ForgejoCtx;
    use axum::body::Bytes;
    use axum::http::{Method, Uri};
    use axum::Json;
    use oreia_forgejo::webhook::parse_event;
    use serde_json::{json, Value};

    type Calls = Arc<Mutex<Vec<(String, String, Value)>>>;

    /// Forgejo falso: grava as chamadas e responde o mínimo necessário.
    async fn mock_forgejo() -> (String, Calls) {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::POST && path.ends_with("/pulls") {
                    Json(json!({"number": 5, "title": "t", "html_url": "http://forgejo.test/acme/app/pulls/5", "state": "open"}))
                } else if method == Method::GET && path.ends_with("/pulls") {
                    Json(json!([]))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), calls)
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn issue_event(action: &str) -> oreia_forgejo::webhook::Event {
        let body = json!({
            "action": action,
            "issue": {"number": 7, "title": "Somar números", "body": "Criar soma.py", "labels": [{"name": "agent:back"}]},
            "repository": {"name": "app", "full_name": "acme/app", "owner": {"login": "acme"}, "default_branch": "main"},
            "sender": {"login": "dono"}
        });
        parse_event("issues", body.to_string().as_bytes()).unwrap()
    }

    #[tokio::test]
    async fn issue_with_label_becomes_pull_request() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-flow-{}", uuid_like()));
        let remote_dir = root.join("remote/acme");
        let seed = root.join("seed");
        std::fs::create_dir_all(&remote_dir).unwrap();
        std::fs::create_dir_all(&seed).unwrap();
        let bare = remote_dir.join("app.git");
        git(&root, &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
        git(&seed, &["init", "-b", "main"]);
        std::fs::write(seed.join("README.md"), "app").unwrap();
        git(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
        git(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "init"]);
        git(&seed, &["push", bare.to_str().unwrap(), "main"]);

        let (api, calls) = mock_forgejo().await;
        std::env::set_var("OREIA_TEST_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_OWNER_TOKEN", "own");
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
owner_token_env = "OREIA_TEST_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_AGENT_TOKEN"
"#,
            d = root.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();
        assert_eq!(forge.as_ref().unwrap().agent_names(), vec!["dev-back"]);

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle_forgejo(issue_event("opened")).await;
        wait_for(|| rec.contains("PR pronto para sua revisão")).await;

        // PR aberto com o branch do agente e referência à issue
        let calls_now = calls.lock().unwrap().clone();
        let pr =
            calls_now.iter().find(|(m, p, _)| m == "POST" && p == "/api/v1/repos/acme/app/pulls").expect("PR criado");
        assert_eq!(pr.2["head"], "agent/dev-back/issue-7");
        assert_eq!(pr.2["base"], "main");
        assert!(pr.2["body"].as_str().unwrap().contains("Closes #7"));
        let comments = calls_now.iter().filter(|(_, p, _)| p == "/api/v1/repos/acme/app/issues/7/comments").count();
        assert!(comments >= 2, "comentário de início e de PR: {calls_now:?}");

        // Commit chegou ao remoto com a autoria do agente
        let files =
            git(&root, &["--git-dir", bare.to_str().unwrap(), "ls-tree", "--name-only", "agent/dev-back/issue-7"]);
        assert!(files.contains("soma.py"), "{files}");
        let author =
            git(&root, &["--git-dir", bare.to_str().unwrap(), "log", "-1", "--format=%an", "agent/dev-back/issue-7"]);
        assert_eq!(author.trim(), "agent-back");

        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.status, TaskStatus::Done);
        assert_eq!(task.pull_request.as_deref(), Some("http://forgejo.test/acme/app/pulls/5"));
        assert!(!maker.is_active(task.work_item.as_ref().unwrap()));

        // Mudança de label depois da entrega não reabre o trabalho
        maker.clone().handle_forgejo(issue_event("label_updated")).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(maker.store.list().await.unwrap().len(), 1);

        // Eventos de contas de agente são ignorados
        let ev = json!({
            "action": "opened",
            "issue": {"number": 8, "title": "x", "labels": [{"name": "agent:back"}]},
            "repository": {"name": "app", "full_name": "acme/app", "owner": {"login": "acme"}},
            "sender": {"login": "agent-back"}
        });
        maker.clone().handle_forgejo(parse_event("issues", ev.to_string().as_bytes()).unwrap()).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(maker.store.list().await.unwrap().len(), 1);

        std::fs::remove_dir_all(root).ok();
    }
}

// ── Escalada: degraus por fase, verificação por test_cmd, aprovação e custo ──

mod escalation_flow {
    use super::*;
    use crate::maker::ForgejoCtx;
    use axum::body::Bytes;
    use axum::http::{Method, Uri};
    use axum::Json;
    use oreia_core::PhaseStatus;
    use oreia_forgejo::webhook::parse_event;
    use serde_json::{json, Value};

    type Calls = Arc<Mutex<Vec<(String, String, Value)>>>;

    async fn mock_forgejo() -> (String, Calls) {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::POST && path.ends_with("/pulls") {
                    Json(json!({"number": 5, "title": "t", "html_url": "http://forgejo.test/acme/app/pulls/5", "state": "open"}))
                } else if method == Method::GET && path.ends_with("/pulls") {
                    Json(json!([]))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), calls)
    }

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git").args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    fn git_missing() -> bool {
        std::process::Command::new("git").arg("--version").output().is_err()
    }

    fn issue_event() -> oreia_forgejo::webhook::Event {
        let body = json!({
            "action": "opened",
            "issue": {"number": 9, "title": "Escrever out.txt", "body": "Escrever out.txt", "labels": [{"name": "agent:back"}]},
            "repository": {"name": "app", "full_name": "acme/app", "owner": {"login": "acme"}, "default_branch": "main"},
            "sender": {"login": "dono"}
        });
        parse_event("issues", body.to_string().as_bytes()).unwrap()
    }

    /// Planner de uma fase só, sempre a mesma: "Write out.txt with the word good".
    struct OnePhasePlanner;
    #[async_trait]
    impl LlmProvider for OnePhasePlanner {
        fn model_name(&self) -> &str {
            "planner"
        }
        async fn generate(&self, _: &[ChatMessage], _: &GenerateOpts) -> Result<Completion, LlmError> {
            Ok(Completion {
                text: r#"{"phases":[{"id":"p1","name":"Write","instruction":"Write out.txt with the word good"}]}"#
                    .into(),
                usage: Usage::default(),
                model: "planner".into(),
            })
        }
    }

    /// Modelo de um degrau: na primeira rodada escreve `out.txt` (e, opcionalmente, um
    /// arquivo extra), na segunda encerra a fase. Grava os prompts "GOAL:..." recebidos,
    /// um por tentativa, para checar quantas vezes o degrau rodou e o que recebeu.
    struct StepLlm {
        content: &'static str,
        extra_file: Option<&'static str>,
        prompts: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl LlmProvider for StepLlm {
        fn model_name(&self) -> &str {
            "step"
        }
        async fn generate(&self, messages: &[ChatMessage], _: &GenerateOpts) -> Result<Completion, LlmError> {
            let last = messages.last().map(|m| m.content.clone()).unwrap_or_default();
            if last.starts_with("GOAL:") {
                self.prompts.lock().unwrap().push(last);
                let mut text = format!(
                    "Escrevendo.\n<action name=\"file_write\"><param name=\"path\">out.txt</param><param name=\"content\">{}</param></action>",
                    self.content
                );
                if let Some(extra) = self.extra_file {
                    text.push_str(&format!(
                        "<action name=\"file_write\"><param name=\"path\">{extra}</param><param name=\"content\">x</param></action>"
                    ));
                }
                Ok(Completion {
                    text,
                    usage: Usage { prompt_tokens: 1_000_000, completion_tokens: 1_000_000 },
                    model: "step".into(),
                })
            } else {
                Ok(Completion { text: "Fase concluída.".into(), usage: Usage::default(), model: "step".into() })
            }
        }
    }

    async fn setup(
        escalation_toml: &str,
        m1: Arc<dyn LlmProvider>,
        m2: Arc<dyn LlmProvider>,
    ) -> (Arc<Maker>, Arc<Recorder>, PathBuf, PathBuf, String) {
        let suffix = uuid_like();
        let root = std::env::temp_dir().join(format!("oreia-seca-esc-{suffix}"));
        let remote_dir = root.join("remote/acme");
        let seed = root.join("seed");
        std::fs::create_dir_all(&remote_dir).unwrap();
        std::fs::create_dir_all(&seed).unwrap();
        let bare = remote_dir.join("app.git");
        git(&root, &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
        git(&seed, &["init", "-b", "main"]);
        std::fs::write(seed.join("README.md"), "app").unwrap();
        git(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
        git(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "init"]);
        git(&seed, &["push", bare.to_str().unwrap(), "main"]);

        let (api, _calls) = mock_forgejo().await;
        let owner_env = format!("OREIA_SECA_ESC_OWNER_TOKEN_{suffix}");
        let agent_env = format!("OREIA_SECA_ESC_AGENT_TOKEN_{suffix}");
        std::env::set_var(&owner_env, "own");
        std::env::set_var(&agent_env, "tok");

        let toml = format!(
            r#"
[paths]
data = "{d}/data"
workspace = "{d}/workspace"
skills = "{d}/skills"

[approval]
sources = []
timeout_secs = 5

[models.planner]
base_url = "http://localhost:1/v1"
model = "planner"

[models.m1]
base_url = "http://localhost:1/v1"
model = "m1"

[models.m2]
base_url = "http://localhost:1/v1"
model = "m2"

[roles]
default = ["planner"]

[forgejo]
url = "{api}"
git_url = "file://{d}/remote"
owner_token_env = "{owner_env}"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "{agent_env}"

{escalation_toml}
"#,
            d = root.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("planner", ModelSpec::local("http://localhost:1/v1", "planner"), Arc::new(OnePhasePlanner));
        router.add_model("m1", ModelSpec::local("http://localhost:1/v1", "m1"), m1);
        router.add_model("m2", ModelSpec::local("http://localhost:1/v1", "m2"), m2);
        router.set_role("default", vec!["planner".into()]);

        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();
        (maker, rec, root, bare, "agent/dev-back/issue-9".to_string())
    }

    fn no_prompts() -> Arc<Mutex<Vec<String>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    #[tokio::test]
    async fn step_one_pass_does_not_escalate() {
        if git_missing() {
            return;
        }
        let esc = r#"
[escalation.dev-back]
steps = [
  { executor = "builtin", model = "m1" },
  { executor = "builtin", model = "m2" },
]
test_cmd = "grep -q good out.txt"
"#;
        let p1 = no_prompts();
        let p2 = no_prompts();
        let m1 = Arc::new(StepLlm { content: "good", extra_file: None, prompts: p1.clone() });
        let m2 = Arc::new(StepLlm { content: "good", extra_file: None, prompts: p2.clone() });

        let (maker, rec, root, _bare, _branch) = setup(esc, m1, m2).await;
        maker.clone().handle_forgejo(issue_event()).await;
        wait_for(|| rec.contains("PR pronto") || rec.contains("concluída")).await;

        assert_eq!(p1.lock().unwrap().len(), 1, "degrau 1 deveria rodar uma vez");
        assert_eq!(p2.lock().unwrap().len(), 0, "não deveria escalar se o degrau 1 passou");

        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.status, TaskStatus::Done, "{}", rec.all());
        assert_eq!(task.phases[0].resolved_step.as_ref().unwrap().step, 1);

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn step_failure_escalates_with_context_and_discards_leftover() {
        if git_missing() {
            return;
        }
        let esc = r#"
[escalation.dev-back]
steps = [
  { executor = "builtin", model = "m1" },
  { executor = "builtin", model = "m2" },
]
test_cmd = "grep -q good out.txt"
"#;
        let p1 = no_prompts();
        let p2 = no_prompts();
        let m1 = Arc::new(StepLlm { content: "bad", extra_file: Some("leftover.txt"), prompts: p1.clone() });
        let m2 = Arc::new(StepLlm { content: "good", extra_file: None, prompts: p2.clone() });

        let (maker, rec, root, bare, branch) = setup(esc, m1, m2).await;
        maker.clone().handle_forgejo(issue_event()).await;
        wait_for(|| rec.contains("PR pronto") || rec.contains("concluída")).await;

        assert_eq!(p1.lock().unwrap().len(), 1);
        assert_eq!(p2.lock().unwrap().len(), 1);
        assert!(p2.lock().unwrap()[0].contains("PREVIOUS ATTEMPT FAILED"), "{:?}", p2.lock().unwrap());

        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.status, TaskStatus::Done, "{}", rec.all());
        assert_eq!(task.phases[0].resolved_step.as_ref().unwrap().step, 2);

        // leftover.txt da tentativa descartada não deve sobreviver no commit final
        let files = git(&root, &["--git-dir", bare.to_str().unwrap(), "ls-tree", "--name-only", &branch]);
        assert!(files.contains("out.txt"), "{files}");
        assert!(!files.contains("leftover.txt"), "{files}");

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn ask_before_step_reject_fails_the_phase() {
        if git_missing() {
            return;
        }
        let esc = r#"
[escalation.dev-back]
steps = [ { executor = "builtin", model = "m1" } ]
ask_before_step = 1
"#;
        let m1 = Arc::new(StepLlm { content: "good", extra_file: None, prompts: no_prompts() });
        let m2 = Arc::new(StepLlm { content: "good", extra_file: None, prompts: no_prompts() });

        let (maker, rec, root, _bare, _branch) = setup(esc, m1, m2).await;
        let m = maker.clone();
        tokio::spawn(async move { m.handle_forgejo(issue_event()).await });

        wait_for(|| !maker.approvals.list_pending().is_empty()).await;
        let (id, _) = maker.approvals.list_pending().remove(0);
        assert!(maker.approvals.respond(&id, false));

        wait_for(|| rec.contains("concluída")).await;
        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.phases[0].status, PhaseStatus::Failed, "{}", rec.all());

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn ask_before_step_approve_continues() {
        if git_missing() {
            return;
        }
        let esc = r#"
[escalation.dev-back]
steps = [ { executor = "builtin", model = "m1" } ]
ask_before_step = 1
test_cmd = "grep -q good out.txt"
"#;
        let p1 = no_prompts();
        let m1 = Arc::new(StepLlm { content: "good", extra_file: None, prompts: p1.clone() });
        let m2 = Arc::new(StepLlm { content: "good", extra_file: None, prompts: no_prompts() });

        let (maker, rec, root, _bare, _branch) = setup(esc, m1, m2).await;
        let m = maker.clone();
        tokio::spawn(async move { m.handle_forgejo(issue_event()).await });

        wait_for(|| !maker.approvals.list_pending().is_empty()).await;
        let (id, _) = maker.approvals.list_pending().remove(0);
        assert!(maker.approvals.respond(&id, true));

        wait_for(|| rec.contains("PR pronto") || rec.contains("concluída")).await;
        assert_eq!(p1.lock().unwrap().len(), 1);
        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.phases[0].status, PhaseStatus::Done, "{}", rec.all());

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn max_cost_usd_stops_the_ladder() {
        if git_missing() {
            return;
        }
        // 1M + 1M tokens a US$1/Mtok de entrada e saída = US$2 por tentativa: já
        // excede o teto de US$1 depois do degrau 1, então o degrau 2 nunca roda.
        let esc = r#"
[escalation.dev-back]
steps = [
  { executor = "builtin", model = "m1", cost_per_mtok_in = 1.0, cost_per_mtok_out = 1.0 },
  { executor = "builtin", model = "m2", cost_per_mtok_in = 1.0, cost_per_mtok_out = 1.0 },
]
test_cmd = "grep -q good out.txt"
max_cost_usd = 1.0
"#;
        let p1 = no_prompts();
        let p2 = no_prompts();
        let m1 = Arc::new(StepLlm { content: "bad", extra_file: None, prompts: p1.clone() });
        let m2 = Arc::new(StepLlm { content: "good", extra_file: None, prompts: p2.clone() });

        let (maker, rec, root, _bare, _branch) = setup(esc, m1, m2).await;
        maker.clone().handle_forgejo(issue_event()).await;
        wait_for(|| rec.contains("concluída")).await;

        assert_eq!(p1.lock().unwrap().len(), 1, "degrau 1 deveria ter rodado");
        assert_eq!(p2.lock().unwrap().len(), 0, "degrau 2 não deveria rodar: custo já passou do teto");
        assert!(rec.contains("teto de custo"), "{}", rec.all());

        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.phases[0].status, PhaseStatus::Failed, "{}", rec.all());

        std::fs::remove_dir_all(root).ok();
    }
}
