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

    fn pr_opened_event(head_branch: &str, sender: &str) -> oreia_forgejo::webhook::Event {
        let body = json!({
            "action": "opened",
            "number": 6,
            "pull_request": {
                "number": 6,
                "title": "Somar números (#7)",
                "html_url": "http://forgejo.test/acme/app/pulls/6",
                "state": "open",
                "user": {"login": sender},
                "head": {"ref": head_branch},
                "merged": false
            },
            "repository": {"name": "app", "full_name": "acme/app", "owner": {"login": "acme"}, "default_branch": "main"},
            "sender": {"login": sender}
        });
        parse_event("pull_request", body.to_string().as_bytes()).unwrap()
    }

    /// Quando dev-back abre um PR, o reviewer deve entrar automaticamente, analisar a
    /// branch JÁ existente (não criar uma nova) e só comentar — nunca aprovar, reprovar
    /// ou fazer merge sozinho, já que essa decisão é sempre do humano.
    #[tokio::test]
    async fn pr_opened_by_agent_triggers_reviewer_that_only_comments() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("oreia-review-{}", uuid_like()));
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
        // dev-back já tinha aberto o PR: a branch já existe no remoto com o trabalho dele.
        git(&seed, &["checkout", "-b", "agent/dev-back/issue-7"]);
        std::fs::write(seed.join("soma.py"), "def soma(): pass").unwrap();
        git(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "add", "."]);
        git(&seed, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "feat: soma"]);
        git(&seed, &["push", bare.to_str().unwrap(), "agent/dev-back/issue-7"]);

        let (api, calls) = mock_forgejo().await;
        std::env::set_var("OREIA_TEST_REVIEW_BACK_TOKEN", "tok-back");
        std::env::set_var("OREIA_TEST_REVIEW_REVIEWER_TOKEN", "tok-review");
        std::env::set_var("OREIA_TEST_REVIEW_OWNER_TOKEN", "own");
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
owner_token_env = "OREIA_TEST_REVIEW_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_REVIEW_BACK_TOKEN"

[[agents]]
name = "reviewer"
label = "agent:review"
user = "agent-reviewer"
token_env = "OREIA_TEST_REVIEW_REVIEWER_TOKEN"
"#,
            d = root.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();
        assert_eq!(forge.as_ref().unwrap().agent_names().len(), 2);

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        // PR aberto pelo dev-back (sender = usuário do agente): deve acionar o reviewer.
        maker.clone().handle_forgejo(pr_opened_event("agent/dev-back/issue-7", "agent-back")).await;
        wait_for(|| rec.contains("Comentário de revisão publicado")).await;

        let calls_now = calls.lock().unwrap().clone();

        // Nunca aprova, reprova nem faz merge sozinho.
        assert!(
            !calls_now.iter().any(|(_, p, _)| p.contains("/reviews") || p.contains("/merge")),
            "reviewer não deve chamar /reviews nem /merge: {calls_now:?}"
        );
        // Não abre um PR novo: é revisão de um PR que já existe.
        assert!(
            !calls_now.iter().any(|(m, p, _)| m == "POST" && p.ends_with("/pulls")),
            "reviewer não deve abrir PR: {calls_now:?}"
        );
        // Comentou no PR (mesmo endpoint de comentário de issue) deixando claro que não é veredito.
        let comment = calls_now
            .iter()
            .find(|(m, p, v)| {
                m == "POST"
                    && p == "/api/v1/repos/acme/app/issues/6/comments"
                    && v["body"].as_str().unwrap_or_default().contains("Revisão automática")
            })
            .expect("comentário de revisão");
        assert!(comment.2["body"].as_str().unwrap().contains("não uma aprovação ou reprovação"), "{:?}", comment.2);

        // Analisou a branch que já existia, não criou uma nova.
        let branches = git(&root, &["--git-dir", bare.to_str().unwrap(), "branch", "--list", "agent/reviewer/*"]);
        assert!(branches.trim().is_empty(), "não deveria existir branch própria do reviewer: {branches}");

        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.work_item.as_ref().unwrap().mode, oreia_core::WorkMode::Review);
        assert_eq!(task.work_item.as_ref().unwrap().issue, 6);

        std::fs::remove_dir_all(root).ok();
    }

    /// `/issue` não toca em git: só lê as labels do repo (pra achar o ID da label do
    /// agente — a API do Forgejo não aceita nome ali) e abre a issue com esse ID.
    #[tokio::test]
    async fn issue_command_opens_an_issue_with_the_agent_label() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/labels") {
                    Json(json!([{"id": 3, "name": "agent:back"}, {"id": 4, "name": "bug"}]))
                } else if method == Method::POST && path.ends_with("/issues") {
                    Json(json!({
                        "number": 9, "title": "Corrigir o botão de login", "body": "",
                        "labels": [{"id": 3, "name": "agent:back"}],
                        "html_url": "http://forgejo.test/acme/app/issues/9"
                    }))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_ISSUECMD_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_ISSUECMD_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-issuecmd-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_ISSUECMD_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_ISSUECMD_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/issue acme/app dev-back Corrigir o botão de login")).await;
        assert!(rec.contains("Issue criada"), "{}", rec.all());
        assert!(rec.contains("acme/app#9"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let created = calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues")).expect("issue criada");
        assert_eq!(created.2["title"], "Corrigir o botão de login");
        assert_eq!(created.2["labels"], json!([3]));

        std::fs::remove_dir_all(dir).ok();
    }

    /// `--base <branch>` no uso direto numa linha só vira o campo "Ramo" da issue
    /// (mesmo efeito de preencher esse campo na UI do Forgejo) — e é removido do
    /// título antes de juntar o resto das palavras.
    #[tokio::test]
    async fn issue_command_with_explicit_base_branch_sets_the_ref_field() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/labels") {
                    Json(json!([{"id": 3, "name": "agent:back"}]))
                } else if method == Method::POST && path.ends_with("/issues") {
                    Json(json!({
                        "number": 15, "title": "Corrigir login", "body": "",
                        "labels": [{"id": 3, "name": "agent:back"}],
                        "html_url": "http://forgejo.test/acme/app/issues/15"
                    }))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_ISSUEBASE_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_ISSUEBASE_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-issuebase-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_ISSUEBASE_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_ISSUEBASE_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/issue acme/app dev-back --base feature-x Corrigir login")).await;
        assert!(rec.contains("branch base: feature-x"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let created = calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues")).expect("issue criada");
        assert_eq!(created.2["title"], "Corrigir login");
        assert_eq!(created.2["ref"], "feature-x");

        std::fs::remove_dir_all(dir).ok();
    }

    /// Sem a label do agente já existindo no repo, não cria a issue "sem label" —
    /// a API do Forgejo só aceita ID, então sem achar o nome a gente teria que
    /// mandar algo errado ou nenhuma label; melhor avisar e não criar nada.
    #[tokio::test]
    async fn issue_command_warns_when_the_agent_label_does_not_exist_yet() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/labels") {
                    Json(json!([{"id": 4, "name": "bug"}]))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_ISSUECMD2_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_ISSUECMD2_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-issuecmd2-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_ISSUECMD2_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_ISSUECMD2_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/issue acme/app dev-back Corrigir o botão de login")).await;
        assert!(rec.contains("não existe em acme/app"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        assert!(!calls_now.iter().any(|(m, p, _)| m == "POST" && p.ends_with("/issues")), "{calls_now:?}");

        std::fs::remove_dir_all(dir).ok();
    }

    /// Sem agente, `/issue` cria a issue sem nenhuma label — fica pra você analisar
    /// antes e usar `/assign` depois, em vez de acionar alguém na hora.
    #[tokio::test]
    async fn issue_command_without_agent_creates_an_unlabeled_issue() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::POST && path.ends_with("/issues") {
                    Json(json!({
                        "number": 11, "title": "Investigar lentidão no checkout", "body": "", "labels": [],
                        "html_url": "http://forgejo.test/acme/app/issues/11"
                    }))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_ISSUECMD3_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_ISSUECMD3_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-issuecmd3-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_ISSUECMD3_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_ISSUECMD3_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        // Sem o nome de um agente no segundo token: tudo depois do repo é título.
        maker.clone().handle(tg("/issue acme/app Investigar lentidão no checkout")).await;
        assert!(rec.contains("Issue criada"), "{}", rec.all());
        assert!(rec.contains("sem agente"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let created = calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues")).expect("issue criada");
        assert_eq!(created.2["title"], "Investigar lentidão no checkout");
        assert_eq!(created.2["labels"], json!([]));
        // Nunca precisou ler as labels do repo: sem agente, não há ID pra procurar.
        assert!(!calls_now.iter().any(|(_, p, _)| p.ends_with("/labels")), "{calls_now:?}");

        std::fs::remove_dir_all(dir).ok();
    }

    /// `/assign` adiciona a label do agente numa issue que já existe (sem passar
    /// por `/issue`) — mesmo endpoint que o Forgejo usa quando você arrasta a label
    /// na UI, então aciona o agente do mesmo jeito que o fluxo manual já fazia.
    #[tokio::test]
    async fn assign_command_adds_the_agent_label_to_an_existing_issue() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/labels") {
                    Json(json!([{"id": 3, "name": "agent:back"}]))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_ASSIGNCMD_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_ASSIGNCMD_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-assigncmd-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_ASSIGNCMD_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_ASSIGNCMD_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/assign acme/app 11 dev-back")).await;
        assert!(rec.contains("adicionada à issue acme/app#11"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let added = calls_now
            .iter()
            .find(|(m, p, _)| m == "POST" && p == "/api/v1/repos/acme/app/issues/11/labels")
            .expect("label adicionada");
        assert_eq!(added.2["labels"], json!([3]));

        std::fs::remove_dir_all(dir).ok();
    }

    // ── Assistentes de botão: /issue, /assign e /prs em passos ───────────────

    /// Sem agente nem título, `/issue` pergunta o agente (botão/texto, aqui caem
    /// como lista numerada porque o `Recorder` de teste não tem botão de verdade),
    /// depois o título por mensagem livre — fiel ao que o botão realmente manda.
    #[tokio::test]
    async fn issue_wizard_prompts_for_agent_then_title_and_creates_the_issue() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/labels") {
                    Json(json!([{"id": 3, "name": "agent:back"}]))
                } else if method == Method::POST && path.ends_with("/issues") {
                    Json(json!({
                        "number": 20, "title": "Corrigir bug no carrinho", "body": "",
                        "labels": [{"id": 3, "name": "agent:back"}],
                        "html_url": "http://forgejo.test/acme/app/issues/20"
                    }))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_WIZ1_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_WIZ1_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-wiz1-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_WIZ1_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_WIZ1_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        // Passo 1: sem nada, pergunta o agente (repo único é resolvido sozinho).
        maker.clone().handle(tg("/issue")).await;
        assert!(rec.contains("Qual agente?"), "{}", rec.all());
        assert!(rec.contains("/issue acme/app dev-back"), "{}", rec.all());
        assert!(rec.contains("/issue acme/app --none"), "{}", rec.all());

        // Passo 2: "toca" no botão do agente (texto idêntico ao callback_data) → pede o título.
        maker.clone().handle(tg("/issue acme/app dev-back")).await;
        assert!(rec.contains("Qual o título da issue? (dev-back)"), "{}", rec.all());

        // Passo 3: título por mensagem livre → pede a descrição.
        maker.clone().handle(tg("Corrigir bug no carrinho")).await;
        assert!(rec.contains("Descreva a issue"), "{}", rec.all());

        // Passo 4: descrição por mensagem livre → pede a branch base.
        maker.clone().handle(tg("O total não soma o frete quando o cupom é aplicado.")).await;
        assert!(rec.contains("Branch base?"), "{}", rec.all());

        // Passo 5: branch base por mensagem livre → só agora cria a issue.
        maker.clone().handle(tg("funcionalidade-carrinho")).await;
        assert!(rec.contains("Issue criada"), "{}", rec.all());
        assert!(rec.contains("acme/app#20"), "{}", rec.all());
        assert!(rec.contains("branch base: funcionalidade-carrinho"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let created = calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues")).expect("issue criada");
        assert_eq!(created.2["title"], "Corrigir bug no carrinho");
        assert_eq!(created.2["body"], "O total não soma o frete quando o cupom é aplicado.");
        assert_eq!(created.2["labels"], json!([3]));
        assert_eq!(created.2["ref"], "funcionalidade-carrinho");

        std::fs::remove_dir_all(dir).ok();
    }

    /// Quando a API de branches responde, o passo de branch base vira botões (um
    /// por branch, mais "Usar padrão") em vez do pedido por texto livre — e tocar
    /// num botão (= mandar o nome da branch como texto puro, o mesmo que o
    /// callback_data do Telegram manda) completa o assistente normalmente.
    #[tokio::test]
    async fn issue_wizard_lists_branches_as_buttons_for_the_base_branch_step() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/labels") {
                    Json(json!([{"id": 3, "name": "agent:back"}]))
                } else if method == Method::GET && path.ends_with("/branches") {
                    Json(json!([{"name": "main"}, {"name": "feature-x"}]))
                } else if method == Method::POST && path.ends_with("/issues") {
                    Json(json!({
                        "number": 22, "title": "Corrigir bug", "body": "",
                        "labels": [{"id": 3, "name": "agent:back"}],
                        "html_url": "http://forgejo.test/acme/app/issues/22"
                    }))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_WIZ7_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_WIZ7_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-wiz7-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_WIZ7_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_WIZ7_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/issue acme/app dev-back")).await;
        assert!(rec.contains("Qual o título da issue?"), "{}", rec.all());

        maker.clone().handle(tg("Corrigir bug")).await;
        assert!(rec.contains("Descreva a issue"), "{}", rec.all());

        maker.clone().handle(tg("-")).await;
        assert!(rec.contains("Qual branch base?"), "{}", rec.all());
        assert!(rec.contains("📌 Usar padrão → -"), "{}", rec.all());
        assert!(rec.contains("🌿 main → main"), "{}", rec.all());
        assert!(rec.contains("🌿 feature-x → feature-x"), "{}", rec.all());

        // "Toca" no botão "feature-x" — manda o texto puro, igual ao callback_data.
        maker.clone().handle(tg("feature-x")).await;
        assert!(rec.contains("Issue criada"), "{}", rec.all());
        assert!(rec.contains("branch base: feature-x"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let created = calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues")).expect("issue criada");
        assert_eq!(created.2["ref"], "feature-x");

        std::fs::remove_dir_all(dir).ok();
    }

    /// Com mais de um repo configurado, `/issue` sem argumentos pergunta o repo
    /// primeiro — um botão por `[forgejo].repos`, cada um completando o comando.
    #[tokio::test]
    async fn issue_wizard_with_multiple_repos_asks_for_repo_first() {
        let (api, _calls) = mock_forgejo().await;

        std::env::set_var("OREIA_TEST_WIZ2_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_WIZ2_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-wiz2-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_WIZ2_OWNER_TOKEN"
repos = ["acme/app", "acme/site"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_WIZ2_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/issue")).await;
        assert!(rec.contains("Qual repositório?"), "{}", rec.all());
        assert!(rec.contains("/issue acme/app"), "{}", rec.all());
        assert!(rec.contains("/issue acme/site"), "{}", rec.all());

        std::fs::remove_dir_all(dir).ok();
    }

    /// O botão "🚫 Sem agente" (`--none`) pula o passo de agente e vai direto pro
    /// título; a issue sai sem nenhuma label, como no uso direto numa linha só.
    #[tokio::test]
    async fn issue_wizard_none_agent_creates_an_unlabeled_issue() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::POST && path.ends_with("/issues") {
                    Json(json!({
                        "number": 21, "title": "Investigar lentidão", "body": "", "labels": [],
                        "html_url": "http://forgejo.test/acme/app/issues/21"
                    }))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_WIZ3_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_WIZ3_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-wiz3-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_WIZ3_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_WIZ3_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/issue acme/app --none")).await;
        assert!(rec.contains("Qual o título da issue? (sem agente)"), "{}", rec.all());

        maker.clone().handle(tg("Investigar lentidão")).await;
        assert!(rec.contains("Descreva a issue"), "{}", rec.all());

        // "-" pula a descrição — a issue sai com o corpo vazio, igual ao uso direto.
        maker.clone().handle(tg("-")).await;
        assert!(rec.contains("Branch base?"), "{}", rec.all());

        // "-" de novo pula a branch base também — fica com a padrão.
        maker.clone().handle(tg("-")).await;
        assert!(rec.contains("Issue criada"), "{}", rec.all());
        assert!(rec.contains("sem agente"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let created = calls_now.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/issues")).expect("issue criada");
        assert!(created.2.get("ref").is_none(), "{:?}", created.2);
        assert_eq!(created.2["labels"], json!([]));
        assert!(!calls_now.iter().any(|(_, p, _)| p.ends_with("/labels")), "{calls_now:?}");

        std::fs::remove_dir_all(dir).ok();
    }

    /// `/assign` sem número nem agente pergunta os dois em passos: número por
    /// texto livre (não dá pra virar botão), agente por botão — e o botão já
    /// manda o comando completo, então o passo final reaproveita o caminho direto.
    #[tokio::test]
    async fn assign_wizard_asks_number_then_agent_and_adds_label() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/labels") {
                    Json(json!([{"id": 5, "name": "agent:back"}]))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_WIZ4_AGENT_TOKEN", "tok");
        std::env::set_var("OREIA_TEST_WIZ4_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-wiz4-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_WIZ4_OWNER_TOKEN"
repos = ["acme/app"]

[[agents]]
name = "dev-back"
label = "agent:back"
user = "agent-back"
token_env = "OREIA_TEST_WIZ4_AGENT_TOKEN"
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/assign")).await;
        assert!(rec.contains("Qual o número da issue?"), "{}", rec.all());

        maker.clone().handle(tg("11")).await;
        assert!(rec.contains("Qual agente pra issue #11?"), "{}", rec.all());
        assert!(rec.contains("/assign acme/app 11 dev-back"), "{}", rec.all());

        maker.clone().handle(tg("/assign acme/app 11 dev-back")).await;
        assert!(rec.contains("adicionada à issue acme/app#11"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let added = calls_now
            .iter()
            .find(|(m, p, _)| m == "POST" && p == "/api/v1/repos/acme/app/issues/11/labels")
            .expect("label adicionada");
        assert_eq!(added.2["labels"], json!([5]));

        std::fs::remove_dir_all(dir).ok();
    }

    /// `/prs` virou um cartão por PR (em vez de uma lista só) com um botão de
    /// Merge e um de "pedir ajuste" já apontando pro comando certo.
    #[tokio::test]
    async fn prs_lists_each_open_pr_as_a_card_with_action_buttons() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/pulls") {
                    Json(json!([{
                        "number": 7, "title": "Ajustar cor do botão", "html_url": "http://forgejo.test/acme/app/pulls/7",
                        "state": "open", "user": {"login": "agent-back"}, "head": {"ref": "agent/dev-back/issue-7"}
                    }]))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_WIZ5_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-wiz5-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_WIZ5_OWNER_TOKEN"
repos = ["acme/app"]
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/prs")).await;
        assert!(rec.contains("🔀 acme/app#7 Ajustar cor do botão"), "{}", rec.all());
        assert!(rec.contains("/merge acme/app 7"), "{}", rec.all());
        assert!(rec.contains("/changes-ask acme/app 7"), "{}", rec.all());

        let _ = calls;
        std::fs::remove_dir_all(dir).ok();
    }

    /// O botão "📝 Pedir ajuste" do /prs pede o texto por mensagem livre em vez de
    /// exigi-lo já no comando; quando a branch não é de um agente ativo, só
    /// registra o pedido (sem acionar ninguém) — mesmo comportamento do /changes direto.
    #[tokio::test]
    async fn changes_ask_then_free_text_registers_the_request() {
        let calls: Calls = Arc::default();
        let c = calls.clone();
        let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
            let c = c.clone();
            async move {
                let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = uri.path().to_string();
                c.lock().unwrap().push((method.to_string(), path.clone(), v));
                if method == Method::GET && path.ends_with("/pulls/9") {
                    Json(json!({
                        "number": 9, "title": "Ajustar texto", "html_url": "http://forgejo.test/acme/app/pulls/9",
                        "state": "open", "head": {"ref": "feature/x"}, "base": {"ref": "main"}
                    }))
                } else {
                    Json(json!({}))
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = format!("http://{addr}");

        std::env::set_var("OREIA_TEST_WIZ6_OWNER_TOKEN", "own");
        let dir = std::env::temp_dir().join(format!("oreia-wiz6-{}", uuid_like()));
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
owner_token_env = "OREIA_TEST_WIZ6_OWNER_TOKEN"
repos = ["acme/app"]
"#,
            d = dir.display()
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        cfg.validate().unwrap();
        let forge = ForgejoCtx::build(&cfg).unwrap();

        let mut router = ModelRouter::new(Duration::from_secs(60), None);
        router.add_model("fake", ModelSpec::local("http://localhost:1/v1", "fake"), Arc::new(ScriptedLlm));
        router.set_role("default", vec!["fake".into()]);
        let rec = Arc::new(Recorder::default());
        let maker =
            Maker::new(Arc::new(cfg), Arc::new(router), vec![rec.clone() as Arc<dyn Channel>], forge).await.unwrap();

        maker.clone().handle(tg("/changes-ask acme/app 9")).await;
        assert!(rec.contains("O que precisa mudar no PR acme/app#9?"), "{}", rec.all());

        maker.clone().handle(tg("Ajustar o texto do botão")).await;
        assert!(rec.contains("não é um branch de agente ativo"), "{}", rec.all());

        let calls_now = calls.lock().unwrap().clone();
        let review = calls_now
            .iter()
            .find(|(m, p, _)| m == "POST" && p == "/api/v1/repos/acme/app/pulls/9/reviews")
            .expect("review registrada");
        assert_eq!(review.2["event"], "REQUEST_CHANGES");
        assert_eq!(review.2["body"], "Ajustar o texto do botão");

        std::fs::remove_dir_all(dir).ok();
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
        planner: Arc<dyn LlmProvider>,
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
        router.add_model("planner", ModelSpec::local("http://localhost:1/v1", "planner"), planner);
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

        let (maker, rec, root, _bare, _branch) = setup(esc, Arc::new(OnePhasePlanner), m1, m2).await;
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

        let (maker, rec, root, bare, branch) = setup(esc, Arc::new(OnePhasePlanner), m1, m2).await;
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

        let (maker, rec, root, _bare, _branch) = setup(esc, Arc::new(OnePhasePlanner), m1, m2).await;
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

        let (maker, rec, root, _bare, _branch) = setup(esc, Arc::new(OnePhasePlanner), m1, m2).await;
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

        let (maker, rec, root, _bare, _branch) = setup(esc, Arc::new(OnePhasePlanner), m1, m2).await;
        maker.clone().handle_forgejo(issue_event()).await;
        wait_for(|| rec.contains("concluída")).await;

        assert_eq!(p1.lock().unwrap().len(), 1, "degrau 1 deveria ter rodado");
        assert_eq!(p2.lock().unwrap().len(), 0, "degrau 2 não deveria rodar: custo já passou do teto");
        assert!(rec.contains("teto de custo"), "{}", rec.all());

        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.phases[0].status, PhaseStatus::Failed, "{}", rec.all());

        std::fs::remove_dir_all(root).ok();
    }

    /// Fase sem nenhum verbo de mudança no NOME, mas cuja instrução longa diz
    /// "do NOT create files" — a palavra "create" aparece aí só por causa da
    /// negação. Reproduz um falso positivo real: a verificação via instrução
    /// completa reprovava essa fase mesmo sem nada para mudar.
    struct InspectOnlyPlanner;
    #[async_trait]
    impl LlmProvider for InspectOnlyPlanner {
        fn model_name(&self) -> &str {
            "planner"
        }
        async fn generate(&self, _: &[ChatMessage], _: &GenerateOpts) -> Result<Completion, LlmError> {
            Ok(Completion {
                text: r#"{"phases":[{"id":"p1","name":"Inspect","instruction":"Do NOT create any files in this phase. Just report what you see."}]}"#.into(),
                usage: Usage::default(),
                model: "planner".into(),
            })
        }
    }

    /// Não escreve nada — só responde em texto, como um agente que de fato só inspecionou.
    struct NoopLlm;
    #[async_trait]
    impl LlmProvider for NoopLlm {
        fn model_name(&self) -> &str {
            "noop"
        }
        async fn generate(&self, _: &[ChatMessage], _: &GenerateOpts) -> Result<Completion, LlmError> {
            Ok(Completion {
                text: "Inspecionei o projeto, nada para reportar.".into(),
                usage: Usage::default(),
                model: "noop".into(),
            })
        }
    }

    #[tokio::test]
    async fn read_only_phase_does_not_fail_just_for_not_changing_files() {
        if git_missing() {
            return;
        }
        let esc = r#"
[escalation.dev-back]
steps = [ { executor = "builtin", model = "m1" } ]
"#;
        let m1 = Arc::new(NoopLlm);
        let m2 = Arc::new(NoopLlm);

        let (maker, rec, root, _bare, _branch) = setup(esc, Arc::new(InspectOnlyPlanner), m1, m2).await;
        maker.clone().handle_forgejo(issue_event()).await;
        wait_for(|| rec.contains("PR pronto") || rec.contains("concluída")).await;

        let task = maker.store.list().await.unwrap().remove(0);
        assert_eq!(task.status, TaskStatus::Done, "{}", rec.all());
        assert_eq!(
            task.phases[0].status,
            PhaseStatus::Done,
            "fase só de inspeção não devia falhar por não mudar arquivo: {}",
            rec.all()
        );

        std::fs::remove_dir_all(root).ok();
    }
}
