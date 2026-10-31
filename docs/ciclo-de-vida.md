# Ativação e fechamento do orçamento

O ciclo nasce `planned`: a renda é distribuída entre envelopes. A ativação confirma
esse planejamento quando `available_cents=0`. O ciclo passa a `active`, com seus
envelopes congelados, e pode então ser encerrado como `closed`. O fechamento é
definitivo. As consultas continuam disponíveis nos três estados. [Despesas e estornos](despesas.md)
são permitidos apenas em ciclos ativos.

| Estado atual | Ativar | Fechar | Alterar envelopes |
| --- | --- | --- | --- |
| `planned` | 200 se totalmente alocado; caso contrário 409 | 409 | Permitido |
| `active` | 200, sem mudança | 200, passa a `closed` | 409 |
| `closed` | 409 | 200, sem mudança | 409 |

Renda zero pode ser ativada sem envelopes. Não há restrição de um único ciclo
ativo, ativação automática por data, reabertura ou edição da renda/período.
Despesas manuais são registradas na etapa de [execução](despesas.md). Não há
receitas realizadas nem transporte de saldo entre ciclos. O fechamento permite
saldo não gasto nos envelopes, que permanece consultável.
Redistribuição durante a execução exigirá uma futura operação atômica própria.

## API

- `POST /api/cycles/{cycle_id}/activate`
- `POST /api/cycles/{cycle_id}/close`

Sem corpo. Exigem sessão, `budget:write`, `X-CSRF-Token` e a validação de origem
existente. Retornam 200 com o mesmo formato de ciclo das consultas, incluindo o
estado real. Repetir ativação em ativo ou fechamento em fechado é idempotente.

Sem sessão: 401; sem permissão/CSRF ou com origem inválida: 403; ciclo inexistente
ou de outro proprietário: 404; saldo não distribuído ou estado incompatível: 409;
armazenamento indisponível: 503. UUID malformado segue a rejeição 400 do Axum.
`budget:write` permite a resposta da própria mutação sem conceder as rotas GET,
que continuam exigindo `budget:read`. Não se aceita proprietário no corpo.

Os contratos estão em [OpenAPI e Postman](api/README.md). O Runner de envelopes e
os roteiros antigos precisam de um ciclo `planned`; use outro ciclo depois de
ativar o primeiro.

## Persistência e privilégios

A migração `0004_cycle_lifecycle.sql` amplia os estados, adiciona uma política RLS
de UPDATE por proprietário habilitado com `budget:write` e protege as transições
com trigger SECURITY INVOKER. As migrações anteriores permanecem intactas.

O adaptador adquire o mesmo advisory lock por ciclo usado pelos envelopes, lê o
estado e os totais atuais e aplica a regra de domínio dentro da transação. Os
triggers repetem as proteções para SQL direto, rejeitam outros níveis de isolamento
e usam READ COMMITTED para observar commits depois de adquirir o lock. Ativação
concorrente com exclusão ou edição nunca confirma um orçamento com saldo pendente.

A ferramenta de migração completa passa a conceder ao runtime **UPDATE somente
da coluna status em budgeting.cycles**, além dos privilégios existentes. Não
concede DELETE em ciclos, edição de renda/proprietário/período ou permissões
financeiras a usuários. A API nunca migra o banco nem recebe credenciais de operador.

Após autorização do operador, com `DATABASE_ADMIN_URL` e `DATABASE_RUNTIME_ROLE`
configurados apenas no processo administrativo:

```sh
cargo run -p obz-api --example migrate
```

Em loopback local, configure também `AUTH_ALLOW_LOCAL_HTTP=true`. Reinicie a API
com a versão atual após preparar schema e privilégios.

## Validação

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./scripts/e2e test lifecycle
```

A regressão completa usa `./scripts/e2e test`. Os testes Rust de SQL/RLS são
ignorados por padrão e precisam das variáveis de PostgreSQL/Redis documentadas em
[AGENTS.md](../AGENTS.md). Criam bancos temporários; nunca migram o banco indicado
como base. Cobrem transições em SQL direto, privilégios por coluna, congelamento
e as duas ordens de disputa do lock entre ativação e exclusão.
