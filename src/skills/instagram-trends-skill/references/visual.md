# Geração do Visual do Post

## Formatos e dimensões

| Formato | Proporção | Pixels recomendados |
|---|---|---|
| Feed quadrado | 1:1 | 1080 × 1080 |
| Feed retrato | 4:5 | 1080 × 1350 |
| Stories / Reels | 9:16 | 1080 × 1920 |
| Carrossel (slide) | 1:1 ou 4:5 | 1080 × 1080 |

## Diretrizes de design

### Hierarquia visual obrigatória
1. **Foco principal** — elemento que o olho vê primeiro (título, número, imagem hero)
2. **Informação secundária** — detalhe que complementa
3. **Marca** — logo ou handle discreto, sempre presente

### Paleta de cores
- Use no máximo 3 cores por post
- Sempre garanta contraste legível (texto claro em fundo escuro ou vice-versa)
- Cores da marca têm prioridade; se não informadas, escolha baseado no nicho:

| Nicho | Paleta sugerida |
|---|---|
| Saúde / Bem-estar | Verde-salva, branco, areia |
| Moda | Preto, off-white, dourado ou tom da estação |
| Tecnologia | Azul escuro, cinza, ciano ou roxo |
| Gastronomia | Terracota, creme, verde escuro |
| Negócios | Azul navy, branco, laranja ou amarelo |

### Tipografia
- **Título/Gancho**: fonte bold, grande, legível em thumbnail
- **Subtítulo/Corpo**: fonte regular, menor, complementar
- Nunca use mais de 2 famílias tipográficas no mesmo post

### Elementos visuais que engajam
- Números grandes em destaque ("67%", "5 dicas")
- Ícones/emojis como elementos gráficos decorativos
- Gradientes suaves no fundo
- Formas geométricas como elementos de composição
- Foto ou ilustração como elemento hero (quando disponível)

## Como gerar o visual

Use SVG ou HTML para criar o post diretamente no chat.

### Template base SVG (feed 1:1)

```svg
<svg viewBox="0 0 1080 1080" xmlns="http://www.w3.org/2000/svg">
  <!-- Fundo -->
  <rect width="1080" height="1080" fill="[COR_FUNDO]"/>

  <!-- Elemento decorativo -->
  <circle cx="900" cy="150" r="200" fill="[COR_DESTAQUE]" opacity="0.15"/>

  <!-- Título principal -->
  <text x="80" y="420" font-size="90" font-weight="bold"
        fill="[COR_TEXTO]" font-family="sans-serif">
    TÍTULO AQUI
  </text>

  <!-- Subtítulo -->
  <text x="80" y="520" font-size="42" fill="[COR_TEXTO]"
        opacity="0.8" font-family="sans-serif">
    subtítulo complementar
  </text>

  <!-- Handle da marca -->
  <text x="80" y="1020" font-size="30" fill="[COR_TEXTO]"
        opacity="0.5" font-family="sans-serif">
    @seuperfil
  </text>
</svg>
```

### Adaptações por tipo de post

**Post educativo (lista/dicas)**
- Fundo neutro (branco, creme, cinza claro)
- Número grande em destaque
- Lista com ícones ou bullets visuais

**Post inspiracional (frase)**
- Fundo com gradiente ou textura sutil
- Frase centralizada em tipografia expressiva
- Elemento decorativo leve (linha, forma geométrica)

**Post de tendência/dados**
- Dado principal em destaque (número, %)
- Gráfico simples ou barra de progresso visual
- Cores vibrantes para chamar atenção

## Checklist do visual

- [ ] Legível em thumbnail pequeno (feed do celular)
- [ ] Handle/marca visível
- [ ] Contraste de cores adequado
- [ ] Não tem texto demais (máx. 20% da área)
- [ ] Tema visualmente coerente com o nicho
