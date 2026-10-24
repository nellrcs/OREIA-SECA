# skill_weather

Consulta a previsão do tempo atual de uma cidade (wttr.in).

## Parâmetros
- `city` (string, obrigatório): nome da cidade, ex.: Florianopolis

## Código
```sh
curl -fsS --max-time 10 "https://wttr.in/$(printf %s "$P_CITY" | sed 's/ /+/g')?format=3&lang=pt"
```
