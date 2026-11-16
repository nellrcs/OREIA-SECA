# Documentação

## Começar

| | |
|---|---|
| [Instalação](instalacao.md) | Requisitos por sistema, o passo a passo de `./oreia install`, a instalação automatizada (script, CI, agente de IA), atualizar e recomeçar do zero |
| [Painel](painel.md) | A tela do terminal (`./oreia`): status, saúde e latência de cada serviço, uso de tokens e custo, e as áreas de configuração |
| [Solução de problemas](problemas.md) | O que fazer quando algo não instala, não sobe ou não responde |

## Usar

| | |
|---|---|
| [Agentes](agentes.md) | Como criar um agente novo e como eles se coordenam |
| [Modelos locais](modelos-locais.md) | Ollama, LM Studio, llama.cpp, APIs e a estratégia híbrida |
| [Executores e escalada](executores.md) | `builtin` vs. CLIs externas, e a escada de escalada por papel |
| [px0](px0.md) | Revisão de pull requests no navegador |
| [Servidor MCP](mcp.md) | P.O. por IA e desenvolvedor sob demanda, pelo Claude Code/Desktop |
| [Rede Docker externa](rede-externa.md) | Anexar forgejo/px0/oreia-seca a uma rede externa (cloudflared, Tailscale, proxy...), em vez de publicar portas |

## Referência

| | |
|---|---|
| [Configuração](configuracao.md) | `oreia-seca.toml` e `.env`, campo a campo |
| [Arquitetura](arquitetura.md) | Componentes e o caminho de uma mensagem |
| [Segurança](seguranca.md) | O que protege o quê, e o que você ainda precisa decidir |
| [Benchmark de hardware](benchmark.md) | Como medir CPU e memória reais na sua máquina |

## Projeto

| | |
|---|---|
| [Roadmap](roadmap.md) | O que já funciona e os próximos passos |
| [Vindo do OREIA-SECA](vindo-do-oreia-seca.md) | O que mudou em relação à versão anterior, e por quê |
| [Plano do MCP](mcp-plano/00-checklist.md) | O registro, etapa por etapa, de como o servidor MCP foi construído |
