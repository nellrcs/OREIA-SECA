COMPOSE := docker compose
ANSWERS ?= setup.answers.toml

.PHONY: help setup setup-answers bootstrap up llm-setup down logs ps status check build test lint fmt ci-up preview-up executors-build

help: ## mostra os comandos
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*## "}{printf "  \033[36m%-12s\033[0m %s\n",$$1,$$2}'

setup: ## configuração inicial interativa, no terminal (Telegram, LLM, Forgejo)
	./oreia setup

setup-answers: ## configuração não interativa a partir de um arquivo (ANSWERS=setup.answers.toml)
	ANSWERS=$(ANSWERS) ./oreia setup-answers

bootstrap: ## cria .env, usuários, tokens, repos, labels e webhook no Forgejo
	./scripts/bootstrap.sh

up: ## sobe tudo (Forgejo + orquestrador)
	./oreia up

llm-setup: ## só o LLM externo (outra máquina, LM Studio, OpenRouter...); grava no .env
	./oreia llm-setup

down: ## para tudo
	$(COMPOSE) down

logs: ## acompanha os logs do orquestrador
	$(COMPOSE) logs -f oreia-seca

ps: ## status dos serviços (docker compose ps puro)
	$(COMPOSE) ps

status: ## o que está rodando, com health check (containers + /health + Forgejo)
	./oreia status

check: ## valida o oreia-seca.toml
	./oreia check

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
