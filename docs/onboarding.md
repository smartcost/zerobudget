# Onboarding financeiro

Login e cadastro continuam sem concessões financeiras. Após o login com e-mail
verificado, o próprio usuário conclui explicitamente seu onboarding com
`POST /api/onboarding`, enviando `{}` e X-CSRF-Token. Nenhum ID, permissão ou
proprietário é aceito no corpo. A habilitação concede somente budget:read e
budget:write ao orçamento do usuário autenticado.

## Contrato

| Método | Rota | Resultado |
| --- | --- | --- |
| GET | `/api/onboarding` | Estado e próximo passo, sem alterações |
| POST | `/api/onboarding` | Conclusão inicial ou resultado atual de uma conclusão anterior |

Ambas exigem sessão válida, introspecção ativa e e-mail verificado. Não exigem
permissões financeiras prévias. POST exige JSON `{}` e CSRF; campos extras são
rejeitados. Sucesso retorna 200 com o mesmo esquema Onboarding do OpenAPI.

```json
{
  "status": "completed",
  "completed_at": 1793489400,
  "permissions": ["budget:read", "budget:write"],
  "setup": {
    "has_accounts": false,
    "has_cycles": false,
    "has_active_cycle": false,
    "has_expenses": false,
    "selected_cycle": null
  },
  "next_step": "create_account"
}
```

completed_at é o instante original em segundos Unix; não muda em retries.
Não é necessário request_id: existe uma única conclusão por usuário. Chamadas
simultâneas são serializadas no mesmo registro local e têm um único efeito.

| status | Significado |
| --- | --- |
| pending | Ainda não concluiu e não tem concessões/decisões administrativas anteriores |
| completed | Existe registro permanente de conclusão, mesmo após revogação de direitos |
| managed | Sem conclusão e com concessões ou decisões administrativas anteriores |

Para contas managed, POST retorna 409 e preserva direitos. Um operador pode
continuar concedendo/revogando acesso pelo auth_admin; o onboarding não substitui
essas decisões nem impede uso financeiro já autorizado. O estado managed não é
um bloqueio: com read/write, o progresso continua indicando a preparação financeira.

## Revogação, bloqueio e auditoria

A primeira habilitação cria em uma transação:

- Um registro imutável em auth.budget_onboardings.
- As duas concessões fixas em auth.permissions.
- Dois eventos grant e um evento onboard em auth.audit_events.

O usuário só é elegível se estiver habilitado e não tiver concessões nem qualquer
decisão administrativa anterior. Assim, uma concessão removida anteriormente não
pode ser restaurada por auto-habilitação. O registro de conclusão permanece após
revogação. POST repetido em conta completed devolve os direitos atuais e nunca os
repara: sem acesso integral, next_step será contact_operator. GET, login e consulta
de progresso não concedem nada. Uma conta bloqueada retorna 403, inclusive no POST;
reabilitar a conta pelo operador não recria permissões revogadas.

O lock da linha auth.users é compartilhado com as alterações administrativas. Se
a revogação ocorrer primeiro, a elegibilidade é recusada; se o onboarding ocorrer
primeiro, o operador espera seu commit e a revogação prevalece em seguida. Falha
na auditoria reverte também marcador e concessões, permitindo uma tentativa limpa.

A API usa apenas a credencial runtime. A migração 0010 cria funções SECURITY
DEFINER, com search_path fixo e EXECUTE público revogado; o operador concede ao
runtime EXECUTE somente nas duas funções. Runtime continua sem INSERT/DELETE em
permissions, sem UPDATE de enabled e sem escrita no marcador ou na auditoria.
As funções conferem o contexto transacional e o par issuer/subject do usuário; a
verificação OIDC/e-mail é responsabilidade do caso de uso com Identity verificada.
Nenhum papel ou claim enviado pelo cliente define as concessões.

## Progresso inicial

setup só é retornado com budget:read atual; caso contrário é null. A consulta usa
snapshot REPEATABLE READ e filtros/RLS por proprietário. has_expenses inclui o
histórico de despesas estornadas, pois registrar a primeira despesa é um marco
que um estorno não desfaz. Nenhum recurso financeiro é criado pelo onboarding.

selected_cycle escolhe primeiro o ciclo ativo mais recente por period_start;
na ausência dele, o planejado mais recente; depois, o fechado mais recente.
Empates são resolvidos por UUID. Isso torna a escolha determinística mesmo com
vários ciclos. O campo informa id, status, has_envelopes e has_unallocated_income.

| next_step | Situação |
| --- | --- |
| complete_onboarding | Conta elegível ainda não habilitada |
| contact_operator | Direitos insuficientes ou conta sob decisão administrativa sem acesso integral |
| create_account | Não há conta bancária/carteira |
| create_cycle | Não há ciclo ou o ciclo selecionado está fechado |
| allocate_income | Ciclo planejado com renda ainda não distribuída |
| activate_cycle | Planejamento totalmente distribuído; inclui renda zero sem envelopes |
| record_expense | Ciclo ativo e nenhuma despesa no histórico |
| ready | Ciclo ativo e primeira despesa já registrada |

Essa indicação orienta a jornada; não relaxa saldos, datas, fundos bancários ou as
regras dos endpoints financeiros. A interface poderá usar o estado e os IDs sem
reimplementar a seleção do ciclo. A implementação da interface fica para outra etapa.

## Preparação e testes

```sh
AUTH_ALLOW_LOCAL_HTTP=true \
DATABASE_ADMIN_URL='postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget' \
DATABASE_RUNTIME_ROLE=zerobudget_runtime \
cargo run --locked -p obz-api --example migrate
./scripts/e2e test onboarding.spec
./scripts/e2e verify
```

Depois da atualização, reinicie a API e faça login novamente. Registros privados
de sessão passam à versão 3 para guardar o resultado verificado de email_verified;
registros anteriores são recusados, sem inferir verificação a partir do e-mail.

O E2E completo percorre signup no Keycloak, e-mail no Mailpit, onboarding sem CLI
de concessões, conta, ciclo, envelopes, ativação e primeira despesa. Regressões
adicionais cobrem retries concorrentes, CSRF, campos proibidos, read-only, isolamento,
revogação após novo login, bloqueio, renda zero, rollback da auditoria e SQL runtime.

Validação local em 31/10/2026: `./scripts/e2e verify` passou com 59 cenários E2E,
87 testes Rust e 2 doctests, incluindo PostgreSQL/Redis, recuperação das 18 tabelas
e reaplicação dos privilégios runtime. Clippy, formatação, tipos TypeScript e build
da imagem da API também passaram. Migração 0010 aplicada no banco local; não
foram habilitados usuários reais durante a preparação.
