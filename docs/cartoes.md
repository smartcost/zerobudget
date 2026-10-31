# Cartões e faturas manuais

O cartão representa dívida, não dinheiro disponível em conta. O envelope indica
a finalidade do gasto. Uma compra no crédito consome o envelope e aumenta a
dívida; pagar a fatura debita uma conta e reduz essa dívida, **sem outra despesa**.

Cartões começam sem dívida, com limite positivo e imutável. O limite é compartilhado
entre todas as faturas. Não há importação de dívidas anteriores, parcelamento,
juros, taxas, crédito rotativo, integração bancária ou agendamento automático.

## Fatura e compra

Faturas são abertas explicitamente com início inclusivo, fim exclusivo e vencimento
igual ou posterior ao fim. Períodos do mesmo cartão não podem se sobrepor. Não se
calculam datas mensais automaticamente. As datas civis vão de 0001 a 9999.

A despesa existente passa a aceitar exatamente um entre `account_id` e `invoice_id`.
O primeiro debita dinheiro imediatamente; o segundo registra uma compra em fatura
aberta, dentro do período e após a abertura do cartão. O ciclo deve estar ativo e
ter saldo no envelope. A compra pode ocorrer sem saldo bancário; o limite não é
tratado como renda e a renda planejada continua informada pelo usuário.

Exemplo de compra (substitua o UUID pela fatura criada):

```json
{"invoice_id":"UUID_DA_FATURA","description":"Supermercado","occurred_on":"2026-10-31","amount_cents":12000}
```

O estorno exige ciclo ativo e fatura aberta, devolvendo saldo ao envelope e limite
ao cartão. Não credita uma conta. Após o fechamento, novos estornos/compras são
recusados; crédito de estabelecimento em fatura posterior fica para outra etapa.
Repetir um estorno já registrado não altera dinheiro e continua idempotente se o
ciclo estiver ativo.

Fechar a fatura congela as compras definitivamente. Repetir o fechamento é
idempotente. Não há reabertura. O status exposto é `open`, `closed` ou `paid`:
`paid` significa fatura fechada com dívida restante zero, inclusive uma fatura vazia.

## Pagamento

O pagamento exige fatura fechada, conta própria, valor positivo até o saldo devido,
e data igual ou posterior ao fim do período e à abertura da conta. Pode ser parcial
ou integral. Pagamento após o vencimento é registrado sem calcular juros ou multa.
O saldo bancário precisa ser suficiente. O ciclo orçamentário pode estar fechado:
a compra já foi contabilizada nele e o pagamento não o altera.

Cada pagamento exige `request_id`, um UUID não nulo que identifica a operação.
A repetição idêntica retorna o mesmo pagamento e HTTP 201, sem novo débito.
O ID da resposta é esse próprio UUID. Reutilização com outra conta, valor, data,
descrição ou fatura retorna 409. Para outro pagamento, gere outro UUID; preserve
o anterior enquanto confirmar uma resposta perdida. A autorização atual continua
sendo exigida inclusive para replay. Descrições são comparadas após aparar espaços.

```json
{"request_id":"UUID_DA_OPERACAO","account_id":"UUID_DA_CONTA","description":"Pagamento de fatura","occurred_on":"2026-11-01","amount_cents":12000}
```

O pagamento e o movimento bancário são imutáveis. Não há estorno de pagamento
nesta etapa. Falta de saldo ou sobrepagamento reverte tudo: sem pagamento parcial
involuntário nem movimento sem correspondência. Não há nova despesa ou consumo
de envelope ao liquidar uma fatura.

## API

Sessão obrigatória. GET exige `budget:read`; POST exige `budget:write`, CSRF e a
validação de origem existente. Escrita isolada permite a resposta da própria
mutação, sem conceder as consultas. O proprietário nunca é aceito no corpo.

| Método | Rota | Resultado |
| --- | --- | --- |
| POST / GET | `/api/cards` | Criar ou listar cartões |
| GET | `/api/cards/{card_id}` | Limite, dívida e limite disponível |
| POST / GET | `/api/cards/{card_id}/invoices` | Abrir ou listar faturas |
| GET | `/api/cards/{card_id}/invoices/{invoice_id}` | Compras líquidas, pagamentos, dívida e status |
| POST | `/api/cards/{card_id}/invoices/{invoice_id}/close` | Fechar sem corpo |
| POST / GET | `/api/cards/{card_id}/invoices/{invoice_id}/payments` | Pagar ou listar pagamentos |
| GET | `/api/cards/{card_id}/invoices/{invoice_id}/purchases` | Compras e estornos com referências ao ciclo/envelope |

Compras usam `POST /api/cycles/{cycle_id}/envelopes/{envelope_id}/expenses`,
retornando `account_id=null` e `invoice_id` preenchido. Despesas anteriores são
preservadas: conta e fatura podem ser nulas apenas no histórico anterior à migração
bancária. Não são criadas dívidas ou contas fictícias.

Listas usam `limit` de 1 a 100 e `offset` até 100000. Cartões ordenam por UUID;
faturas por início decrescente e UUID decrescente; compras e pagamentos por data
decrescente e UUID decrescente. Paginação por offset pode mudar sob concorrência.

Criação/pagamento retorna 201; consulta/fechamento retorna 200. Sem sessão: 401;
sem direitos/CSRF ou origem inválida: 403; recurso alheio/inexistente ou vínculo
incorreto: 404; estado incompatível, limite, sobrepagamento, período sobreposto ou
UUID reutilizado com outros dados: 409; entrada/data inválida: 422; indisponível:
503. JSON/UUID/corpo de 4096 bytes seguem as rejeições já existentes.

## Segurança e persistência

A migração `0007_credit_cards.sql` acrescenta cartões, faturas e pagamentos com
RLS forçada, chaves compostas por proprietário e histórico imutável. Preserva
as migrações e os dados anteriores. Faturas concedem UPDATE **somente de status**;
cartões e pagamentos recebem SELECT e INSERT somente nas colunas de negócio.
Acrescenta INSERT(invoice_id) em despesas e INSERT(card_payment_id) em movimentos,
sem permissões novas a usuários e sem UPDATE/DELETE no histórico financeiro.

Compras/estornos adquirem primeiro o lock de ciclo e depois o bancário do
proprietário. Faturas/pagamentos usam só o lock bancário. A mesma serialização
protege limite entre faturas, fechamento, pagamento, replay e saldo bancário.
READ COMMITTED e triggers SECURITY INVOKER repetem as invariantes no SQL direto.
Somas em `numeric` permitem que o histórico acumulado exceda bigint, com cada
saldo e limite dentro de `i64`. O pagamento gera um movimento `card_payment`,
referenciado por `card_payment_id` no extrato e único por pagamento.

Como operador, com `DATABASE_ADMIN_URL`, `DATABASE_RUNTIME_ROLE` e, em loopback,
`AUTH_ALLOW_LOCAL_HTTP=true`, prepare o banco e reinicie a API atualizada:

```sh
cargo run -p obz-api --example migrate
```

A API não migra nem recebe credenciais administrativas. OpenAPI, coleção e ambiente
Postman estão em [API](api/README.md). O cliente Postman mantém o identificador de
pagamento até você limpar `card_payment_request_id` para uma nova operação.

## Testes

```sh
./scripts/e2e test credit
```

A suíte E2E completa usa `./scripts/e2e test` com Dex real e bancos descartáveis.
Os testes Rust cobrem autorização, domínio, SQL/RLS, concorrência de compras entre
faturas, idempotência e compras/pagamentos cujo histórico bruto supera `i64::MAX`.
As integrações ignoradas devem usar `--include-ignored --test-threads=2`, conforme
[AGENTS.md](../AGENTS.md), para limitar checkpoints de bancos temporários. As
corridas financeiras dentro de cada cenário continuam sendo executadas em paralelo.
