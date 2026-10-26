//! Comandos de barra, iguais em todos os canais (terminal, Telegram, REST).

use std::sync::Arc;

use oreia_core::{Incoming, PhaseStatus, TaskStatus, WorkItem, WorkMode};
use oreia_forgejo::{parse_agent_branch, split_repo, ReviewEvent};

use super::Maker;

pub const HELP: &str = "Comandos:
/status — fila, aprovações e modelos ativos
/queue — detalhes da fila
/tasks — suas últimas tarefas
/feature <descrição> — cria uma tarefa sem passar pelo classificador
/issue [dono/repo] [agente] [--base <branch>] [título] — abre uma issue (PR vai pra essa branch, se informada); sem título, também pergunto descrição e branch
/assign [dono/repo] [n] [agente] — liga um agente a uma issue já aberta; o que faltar, eu pergunto
/approve <id> · /reject <id> — decide tarefas pendentes
/pending — aprovações pendentes
/retry <id> — retoma uma tarefa interrompida
/models — modelo ativo por papel
/stats [papel] — estatísticas de escalada (degraus, custo)
/agents — agentes do Forgejo
/prs — PRs abertos, um cartão por PR com botões de Merge/Pedir ajuste
/merge [dono/repo] <n> — aprova e integra um PR
/changes [dono/repo] <n> <texto> — pede ajustes; o agente volta ao PR
/skills [reload] — ações disponíveis
/resumir — resume e compacta esta conversa
/limpar — apaga o histórico desta conversa";

/// O que fazer com a PRÓXIMA mensagem de texto livre de uma sessão — os passos dos
/// assistentes de `/issue`/`/assign`/`/prs` que não cabem num botão (título de
/// issue, número de issue, texto de ajuste de PR). Ver `Maker::pending` e
/// `Maker::resolve_pending`.
#[derive(Clone)]
pub(super) enum Pending {
    IssueTitle {
        owner: String,
        repo: String,
        agent: Option<String>,
    },
    /// Depois do título, pede a descrição — é ela que o agente lê de verdade pra
    /// entender o que fazer; o título sozinho costuma ser curto demais pra isso.
    IssueBody {
        owner: String,
        repo: String,
        agent: Option<String>,
        title: String,
    },
    /// Por último, a branch base — opcional, "-" mantém o padrão de
    /// `[forgejo].base_branch`. Vira o campo "Ramo" da issue no Forgejo.
    IssueBranch {
        owner: String,
        repo: String,
        agent: Option<String>,
        title: String,
        body: String,
    },
    AssignNumber {
        owner: String,
        repo: String,
    },
    ChangesText {
        owner: String,
        repo: String,
        number: u64,
    },
}

impl Maker {
    pub(crate) async fn command(self: Arc<Self>, msg: &Incoming, line: &str) -> anyhow::Result<()> {
        let mut parts = line.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("").to_lowercase();
        let rest = parts.next().unwrap_or("").trim();
        let args: Vec<&str> = rest.split_whitespace().collect();
        let say = |t: String| {
            let me = self.clone();
            let (src, uid) = (msg.source, msg.user_id.clone());
            async move { me.reply(src, &uid, &t).await }
        };

        match cmd.as_str() {
            "help" | "ajuda" | "start" => say(HELP.into()).await,

            "approve" | "reject" => {
                let Some(id) = args.first() else {
                    say(format!("uso: /{cmd} <id>")).await;
                    return Ok(());
                };
                let ok = self.approvals.respond(id, cmd == "approve");
                if !ok {
                    say(format!("Nenhuma aprovação pendente para {id}.")).await;
                }
            }

            "pending" => {
                let list = self.approvals.list_pending();
                if list.is_empty() {
                    say("Nenhuma aprovação pendente.".into()).await;
                } else {
                    let t: Vec<String> = list.iter().map(|(id, q)| format!("• {id}: {q}")).collect();
                    say(format!("Aprovações pendentes:\n{}", t.join("\n"))).await;
                }
            }

            "status" => {
                let q = self.queue.status();
                let mut parts = Vec::new();
                if q.running > 0 {
                    parts.push(format!("{} executando", q.running));
                }
                if q.queued > 0 {
                    parts.push(format!("{} na fila", q.queued));
                }
                let pend = self.approvals.list_pending().len();
                if pend > 0 {
                    parts.push(format!("{pend} aguardando aprovação"));
                }
                let head = if parts.is_empty() { "Nenhuma tarefa ativa".to_string() } else { parts.join(", ") };

                let running = self.store.list_by_status(TaskStatus::Running).await.unwrap_or_default();
                let mut lines = vec![head];
                for t in running.iter().take(5) {
                    let done = t.phases.iter().filter(|p| p.status != PhaseStatus::Pending).count();
                    lines.push(format!(
                        "• {} — \"{}\" ({done}/{} fases)",
                        t.id,
                        t.goal.chars().take(45).collect::<String>(),
                        t.phases.len()
                    ));
                }
                lines.push(String::new());
                lines.push(models_text(&self));
                say(lines.join("\n")).await;
            }

            "queue" | "fila" => {
                let s = self.queue.status();
                say(format!(
                    "Fila: {} rodando, {} aguardando\nConcorrência: {}\nTotal: {} concluídas, {} com erro",
                    s.running, s.queued, s.concurrency, s.completed, s.failed
                ))
                .await;
            }

            "tasks" | "tarefas" => {
                let tasks = self.store.list_by_user(&msg.user_id).await?;
                if tasks.is_empty() {
                    say("Nenhuma tarefa encontrada.".into()).await;
                } else {
                    let t: Vec<String> = tasks
                        .iter()
                        .take(8)
                        .map(|t| {
                            let pr = t.pull_request.as_deref().map(|u| format!(" → {u}")).unwrap_or_default();
                            format!(
                                "[{}] {} ({}){pr}",
                                t.status.as_str(),
                                t.goal.chars().take(50).collect::<String>(),
                                t.id
                            )
                        })
                        .collect();
                    say(format!("Últimas tarefas:\n{}", t.join("\n"))).await;
                }
            }

            "feature" | "tarefa" => {
                if rest.is_empty() {
                    say("uso: /feature <descrição do que fazer>".into()).await;
                } else {
                    return self.start_task(msg, rest).await;
                }
            }

            "retry" => {
                let Some(id) = args.first() else {
                    say("uso: /retry <id>".into()).await;
                    return Ok(());
                };
                let Some(mut task) = self.store.load(id).await? else {
                    say(format!("Tarefa não encontrada: {id}")).await;
                    return Ok(());
                };
                if let Some(w) = &task.work_item {
                    if !self.claim_work(w) {
                        say(format!("{} já tem um agente trabalhando.", Maker::work_key(w))).await;
                        return Ok(());
                    }
                }
                for p in task.phases.iter_mut().filter(|p| p.status == PhaseStatus::Failed) {
                    p.status = PhaseStatus::Pending;
                }
                task.set_status(TaskStatus::Running);
                self.store.save(&task).await?;
                say(format!("🔁 Retomando {}: {}", task.id, task.goal.chars().take(60).collect::<String>())).await;
                self.enqueue(task);
            }

            "models" | "modelos" => say(models_text(&self)).await,

            "stats" | "estatisticas" | "estatísticas" => {
                let path = self.cfg.paths.data.join("escalation.jsonl");
                let entries = crate::executor::log::load(&path).await.unwrap_or_default();
                let stats = crate::executor::log::aggregate(&entries, args.first().copied());
                if stats.is_empty() {
                    say("Sem dados de escalada ainda.".into()).await;
                } else {
                    let lines: Vec<String> = stats
                        .iter()
                        .map(|s| {
                            let steps: Vec<String> = s
                                .by_step
                                .iter()
                                .map(|(step, n)| {
                                    let pct = if s.resolved_phases > 0 {
                                        *n as f64 / s.resolved_phases as f64 * 100.0
                                    } else {
                                        0.0
                                    };
                                    format!("degrau {step}: {pct:.0}%")
                                })
                                .collect();
                            format!(
                                "• {} — {} fase(s) resolvida(s) ({}), custo total US$ {:.2}, médio US$ {:.2}/tarefa",
                                s.role,
                                s.resolved_phases,
                                steps.join(", "),
                                s.total_cost_usd,
                                s.avg_cost_usd_per_task
                            )
                        })
                        .collect();
                    say(format!("📊 Estatísticas de escalada:\n{}", lines.join("\n"))).await;
                }
            }

            "agents" | "agentes" => {
                let Some(f) = &self.forgejo else {
                    say("Forgejo não configurado.".into()).await;
                    return Ok(());
                };
                let lines: Vec<String> = self
                    .cfg
                    .agents
                    .iter()
                    .map(|a| {
                        let on = if f.agents.contains_key(&a.name) { "🟢" } else { "🔴 sem token" };
                        let model = self.router.active_model(a.role()).unwrap_or_else(|| "nenhum".into());
                        format!("{on} {} — label `{}`, usuário {}, modelo {model}", a.name, a.label, a.user)
                    })
                    .collect();
                say(if lines.is_empty() { "Nenhum agente em [[agents]].".into() } else { lines.join("\n") }).await;
            }

            "issue" => {
                let Some(f) = &self.forgejo else {
                    say("Forgejo não configurado.".into()).await;
                    return Ok(());
                };
                if f.owner.is_none() {
                    say(format!("Defina {} para usar /issue.", f.cfg.owner_token_env)).await;
                    return Ok(());
                }
                let (repo, rest_args) = match self.pick_or_resolve_repo(msg, &args, "/issue").await {
                    Ok(Some(v)) => v,
                    Ok(None) => return Ok(()), // já perguntou o repo por botão
                    Err(e) => {
                        say(format!("{e}\nuso: /issue [dono/repo] [agente] <título>")).await;
                        return Ok(());
                    }
                };
                // Agente é opcional: sem ele, a issue fica sem label pra você analisar
                // antes — depois use /assign pra acionar. "--none" (botão "Sem agente")
                // marca isso explicitamente; sem nenhum token ainda, pergunta por botão;
                // um token que bate com um nome configurado é consumido como agente;
                // qualquer outra coisa já é início do título (uso direto, numa linha só).
                let (agent, title_args): (Option<&str>, &[&str]) = match rest_args.first() {
                    Some(&"--none") => (None, &rest_args[1..]),
                    Some(&tok) if self.cfg.agents.iter().any(|a| a.name == tok) => (Some(tok), &rest_args[1..]),
                    Some(_) => (None, rest_args),
                    None => {
                        let cmd = format!("/issue {}/{}", repo.0, repo.1);
                        let options = self.agent_options(&cmd, true);
                        self.pick(msg.source, &msg.user_id, "Qual agente? (ou nenhum, pra analisar antes)", &options)
                            .await;
                        return Ok(());
                    }
                };
                // "--base <branch>" pode aparecer em qualquer posição no título (uso
                // direto) pra abrir contra uma branch que não seja a padrão — mesmo
                // campo "Ramo" que dá pra preencher direto no Forgejo.
                let (base_branch, title_args): (Option<&str>, Vec<&str>) =
                    match title_args.iter().position(|&t| t == "--base") {
                        Some(i) if i + 1 < title_args.len() => {
                            let mut rest: Vec<&str> = title_args[..i].to_vec();
                            rest.extend_from_slice(&title_args[i + 2..]);
                            (Some(title_args[i + 1]), rest)
                        }
                        _ => (None, title_args.to_vec()),
                    };
                let title = title_args.join(" ");
                if title.is_empty() {
                    self.pending.lock().expect("lock").insert(
                        msg.session_key(),
                        Pending::IssueTitle {
                            owner: repo.0.clone(),
                            repo: repo.1.clone(),
                            agent: agent.map(String::from),
                        },
                    );
                    let who = agent.unwrap_or("sem agente");
                    say(format!("✏️ Qual o título da issue? ({who})")).await;
                    return Ok(());
                }
                // Uso direto numa linha só: não dá pra separar título de descrição aqui
                // (os dois viram uma sequência de palavras), então sai sem descrição —
                // pra descrever o que o agente deve fazer, use /issue sem nada no final
                // e deixe o assistente perguntar os dois separadamente.
                self.create_issue_and_reply(msg, (&repo.0, &repo.1), agent, &title, "", base_branch).await?;
            }

            "assign" => {
                let Some(f) = &self.forgejo else {
                    say("Forgejo não configurado.".into()).await;
                    return Ok(());
                };
                if f.owner.is_none() {
                    say(format!("Defina {} para usar /assign.", f.cfg.owner_token_env)).await;
                    return Ok(());
                }
                let (repo, rest_args) = match self.pick_or_resolve_repo(msg, &args, "/assign").await {
                    Ok(Some(v)) => v,
                    Ok(None) => return Ok(()),
                    Err(e) => {
                        say(format!("{e}\nuso: /assign [dono/repo] <n> <agente>")).await;
                        return Ok(());
                    }
                };
                let Some(&n_tok) = rest_args.first() else {
                    self.pending.lock().expect("lock").insert(
                        msg.session_key(),
                        Pending::AssignNumber { owner: repo.0.clone(), repo: repo.1.clone() },
                    );
                    say("✏️ Qual o número da issue?".into()).await;
                    return Ok(());
                };
                let Ok(n) = n_tok.trim_start_matches('#').parse::<u64>() else {
                    say(format!("\"{n_tok}\" não é um número de issue válido.")).await;
                    return Ok(());
                };
                let Some(&agent_name) = rest_args[1..].first() else {
                    let cmd = format!("/assign {}/{} {n}", repo.0, repo.1);
                    let options = self.agent_options(&cmd, false);
                    self.pick(msg.source, &msg.user_id, &format!("Qual agente pra issue #{n}?"), &options).await;
                    return Ok(());
                };
                self.assign_and_reply(msg, &repo.0, &repo.1, n, agent_name).await;
            }

            "prs" => {
                let Some(f) = &self.forgejo else {
                    say("Forgejo não configurado.".into()).await;
                    return Ok(());
                };
                let Some(owner) = &f.owner else {
                    say(format!("Defina {} para usar /prs.", f.cfg.owner_token_env)).await;
                    return Ok(());
                };
                let mut any = false;
                for full in &f.cfg.repos {
                    let Some((o, r)) = split_repo(full) else { continue };
                    match owner.list_open_pulls(o, r).await {
                        Ok(prs) => {
                            for p in prs {
                                any = true;
                                let by = p.user.map(|u| u.login).unwrap_or_default();
                                let text = format!("🔀 {full}#{} {} (por {by})\n{}", p.number, p.title, p.html_url);
                                let options = vec![
                                    ("✅ Merge".to_string(), format!("/merge {full} {}", p.number)),
                                    ("📝 Pedir ajuste".to_string(), format!("/changes-ask {full} {}", p.number)),
                                ];
                                self.pick(msg.source, &msg.user_id, &text, &options).await;
                            }
                        }
                        Err(e) => {
                            any = true;
                            say(format!("⚠ {full}: {e}")).await;
                        }
                    }
                }
                if !any {
                    say("Nenhum PR aberto. 🎉".into()).await;
                }
            }

            "merge" => {
                let (o, r, n, _) = match self.repo_and_number(&args) {
                    Ok(v) => v,
                    Err(e) => {
                        say(format!("{e}\nuso: /merge [dono/repo] <n>")).await;
                        return Ok(());
                    }
                };
                let f = self.forgejo.as_ref().expect("checado em repo_and_number");
                let owner = f.owner.as_ref().expect("checado em repo_and_number");
                if let Err(e) = owner.review_pull(&o, &r, n, ReviewEvent::Approve, "Aprovado via OREIA-SECA.").await {
                    tracing::warn!(error = %e, "não consegui registrar a aprovação (seguindo para o merge)");
                }
                owner.merge_pull(&o, &r, n, &f.cfg.merge_style).await?;
                say(format!("✅ {o}/{r}#{n} integrado.")).await;
            }

            "changes" | "ajustes" => {
                let (o, r, n, text) = match self.repo_and_number(&args) {
                    Ok(v) => v,
                    Err(e) => {
                        say(format!("{e}\nuso: /changes [dono/repo] <n> <o que mudar>")).await;
                        return Ok(());
                    }
                };
                if text.is_empty() {
                    say("Diga o que precisa mudar: /changes [dono/repo] <n> <texto>".into()).await;
                    return Ok(());
                }
                return self.run_changes(msg, &o, &r, n, &text).await;
            }

            // Só o botão "📝 Pedir ajuste" do /prs chama isto — pede o texto por
            // mensagem livre em vez de exigi-lo já no comando.
            "changes-ask" => {
                let (o, r, n, _) = match self.repo_and_number(&args) {
                    Ok(v) => v,
                    Err(e) => {
                        say(format!("{e}")).await;
                        return Ok(());
                    }
                };
                self.pending
                    .lock()
                    .expect("lock")
                    .insert(msg.session_key(), Pending::ChangesText { owner: o.clone(), repo: r.clone(), number: n });
                say(format!("✏️ O que precisa mudar no PR {o}/{r}#{n}?")).await;
            }

            "skills" => {
                if args.first() == Some(&"reload") {
                    let n = self.reload_skills();
                    say(format!("✅ Skills recarregadas: {n} skill(s) além das ações embutidas.")).await;
                } else {
                    let lines: Vec<String> = self
                        .actions()
                        .describe()
                        .into_iter()
                        .map(|(name, desc, params)| {
                            let ps: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
                            let ps = if ps.is_empty() { String::new() } else { format!(" ({})", ps.join(", ")) };
                            format!("• {name}{ps}: {}", desc.lines().next().unwrap_or(""))
                        })
                        .collect();
                    say(format!("🛠 Ações e skills:\n{}\n\nUse /skills reload após editar skills/.", lines.join("\n")))
                        .await;
                }
            }

            "resumir" => {
                let file = self.summarize_session(&msg.session_key()).await?;
                say(format!("✅ Conversa resumida e salva em {file}.")).await;
            }

            "limpar" | "clear" => {
                self.sessions.clear(&msg.session_key());
                self.sessions.flush()?;
                say("🧹 Histórico desta conversa apagado.".into()).await;
            }

            _ => say(format!("Comando desconhecido: /{cmd}\n\n{HELP}")).await,
        }
        Ok(())
    }

    /// Lê o prefixo opcional `[dono/repo]` de `args`, usando o único repo de
    /// `[forgejo].repos` quando omitido. Devolve o resto de `args`.
    fn resolve_repo<'a>(&self, args: &'a [&'a str]) -> anyhow::Result<((String, String), &'a [&'a str])> {
        let f = self.forgejo.as_ref().ok_or_else(|| anyhow::anyhow!("Forgejo não configurado."))?;
        match args.first().and_then(|a| split_repo(a)) {
            Some((o, r)) => Ok(((o.to_string(), r.to_string()), &args[1..])),
            None => match f.cfg.repos.as_slice() {
                [only] => {
                    let (o, r) =
                        split_repo(only).ok_or_else(|| anyhow::anyhow!("repo inválido em [forgejo].repos: {only}"))?;
                    Ok(((o.to_string(), r.to_string()), args))
                }
                _ => anyhow::bail!("Informe o repositório (dono/repo)."),
            },
        }
    }

    /// Lê `[dono/repo] <n> [texto…]`. O repositório pode ser omitido se só há um em `[forgejo].repos`.
    fn repo_and_number(&self, args: &[&str]) -> anyhow::Result<(String, String, u64, String)> {
        let f = self.forgejo.as_ref().ok_or_else(|| anyhow::anyhow!("Forgejo não configurado."))?;
        anyhow::ensure!(f.owner.is_some(), "Defina {} para usar este comando.", f.cfg.owner_token_env);
        let (repo, rest) = self.resolve_repo(args)?;
        let n: u64 = rest
            .first()
            .and_then(|s| s.trim_start_matches('#').parse().ok())
            .ok_or_else(|| anyhow::anyhow!("Informe o número do PR."))?;
        Ok((repo.0, repo.1, n, rest[1..].join(" ")))
    }

    /// Como [`Self::resolve_repo`], mas pensado pros assistentes de botão: se vier
    /// vazio e houver mais de um repo em `[forgejo].repos`, pergunta por botão (um
    /// `{cmd} <repo>` por opção) e devolve `None` — quem chamou já deve retornar
    /// depois disso em vez de seguir o fluxo.
    async fn pick_or_resolve_repo<'a>(
        &self,
        msg: &Incoming,
        args: &'a [&'a str],
        cmd: &str,
    ) -> anyhow::Result<Option<((String, String), &'a [&'a str])>> {
        if args.is_empty() {
            let f = self.forgejo.as_ref().ok_or_else(|| anyhow::anyhow!("Forgejo não configurado."))?;
            if f.cfg.repos.len() > 1 {
                let options: Vec<(String, String)> =
                    f.cfg.repos.iter().map(|r| (r.clone(), format!("{cmd} {r}"))).collect();
                self.pick(msg.source, &msg.user_id, "Qual repositório?", &options).await;
                return Ok(None);
            }
        }
        Ok(Some(self.resolve_repo(args)?))
    }

    /// Um botão por agente configurado, cada um completando `{cmd_prefix} <nome>`.
    /// `include_none` acrescenta "🚫 Sem agente" (`{cmd_prefix} --none`) — não faz
    /// sentido em `/assign`, que já é "escolher o agente" por definição.
    fn agent_options(&self, cmd_prefix: &str, include_none: bool) -> Vec<(String, String)> {
        let mut opts: Vec<(String, String)> =
            self.cfg.agents.iter().map(|a| (format!("🤖 {}", a.name), format!("{cmd_prefix} {}", a.name))).collect();
        if include_none {
            opts.push(("🚫 Sem agente".to_string(), format!("{cmd_prefix} --none")));
        }
        opts
    }

    /// Resolve o ID da label do agente (exige que já exista no repo) e abre a
    /// issue — usado tanto pelo `/issue` numa linha só (sem descrição: não dá pra
    /// separar título de descrição numa linha de comando) quanto pelo passo final
    /// do assistente, que pede os dois (e a branch base, opcional).
    async fn create_issue_and_reply(
        &self,
        msg: &Incoming,
        repo: (&str, &str),
        agent_name: Option<&str>,
        title: &str,
        body: &str,
        base_branch: Option<&str>,
    ) -> anyhow::Result<()> {
        let (o, r) = repo;
        let f = self.forgejo.as_ref().expect("checado antes de chamar create_issue_and_reply");
        let owner = f.owner.as_ref().expect("checado antes de chamar create_issue_and_reply");
        let agent = agent_name.and_then(|n| self.cfg.agents.iter().find(|a| a.name == n));
        let label_ids: Vec<i64> = match agent {
            Some(agent) => {
                let labels = match owner.list_labels(o, r).await {
                    Ok(l) => l,
                    Err(e) => {
                        self.reply(msg.source, &msg.user_id, &format!("⚠ Falha ao ler labels de {o}/{r}: {e:#}")).await;
                        return Ok(());
                    }
                };
                let Some(label) = labels.iter().find(|l| l.name == agent.label) else {
                    self.reply(
                        msg.source,
                        &msg.user_id,
                        &format!(
                            "⚠ Label \"{}\" não existe em {o}/{r}. Crie-a no Forgejo antes (Issues → Labels).",
                            agent.label
                        ),
                    )
                    .await;
                    return Ok(());
                };
                vec![label.id]
            }
            None => Vec::new(),
        };
        match owner.create_issue(o, r, title, body, &label_ids, base_branch).await {
            Ok(issue) => {
                let note = match agent {
                    Some(a) => format!(" → {} vai pegar em breve.", a.name),
                    None => " → sem agente; analise e use /assign quando decidir.".to_string(),
                };
                let branch_note = base_branch.map(|b| format!(" (branch base: {b})")).unwrap_or_default();
                self.reply(
                    msg.source,
                    &msg.user_id,
                    &format!(
                        "📬 Issue criada: {o}/{r}#{} \"{}\"{branch_note}{note}\n{}",
                        issue.number, issue.title, issue.html_url
                    ),
                )
                .await;
            }
            Err(e) => self.reply(msg.source, &msg.user_id, &format!("⚠ Falha ao criar a issue: {e:#}")).await,
        }
        Ok(())
    }

    /// Lista as branches do repo como botões (mais "📌 Usar padrão") pro passo de
    /// branch base do assistente — melhor que pedir pra digitar de cor, já que a
    /// branch precisa existir mesmo. Se a chamada falhar (Forgejo antigo, rede
    /// etc.), cai pro pedido por texto livre de sempre; o `Pending::IssueBranch`
    /// continua aceitando uma branch digitada à mão nos dois casos.
    async fn ask_branch(&self, msg: &Incoming, o: &str, r: &str) {
        let f = self.forgejo.as_ref().expect("checado antes de chamar ask_branch");
        let owner = f.owner.as_ref().expect("checado antes de chamar ask_branch");
        match owner.list_branches(o, r).await {
            Ok(branches) if !branches.is_empty() => {
                let mut options: Vec<(String, String)> = vec![("📌 Usar padrão".to_string(), "-".to_string())];
                options.extend(branches.into_iter().take(30).map(|b| (format!("🌿 {}", b.name), b.name)));
                self.pick(msg.source, &msg.user_id, "Qual branch base?", &options).await;
            }
            _ => {
                self.reply(
                    msg.source,
                    &msg.user_id,
                    "🌱 Branch base? (nome de uma branch já existente, ou \"-\" pra usar a padrão)",
                )
                .await;
            }
        }
    }

    /// Resolve o ID da label do agente e adiciona na issue — usado tanto pelo
    /// `/assign` numa linha só quanto pelo passo final do assistente.
    async fn assign_and_reply(&self, msg: &Incoming, o: &str, r: &str, n: u64, agent_name: &str) {
        let Some(agent) = self.cfg.agents.iter().find(|a| a.name == agent_name) else {
            self.reply(
                msg.source,
                &msg.user_id,
                &format!(
                    "Agente \"{agent_name}\" não existe. Agentes: {}",
                    self.cfg.agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
                ),
            )
            .await;
            return;
        };
        let f = self.forgejo.as_ref().expect("checado antes de chamar assign_and_reply");
        let owner = f.owner.as_ref().expect("checado antes de chamar assign_and_reply");
        let labels = match owner.list_labels(o, r).await {
            Ok(l) => l,
            Err(e) => {
                self.reply(msg.source, &msg.user_id, &format!("⚠ Falha ao ler labels de {o}/{r}: {e:#}")).await;
                return;
            }
        };
        let Some(label) = labels.iter().find(|l| l.name == agent.label) else {
            self.reply(
                msg.source,
                &msg.user_id,
                &format!(
                    "⚠ Label \"{}\" não existe em {o}/{r}. Crie-a no Forgejo antes (Issues → Labels).",
                    agent.label
                ),
            )
            .await;
            return;
        };
        let note = match owner.add_issue_labels(o, r, n, &[label.id]).await {
            Ok(()) => format!(
                "🏷️ Label \"{}\" adicionada à issue {o}/{r}#{n}; {} vai pegar em breve.",
                agent.label, agent.name
            ),
            Err(e) => format!("⚠ Falha ao adicionar a label: {e:#}"),
        };
        self.reply(msg.source, &msg.user_id, &note).await;
    }

    /// Pede a mudança no PR e, se a branch for de um agente ativo, o aciona de
    /// volta — usado tanto pelo `/changes` numa linha só quanto pelo passo final
    /// do assistente (texto digitado em resposta ao botão "📝 Pedir ajuste").
    async fn run_changes(self: Arc<Self>, msg: &Incoming, o: &str, r: &str, n: u64, text: &str) -> anyhow::Result<()> {
        let f = self.forgejo.as_ref().expect("checado antes de chamar run_changes");
        let owner = f.owner.as_ref().expect("checado antes de chamar run_changes");
        let pr = owner.get_pull(o, r, n).await?;
        owner.review_pull(o, r, n, ReviewEvent::RequestChanges, text).await?;

        match pr.head.as_ref().and_then(|h| parse_agent_branch(&h.name)) {
            Some((agent, issue)) if f.agents.contains_key(agent) => {
                let item = WorkItem {
                    owner: o.to_string(),
                    repo: r.to_string(),
                    issue,
                    agent: agent.to_string(),
                    title: pr.title.clone(),
                    mode: WorkMode::Code,
                    review_branch: None,
                    base_branch: pr.base.as_ref().map(|b| b.name.clone()),
                };
                self.reply(
                    msg.source,
                    &msg.user_id,
                    &format!("📝 Pedido de ajuste registrado. {agent} vai voltar ao PR #{n}."),
                )
                .await;
                let goal = format!("{}\n\nAjustes pedidos na revisão do PR #{n}:\n{text}", pr.title);
                self.clone().dispatch_work(item, goal, &msg.user_id).await;
            }
            _ => {
                self.reply(
                    msg.source,
                    &msg.user_id,
                    &format!("📝 Pedido registrado no PR #{n} (não é um branch de agente ativo)."),
                )
                .await
            }
        }
        Ok(())
    }

    /// Completa o passo que faltava num assistente de botão com a mensagem de
    /// texto livre que acabou de chegar. Chamado por `Maker::handle_inner` quando
    /// a sessão tinha um [`Pending`] em aberto.
    pub(super) async fn resolve_pending(
        self: Arc<Self>,
        msg: &Incoming,
        pending: Pending,
        text: &str,
    ) -> anyhow::Result<()> {
        let text = text.trim();
        match pending {
            Pending::IssueTitle { owner, repo, agent } => {
                if text.is_empty() {
                    self.pending
                        .lock()
                        .expect("lock")
                        .insert(msg.session_key(), Pending::IssueTitle { owner, repo, agent });
                    self.reply(msg.source, &msg.user_id, "O título não pode ficar vazio — manda de novo.").await;
                    return Ok(());
                }
                // O agente trabalha com base na DESCRIÇÃO, não no título — ainda falta
                // perguntar ela antes de criar a issue de verdade.
                let title = text.to_string();
                self.pending
                    .lock()
                    .expect("lock")
                    .insert(msg.session_key(), Pending::IssueBody { owner, repo, agent, title });
                self.reply(
                    msg.source,
                    &msg.user_id,
                    "📝 Descreva a issue pro agente entender o que fazer (ou responda \"-\" pra deixar sem descrição).",
                )
                .await;
                Ok(())
            }
            Pending::IssueBody { owner, repo, agent, title } => {
                let body = if text.is_empty() || text == "-" { String::new() } else { text.to_string() };
                self.ask_branch(msg, &owner, &repo).await;
                self.pending
                    .lock()
                    .expect("lock")
                    .insert(msg.session_key(), Pending::IssueBranch { owner, repo, agent, title, body });
                Ok(())
            }
            Pending::IssueBranch { owner, repo, agent, title, body } => {
                let base_branch = if text.is_empty() || text == "-" { None } else { Some(text) };
                self.create_issue_and_reply(msg, (&owner, &repo), agent.as_deref(), &title, &body, base_branch).await
            }
            Pending::AssignNumber { owner, repo } => {
                match text.trim_start_matches('#').parse::<u64>() {
                    Ok(n) => {
                        let cmd = format!("/assign {owner}/{repo} {n}");
                        let options = self.agent_options(&cmd, false);
                        self.pick(msg.source, &msg.user_id, &format!("Qual agente pra issue #{n}?"), &options).await;
                    }
                    Err(_) => {
                        self.pending
                            .lock()
                            .expect("lock")
                            .insert(msg.session_key(), Pending::AssignNumber { owner, repo });
                        self.reply(msg.source, &msg.user_id, "Manda só o número da issue (ex.: 11).").await;
                    }
                }
                Ok(())
            }
            Pending::ChangesText { owner, repo, number } => {
                if text.is_empty() {
                    self.pending
                        .lock()
                        .expect("lock")
                        .insert(msg.session_key(), Pending::ChangesText { owner, repo, number });
                    self.reply(msg.source, &msg.user_id, "Diga o que precisa mudar — não pode ficar vazio.").await;
                    return Ok(());
                }
                self.run_changes(msg, &owner, &repo, number, text).await
            }
        }
    }
}

fn models_text(m: &Maker) -> String {
    let roles: Vec<String> = m
        .router
        .describe()
        .into_iter()
        .map(|r| format!("{} {} → {}", if r.ready { "🟢" } else { "🔴" }, r.role, r.model))
        .collect();
    let models: Vec<String> = m
        .router
        .describe_models()
        .into_iter()
        .map(|mi| {
            let icon = match mi.state {
                oreia_llm::ModelState::Ok => "🟢",
                oreia_llm::ModelState::Unreachable => "🔴",
                oreia_llm::ModelState::NotConfigured => "⚪",
            };
            if mi.state == oreia_llm::ModelState::NotConfigured {
                format!("{icon} {}: {}", mi.key, mi.state.as_str())
            } else {
                format!("{icon} {}: {} — {} ({})", mi.key, mi.state.as_str(), mi.model, mi.base_url)
            }
        })
        .collect();
    format!("Modelos por papel:\n{}\n\nModelos configurados:\n{}", roles.join("\n"), models.join("\n"))
}
