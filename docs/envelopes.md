# Envelopes e distribuição da renda

Para regressão com login automático, use [Playwright E2E](api/e2e.md).
Para exploração manual, há também a [coleção Postman e seu guia](api/README.md).
O roteiro de console abaixo permanece como alternativa.

Um envelope representa uma finalidade para o dinheiro de um ciclo: moradia,
alimentação ou reserva, por exemplo. Ele não representa uma conta bancária.
Os envelopes planejam a distribuição; [despesas manuais](despesas.md) registram
a execução e atualizam o saldo para gastar. A [ativação do
ciclo](ciclo-de-vida.md) confirma o planejamento e bloqueia novas alterações.

## Regras e contrato

- Cada envelope pertence a um ciclo e ao proprietário autenticado desse ciclo.
- O nome é aparado nas extremidades e aceita de 1 a 100 caracteres Unicode, sem
  caracteres de controle. A unicidade é por ciclo e diferencia maiúsculas de
  minúsculas: `Casa` e `casa` são nomes distintos. Não há normalização Unicode.
- `allocated_cents` é um inteiro de centavos, de zero até `i64::MAX`.
- A soma dos envelopes nunca pode ultrapassar a renda do ciclo. Valor zero é
  válido. Reduzir ou excluir uma alocação libera o valor para distribuição.
- `GET /api/cycles` e `GET /api/cycles/{id}` agora calculam `allocated_cents` e
  `available_cents` a partir das alocações persistidas. A criação ainda retorna
  alocado zero e disponível igual à renda.
- Criação, edição e remoção só são permitidas em ciclos `planned`; em `active`
  e `closed`, retornam 409. A leitura permanece disponível com `budget:read`.
- A resposta inclui `spent_cents` (despesas não estornadas) e `remaining_cents`
  (alocado menos gasto). A alocação permanece inalterada por despesas e estornos.
- Não há edição da renda, contas financeiras ou histórico de alterações da alocação.

| Método | Rota | Permissão | Sucesso |
| --- | --- | --- | --- |
| POST | `/api/cycles/{cycle_id}/envelopes` | `budget:write` | 201 + envelope |
| GET | `/api/cycles/{cycle_id}/envelopes` | `budget:read` | 200 + array |
| PATCH | `/api/cycles/{cycle_id}/envelopes/{id}` | `budget:write` | 200 + envelope |
| DELETE | `/api/cycles/{cycle_id}/envelopes/{id}` | `budget:write` | 204 |

Criação:

```json
{"name":"Moradia","allocated_cents":200000}
```

Alteração parcial (um ou ambos os campos):

```json
{"allocated_cents":180000}
```

PATCH exige pelo menos um campo. Omitir um campo preserva o valor atual; `null`
é inválido. Campos desconhecidos, incluindo proprietário, ciclo e ID, são
rejeitados. Cada resposta de envelope contém `id`, `cycle_id`, `name` e
`allocated_cents`, `spent_cents` e `remaining_cents`.

A listagem aceita `limit` (padrão 20, de 1 a 100) e `offset` (padrão zero, até
100000), ordenada por UUID crescente. A paginação por offset pode mudar entre
consultas quando há criações ou remoções concorrentes.

As mutações exigem `X-CSRF-Token` e mantêm a validação de origem da API. Acesso
sem sessão retorna 401; sem permissão ou CSRF, 403. Ciclo ou envelope inexistente,
de outro proprietário ou pertencente a outro ciclo retorna 404. Nome repetido ou
alocação acima da renda ou mutação em ciclo ativo/fechado retorna 409. Entradas inválidas retornam 422, corpo maior
que 4096 bytes retorna 413, e indisponibilidade retorna 503. IDs e query strings
malformados seguem as rejeições 400 do Axum.

## Persistência e concorrência

A migração `0003_envelopes.sql` acrescenta tabela, índices, chave estrangeira
composta entre ciclo e proprietário, RLS forçada e trigger de proteção.
As migrações anteriores permanecem inalteradas.

O adaptador PostgreSQL abre uma transação READ COMMITTED, define o usuário
somente nessa transação e adquire um advisory lock exclusivo por ciclo. Após
adquirir o lock, consulta os valores atuais e aplica a regra de domínio antes de
persistir. O lock dura até commit ou rollback; hashes coincidentes apenas
serializam ciclos adicionais, sem enfraquecer o limite.

O trigger também adquire esse lock e compara a soma com a renda, inclusive para
SQL direto. Ele é SECURITY INVOKER e não eleva privilégios. Usa soma numeric para
não estourar bigint durante a verificação e rejeita mutações em outros níveis de
isolamento para não aceitar snapshots antigos. O runtime não pode alterar IDs,
proprietários, vínculo do envelope ou renda do ciclo.

As políticas SELECT de ciclos e envelopes permitem `budget:read` **ou**
`budget:write` para consultar os valores necessários às mutações. As rotas de
consulta continuam exigindo `budget:read`; escrita isolada não concede acesso HTTP
de leitura. As credenciais runtime são exclusivas do backend confiável, capaz de
definir o contexto de usuário; não devem ser entregues aos clientes.

O operador prepara a aplicação pelo comando já existente:

```sh
cargo run -p obz-api --example migrate
```

Forneça `DATABASE_ADMIN_URL` somente ao processo administrativo e
`DATABASE_RUNTIME_ROLE` com o papel runtime. Em loopback local, use também
`AUTH_ALLOW_LOCAL_HTTP=true`. Além dos privilégios anteriores, a ferramenta
concede em `budgeting.envelopes`: SELECT, DELETE, INSERT de id/cycle_id/owner_id/
name/allocated_cents e UPDATE somente de name/allocated_cents. Não concede novos
direitos financeiros a usuários. A migração de ciclo de vida concede UPDATE
somente de status em ciclos; DELETE de ciclos continua proibido.

## Roteiro manual no navegador

Inicie os serviços e a API conforme o guia da API e faça login em
`http://localhost:3000/auth/session`. O usuário de teste precisa das permissões
explícitas `budget:read` e `budget:write`; use a CLI administrativa documentada
no guia de identidades. Não publique cookies nem tokens de sessão.

Na página `/auth/session`, execute no console, substituindo `ID_DO_CICLO` por um
ciclo seu em estado `planned` que tenha saldo disponível de pelo menos R$ 2.000:

```js
(async () => {
  const cycle = 'ID_DO_CICLO';
  const session = await fetch('/api/auth/me');
  if (!session.ok) throw new Error(`Sessão: HTTP ${session.status}`);
  const user = await session.json();
  const base = `/api/cycles/${cycle}/envelopes`;
  const write = (method, path, body) => fetch(path, {
    method,
    headers: {
      'Content-Type': 'application/json',
      'X-CSRF-Token': user.csrf_token
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) })
  });

  const created = await write('POST', base, {
    name: 'Moradia', allocated_cents: 200000
  });
  const envelope = await created.json();
  console.log('Criação:', created.status, envelope);
  if (created.status !== 201) return;

  const updated = await write('PATCH', `${base}/${envelope.id}`, {
    allocated_cents: 180000
  });
  console.log('Alteração:', updated.status, await updated.json());
  const listed = await fetch(base);
  console.log('Envelopes:', listed.status, await listed.json());
  const budget = await fetch(`/api/cycles/${cycle}`);
  console.log('Ciclo:', budget.status, await budget.json());
  // O envelope criado permanece no ciclo para revisão.
})().catch(console.error);
```

Para remover um envelope, envie DELETE à mesma rota individual com o header CSRF.
Isso devolve sua alocação ao saldo disponível e retorna 204, sem corpo JSON.

## Validação automatizada

A suíte de domínio testa nomes, valores negativos, overflow, alocação parcial,
completa, redução e remoção. Os casos de uso usam um repositório falso para
validar permissões, entradas e encaminhamento do proprietário autenticado.

Os testes HTTP/PostgreSQL usam bancos descartáveis e cobrem CRUD, duplicidade,
paginação, CSRF, revogação, RLS, propriedade imutável, renda zero e `i64::MAX`.
A prova de concorrência inclui criações/alterações simultâneas e SQL direto: a
segunda escrita aguarda o lock da primeira e recalcula o saldo após seu commit.
Esses testes não alteram o banco de desenvolvimento ou permissões de seus usuários.

## Script de teste da API

O arquivo [`scripts/test-envelopes.js`](../scripts/test-envelopes.js) automatiza o
roteiro no navegador. Após iniciar a API atualizada, faça login em `/auth/session`,
abra o console e cole o conteúdo do arquivo. O script solicita o UUID de um ciclo
planejado com ao menos 2 centavos disponíveis. O usuário precisa de `budget:read`
e `budget:write` previamente concedidos.

Use um ciclo de teste sem outras edições durante a execução. O script cria nomes
únicos, valida CRUD, totais, erros de entrada, CSRF e duas criações concorrentes.
Ele remove somente os envelopes que criou e confere a restauração do saldo,
inclusive se alguma verificação falhar. Não cria ciclos ou concede permissões.
Não testa isolamento entre duas sessões nem revogação; esses cenários estão na
suíte Rust/PostgreSQL. Valores acima do limite inteiro seguro do JavaScript são
recusados pelo script e permanecem cobertos pelos testes Rust.

Uma falha de rede pode impedir que o navegador saiba se uma criação foi persistida.
Nesse caso, confira os envelopes com prefixo `Teste-` antes de executar novamente.
Não cole o script no visualizador JSON de `/api/auth/me`, cuja CSP pode bloquear
as requisições. O script mantém cookies e CSRF dentro da sessão do navegador.

O script aceita erros HTTP tanto em JSON quanto em texto, pois as rejeições de
campos desconhecidos e tipos inválidos do Axum podem retornar `text/plain`.
As respostas de sucesso com corpo continuam exigindo JSON. Para verificar o
próprio script sem serviços externos (API simulada e limpeza após falha), execute:

```sh
node --test scripts/test-envelopes.test.cjs
```
