# Contas manuais, recebimentos e transferências

A conta descreve **onde** o dinheiro está. O envelope descreve **para quê** ele foi
reservado. Esta etapa registra contas bancárias manuais (`bank`) e carteira
(`cash`), sempre em BRL, sem integração bancária ou cheque especial.
[Cartões e faturas](cartoes.md) são passivos separados das contas com dinheiro.

O saldo inicial é informado pelo usuário para a data `opened_on`. O saldo atual
registrado é saldo inicial + entradas − saídas, em centavos inteiros de zero até
`i64::MAX`. Não é um saldo conciliado com um banco. Movimentos futuros dentro das
regras de datas são contabilizados imediatamente, sem agendamento. O limite de
saldo é verificado no registro da operação; não há reconstrução de solvência diária.

## Efeitos

| Operação | Conta | Envelope / renda planejada |
| --- | --- | --- |
| Recebimento | Credita uma conta | Não altera automaticamente |
| Transferência própria | Debita origem e credita destino | Não altera |
| Despesa | Debita conta de pagamento | Consome envelope uma vez |
| Estorno da despesa paga pela conta | Credita a conta original uma vez | Devolve saldo uma vez |
| Pagamento de fatura | Debita a conta e reduz a dívida do cartão | Não consome novamente |

É possível planejar um ciclo com renda esperada ainda não recebida. A despesa
só será aceita se houver saldo tanto no envelope quanto na conta.

Despesas com débito imediato **exigem `account_id`**. Alternativamente, uma compra
no crédito informa `invoice_id`, consumindo o envelope sem débito bancário imediato. O débito e o consumo do envelope acontecem
na mesma transação: qualquer falha reverte ambos. O mesmo vale para estornos,
inclusive se o crédito exceder `i64::MAX`. O estorno ainda exige ciclo ativo.

O histórico anterior à migração 0006 preserva `account_id=null`, sem criar conta,
saldo inicial ou movimentação fictícia. Estornar uma dessas despesas devolve
apenas o saldo do envelope, pois a despesa original não debitou conta alguma.

## API

Todas as rotas exigem sessão. GET exige `budget:read`; POST exige `budget:write`,
CSRF e a validação de origem existente. O proprietário sempre vem da sessão.

| Método | Rota | Resultado |
| --- | --- | --- |
| POST | `/api/accounts` | 201 com conta e saldo |
| GET | `/api/accounts?limit=20&offset=0` | 200 com contas e saldos |
| GET | `/api/accounts/{account_id}` | 200 com conta e saldo |
| POST | `/api/accounts/{account_id}/receipts` | 201 com recebimento |
| POST | `/api/transfers` | 201 com uma transferência atômica |
| GET | `/api/accounts/{account_id}/movements?limit=20&offset=0` | 200 com extrato |

Criar conta:

```json
{"name":"Principal","kind":"bank","opened_on":"2026-01-01","opening_balance_cents":100000}
```

Receber:

```json
{"description":"Salário","occurred_on":"2026-10-31","amount_cents":500000}
```

Transferir (substitua os identificadores pelos UUIDs retornados):

```json
{"from_account_id":"UUID_ORIGEM","to_account_id":"UUID_DESTINO","description":"Carteira","occurred_on":"2026-10-31","amount_cents":20000}
```

Nomes são aparados, de 1 a 100 caracteres sem controles, únicos por proprietário
com distinção de maiúsculas. Movimentos têm descrição aparada de 1 a 200
caracteres e valor positivo. Datas civis estritas entre 0001 e 9999 não podem
anteceder a abertura de nenhuma conta envolvida. Origem e destino devem ser
contas próprias distintas. Saldo insuficiente e overflow retornam 409; recurso
alheio/inexistente retorna 404; entradas inválidas retornam 422. Sem sessão: 401;
sem permissão/CSRF ou com origem inválida: 403. Indisponibilidade: 503. Rejeições
JSON, UUID, corpo de 4096 bytes e paginação seguem o contrato OpenAPI existente.

O extrato mostra cada movimento completo e `delta_cents` relativo à conta
consultada. A mesma transferência aparece nos dois extratos com o mesmo ID e
sinais opostos. O saldo inicial fica na conta, sem movimento duplicado. Ordem:
data decrescente, UUID decrescente. `limit` vai de 1 a 100 e `offset` até 100000.
A paginação por offset pode mudar durante escritas concorrentes.

Contas e movimentos são imutáveis nesta etapa. Não há renomeação, exclusão,
conciliação, estorno de recebimentos/transferências ou conexão com bancos. Uma
transferência inversa é uma nova operação explícita. Correções de recebimentos
precisarão de uma operação própria futura. POST não tem deduplicação automática;
se a resposta se perder, consulte o extrato antes de repetir. Preserve os limites
de inteiros de 64 bits ao consumir JSON em JavaScript.

## Arquitetura e migração

`obz-domain` contém valores e regras sem dependências externas. `obz-banking`
contém os casos de uso e a porta `BankingRepository`, sem depender de ciclos.
`obz-infrastructure` implementa o PostgreSQL; `obz-api` compõe os serviços.
O domínio do orçamento continua separado da localização física do dinheiro.

A migração `0006_banking.sql` cria o schema `banking`, contas e movimentos
imutáveis, RLS forçada e vínculos compostos por proprietário. Exige conta nas
novas despesas sem alterar registros antigos. Triggers SECURITY INVOKER registram
pagamentos e devoluções automaticamente, inclusive no SQL direto.

O lock bancário é por proprietário: serializa operações das suas contas, incluindo
os dois lados da transferência. Quando há orçamento envolvido, a ordem é lock
do ciclo e depois lock bancário. Operações puramente bancárias não adquirem locks
de ciclo. Transações READ COMMITTED e somas `numeric` protegem concorrência e
históricos acumulados maiores que bigint, mantendo cada saldo em `i64`.

A ferramenta completa concede USAGE no schema banking e SELECT/INSERT somente
nas colunas de negócio de accounts/movements, além de INSERT(account_id) em
expenses. Não concede UPDATE/DELETE, edição de timestamps ou direitos a usuários.
Após configurar `DATABASE_ADMIN_URL`, `DATABASE_RUNTIME_ROLE` e, em loopback,
`AUTH_ALLOW_LOCAL_HTTP=true`, execute como operador:

```sh
cargo run -p obz-api --example migrate
```

Reinicie a API com a nova versão após preparar o banco. Ela não migra nem recebe
a credencial administrativa. OpenAPI e coleção Postman estão em [API](api/README.md).

## Testes

```sh
./scripts/e2e test banking
```

A suíte completa é `./scripts/e2e test`. Os testes Rust com PostgreSQL descartável
cobrem limites de 64 bits, rollback de estorno, disputa de saldo entre transferência
e despesa, imutabilidade, RLS e migração de despesas antigas sem conta. Consulte
[AGENTS.md](../AGENTS.md) para configurar os testes ignorados.
