//! Texto estático do prompt `po_guidelines` (ver `docs/mcp-plano/03-crate-mcp-stdio-leitura.md`).

pub const PO_GUIDELINES: &str = "\
Como escrever uma boa história (issue) pro OREIA:

1. Pequena: uma história por agente (dev-back OU dev-front, não os dois). Se a mudança
   precisa dos dois lados, escreva DUAS histórias e defina o contrato entre elas primeiro
   (endpoints, formato dos dados) — cada agente trabalha só com a parte dele.
2. Verificável: todo critério de aceite tem que dar pra checar de fora (rodar algo, ler uma
   resposta, ver um arquivo) — nunca algo subjetivo como \"funcionar bem\".
3. Objetivo primeiro, critérios depois: diga O QUE deve ser verdade quando a história
   estiver pronta, não COMO implementar — isso é decisão do agente (ou de quem assumir via
   checkout_issue).
4. Arquivos prováveis ajudam o agente a começar mais rápido, mas não são obrigação — ele
   pode (e deve) tocar em outros arquivos se for o caso.
5. Fora de escopo evita que o agente \"resolva\" coisas que não foram pedidas.
";
