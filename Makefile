COMPOSE := docker compose
ANSWERS ?= setup.answers.toml

.PHONY: help setup setup-web setup-answers bootstrap up llm-setup down logs ps status check build test lint fmt ci-up preview-up executors-build

help: ## mostra os comandos
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*## "}{printf "  \033[36m%-12s\033[0m %s\n",$$1,$$2}'

setup: ## configuração inicial interativa, no terminal (Telegram, LLM, Forgejo)
	cargo run -q -p oreia-seca -- setup

setup-web: ## o mesmo setup, numa página local (http://127.0.0.1:3121)
	cargo run -q -p oreia-seca -- setup --web

setup-answers: ## configuração não interativa a partir de um arquivo (ANSWERS=setup.answers.toml)
	cargo run -q -p oreia-seca -- setup --answers $(ANSWERS)

bootstrap: ## cria .env, usuários, tokens, repos, labels e webhook no Forgejo
	./scripts/bootstrap.sh

up: ## sobe tudo (Forgejo + orquestrador)
	$(COMPOSE) up -d --build

llm-setup: ## só o LLM externo (outra máquina, LM Studio, OpenRouter...); grava no .env
	cargo run -q -p oreia-seca -- setup llm

down: ## para tudo
	$(COMPOSE) down

logs: ## acompanha os logs do orquestrador
	$(COMPOSE) logs -f oreia-seca

ps: ## status dos serviços (docker compose ps puro)
	$(COMPOSE) ps

status: ## o que está rodando, com health check (containers + /health + Forgejo)
	cargo run -q -p oreia-seca -- status

check: ## valida o oreia-seca.toml
	cargo run -q -p oreia-seca -- --check --config oreia-seca.toml

ci-up: ## sobe o runner do Forgejo Actions (precisa de runner/config.yml)
	$(COMPOSE) --profile ci up -d

preview-up: ## sobe o Traefik para os previews por PR
	$(COMPOSE) --profile preview up -d

executors-build: ## builda as imagens dos containers por executor (opencode, codex)
	docker build -f executors/opencode.Dockerfile -t oreia-seca-exec-opencode .
	docker build -f executors/codex.Dockerfile -t oreia-seca-exec-codex .

build: ## compila em modo release
	cargo build --release

test: ## roda os testes
	cargo test --workspace

lint: ## formatação e clippy
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

fmt: ## formata o código
	cargo fmt --all
