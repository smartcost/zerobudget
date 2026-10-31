# Despesas manuais e estornos

Uma despesa registra o uso do dinheiro de um envelope em um ciclo **ativo**.
Não representa uma transferência bancária. A alocação permanece fixa; o saldo
para gastar é `allocated_cents - spent_cents`. O gasto inclui somente despesas
que não foram estornadas.

Exemplo: R$ 800 alocados e uma despesa de R$ 120 resultam em R$ 680 restantes.
O ciclo continua com a renda totalmente alocada e `available_cents=0`: esse campo
mede dinheiro ainda não distribuído, não o dinheiro restante para gastar.

## Regras

- Descrição aparada, de 1 a 200 caracteres Unicode e sem caracteres de controle.
- Valor positivo em centavos inteiros, até `i64::MAX`; não há ponto flutuante.
- Data civil `YYYY-MM-DD` dentro do período do ciclo, incluindo o início e
  excluindo o fim. Datas futuras dentro do período são permitidas; a data não
  agenda uma execução posterior: o gasto é contabilizado imediatamente.
- Somente ciclos ativos aceitam despesas e estornos. Planejados e fechados
  retornam 409. As consultas permanecem disponíveis em todos os estados.
- Saldo insuficiente retorna 409, inclusive sob requisições simultâneas.
- O lançamento original é imutável: sem PATCH ou DELETE. Para corrigir, estorne
  e registre outra despesa enquanto o ciclo estiver ativo.
- O estorno é integral, preserva o original e devolve saldo uma única vez.
  Repeti-lo em ciclo ativo retorna 200 com a mesma despesa estornada.
- O fechamento pode ocorrer com dinheiro não gasto; o saldo permanece consultável.
  Não há reabertura, estorno em ciclo fechado ou transporte automático de saldo.

## HTTP

Todas as rotas ficam sob
`/api/cycles/{cycle_id}/envelopes/{envelope_id}/expenses`.

| Método | Sufixo | Permissão | Resultado |
| --- | --- | --- | --- |
| POST | nenhum | `budget:write` + CSRF | 201 com despesa |
| GET | `?limit=20&offset=0` | `budget:read` | 200 com array, incluindo estornadas |
| POST | `/{expense_id}/reverse` | `budget:write` + CSRF | 200 com despesa estornada; sem corpo |

Corpo de criação (substitua a data por uma dentro do ciclo selecionado):

```json
{"description":"Alimentação","occurred_on":"2026-10-30","amount_cents":12000}
```

A resposta contém `id`, `cycle_id`, `envelope_id`, `description`, `occurred_on`,
`amount_cents` e `status` (`posted` ou `reversed`). A listagem ordena por data
decrescente e UUID decrescente; `limit` vai de 1 a 100 e `offset` de 0 a 100000.
Paginação por offset pode mudar sob escritas concorrentes.

`GET /api/cycles/{cycle_id}/envelopes` acrescenta `spent_cents` e `remaining_cents`.
Respostas de criação/edição de envelope também incluem esses campos: no
planejamento, gasto zero e restante igual ao alocado.

Sem sessão: 401; sem permissão/CSRF ou origem inválida: 403; recurso inexistente,
de outro proprietário ou com vínculo incorreto: 404; saldo insuficiente ou estado
incompatível: 409; entrada inválida: 422; JSON acima de 4096 bytes: 413;
indisponibilidade: 503. Campos desconhecidos e valores nulos são rejeitados.
Formato inválido de UUID/query segue as rejeições 400 do Axum. Escrita isolada
não concede leitura HTTP, mas permite receber a resposta da própria mutação.

POST de criação não tem deduplicação automática. Se a resposta se perder, consulte
a lista antes de repetir. O cliente E2E não repete mutações por falha de transporte.
Clientes JavaScript devem respeitar `Number.MAX_SAFE_INTEGER` ou usar um parser
para inteiros de 64 bits; a API e o banco calculam centavos em inteiros exatos.

## Persistência

A migração `0005_expenses.sql` cria `budgeting.expenses` e
`budgeting.expense_reversals`, com RLS forçada e chaves estrangeiras compostas
que fixam ciclo, envelope e proprietário. O estorno é um registro separado com
uma chave única por despesa. As tabelas guardam `created_at` do servidor para
preservar o instante de gravação, sem aceitar esse campo do cliente.

O runtime recebe SELECT e INSERT somente nas colunas de negócio das duas tabelas.
Não recebe UPDATE/DELETE nem INSERT em timestamps. Triggers de imutabilidade
acrescentam proteção caso um operador amplie privilégios indevidamente.

Todas as mutações usam o mesmo advisory lock por ciclo de alocação e fechamento,
em transações READ COMMITTED. Triggers SECURITY INVOKER também validam estado,
período e saldo no SQL direto. A soma exclui estornos antes de converter para
bigint: o histórico bruto pode superar `i64::MAX` sem estourar o saldo corrente.

A preparação completa usa, com credenciais apenas no processo administrativo:

```sh
cargo run -p obz-api --example migrate
```

Configure `DATABASE_ADMIN_URL`, `DATABASE_RUNTIME_ROLE` e, em loopback local,
`AUTH_ALLOW_LOCAL_HTTP=true`. A API usa apenas a credencial runtime e verifica o
schema no início. Nenhum direito financeiro é concedido a usuários automaticamente.

## Testar

```sh
./scripts/e2e test expenses
```

A regressão completa é `./scripts/e2e test`, com Dex real e serviços descartáveis.
`cargo test --workspace` cobre domínio e casos de uso. Os testes SQL ignorados
usam bancos temporários e as variáveis descritas em [AGENTS.md](../AGENTS.md);
cobrem RLS, imutabilidade, aritmética de 64 bits e ordens controladas do lock.

O [contrato OpenAPI e a coleção Postman](api/README.md) incluem as três operações.
Não há contas bancárias, receitas realizadas, importação ou integração financeira.
