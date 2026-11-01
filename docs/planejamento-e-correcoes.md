# Planejamento e correções do MVP

Todos os endpoints abaixo usam a sessão existente. GET exige `budget:read`, POST
exige `budget:write` e `X-CSRF-Token`. Proprietário vem exclusivamente da sessão.
Valores são centavos inteiros de BRL; o limite suportado é i64, não float.
O OpenAPI em `docs/api/openapi.json` contém corpos e respostas completos.

## Repetição de requisições

Despesas, recebimentos e transferências agora exigem `request_id`, assim como
pagamentos, realocações, compensações, dívida inicial, créditos e próximo ciclo.
Gere um UUID não nulo uma vez por intenção e preserve-o se a resposta se perder.
Mesmo identificador e mesmos dados canônicos retornam o registro existente com
201, sem repetir efeitos. Dados divergentes retornam 409. A descrição é aparada.
Permissões e propriedade continuam sendo verificadas em cada tentativa.

O id retornado coincide com request_id. Após compensação/estorno, replay devolve
estado atual (`reversed`) e não recria o lançamento. Novo lançamento exige nova
chave. O cliente E2E fornece uma chave nova quando o teste inicia uma operação;
os testes de replay passam a mesma chave explicitamente. Não há retry de escrita
sem chave preservada. Contas/cartões/envelopes/ciclos criados pelas rotas antigas
não ganharam idempotência de criação; seus conflitos de unicidade permanecem.

## Realocação de envelopes

`POST /api/cycles/{cycle_id}/reallocations` recebe request_id, from_envelope_id,
to_envelope_id e amount_cents positivo. Exige ciclo ativo, envelopes distintos do
mesmo ciclo e saldo ainda não gasto na origem. GET na mesma rota lista o histórico
com limit/offset. Despesa concorrente e realocação usam o mesmo lock do ciclo.

A alocação efetiva é original + entradas − saídas. Alocação original permanece
imutável; GET de envelopes e summary retornam a efetiva. A soma do ciclo não muda,
não há movimento bancário. Não é possível retirar alocação já consumida nem
realocar em ciclo fechado. Corrija uma realocação com outra no sentido inverso,
se ainda houver saldo disponível.

## Compensações bancárias

`POST /api/movements/{movement_id}/reverse` recebe request_id, description e
occurred_on. Aceita somente recebimento, transferência ou pagamento de fatura.
Compensa integralmente com novo movimento inverso, sem apagar o original. A data
não pode preceder o original nem a abertura das contas. Cada original admite uma
compensação; uma segunda intenção com outra chave retorna 409.

Desfazer recebimento exige saldo na conta que será debitada; desfazer transferência
exige saldo no destino original. Compensar pagamento devolve dinheiro à conta e
restaura dívida; a operação é recusada se exceder o limite compartilhado já usado.
Qualquer falha reverte marcador e movimento juntos. Extratos indicam status do
original e adjustment_id do movimento inverso. Para localizar pagamento no extrato,
use card_payment_id. Despesas continuam usando a rota própria de estorno e suas
restrições de ciclo/fatura; movimentos de correção não são compensáveis novamente.

## Dívida inicial e devoluções

`POST /api/cards/{card_id}/opening-debts` recebe request_id, invoice_id,
description, occurred_on e amount_cents. Registra dívida em fatura aberta, dentro
do período e limite, sem consumir envelope ou alterar dinheiro em conta. Há apenas
uma dívida inicial não compensada por cartão; correção libera uma nova entrada.
GET lista o histórico com paginação.

`POST /api/cards/{card_id}/credits` recebe request_id, invoice_id (destino aberto),
description, occurred_on e exatamente uma origem: expense_id ou opening_debt_id.
O servidor deriva o valor integral da origem. Compra exige fatura de origem fechada,
mesmo cartão e ausência de estorno/devolução anterior. Dívida inicial pode estar
em fatura aberta ou fechada. Data deve pertencer à fatura destino e não preceder
origem/abertura do cartão. GET lista os créditos com paginação.

A compra histórica e o gasto do ciclo permanecem intactos. O crédito reduz a dívida
do cartão, podendo produzir credit_balance_cents; não gera recebimento bancário.
A fatura original mantém seus valores. Exemplo: compra fechada de 100 e devolução
futura de 100 deixam dívida original 100 e crédito futuro 100, posição líquida do
cartão zero. Isso não quita automaticamente a fatura antiga: pagá-la com dinheiro
ainda reduz a conta e deixa crédito no cartão para compras posteriores. O resumo
geral mostra obrigações por fatura e créditos separadamente, sem supor compensação
entre faturas. available_credit_cents é o limite contratual menos dívida líquida;
o saldo credor aparece à parte.

## Resumos consistentes

`GET /api/cycles/{cycle_id}/summary` usa uma transação de leitura com snapshot
REPEATABLE READ. Retorna ciclo, envelopes efetivos, gasto líquido de estornos,
saldo ainda não gasto, recebimento efetivo do período e lacuna da renda prevista.
Recebimentos entram pela data; compensações de recebimento saem pela data da
compensação. Uma correção posterior não reescreve a entrada do período anterior.
O realizado pode ser negativo. Renda planejada não é atualizada automaticamente.

`GET /api/overview` retorna dinheiro em contas, obrigações positivas de faturas,
créditos separados, saldo ainda comprometido nos ciclos ativos, funding_gap_cents
e uncommitted_cash_cents. Não presume que dívida foi paga nem que crédito de cartão
seja caixa. Fórmulas: compromisso = saldo dos envelopes ativos + dívida das faturas;
lacuna = max(compromisso − caixa, 0); livre = max(caixa − compromisso, 0). Falta de
caixa não impede planejamento/ativação; esse indicador torna a diferença visível.
Totais que excedem i64 falham explicitamente, sem truncamento monetário.

## Continuidade

`POST /api/cycles/{cycle_id}/next` exige origem fechada. Recebe request_id,
period_start, period_end, new_income_cents e mode:

| mode | Envelopes no novo ciclo | Renda planejada total |
| --- | --- | --- |
| empty | Nenhum | Renda nova |
| copy | Nomes e alocações efetivas da origem | Renda nova, que deve cobrir a cópia |
| carry_remaining | Nomes e saldos ainda não gastos | Renda nova + saldo transportado |

Novo período começa a partir do fim exclusivo da origem. O resultado é sempre
planejado. Há um único sucessor por origem, impedindo carregar o saldo duas vezes.
carried_cents e source_cycle_id registram a proveniência. Não se copiam despesas,
faturas ou movimentos e não se cria recebimento bancário. A renda nova comparada
com recebimentos efetivos exclui carried_cents. Se o saldo existente não estava
coberto por caixa, transportá-lo não cria caixa: overview continua mostrando lacuna.
