//! Executor externo: roda uma CLI de terceiros (Aider, OpenCode, Goose, Codex,
//! ou qualquer outra) a partir de um template de comando.
//!
//! A instrução da fase vai num arquivo temporário (`{message_file}`), nunca
//! interpolada no shell. A chave do modelo só entra no comando de fato
//! executado — nunca na string de log (`display`), para não vazar em
//! mensagens nem no `data/escalation.jsonl`. Nenhuma outra variável do `.env`
//! chega ao processo: `sandboxed_shell` já roda com ambiente limpo.

use std::path::PathBuf;

use async_trait::async_trait;
use oreia_core::actions::builtin::{run_command, sandboxed_shell};
use oreia_core::actions::ActionContext;
use serde_json::json;

use super::{ModelSelector, PhaseExecutor, PhaseOutcome, PhaseRequest};
use crate::config::env;

/// Presets embutidos. Testados de verdade (não só por leitura do `--help`)
/// contra OpenRouter com deepseek e com google/gemini-2.5-flash-lite em
/// 2026-10-25 — mas as flags mudam entre versões das ferramentas, confira a
/// documentação atual de cada uma antes de depender destes comandos em
/// produção. Detalhes de cada um em docs/executores.md.
///
/// Nenhum dos quatro aceita `--api-base`/`--api-key` do jeito que a primeira
/// versão deste preset supunha (só o aider aceita; os outros três não têm
/// essas flags e preferem variáveis de ambiente ou um provedor nomeado) — por
/// isso cada um resolve isso de um jeito diferente abaixo.
pub const BUILTIN_PRESETS: &[(&str, &str)] = &[
    (
        "aider",
        "aider --model openai/{model} --openai-api-base {base_url} --openai-api-key {api_key} \
         --no-auto-commits --yes-always --no-show-model-warnings --message-file {message_file}",
    ),
    // Sem --api-base/--api-key (não existem), e os provedores conhecidos do
    // opencode (ex. "openrouter") validam o modelo contra o catálogo PRÓPRIO
    // dele (models.dev, cacheado) — um modelo que o provedor serve de verdade
    // mas não está nesse catálogo falha com "Model not found" mesmo a chamada
    // estando certa (confirmado com google/gemini-2.5-flash-lite). A saída:
    // `run()` escreve um `opencode.jsonc` (em {config_dir}, isolado do projeto
    // via XDG_CONFIG_HOME) definindo um provedor "oreiaseca" ad-hoc via
    // @ai-sdk/openai-compatible, com o {base_url}/{model} exatos — não passa
    // pelo catálogo, então aceita qualquer modelo que o endpoint realmente sirva.
    ("opencode", "XDG_CONFIG_HOME={config_dir} opencode run -m oreiaseca/{model} --auto < {message_file}"),
    // --provider/--model existem, mas não há flag de base_url: só a variável
    // OPENAI_HOST, que precisa SEM o "/v1" final (a CLI completa o resto do
    // caminho sozinha) — por isso o `${BASE_URL%/v1}` em vez de {base_url} direto.
    (
        "goose",
        "BASE_URL={base_url}; GOOSE_PROVIDER=openai OPENAI_API_KEY={api_key} \
         OPENAI_HOST=\"${BASE_URL%/v1}\" GOOSE_MODEL={model} \
         goose run --instructions {message_file} --no-session -q",
    ),
    // Sem --base-url/--api-key (não existem) e OPENAI_BASE_URL é ignorado (vai
    // sempre pra api.openai.com): o jeito que funciona é definir um provedor
    // ad-hoc via -c (suportado pela própria CLI), que respeita o base_url e lê
    // a chave da variável de ambiente nomeada em `env_key`.
    (
        "codex",
        "OREIA_SECA_EXT_API_KEY={api_key} codex exec \
         -c model_providers.custom.name=custom -c model_providers.custom.base_url={base_url} \
         -c model_providers.custom.env_key=OREIA_SECA_EXT_API_KEY -c model_provider=custom \
         -m {model} --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox < {message_file}",
    ),
];

pub fn builtin_preset(name: &str) -> Option<&'static str> {
    BUILTIN_PRESETS.iter().find(|(n, _)| *n == name).map(|(_, cmd)| *cmd)
}

pub struct ExternalExecutor {
    name: String,
    template: String,
}

impl ExternalExecutor {
    pub fn new(name: impl Into<String>, template: impl Into<String>) -> Self {
        Self { name: name.into(), template: template.into() }
    }
}

fn render(
    template: &str,
    model: &str,
    base_url: &str,
    api_key: &str,
    message_file: &str,
    test_cmd: &str,
    config_dir: &str,
) -> String {
    template
        .replace("{model}", model)
        .replace("{base_url}", base_url)
        .replace("{api_key}", api_key)
        .replace("{message_file}", message_file)
        .replace("{test_cmd}", test_cmd)
        .replace("{config_dir}", config_dir)
}

/// Só pro preset do opencode (ver `BUILTIN_PRESETS`): escreve um provedor
/// ad-hoc em `$XDG_CONFIG_HOME/opencode/opencode.jsonc` apontando pro
/// `base_url`/modelo exatos, porque os provedores que o opencode já conhece
/// (ex. "openrouter") validam o modelo contra o catálogo próprio dele, não
/// contra a API de verdade. `XDG_CONFIG_HOME` fica no diretório de temp do
/// sistema (`dir`, fora do repositório — ver `run()`), nunca no diretório do
/// projeto: por um lado para não arriscar sobrescrever um `opencode.jsonc`
/// legítimo que o repositório já tenha, por outro porque este arquivo leva a
/// chave da API em texto puro — dentro do repo ela acabaria comitada e indo
/// pro PR.
async fn write_opencode_config(
    dir: &std::path::Path,
    base_url: &str,
    model: &str,
    api_key: &str,
) -> anyhow::Result<PathBuf> {
    let config_dir = dir.join("opencode-config");
    tokio::fs::create_dir_all(config_dir.join("opencode")).await?;
    let config = json!({
        "$schema": "https://opencode.ai/config.json",
        "provider": {
            "oreiaseca": {
                "npm": "@ai-sdk/openai-compatible",
                "name": "OREIA-SECA",
                "options": { "baseURL": base_url, "apiKey": api_key },
                "models": { model: {} }
            }
        }
    });
    tokio::fs::write(config_dir.join("opencode/opencode.jsonc"), config.to_string()).await?;
    Ok(tokio::fs::canonicalize(&config_dir).await?)
}

/// Mensagem enviada à CLI externa — mesmo espírito do `prompts::executor` do
/// builtin (papel, contexto do repositório, regras), porque a ferramenta de
/// terceiros não tem acesso a esse prompt de sistema, só a este arquivo.
fn message_body(req: &PhaseRequest) -> String {
    let mut s = String::new();
    if let Some(agent) = &req.agent_prompt {
        s.push_str("YOUR ROLE:\n");
        s.push_str(agent);
        s.push_str("\n\n");
    }
    if let Some(w) = &req.work_item {
        s.push_str(&format!(
            "REPOSITORY CONTEXT:\n\
             - Repository {} is already cloned in the current directory.\n\
             - You are on branch `{}`, created from `{}`, for issue #{}.\n\
             - Do NOT run git commit, git push, or open a pull request: the orchestrator does \
             that after this phase, once you stop.\n\
             - Keep changes focused on the issue.\n\n",
            w.repo_full_name(),
            w.checkout_branch(),
            req.base_branch,
            w.issue
        ));
    }
    s.push_str(
        "RULES:\n\
         1. Execute ONLY the current phase below — not the whole goal.\n\
         2. Write complete, working code. No placeholders like \"// implement here\".\n\
         3. Do not run build, lint or test commands unless the phase instruction below explicitly \
         asks for it.\n\
         4. When the phase is complete, just stop — do not commit, push, or open a pull request.\n\n",
    );
    s.push_str(&format!(
        "GOAL: {}\n\nCOMPLETED PHASES:\n{}\n\nCURRENT PHASE ({}/{}): {}\n{}",
        req.goal, req.completed_summary, req.phase_index, req.phase_total, req.phase_name, req.phase_instruction
    ));
    if let Some(f) = &req.failure {
        s.push_str(&format!("\n\nPREVIOUS ATTEMPT FAILED: {}\n", f.error));
        if !f.test_output.is_empty() {
            s.push_str(&format!("\nTest output (tail):\n{}\n", f.test_output));
        }
        if !f.diff.is_empty() {
            s.push_str(&format!("\nAttempted diff (truncated):\n{}\n", f.diff));
        }
    }
    s
}

#[async_trait]
impl PhaseExecutor for ExternalExecutor {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, req: &PhaseRequest) -> anyhow::Result<PhaseOutcome> {
        let ModelSelector::Pinned { key, spec } = &req.selector else {
            anyhow::bail!("executor externo \"{}\" exige um modelo fixo (degrau de escalada)", self.name);
        };
        let resolved = spec
            .resolve()
            .map_err(|var| anyhow::anyhow!("modelo \"{key}\" não está configurado (falta a variável {var})"))?;

        // FORA de req.workdir de propósito: workdir é o próprio repositório clonado, e um
        // arquivo de apoio (a instrução da fase, a chave da API no config do opencode) ali
        // dentro aparece em `git status` como "alterado" — isso já causou um bug real: a
        // verificação (`verify()`, que só olha se ALGUM arquivo mudou) passava mesmo quando
        // o modelo não tocou em nada do código de verdade, porque o próprio `phase-N.md`
        // contava como "mudança". E pior: o `opencode.jsonc` com a chave da API acabava
        // comitado e indo pro PR. Usa o diretório de temp do sistema, isolado por tarefa+fase.
        // Chave pelo workdir (não só `task_id`) para não colidir entre tarefas que, em
        // testes, reusam o mesmo `task_id` literal com workdirs diferentes.
        let workdir_abs = tokio::fs::canonicalize(&req.workdir).await?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&workdir_abs, &mut hasher);
        let dir = std::env::temp_dir()
            .join("oreia-seca-scratch")
            .join(format!("{:x}", std::hash::Hasher::finish(&hasher)))
            .join(req.phase_index.to_string());
        tokio::fs::create_dir_all(&dir).await?;
        let message_file: PathBuf = dir.join(format!("phase-{}.md", req.phase_index));
        tokio::fs::write(&message_file, message_body(req)).await?;
        let message_file = tokio::fs::canonicalize(&message_file).await?.to_string_lossy().into_owned();

        let api_key = spec.api_key_env.as_deref().and_then(env).unwrap_or_default();
        let test_cmd = req.test_cmd.as_deref().unwrap_or("");

        let config_dir = if self.name == "opencode" {
            write_opencode_config(&dir, &resolved.base_url, &resolved.model, &api_key)
                .await?
                .to_string_lossy()
                .into_owned()
        } else {
            String::new()
        };

        let real =
            render(&self.template, &resolved.model, &resolved.base_url, &api_key, &message_file, test_cmd, &config_dir);
        let display =
            render(&self.template, &resolved.model, &resolved.base_url, "***", &message_file, test_cmd, &config_dir);

        let ctx = ActionContext {
            task_id: Some(req.task_id.clone()),
            user_id: String::new(),
            workdir: req.workdir.clone(),
            shell_timeout: req.step_timeout,
        };
        // `sandboxed_shell` seta HOME = ctx.workdir (o repositório clonado) — correto pro
        // executor `builtin`, onde isso é só confinamento. Pra uma CLI externa é um bug real:
        // ferramentas como o opencode gravam cache/dados em $HOME/.cache e $HOME/.local/share
        // por padrão, e isso ia direto pro repositório e acabava comitado e empurrado pro PR
        // (confirmado em produção: `.cache/opencode/`, `.local/share/opencode/opencode.db` de
        // ~40MB, e até um diretório git interno da ferramenta). Sobrescreve pra apontar pro
        // diretório de temp isolado (o mesmo de `{message_file}`/`{config_dir}`), que é
        // removido ao fim da tentativa.
        let mut cmd = sandboxed_shell(&real, &ctx);
        cmd.env("HOME", &dir);
        let raw_output = run_command(cmd, &display, &ctx).await;
        tokio::fs::remove_dir_all(&dir).await.ok();
        let raw_output = raw_output?;

        let changed_files = super::git_changed_files(&req.workdir).await;
        let summary = format!("executor externo \"{}\" ({key}) rodou a fase \"{}\"", self.name, req.phase_name);
        Ok(PhaseOutcome {
            summary,
            changed_files,
            prompt_tokens: 0,
            completion_tokens: 0,
            action_errors: 0,
            raw_output,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{ModelSelector, PhaseRequest};
    use oreia_llm::ModelSpec;
    use std::time::Duration;

    fn req(workdir: PathBuf, template_test_cmd: Option<&str>) -> PhaseRequest {
        PhaseRequest {
            task_id: "t1".into(),
            workdir,
            goal: "goal".into(),
            completed_summary: "(none)".into(),
            phase_name: "Write".into(),
            phase_instruction: "write it".into(),
            phase_index: 1,
            phase_total: 1,
            agent_prompt: None,
            work_item: None,
            base_branch: "main".into(),
            selector: ModelSelector::Pinned {
                key: "m".into(),
                spec: ModelSpec::local("http://x/v1", "upstream-model"),
            },
            failure: None,
            shell_timeout: Duration::from_secs(5),
            step_timeout: Duration::from_secs(5),
            test_cmd: template_test_cmd.map(String::from),
        }
    }

    #[tokio::test]
    async fn renders_template_writes_message_file_and_hides_secret() {
        let dir = std::env::temp_dir().join(format!("oreia-seca-ext-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        std::env::set_var("OREIA_SECA_TEST_EXT_KEY", "s3cr3t");

        let mut r = req(dir.clone(), Some("cargo test"));
        if let ModelSelector::Pinned { spec, .. } = &mut r.selector {
            spec.api_key_env = Some("OREIA_SECA_TEST_EXT_KEY".into());
        }

        // Script falso: grava os argumentos recebidos para provar o template e o ambiente limpo.
        let exe = ExternalExecutor::new(
            "fake",
            "echo \"model={model} base={base_url} key={api_key} test={test_cmd}\" > out.txt && cat {message_file} >> out.txt && echo \"SECRET=[$OREIA_SECA_TEST_EXT_KEY]\" >> out.txt",
        );
        exe.run(&r).await.unwrap();

        let out = tokio::fs::read_to_string(dir.join("out.txt")).await.unwrap();
        assert!(out.contains("model=upstream-model"), "{out}");
        assert!(out.contains("base=http://x/v1"), "{out}");
        assert!(out.contains("key=s3cr3t"), "{out}");
        assert!(out.contains("test=cargo test"), "{out}");
        assert!(out.contains("CURRENT PHASE (1/1): Write"), "{out}");
        assert!(out.contains("SECRET=[]"), "o segredo do processo pai não deveria chegar ao script: {out}");

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    /// Regressão real: `sandboxed_shell` seta `HOME=ctx.workdir` (o repositório clonado).
    /// Ferramentas externas gravam cache/dados em `$HOME/.cache` e `$HOME/.local/share`
    /// por padrão — sem sobrescrever isso, elas poluem o repositório, e tudo isso acaba
    /// comitado e empurrado pro PR (visto em produção com o opencode: um `opencode.db` de
    /// ~40MB). `run()` precisa sobrescrever `$HOME` pra fora do repositório.
    #[tokio::test]
    async fn run_overrides_home_away_from_the_repo_workdir() {
        let workdir = std::env::temp_dir().join(format!("oreia-seca-ext-home-{}", std::process::id()));
        tokio::fs::create_dir_all(&workdir).await.unwrap();

        let r = req(workdir.clone(), None);
        let exe = ExternalExecutor::new("fake", "echo \"$HOME\" > out.txt");
        exe.run(&r).await.unwrap();

        let out = tokio::fs::read_to_string(workdir.join("out.txt")).await.unwrap();
        let home = out.trim();
        let workdir_abs = tokio::fs::canonicalize(&workdir).await.unwrap();
        assert_ne!(home, workdir_abs.to_string_lossy(), "HOME não deveria ser o repositório");
        assert!(!PathBuf::from(home).starts_with(&workdir_abs), "HOME não deveria ficar dentro do repositório: {home}");

        tokio::fs::remove_dir_all(&workdir).await.ok();
    }

    /// Regressão real: em produção `req.workdir` é relativo (`workspace/task-x`,
    /// relativo ao cwd do orquestrador — ver `exec.rs`), e `sandboxed_shell` já
    /// roda o comando com esse mesmo diretório como cwd. Reproduz isso de verdade
    /// (sem mudar o cwd do processo de teste, só criando o diretório relativo).
    #[tokio::test]
    async fn message_file_is_absolute_even_with_a_relative_workdir() {
        let name = format!("oreia-seca-ext-reltest-{}", std::process::id());
        let workdir = PathBuf::from(&name);
        tokio::fs::create_dir_all(&workdir).await.unwrap();

        let r = req(workdir.clone(), None);
        // Mesmo formato do preset real ("< {message_file}"): só funciona se o
        // caminho for resolvível a partir do cwd do comando (= workdir).
        let exe = ExternalExecutor::new("fake", "cat < {message_file} > out.txt");
        exe.run(&r).await.unwrap();

        let out = tokio::fs::read_to_string(workdir.join("out.txt")).await.unwrap();
        assert!(out.contains("CURRENT PHASE (1/1): Write"), "{out}");

        tokio::fs::remove_dir_all(&workdir).await.ok();
    }

    /// Regressão real: `.oreia_seca/phase-N.md` (e, no opencode, o `opencode.jsonc` com a
    /// chave da API) viviam DENTRO de `req.workdir` — o próprio repositório clonado. Isso
    /// fazia `git status` marcar esses arquivos de apoio como "mudança", e `verify()`
    /// (que só olha se algum arquivo mudou) dava passou mesmo quando a ferramenta não
    /// tocou em nenhum arquivo real do projeto — e, piorando, esse `opencode.jsonc` com a
    /// chave acabaria comitado e empurrado pro PR. Prova que não sobra nada em `workdir`.
    #[tokio::test]
    async fn run_leaves_no_scratch_files_inside_the_repo_workdir() {
        let workdir = std::env::temp_dir().join(format!("oreia-seca-ext-noleak-{}", std::process::id()));
        tokio::fs::create_dir_all(&workdir).await.unwrap();

        let mut r = req(workdir.clone(), None);
        r.phase_index = 1;
        // Comando fake que não toca em nada do workdir (como se a ferramenta não tivesse
        // feito a edição pedida) — só lê o message_file pra provar que ele existe e some depois.
        let exe = ExternalExecutor::new("opencode", "cat {message_file} > /dev/null");
        exe.run(&r).await.unwrap();

        let mut entries = tokio::fs::read_dir(&workdir).await.unwrap();
        assert!(
            entries.next_entry().await.unwrap().is_none(),
            "run() deixou arquivo de apoio dentro do repositório (workdir)"
        );

        tokio::fs::remove_dir_all(&workdir).await.ok();
    }

    fn work_item(
        agent: &str,
        issue: u64,
        mode: oreia_core::WorkMode,
        review_branch: Option<&str>,
    ) -> oreia_core::WorkItem {
        oreia_core::WorkItem {
            owner: "acme".into(),
            repo: "app".into(),
            issue,
            agent: agent.into(),
            title: String::new(),
            mode,
            review_branch: review_branch.map(String::from),
            base_branch: None,
        }
    }

    #[test]
    fn message_body_includes_repo_context_and_rules_when_work_item_present() {
        let mut r = req(PathBuf::from("/tmp/x"), None);
        r.work_item = Some(work_item("dev-front", 9, oreia_core::WorkMode::Code, None));
        let body = message_body(&r);
        assert!(body.contains("REPOSITORY CONTEXT"), "{body}");
        assert!(body.contains("acme/app"), "{body}");
        assert!(body.contains("agent/dev-front/issue-9"), "{body}");
        assert!(body.contains("Do NOT run git commit"), "{body}");
        assert!(body.contains("RULES:"), "{body}");
    }

    #[test]
    fn message_body_has_no_repo_context_without_work_item() {
        let r = req(PathBuf::from("/tmp/x"), None);
        let body = message_body(&r);
        assert!(!body.contains("REPOSITORY CONTEXT"), "{body}");
        assert!(body.contains("RULES:"), "as regras valem mesmo sem item do Forgejo: {body}");
    }

    /// Regressão da mesma classe do bug do `prompts::executor` do builtin: em
    /// modo Review, a branch de verdade é a do PR (review_branch), não a que
    /// `branch_name()` calcularia a partir do agente "reviewer".
    #[test]
    fn message_body_uses_the_pr_branch_in_review_mode_not_the_reviewer_agent_branch() {
        let mut r = req(PathBuf::from("/tmp/x"), None);
        r.work_item = Some(work_item("reviewer", 12, oreia_core::WorkMode::Review, Some("agent/dev-front/issue-9")));
        let body = message_body(&r);
        assert!(body.contains("agent/dev-front/issue-9"), "{body}");
        assert!(!body.contains("agent/reviewer/issue-12"), "{body}");
    }

    #[test]
    fn builtin_presets_cover_the_four_tools() {
        for name in ["aider", "opencode", "goose", "codex"] {
            assert!(builtin_preset(name).is_some(), "{name}");
        }
        assert!(builtin_preset("nope").is_none());
    }

    /// Fixa os pontos que custou testar de verdade contra ferramentas reais
    /// (ver docs/executores.md): nenhum dos três usa --api-base/--api-key
    /// (não existem), cada um resolve a chave/URL do próprio jeito, e todos
    /// ainda levam {model}/{message_file}. Não prova que a CLI de verdade
    /// aceita isso (só o teste ao vivo prova), mas pega quem reverter sem querer.
    #[test]
    fn presets_use_the_mechanisms_verified_live_not_api_base_api_key_flags() {
        let opencode = builtin_preset("opencode").unwrap();
        assert!(opencode.contains("XDG_CONFIG_HOME={config_dir}"), "{opencode}");
        assert!(opencode.contains("-m oreiaseca/{model}"), "{opencode}");
        assert!(opencode.contains("--auto"), "{opencode}");
        assert!(!opencode.contains("--api-base") && !opencode.contains("--api-key"), "{opencode}");

        let goose = builtin_preset("goose").unwrap();
        assert!(goose.contains("OPENAI_HOST=\"${BASE_URL%/v1}\""), "{goose}");
        assert!(goose.contains("GOOSE_MODEL={model}"), "{goose}");
        assert!(!goose.contains("--api-base"), "{goose}");

        let codex = builtin_preset("codex").unwrap();
        assert!(codex.contains("model_providers.custom.base_url={base_url}"), "{codex}");
        assert!(codex.contains("model_providers.custom.env_key=OREIA_SECA_EXT_API_KEY"), "{codex}");
        assert!(!codex.contains("--base-url") && !codex.contains("--api-key"), "{codex}");

        // O aider é o único dos quatro que de fato tem --openai-api-base/--openai-api-key.
        let aider = builtin_preset("aider").unwrap();
        assert!(aider.contains("--openai-api-base {base_url}") && aider.contains("--openai-api-key {api_key}"));
    }

    #[test]
    fn render_substitutes_every_placeholder_in_all_four_presets() {
        for name in ["aider", "opencode", "goose", "codex"] {
            let rendered =
                render(builtin_preset(name).unwrap(), "upstream-model", "http://x/v1", "s3cr3t", "/msg.md", "", "/cfg");
            // Não "sem nenhum {" — o preset do goose tem "${BASE_URL%/v1}" de
            // propósito (sintaxe de shell, não placeholder nosso).
            for token in ["{model}", "{base_url}", "{api_key}", "{message_file}", "{config_dir}"] {
                assert!(!rendered.contains(token), "{name} deixou {token} sem substituir: {rendered}");
            }
            assert!(rendered.contains("upstream-model"), "{name}: {rendered}");
            assert!(rendered.contains("/msg.md"), "{name}: {rendered}");
        }
    }

    #[tokio::test]
    async fn write_opencode_config_produces_a_custom_provider_pointing_at_the_real_base_url() {
        let dir = std::env::temp_dir().join(format!("oreia-seca-ocfg-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let config_dir =
            write_opencode_config(&dir, "https://openrouter.ai/api/v1", "google/gemini-2.5-flash-lite", "s3cr3t")
                .await
                .unwrap();
        let content = tokio::fs::read_to_string(config_dir.join("opencode/opencode.jsonc")).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["provider"]["oreiaseca"]["options"]["baseURL"], "https://openrouter.ai/api/v1");
        assert_eq!(parsed["provider"]["oreiaseca"]["options"]["apiKey"], "s3cr3t");
        assert!(parsed["provider"]["oreiaseca"]["models"].get("google/gemini-2.5-flash-lite").is_some());
        let canon_dir = tokio::fs::canonicalize(&dir).await.unwrap();
        assert!(
            config_dir.starts_with(&canon_dir),
            "config_dir deveria ficar dentro do workdir da tarefa, não do projeto"
        );

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    async fn run_preset_live(name: &str, model: &str, key: &str) -> anyhow::Result<PhaseOutcome> {
        let dir = std::env::temp_dir().join(format!(
            "oreia-seca-live-ext-{name}-{}-{}",
            model.replace(['/', '.'], "-"),
            std::process::id()
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        if name == "aider" {
            // aider recusa rodar fora de um repositório git.
            let ok = std::process::Command::new("git").args(["init", "-q"]).current_dir(&dir).status().unwrap();
            assert!(ok.success());
        }

        let mut spec = ModelSpec::local("https://openrouter.ai/api/v1", model);
        spec.api_key_env = Some("OREIA_SECA_LIVE_EXT_TEST_KEY".into());
        std::env::set_var("OREIA_SECA_LIVE_EXT_TEST_KEY", key);

        let mut r = req(dir.clone(), None);
        r.phase_instruction = "Reply with just the word \"ok\". Do not create, read or edit any files.".into();
        r.selector = ModelSelector::Pinned { key: "live".into(), spec };
        r.step_timeout = Duration::from_secs(60);

        let exe = ExternalExecutor::new(name, builtin_preset(name).unwrap());
        let result = exe.run(&r).await;
        tokio::fs::remove_dir_all(&dir).await.ok();
        result
    }

    /// Teste de verdade contra as 4 CLIs reais + a API da OpenRouter, com dois
    /// modelos (deepseek e google/gemini-2.5-flash-lite, para pegar diferença
    /// de comportamento entre provedores) — não roda em CI nem no `cargo test`
    /// normal (precisa das ferramentas instaladas, de rede e de uma chave de
    /// verdade). Só roda com `OREIA_SECA_LIVE_EXTERNAL_TESTS=1` e
    /// `OPENROUTER_KEY` no ambiente. Valida a cadeia inteira através do código
    /// real (`ExternalExecutor::run` → `sandboxed_shell` → CLI real → API
    /// real), não só o template.
    #[tokio::test]
    async fn live_presets_actually_work_against_openrouter() {
        if std::env::var("OREIA_SECA_LIVE_EXTERNAL_TESTS").as_deref() != Ok("1") {
            return;
        }
        let Ok(key) = std::env::var("OPENROUTER_KEY") else {
            panic!("OREIA_SECA_LIVE_EXTERNAL_TESTS=1 mas falta OPENROUTER_KEY no ambiente");
        };

        for name in ["aider", "opencode", "goose", "codex"] {
            for model in ["deepseek/deepseek-v4.1-flash-1017", "google/gemini-2.5-flash-lite"] {
                let result = run_preset_live(name, model, &key).await;
                let outcome = result.unwrap_or_else(|e| panic!("{name} com {model} falhou: {e:#}"));
                assert!(!outcome.raw_output.trim().is_empty(), "{name} com {model} não devolveu saída nenhuma");
            }
        }
    }

    /// Isolado (não misturado com os outros 7 casos de
    /// `live_presets_actually_work_against_openrouter`): só opencode +
    /// google/gemini-2.5-flash-lite, com o prompt elaborado (`message_body`)
    /// e a resposta crua impressos — rode com `-- --exact --nocapture` pra
    /// ver os dois. Prova a correção do provedor ad-hoc (`write_opencode_config`)
    /// de ponta a ponta: não é só "o teste passou", é "aqui está o que foi
    /// mandado e o que voltou". Mesmo gate das outras: só com
    /// `OREIA_SECA_LIVE_EXTERNAL_TESTS=1` e `OPENROUTER_KEY`.
    #[tokio::test]
    async fn live_opencode_with_gemini_shows_the_prompt_and_response() {
        if std::env::var("OREIA_SECA_LIVE_EXTERNAL_TESTS").as_deref() != Ok("1") {
            return;
        }
        let Ok(key) = std::env::var("OPENROUTER_KEY") else {
            panic!("OREIA_SECA_LIVE_EXTERNAL_TESTS=1 mas falta OPENROUTER_KEY no ambiente");
        };
        let model = "google/gemini-2.5-flash-lite";

        let dir = std::env::temp_dir().join(format!("oreia-seca-prove-opencode-gemini-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let mut spec = ModelSpec::local("https://openrouter.ai/api/v1", model);
        spec.api_key_env = Some("OREIA_SECA_LIVE_EXT_TEST_KEY".into());
        std::env::set_var("OREIA_SECA_LIVE_EXT_TEST_KEY", &key);

        let mut r = req(dir.clone(), None);
        r.agent_prompt = Some("You are **dev-front**, the frontend developer of this team.".into());
        r.goal = "Ajustar background do index".into();
        r.phase_name = "Confirm understanding".into();
        r.phase_instruction = "Do not create, read or edit any files. Just reply in one short \
            sentence confirming you understood the goal above."
            .into();
        r.selector = ModelSelector::Pinned { key: "live".into(), spec };
        r.step_timeout = Duration::from_secs(60);

        println!("\n=== PROMPT ENVIADO (message_body) ===\n{}", message_body(&r));

        let exe = ExternalExecutor::new("opencode", builtin_preset("opencode").unwrap());
        let outcome = exe.run(&r).await.unwrap_or_else(|e| panic!("opencode com {model} falhou: {e:#}"));

        println!("\n=== SAÍDA CRUA DO OPENCODE (resposta real do gemini-2.5-flash-lite) ===\n{}", outcome.raw_output);
        assert!(!outcome.raw_output.trim().is_empty(), "opencode com {model} não devolveu saída nenhuma");

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
