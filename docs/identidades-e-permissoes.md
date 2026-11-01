# Identidades locais e permissões persistentes

Status: implementado e validado. A migração e os privilégios mínimos do papel
da API foram aplicados no banco local da aplicação, após autorização explícita.

## Comportamento

Após validar o login OIDC no Keycloak, a API cadastra ou encontra o usuário pelo par
`(issuer, subject)`. Esse par é único no banco e preserva o UUID local, inclusive
em logins concorrentes e em reinícios da API. Email e nome não identificam contas,
não são usados para vincular identidades e não concedem permissões.

O primeiro login cria uma conta habilitada, sem permissões financeiras. O schema
aceita `budget:read` e `budget:write`, concedidos explicitamente pelo operador.
Leitura não implica escrita e nenhuma permissão dispensa conferir o proprietário
do recurso financeiro. Issuer e subject têm limites de 2048 e 255 bytes,
respectivamente, mantendo a chave única dentro dos limites do índice PostgreSQL.

Cada requisição com `Authenticated` consulta no PostgreSQL o estado atual da
conta e suas permissões. Redis guarda a sessão, não uma cópia das permissões.
Assim, concessões, revogações e bloqueios afetam a próxima requisição protegida,
sem exigir um novo login. Requisições já em andamento não são canceladas.

Conta bloqueada retorna 403 e não pode completar novos logins. Desbloquear permite
novamente as sessões que ainda não expiraram: bloqueio não equivale a excluir
permanentemente todos os IDs de sessão. Logout continua revogando a sessão atual
mesmo se a conta estiver bloqueada ou o banco da aplicação estiver indisponível,
desde que Redis, o cookie e o token CSRF estejam válidos.

A resposta de `/api/auth/me` mantém os campos anteriores e acrescenta:

```json
{
  "user_id": "uuid-local-do-usuario",
  "permissions": ["budget:read"]
}
```

A página `/auth/session` exibe esses campos. Não há endpoint HTTP de administração
nem concessão automática a usuários chamados `admin` no Keycloak.

## Banco separado e privilégios

O serviço `app-postgres` do Compose usa banco `zerobudget`, volume próprio e porta
local `127.0.0.1:5440`. O PostgreSQL do Keycloak permanece separado, com seu banco e
volume anteriores. `APP_POSTGRES_PORT` altera a porta publicada; ajuste as URLs.

A migração `migrations/0001_identity_and_permissions.sql` cria:

- `auth.users`: UUID, issuer, subject, habilitação e datas de criação/último login.
- `auth.permissions`: permissões explícitas por UUID de usuário, com chave única.
- `auth.audit_events`: concessão, revogação, habilitação e bloqueio realizados pela
  CLI. As mudanças e seu evento de auditoria são confirmados na mesma transação.

A auditoria usa `session_user` do PostgreSQL como ator. Para distinguir operadores
em produção, use credenciais de banco individuais. Ela registra as operações da
CLI; um proprietário do banco continua sendo uma autoridade confiável capaz de
alterar dados diretamente.

Dois papéis locais têm funções diferentes:

| Papel | Finalidade | Permissões relevantes |
| --- | --- | --- |
| `zerobudget_admin` | Migração/administração local | Proprietário do banco |
| `zerobudget_runtime` | Processo da API | `USAGE` no schema auth; `SELECT` em users/permissions; `INSERT` de id/issuer/subject/last_login_at; `UPDATE` apenas de last_login_at |

O papel da API não pode conceder/revogar permissões, modificar habilitação ou
identidade, excluir usuários ou gravar auditoria. A API recebe somente
`DATABASE_URL` de runtime. O processo da API não aplica migrações e não recebe
`DATABASE_ADMIN_URL`.

## Preparação local

```bash
docker compose up -d

# Processo administrativo separado, com credenciais exclusivas de desenvolvimento.
AUTH_ALLOW_LOCAL_HTTP=true \
DATABASE_ADMIN_URL=postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget \
DATABASE_RUNTIME_ROLE=zerobudget_runtime \
cargo run -p obz-infrastructure --example auth_admin -- migrate

# Processo da API: .env.example contém apenas credenciais de runtime ativas.
set -a
. ./.env.example
set +a
cargo run -p obz-api
```

A CLI aplica migrações SQLx e, se `DATABASE_RUNTIME_ROLE` for fornecido, concede
os privilégios mínimos descritos acima. O comando é idempotente e deve ser usado
com o proprietário do banco dedicado da aplicação. O papel de runtime é criado
pelo script de inicialização do serviço local; em um volume existente, scripts
`docker-entrypoint-initdb.d` não são executados de novo. Nunca apague volumes para
corrigir configuração: use provisionamento explícito de papéis.

A API verifica se o schema está acessível ao iniciar. Schema ausente ou conexão
indisponível impede a inicialização. Após iniciar, indisponibilidade do PostgreSQL
retorna 503 nas consultas de identidade e nas rotas protegidas; não ativa fallback
para memória nem aceita permissões do cliente. Uma sessão Redis de uma versão
anterior, sem usuário local cadastrado, retorna 401: basta fazer novo login.

## Administração de permissões

Copie o `user_id` da sessão. Use a CLI somente no terminal confiável de um
operador e com `DATABASE_ADMIN_URL` nesse processo, nunca no ambiente da API:

```bash
AUTH_ALLOW_LOCAL_HTTP=true \
DATABASE_ADMIN_URL=postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget \
cargo run -p obz-infrastructure --example auth_admin -- grant USER_UUID budget:read
```

As demais operações usam o mesmo prefixo de ambiente:

```text
auth_admin revoke USER_UUID budget:read
auth_admin grant USER_UUID budget:write
auth_admin disable USER_UUID
auth_admin enable USER_UUID
```

Concessões/revogações repetidas são idempotentes. Usuário inexistente e permissão
não reconhecida são recusados. Bloqueio não apaga a identidade nem muda o UUID.
Não há papel de administrador da aplicação, vinculação por email ou self-service
de permissões nesta fase.

## Uso em handlers

Use `Authenticated::require` ou `require_owner`, que consultam o snapshot atual
carregado do banco no começo da requisição. O proprietário deve vir de uma
consulta confiável ao recurso, ou a consulta deve ser filtrada pelo UUID local.

```rust,ignore
async fn read_budget(authenticated: Authenticated) -> Result<StatusCode, ApiError> {
    authenticated.require(Permission::ReadBudget)?;
    // Repositório futuro: buscar o orçamento filtrando por authenticated.user.id().
    Ok(StatusCode::NO_CONTENT)
}
```

O exemplo descreve o contrato para a próxima funcionalidade financeira; não cria
uma rota pública de orçamento nesta entrega. A `Policy` em memória do crate auth
continua disponível para consumidores independentes, mas não é a fonte de
permissões da API. As alterações financeiras em andamento no domínio foram
preservadas.

## Configuração e produção

- `DATABASE_URL`: obrigatório, com o papel restrito da API.
- `DATABASE_TIMEOUT_MS`: padrão 3000, de 1 a 30000 ms por operação.
- `DATABASE_MAX_CONNECTIONS`: padrão 10, de 1 a 100 por processo.
- `DATABASE_ADMIN_URL`: somente na CLI de administração, nunca lido pela API.
- `DATABASE_RUNTIME_ROLE`: somente no comando administrativo de migração.

Produção usa TLS `VerifyFull` e senha, independentemente de um `sslmode` mais fraco
na URL. `sslrootcert` permite indicar uma CA confiável. Texto puro só é aceito
no modo local explícito e em loopback. Não são aceitas opções que alterem host,
porta ou parâmetros de sessão pelo query string. Queries são parametrizadas e o
logging de statements do SQLx está desativado para não registrar dados de login.
Credenciais e mensagens de erro de conexão não são devolvidas pelo HTTP.

## Validação

```bash
cargo test -p obz-api -p obz-auth
cargo clippy -p obz-api -p obz-auth --all-targets -- -D warnings

# Bases descartáveis e namespaces Redis exclusivos por teste.
AUTH_TEST_DATABASE_URL=postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget \
AUTH_TEST_RUNTIME_DATABASE_URL=postgres://zerobudget_runtime:runtime-dev-password@127.0.0.1:5440/zerobudget \
AUTH_TEST_REDIS_URL=redis://127.0.0.1:6380/0 \
cargo test -p obz-api -p obz-auth -- --include-ignored
```

Os testes PostgreSQL criam bases `zb_auth_test_<UUID>`, migram e concedem acesso
somente dentro delas e as removem ao terminar. Não aplicam a migração nem alteram
privilégios do banco `zerobudget`. Em caso de interrupção/falha, uma base de teste
pode permanecer para diagnóstico; remova somente bases comprovadamente criadas
pelo teste. O runtime de teste deve existir previamente no serviço local.

Resultado atual: 36 testes de auth/API e dois exemplos de documentação passaram
com PostgreSQL e Redis reais. Foram verificados cadastro concorrente e estável,
isolamento entre issuers, persistência, negação por padrão, revogação, bloqueio,
auditoria, ausência de privilégios administrativos no runtime e falha de
transporte sem concessão indevida. O teste HTTP usa um provedor OIDC local com
tokens RSA assinados. A validação manual com Keycloak real e a credencial restrita da API também passou:
foi criado um UUID local sem concessões financeiras, o UUID e a sessão permaneceram
válidos após reiniciar a API, e logout retornou 204 seguido de consulta 401.
Os privilégios aplicados foram conferidos por leitura no PostgreSQL. As operações
de concessão, revogação e bloqueio foram validadas somente nas bases descartáveis.

## Próxima entrega

Implementar a primeira operação financeira com o UUID local como proprietário,
permissão explícita e filtragem no repositório. Revogação global e irreversível de
sessões de uma conta (inclusive após desbloqueio) requer um mecanismo adicional,
como uma geração de sessão por usuário; não está incluída nesta fase.

## Localização da infraestrutura

`PostgresIdentityRepository`, `PostgresCycleRepository`, `Database`,
`DatabaseConfig` e o executor de migrações pertencem a `obz-infrastructure`.
O antigo crate `shared` foi removido. Os contratos de identidade permanecem em
`obz-auth`; os casos de uso e o contrato de ciclos permanecem em `obz-cycle`.
A API faz a composição das dependências. A CLI administrativa agora é executada
com `cargo run -p obz-infrastructure --example auth_admin -- ...`.
O comando de migração completa continua em `obz-api --example migrate`.
A mudança não altera o conteúdo nem o histórico das migrações existentes.
