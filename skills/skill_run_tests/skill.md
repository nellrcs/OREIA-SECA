# skill_run_tests

Detecta a stack do projeto no diretório atual e roda a suíte de testes adequada (cargo, npm, pytest, go).

## Parâmetros
- `filter` (string, opcional): filtro de nome de teste, quando a ferramenta suporta

## Código
```sh
set -e
if [ -f Cargo.toml ]; then cargo test ${P_FILTER:+"$P_FILTER"} 2>&1 | tail -60
elif [ -f package.json ]; then npm test --silent -- ${P_FILTER:+"$P_FILTER"} 2>&1 | tail -60
elif [ -f pyproject.toml ] || [ -f pytest.ini ] || [ -d tests ]; then python3 -m pytest -q ${P_FILTER:+-k "$P_FILTER"} 2>&1 | tail -60
elif [ -f go.mod ]; then go test ./... ${P_FILTER:+-run "$P_FILTER"} 2>&1 | tail -60
else echo "nenhuma suíte de testes reconhecida neste diretório"; exit 1
fi
```
