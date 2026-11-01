# PostgreSQL 18: decisões de arquitetura

## Identificadores UUIDv7

Novos IDs locais usam UUIDv7: usuários locais, ciclos, envelopes, despesas,
contas, movimentos, cartões, faturas, pagamentos, realocações, compensações,
dívidas iniciais, créditos e auditoria. Rust usa `Uuid::now_v7()`; SQL usa
`pg_catalog.uuidv7()`. Ambas as implementações seguem o mesmo formato, sem exigir
uma ida ao banco para gerar o identificador na aplicação.

A migração `0011_uuid_v7.sql` adiciona defaults às 14 tabelas que possuem chave
própria `id`, atualiza as quatro funções que geram IDs e exige versão 7 com variante
RFC nos INSERTs. IDs que são referências (inclusive chaves primárias compartilhadas,
como `expense_reversals.expense_id` e `budget_onboardings.user_id`) conservam o ID da
entidade original. As migrações anteriores permanecem intactas.

Não há reescrita de dados ou chaves estrangeiras. Registros históricos UUIDv4
continuam consultáveis e atualizáveis conforme as regras de negócio existentes.
`request_id` de novas operações deve ser UUIDv7; outro formato/versão/variante
retorna HTTP 422. Essa validação também se aplica a chamadas de retry com chaves
antigas v4: clientes precisam concluir operações pendentes antes desta atualização.
Nunca gere outro request_id automaticamente ao repetir uma operação incerta.
Retries v7 mantêm a chave original; replay idêntico retorna o registro original e
payload divergente retorna 409. IDs em URLs continuam aceitando referências históricas.

Playwright possui gerador RFC 9562 usando `node:crypto`; a coleção Postman combina
um timestamp de 48 bits com a entropia do gerador UUID do sandbox e preserva a chave
no ambiente para retries. Limpe somente chaves de operações já concluídas quando
atualizar um ambiente Postman que ainda contenha v4.

`subject` do Keycloak é uma identidade externa opaca, não uma chave local sob nosso
controle. Cookies, CSRF, state, nonce e PKCE continuam segredos aleatórios próprios
do protocolo. UUIDv7 não substitui esses segredos, autorização nem timestamps de
negócio: o relógio do cliente não é confiável. A ordem dos IDs não é uma garantia de
ordem de commit ou de criação entre máquinas.

Os novos UUIDs têm melhor localidade temporal em índices B-tree que UUIDv4. Isso
não implica um ganho percentual conhecido no nosso workload: faltam medições com
volume e concorrência representativos. UUID continua ocupando 16 bytes.

Aplicação da migração é uma operação explícita do operador, com backup conforme
[operação](operacao.md). O servidor não migra. No terminal de operador, configure
`DATABASE_ADMIN_URL`, `AUTH_ALLOW_LOCAL_HTTP=true` apenas para o banco local e execute
`cargo run --locked -p obz-api --example migrate`. Não é preciso ampliar privilégios
runtime. A migração foi validada em bancos descartáveis e consolidada no banco
local em 01/11/2026, após backup validado. A API precisa ser reiniciada pelo operador
para carregar o código atualizado; não houve implantação pública nesta etapa.

## Colunas geradas virtuais

Não adicionamos colunas geradas nesta etapa. Não existe coluna gerada STORED a
converter no schema atual. Saldo de conta, alocação efetiva, gasto por envelope,
dívida/crédito de fatura e overview dependem de agregações entre registros ou
tabelas. Expressões de colunas geradas não podem consultar outras linhas nem usar
subqueries; portanto não resolvem esses cálculos.

As views/projeções atuais já evitam persistir esses resultados derivados. Continuam
com snapshots e autorização existentes. Materializar saldos exigiria um desenho
transacional próprio e medição; trocar por VIRTUAL não oferece esse mecanismo.

Para futuras expressões baratas e imutáveis da mesma linha, considerar
`GENERATED ALWAYS AS (...) VIRTUAL` explicitamente. STORED pode fazer sentido para
cálculo caro e repetidamente lido, mediante medição e custo de escrita/armazenamento.
Não criar campos formatados apenas para utilizar uma funcionalidade nova.

## I/O assíncrono

Consulta somente de leitura no Compose local em 2026-11-01 encontrou:

| Configuração | Valor |
| --- | --- |
| server_version | 18.6 |
| io_method | worker |
| io_workers | 3 |
| effective_io_concurrency | 16 |
| maintenance_io_concurrency | 16 |
| io_max_concurrency | 64 |

AIO já está habilitado pelo método padrão `worker`. Mantivemos os valores atuais;
não é necessário alterar SQLx, transações ou o runtime Tokio para aproveitá-lo.
O mecanismo se aplica a leituras elegíveis como sequential scans, bitmap heap scans
e vacuum, não a todas as operações de disco ou a qualquer consulta indexada.
Em um banco pequeno e majoritariamente em cache, o efeito pode ser pequeno.

Não habilitamos `io_uring` automaticamente: ele depende de build com liburing e de
suporte/permissões do kernel/container. Não se deve relaxar isolamento do container
para buscar ganho sem evidência. `io_method` exige reinício; sua avaliação deve
ocorrer num ambiente isolado, fora das migrações de schema.

Para revalidar configuração, o artefato [diagnóstico SQL](sql/postgresql-18.sql)
consulta apenas metadados. Antes de tuning, medir consultas reais (resumos, overview,
listagens por proprietário) com `EXPLAIN (ANALYZE, BUFFERS)` em base descartável
representativa, repetindo com cache quente e frio, mesma concorrência e dataset.
Comparar p50/p95, throughput, buffers e I/O, incluindo escrita/WAL para UUIDs. Não
limpar caches do host de desenvolvimento/produção. Para AIO, comparar worker e sync
e, somente se suportado, io_uring após reiniciar a instância de benchmark. Preservar
os filtros e RLS nos planos medidos; um scan irrestrito não representa a API.

Fontes: [UUID no PostgreSQL 18](https://www.postgresql.org/docs/18/functions-uuid.html),
[colunas geradas](https://www.postgresql.org/docs/18/ddl-generated-columns.html),
[configuração de I/O](https://www.postgresql.org/docs/18/runtime-config-resource.html#RUNTIME-CONFIG-RESOURCE-IO)
e [guia de AIO citado](https://betterstack.com/community/guides/databases/postgresql-asynchronous-io/).
