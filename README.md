# 🤖 O.R.E.I.A.S.E.C.A

> **Orquestrador de Recursos e Experimentos em IA para Sistemas de Execução e Controle Automatizado**

O **OREIASECA** é um agente autônomo de desenvolvimento de software e orquestração de tarefas, equipado com execução local em sandbox, múltiplos canais de comunicação e controle de aprovação humana (*Human-in-the-Loop*) inteligente para gatilhos automatizados.

---

## 🚀 Funcionalidades Principais

* **Roteamento Inteligente de Modelos (`ModelRouter`)**:
  * Divisão flexível de papéis para processamento: utilize um modelo avançado para **Planejamento** (ex: *Gemini 1.5/3.5 Pro* via Gemini API), um modelo otimizado para **Execução** (ex: *Nemotron* via OpenRouter) e um modelo local leve para **Uso Direto / Fallback** (ex: *Qwen 2.5/3.5* no LM Studio).
* **Entradas Multicanal Integradas**:
  * 💻 **Terminal**: Interação direta por linha de comando local.
  * 💬 **Telegram**: Controle remoto via bot com whitelist de usuários permitidos.
  * 🌐 **REST API (Webhooks/Cron)**: Endpoint HTTP seguro (`POST /api/message`) para agendamento automático de tarefas por sistemas externos.
* **Aprovação Cross-Canal (*Human-in-the-Loop*)**:
  * Tarefas solicitadas via fontes automatizadas (REST) são planejadas, colocadas em estado `planned` e congeladas.
  * O plano de ação é transmitido para os canais humanos ativos (Terminal e Telegram).
  * Qualquer operador humano autorizado pode analisar o plano e aprovar (`/approve <taskId>`) ou rejeitar (`/reject <taskId>`) a partir de qualquer canal disponível.
* **Fila de Tarefas Robusta (`TaskQueue`)**:
  * Controle estrito de concorrência serializada ou concorrente.
  * Tratamento de timeout para evitar travamento de agentes por prompts lentos.
  * Recuperação automática de tarefas interrompidas na reinicialização do sistema via comando `/retry`.
* **Persistência Graceful**:
  * Interrupções via `SIGINT` ou `SIGTERM` salvam instantaneamente o estado da fila e as sessões em disco de forma assíncrona.
* **Pronto para Docker**:
  * Containerizado com suporte a volumes para manter seu histórico de tarefas (`tasks/`) e arquivos gerados (`workspace/`) persistentes no host.
* **Modo Debug com Métricas de Tokens** *(novo)*:
  * Ative com `npm run debug` para exibir, a cada chamada ao modelo, os tokens de entrada (↑), saída (↓) e total (Σ).
  * Barra de progresso colorida mostra o consumo do contexto em relação ao limite configurado: 🟢 verde `< 60%`, 🟡 amarelo `< 85%`, 🔴 vermelho `≥ 85%`.
* **Resumo de Contexto por LLM** *(novo)*:
  * **Automático**: quando a janela de contexto de uma sessão direta está prestes a estourar, o histórico é comprimido semanticamente via LLM antes de cada resposta.
  * **Sob demanda**: use `/resumir` a qualquer momento para compactar e salvar a sessão atual.
  * Os resumos são persistidos em `resumos-contextos/` como arquivos `.md` e podem ser recarregados em sessões futuras com `/carregar <arquivo>`.

---

## 🛠️ Arquitetura do Sistema

```
                    POST /api/message
Agendador (Cron) ───────────────────→ RestInput (HTTP)
                                          │
                                          ▼
                                    Maker.planTask()
                                          │
                                Origem externa ("rest")?
                                   ┌──────┴──────┐
                                  SIM           NÃO (Terminal/Telegram)
                                   │             │
                                   ▼             ▼
                           (Estado: Planned)   Execução Direta
                         Broadcast de Aprovação
                         ┌─────────┴─────────┐
                         ▼                   ▼
                  Terminal (CLI):      Telegram (Bot):
                  🔔 Pendente           🔔 Pendente
                  /approve task-abc     /approve task-abc
                         │                   │
                         └─────────┬─────────┘
                                   ▼
                            Aprovação Humana
                                   │
                                   ▼
                           TaskQueue (Executa)
```

---

## ⚙️ Configuração do Ambiente (`.env`)

Renomeie o arquivo `.env.example` para `.env` e preencha as suas chaves e credenciais:

```bash
# Telegram Bot (Obtenha via @BotFather)
TELEGRAM_TOKEN=seu_token_aqui
TELEGRAM_ALLOWED=seu_chat_id_aqui  # Lista separada por vírgula

# APIs de Modelos de Linguagem (Opcionais se utilizar LM Studio local)
GEMINI_KEY=sua_chave_gemini_aqui
OPENROUTER_KEY=sua_chave_openrouter_aqui

# REST API (Porta do webhook e token de autenticação)
REST_PORT=3120
REST_API_KEY=sua_chave_secreta_aqui # Deixe vazio para desabilitar autenticação externa

# LM Studio Base URL (Endereço de rede do seu LM Studio)
LMSTUDIO_URL=http://localhost:1234  # Ex: http://192.168.2.140:1234
```

> 💡 **Nota sobre Docker**: Dentro de um container Docker, `localhost` aponta para o próprio container. Se o LM Studio estiver rodando no seu host físico local, altere o `LMSTUDIO_URL` para `http://host.docker.internal:1234` ou utilize o IP de rede da máquina (ex: `http://192.168.0.100:1234`).

---

## 📦 Como Executar

### Método 1: Localmente (Node.js)

1. Certifique-se de usar o **Node.js v20** ou superior.
2. Instale as dependências:
   ```bash
   npm install
   ```
3. Inicie o agente no modo desejado:
   ```bash
   # Produção
   npm start

   # Desenvolvimento (com hot-reload automático)
   npm run dev

   # Depuração — exibe métricas de tokens e barra de progresso a cada chamada ao modelo
   npm run debug
   ```

   No modo `debug`, o terminal exibe para cada chamada ao modelo:
   ```
   [tokens] papel=executor  modelo=qwen2.5-7b
            ↑ entrada:  4.201  ↓ saída:    511  Σ total:  4.712
            [████████████████████████░░░░░░░░░░░░░░░░] 57.5% (4.712 / 8.192 tokens)
   ```

### Método 2: Via Docker Compose (Recomendado)

Utilize o Docker Compose para subir a aplicação em segundo plano com persistência automática de volumes:

```bash
# Constrói a imagem e sobe os serviços
docker compose up --build -d

# Visualiza os logs em tempo real
docker compose logs -f
```

O container mapeia três pastas locais importantes:
* `./tasks`: Armazena o banco de dados de tarefas e sessões ativas.
* `./workspace`: Pasta sandbox onde o agente cria, analisa e manipula códigos.
* `./resumos-contextos`: Resumos de sessão gerados pelo agente (`.md`).

#### 🗂️ Organização em Diretório Pai (Ex: `docker-compose.yml` acima da pasta do projeto)

Se você preferir manter o arquivo `docker-compose.yml` no diretório **pai** (um nível acima da pasta do agente, como `OREIA-SECA/Dockerfile` ou `maker-agent/Dockerfile`), configure o seu `docker-compose.yml` assim:

```yaml
version: '3.8'

services:
  maker-agent:
    build:
      context: ./maker-agent  # Substitua por ./OREIA-SECA se a pasta tiver esse nome
      dockerfile: Dockerfile
    container_name: maker-agent
    restart: unless-stopped
    ports:
      - "${REST_PORT:-3120}:3120"
    env_file:
      - ./maker-agent/.env
    volumes:
      - ./maker-agent/tasks:/app/tasks
      - ./maker-agent/workspace:/app/workspace
```

---

## 🧪 Executando os Testes

O projeto conta com uma suíte de testes robusta abrangendo testes unitários, testes de boot, controle de custos, integrações de fila e o fluxo completo de aprovação cross-canal.

Para rodar todos os testes localmente:
```bash
node test/run.js
```

Se o seu sistema permitir execução de scripts no PowerShell/Bash, você também pode usar o alias padrão do npm:
```bash
npm test
```

---

## 💬 Comandos Disponíveis (Terminal / Telegram)

Quando estiver interagindo com o agente pelo bot do Telegram ou pelo terminal CLI, você pode usar os seguintes comandos:

### Controle Geral

* `/status`: Retorna o status atual do agente, número de tarefas concluídas, falhas e tarefas ativas.
* `/queue`: Exibe a fila atual de tarefas ativas e agendadas em andamento.
* `/tasks`: Lista as últimas 5 tarefas do usuário com status e ID.
* `/skills`: Lista todas as ações e skills dinâmicas disponíveis. Use `/skills reload` para recarregar sem reiniciar.

### Gerenciamento de Tarefas

* `/approve <taskId>`: Aprova uma tarefa em espera solicitada via API externa, liberando-a para execução na fila.
* `/reject <taskId>`: Rejeita e aborta uma tarefa em espera, cancelando sua execução.
* `/retry <taskId>`: Retoma a execução de uma tarefa que tenha sido interrompida por desligamento abrupto ou erro temporário.
* `/pending`: Lista todas as aprovações pendentes no momento.

### Resumo de Contexto *(novo)*

* `/resumir`: Comprime o histórico da sessão atual em um resumo semântico via LLM, salva em `resumos-contextos/<sessao>_<timestamp>.md` e substitui a sessão pelo resumo compacto.
* `/resumos`: Lista os arquivos de resumo disponíveis em `resumos-contextos/` (até 10 mais recentes).
* `/carregar <arquivo.md>`: Carrega um resumo salvo e o injeta como contexto inicial da sessão atual, permitindo retomar conversas anteriores.

**Fluxo típico de uso do resumo:**
```
# Durante uma conversa longa:
/resumir
# → ✅ Contexto resumido e salvo!
# → 📄 Arquivo: terminal_local_2026-06-03T21-00-00.md

# Em uma nova sessão (ou outro canal):
/resumos
# → 📁 Resumos disponíveis: ...
/carregar terminal_local_2026-06-03T21-00-00.md
# → ✅ Contexto carregado! Agora posso continuar de onde paramos.
```

> 💡 **Compressão automática**: se o contexto estourar durante uma conversa, o agente comprime e salva automaticamente sem precisar do comando `/resumir`.

---

## 🌐 Endpoints REST (API)

A API REST do OREIASECA permite que scripts, tarefas agendadas (Cron) e gatilhos de CI/CD deleguem tarefas para o agente:

### 1. Enviar Tarefa
* **Endpoint**: `POST /api/message`
* **Headers**: 
  * `Authorization`: `Bearer <REST_API_KEY>` (se a chave API estiver configurada no `.env`)
  * `Content-Type`: `application/json`
* **Corpo da Requisição**:
  ```json
  {
    "text": "crie um arquivo de script para calcular a média de valores no diretório workspace",
    "userId": "sistema-cron"
  }
  ```
* **Resposta**:
  ```json
  {
    "status": "planned",
    "taskId": "task-ced92eb2",
    "message": "📋 Tarefa recebida via rest e aguardando aprovação humana."
  }
  ```

### 2. Status da Fila
* **Endpoint**: `GET /api/status`
* **Resposta**: Retorna estatísticas de execução, concorrência ativa e itens na fila.

### 3. Log Recente
* **Endpoint**: `GET /api/log`
* **Resposta**: Retorna as últimas respostas e prints de interação do agente em formato JSON.

---

## 📄 Licença

Este projeto está licenciado sob a licença descrita nas políticas de propriedade e controle do orquestrador local.
