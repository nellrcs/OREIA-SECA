# Benchmark e hardware recomendado

Este documento é pra responder uma pergunta concreta: **o orquestrador (Forgejo +
Postgres + oreia-seca, e opcionalmente um executor externo) roda bem numa
máquina pequena, sem GPU — ex. um i5 com 8 GB de RAM —, enquanto o LLM fica
numa máquina separada com GPU?** As faixas abaixo são ponto de partida; use
`scripts/benchmark.sh` (seção [Medindo na sua máquina](#medindo-na-sua-máquina))
pra confirmar com números reais do seu ambiente.

## Topologia recomendada

```
┌─────────────────────────┐        LAN         ┌──────────────────────────┐
│ Máquina com GPU          │◄───────────────────┤ Máquina pequena (i5/8GB) │
│ Ollama / LM Studio /     │   HTTP (LLM calls)  │ docker compose:          │
│ llama.cpp / vLLM         │                     │  - postgres              │
└─────────────────────────┘                     │  - forgejo               │
                                                 │  - oreia-seca            │
                                                 │  (+ opencode/aider/goose │
                                                 │   só quando um degrau de │
                                                 │   escalada os usa)       │
                                                 └──────────────────────────┘
```

Isso já é o caminho documentado em [Modelos locais](modelos-locais.md#rodando-o-modelo-em-outra-máquina):
o `docker-compose.yml` deste projeto **nunca** sobe um LLM junto, então a
máquina pequena não precisa de GPU nenhuma — ela só fala HTTP com quem tem.

Ponto importante que não é óbvio: um executor **externo** (`opencode`,
`aider`, `goose`, `codex`, quando configurado num degrau de
[escalada](executores.md)) roda como processo filho do `oreia-seca`, ou seja,
**na máquina pequena**, não na máquina com GPU — só a chamada HTTP pro modelo
é que viaja até lá. O mesmo vale pro `test_cmd` da escalada e pra qualquer
build/lint/teste do repositório que o agente está editando: tudo isso
acontece na máquina pequena, porque é onde o clone do repositório vive.

## Orçamento de memória (i5 + 8 GB, estimativas de partida)

| Componente | RAM (repouso) | Observação |
|---|---|---|
| SO (Linux headless) | ~200–400 MB | Debian/Ubuntu server, sem interface gráfica |
| Docker (`dockerd`/`containerd`) | ~100–200 MB | overhead fixo, fora da contagem por container |
| `postgres` | ~50–100 MB | Forgejo para um usuário/time pequeno é uma carga leve pro Postgres |
| `forgejo` | ~150–300 MB | binário Go, instância pequena |
| `oreia-seca` | ~20–60 MB | binário Rust assíncrono, sem modelo embutido — é só o orquestrador |
| **Total em repouso** | **~0,5–1 GB** | sobra bastante dos 8 GB antes de qualquer tarefa rodar |

**O risco real não é o orquestrador — é o que ele manda rodar.** Dois
consumidores variáveis, que só aparecem durante uma tarefa:

1. **O executor externo escolhido**, se algum degrau de escalada usa um:
   - `opencode` (Node.js + AI SDK): ~150–300 MB, só durante a tentativa
   - `aider` (Python + litellm): ~150–300 MB, só durante a tentativa
   - `goose` (binário Rust): tende a ser mais leve, <100–150 MB
   - `builtin` (o loop de ações deste projeto): **não soma nada** — é código
     dentro do próprio processo `oreia-seca` já contado acima, só mais
     chamadas HTTP pro LLM.
2. **O `test_cmd` da escalada e qualquer build do repositório sendo editado**
   — isso depende inteiramente do PROJETO alvo, não do OREIA-SECA: um
   `cargo build` de um crate Rust médio pode usar 1–3 GB sozinho; um
   `npm install` + bundler (webpack/vite) facilmente 0,5–1,5 GB; uma suíte
   Python/pytest costuma ser bem mais leve (<300 MB). Isso é o fator que mais
   provavelmente aperta 8 GB, não o orquestrador em si.

## Recomendações concretas pro seu caso (8 GB RAM, i5, um agente por vez)

Como o cenário é **um agente por vez, custo mínimo** (não throughput), não há
concorrência disputando RAM — o risco é só um pico isolado passar dos 8 GB
durante uma única tarefa. Dá pra ser bem conservador:

- **Prefira o executor `builtin` como primeiro degrau** da escalada, e deixe
  um externo (ex. `opencode`) só como degrau mais caro, tentado apenas se o
  `builtin` falhar a verificação — você só paga a RAM extra do Node/Python
  quando realmente precisar, e só pelos segundos/minutos daquela tentativa.
  Veja [Executores e escalada](executores.md#escalada-por-papel).
- **Adicione 2–4 GB de swap** nessa máquina (`fallocate`/`mkswap`/`swapon`, ou
  o equivalente da sua distro). Com um agente só por vez, swapping ocasional
  custa tempo, não throughput — é uma rede de segurança contra um pico de
  build que passaria dos 8 GB, em troca de ficar lento naquele momento em vez
  de o processo morrer com OOM.
- **Não suba os perfis `ci`/`preview` nessa máquina sem necessidade.**
  `docker-compose.yml` só os inicia com `--profile ci`/`--profile preview`
  explícito (não sobem com `docker compose up -d` normal) — mas se você usa
  Forgejo Actions, o runner roda o build do repositório alvo de novo, mesma
  pressão de memória do `test_cmd` acima, agora em paralelo com uma tarefa do
  agente. Numa máquina de 8 GB, rode CI e agente em momentos separados, ou
  mantenha esses perfis desligados.
- **SSD, não HD.** O workdir do agente faz vários `git clone`/`checkout` por
  tarefa (um por degrau de escalada que tentar); em HD isso é a diferença
  entre segundos e minutos por tentativa.
- **Rede cabeada entre as duas máquinas, se possível.** Toda fase faz pelo
  menos uma chamada HTTP pro LLM na máquina com GPU; com escalada e retries
  isso pode ser várias por tarefa — Wi-Fi instável soma latência em cada uma.
- Se o `test_cmd` de um papel for pesado (build completo do projeto), considere
  trocar por algo mais focado (ex. só os testes do módulo tocado, não a suíte
  inteira) — reduz tanto o tempo quanto o pico de memória daquele degrau.

## Medindo na sua máquina

`scripts/benchmark.sh` não precisa de nada além de `docker` (e `jq`,
opcional) — roda direto na máquina pequena, contra o `docker compose` já de
pé:

```bash
# 1. chão em repouso, sem tarefa nenhuma rodando
scripts/benchmark.sh baseline 30

# 2. abra/rotule a issue de teste no Forgejo, DEPOIS rode:
scripts/benchmark.sh task 600
# ... Ctrl+C quando o PR aparecer
```

O `baseline` mostra o pico de memória de `postgres`/`forgejo`/`oreia-seca` em
repouso — compare com a tabela acima pra calibrar as estimativas pro seu
ambiente real (versão do Forgejo, tamanho do repositório, etc.). O `task`
mostra o pico de CPU/memória **durante** a tarefa (incluindo o executor
externo e o `test_cmd`, se algum degrau rodar) e, com `jq` instalado, cruza a
janela de tempo com `data/escalation.jsonl` pra listar executor, modelo,
duração, tokens e custo de cada tentativa feita naquela janela — os mesmos
dados que alimentam [`/stats`](executores.md#stats-papel), só filtrados pra
essa tarefa.

Rode `baseline` de novo depois de qualquer mudança relevante (versão do
Forgejo, do `oreia-seca`, do `oreia-seca.toml`) pra manter a referência
atualizada, e `task` com cada combinação de executor que você pretende usar
de verdade (`builtin` vs. `opencode`, por exemplo) pra comparar o custo real
de cada um nessa máquina, não só em teoria.

## Quando migrar de hardware

Com um agente por vez, o sinal de que 8 GB deixou de bastar não é "ficou
devagar" (swap ocasional é esperado e aceitável nesse cenário) — é **o
processo `oreia-seca` ou o `test_cmd` sendo matado pelo OOM killer** (confira
`dmesg | grep -i oom` depois de uma falha sem explicação) ou o `baseline`
sozinho já perto do teto antes de qualquer tarefa rodar. Se isso acontecer
mesmo depois de aplicar as recomendações acima (swap, `builtin` como primeiro
degrau, `test_cmd` mais focado), a RAM da máquina pequena é o próximo ponto a
aumentar — o CPU (i5) tende a segurar bem esse perfil de carga, já que nada
aqui é CPU-bound de verdade (o trabalho pesado é a chamada de rede pro LLM, que
roda do lado da GPU).

Para a máquina com GPU — fora do escopo deste documento, que é sobre a máquina
pequena —, a regra grosseira de VRAM por tamanho de modelo já está em
[Modelos locais](modelos-locais.md#o-que-considerar-ao-escolher).
