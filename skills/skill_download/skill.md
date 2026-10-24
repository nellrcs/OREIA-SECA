# skill_download

Baixa um arquivo da internet para o diretório da tarefa.

## Parâmetros
- `url` (string, obrigatório): endereço http(s) do arquivo
- `filename` (string, obrigatório): nome do arquivo de destino, sem barras (ex.: dados.csv)

## Código
```sh
case "$P_URL" in http://*|https://*) ;; *) echo "url precisa começar com http:// ou https://" >&2; exit 2;; esac
case "$P_FILENAME" in */*|..*|"") echo "filename inválido" >&2; exit 2;; esac
curl -fsSL --max-time 120 --max-filesize 104857600 "$P_URL" -o "$P_FILENAME"
ls -l "$P_FILENAME"
```
