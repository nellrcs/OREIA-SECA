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

> 💡 **Nota sobre Docker**: Dentro de um container Docker, `localhost` aponta para o próprio container. Se o LM Studio estiver rodando no seu host físico local, altere o `LMSTUDIO_URL` para `http://host.docker.internal:1234` ou utilize o IP de rede da máquina (ex: `http://192.168.2.140:1234`).

---

## 📦 Como Executar

### Método 1: Localmente (Node.js)

1. Certifique-se de usar o **Node.js v20** ou superior.
2. Instale as dependências:
   ```bash
   npm install
   ```
3. Inicie o agente em modo de desenvolvimento ou produção:
   ```bash
   # Produção
   npm start

   # Desenvolvimento (com hot-reload automático)
   npm run dev
   ```

### Método 2: Via Docker Compose (Recomendado)

Utilize o Docker Compose para subir a aplicação em segundo plano com persistência automática de volumes:

```bash
# Constrói a imagem e sobe os serviços
docker compose up --build -d

# Visualiza os logs em tempo real
docker compose logs -f
```

O container mapeia duas pastas locais importantes:
* `./tasks`: Armazena o banco de dados de tarefas e sessões ativas.
* `./workspace`: Pasta sandbox onde o agente cria, analisa e manipula códigos.

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

* `/status`: Retorna o status atual do agente, número de tarefas concluídas, falhas e tarefas ativas.
* `/queue`: Exibe a fila atual de tarefas ativas e agendadas em andamento.
* `/approve <taskId>`: Aprova uma tarefa em espera solicitada via API externa, liberando-a para execução na fila.
* `/reject <taskId>`: Rejeita e aborta uma tarefa em espera, cancelando sua execução.
* `/retry <taskId>`: Retoma a execução de uma tarefa que tenha sido interrompida por desligamento abrupto ou erro temporário.

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
