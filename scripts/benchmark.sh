#!/usr/bin/env bash
# Mede o custo real de hardware do OREIA-SECA nesta máquina: CPU/memória de
# cada container (via `docker stats`) em repouso e durante uma tarefa, cruzado
# com data/escalation.jsonl (duração/tokens/custo de cada tentativa). Pensado
# pra responder uma pergunta concreta: esta máquina (ex. i5 + 8GB RAM, sem
# GPU, com o LLM rodando em outra máquina — veja docs/modelos-locais.md) é
# suficiente pra rodar Forgejo + Postgres + oreia-seca + um executor externo
# (opencode/aider/goose), um agente por vez? Veja docs/benchmark.md.
#
# Uso (na raiz do repo, com `docker compose up -d` já rodando):
#
#   scripts/benchmark.sh baseline [segundos=30]
#     Amostra em repouso, sem tarefa nenhuma rodando: o "chão" sempre ocupado
#     (Postgres + Forgejo + oreia-seca, antes de qualquer agente trabalhar).
#
#   scripts/benchmark.sh task [segundos=600]
#     Abra/rotule a issue de teste ANTES de rodar este comando. Amostra a
#     cada 2s; pare com Ctrl+C quando o PR aparecer, ou deixe rodar até o
#     timeout. Ao final, cruza a janela de tempo amostrada com
#     data/escalation.jsonl (se o jq estiver instalado) pra mostrar
#     executor/modelo/duração/tokens/custo de cada tentativa feita durante a
#     amostra.
#
# Saída: CSV bruto em scripts/.benchmark-<epoch>.csv e um resumo no terminal.
# Requer: docker (com `docker compose` v2) e GNU date (padrão em qualquer
# distro Linux). jq é opcional, só pro cruzamento com escalation.jsonl.
set -euo pipefail
cd "$(dirname "$0")/.."

INTERVAL=2

usage() {
    echo "uso: $0 baseline [segundos] | $0 task [segundos]" >&2
    exit 1
}

[ $# -ge 1 ] || usage
mode="$1"
case "$mode" in
    baseline | task) ;;
    *) usage ;;
esac
seconds="${2:-$([ "$mode" = task ] && echo 600 || echo 30)}"

ids=$(docker compose ps -q)
if [ -z "$ids" ]; then
    echo "nenhum container deste projeto está rodando — rode 'docker compose up -d' primeiro" >&2
    exit 1
fi

# docker stats devolve algo como "123.4MiB" / "1.2GiB" / "456kB" — normaliza
# pra MiB (aproximado, suficiente pra comparar containers entre si e entre
# execuções, não precisa ser exato ao byte).
to_mib() {
    awk -v s="$1" 'BEGIN{
        val = s + 0;
        if (s ~ /GiB/)      v = val * 1024;
        else if (s ~ /GB/)  v = val * 953.674;
        else if (s ~ /MiB/) v = val;
        else if (s ~ /MB/)  v = val * 0.953674;
        else if (s ~ /KiB/) v = val / 1024;
        else if (s ~ /kB/)  v = val / 1024 * 0.953674;
        else                 v = val / 1048576;
        printf "%.1f", v;
    }'
}

csv="scripts/.benchmark-$(date +%s).csv"
echo "ts,container,cpu_pct,mem_mib" >"$csv"

stop=0
trap 'stop=1' INT

start_ts=$(date +%s)
end_ts=$((start_ts + seconds))
echo "amostrando (${mode}) por até ${seconds}s, a cada ${INTERVAL}s — Ctrl+C pra parar antes — CSV: $csv"

while [ "$stop" -eq 0 ] && [ "$(date +%s)" -lt "$end_ts" ]; do
    now=$(date +%s)
    # shellcheck disable=SC2086
    docker stats --no-stream --format '{{.Name}}	{{.CPUPerc}}	{{.MemUsage}}' $ids | while IFS=$'\t' read -r name cpu mem; do
        mem_used="${mem%% / *}"
        cpu_num="${cpu%\%}"
        mem_mib=$(to_mib "$mem_used")
        echo "${now},${name},${cpu_num},${mem_mib}" >>"$csv"
    done
    sleep "$INTERVAL"
done
real_end_ts=$(date +%s)

echo
printf "%-28s %9s %9s %13s\n" "container" "cpu_avg%" "cpu_pico%" "mem_pico(MiB)"
awk -F',' 'NR>1{print $2}' "$csv" | sort -u | while IFS= read -r name; do
    awk -F',' -v name="$name" '
        $2==name { n++; cpusum+=$3; if($3+0>cpupeak) cpupeak=$3+0; if($4+0>mempeak) mempeak=$4+0; }
        END { if(n>0) printf "%-28s %8.1f%% %8.1f%% %13.1f\n", name, cpusum/n, cpupeak, mempeak; }
    ' "$csv"
done

elapsed=$((real_end_ts - start_ts))
echo
echo "tempo amostrado: ${elapsed}s"

if [ "$mode" = task ]; then
    log="volumes/oreia-seca/data/escalation.jsonl"
    if ! command -v jq >/dev/null 2>&1; then
        echo "jq não encontrado — pulando o cruzamento com $log (instale jq pra ver duração/tokens/custo por tentativa)"
    elif [ ! -f "$log" ]; then
        echo "$log ainda não existe (nenhuma tentativa de escalada registrada ainda)"
    else
        from=$(date -u -d "@$start_ts" +%Y-%m-%dT%H:%M:%S)
        to=$(date -u -d "@$real_end_ts" +%Y-%m-%dT%H:%M:%S)
        echo
        echo "tentativas registradas entre ${from}Z e ${to}Z:"
        jq -r --arg from "$from" --arg to "$to" '
            select(.at >= $from and .at <= $to) |
            [.role, .phase, .step, .executor, .model, .duration_ms, .prompt_tokens, .completion_tokens, .cost_usd] | @tsv
        ' "$log" | awk -F'\t' '
            BEGIN{printf "%-10s %5s %5s %-10s %-20s %10s %8s %8s %8s\n","role","fase","deg","executor","modelo","dur_ms","p_tok","c_tok","custo"}
            {printf "%-10s %5s %5s %-10s %-20s %10s %8s %8s %8s\n",$1,$2,$3,$4,$5,$6,$7,$8,$9; tdur+=$6; ptok+=$7; ctok+=$8; cost+=$9}
            END{printf "\nTOTAL: %dms, %d prompt tok, %d completion tok, $%.4f\n", tdur, ptok, ctok, cost}'
    fi
fi

echo
echo "CSV bruto: $csv"
