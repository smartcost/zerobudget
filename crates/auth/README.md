# obz-auth

Núcleo de autenticação de usuários pelo Keycloak e autorização do ZeroBudget.
O crate é uma biblioteca independente de Axum, banco de dados e regras financeiras.

## Fluxo de login

1. `OidcAuth::discover` verifica a descoberta OIDC e carrega as chaves públicas do provedor configurado.
2. `begin_login` cria a URL de autorização e uma `LoginAttempt` com state, nonce e PKCE S256 aleatórios.
3. O adaptador HTTP guarda a tentativa **no servidor, vinculada à sessão do navegador**, por no máximo quinze minutos, e redireciona para a URL.
4. No callback, o adaptador remove a tentativa atomicamente e chama `finish_login` com state e code recebidos. Não deve selecionar tentativas apenas pelo state fornecido na URL: a sessão iniciadora também precisa corresponder.
5. O crate troca o código com o verificador PKCE e valida assinatura RS256, issuer, audience, expiração, nonce, subject não vazio, emissão futura (tolerância de 60 segundos) e `at_hash`, quando presente.
6. O resultado é uma `Identity` verificada, identificada pelo par `(issuer, subject)`. Access tokens e refresh tokens não são expostos.

```rust,no_run
use obz_auth::{AuthConfig, AuthError, OidcAuth, Identity, LoginAttempt};

async fn create_auth(secret: String) -> Result<OidcAuth, AuthError> {
    OidcAuth::discover(AuthConfig::new(
        "https://login.example.com/realms/zerobudget".into(),
        "zerobudget".into(),
        secret,
        "https://app.example.com/api/auth/callback".into(),
        false,
    )?.keycloak()).await
}

async fn callback(
    auth: &OidcAuth,
    browser_bound_attempt: LoginAttempt,
    returned_state: &str,
    code: String,
) -> Result<Identity, AuthError> {
    auth.finish_login(browser_bound_attempt, returned_state, code).await
}
```

URLs exigem HTTPS. Apenas no desenvolvimento, `allow_loopback_http = true`
permite HTTP para `localhost`, `127.0.0.1` e `::1`. A configuração deve vir de
uma fonte confiável do servidor, nunca de parâmetros enviados pelo usuário.
O cliente HTTP tem timeout de dez segundos e não segue redirecionamentos.

## Autorização

Para a API, as permissões atuais vêm de `IdentityRepository` e são aplicadas
por `Authenticated::require`/`require_owner`. O registro local usa `(issuer, subject)`
e não cria concessões automaticamente. Veja o [guia de persistência](../../docs/identidades-e-permissoes.md).

A `Policy` abaixo é uma alternativa em memória para consumidores independentes
do crate; não substitui a consulta ao PostgreSQL na API. `Policy` nega acesso por padrão. Permissões são concedidas por configuração
confiável da aplicação para `(issuer, subject)`, nunca por email ou campos da
requisição. Não há promoção automática a administrador.

```rust
use obz_auth::{AuthError, Identity, Permission, Policy};

fn read_budget(
    policy: &Policy,
    identity: &Identity,
    owner_issuer_from_database: &str,
    owner_subject_from_database: &str,
) -> Result<(), AuthError> {
    policy.require_owner(
        identity,
        Permission::ReadBudget,
        owner_issuer_from_database,
        owner_subject_from_database,
    )
}
```

O adaptador deve carregar o proprietário do recurso do banco, ou filtrar suas
consultas pelo par autenticado. IDs de proprietário enviados pelo cliente não
são prova de propriedade. `require` e `require_owner` rejeitam identidades
expiradas, mesmo com permissão cadastrada.

## Integração HTTP e sessões

O [crate API](../api/README.md) implementa login, callback, consulta de identidade
e logout por meio do contrato assíncrono `SessionStore`. `MemorySessionStore`
é o backend explícito de desenvolvimento; `RedisSessionStore` compartilha sessões
e tentativas entre instâncias. A API não mantém um mutex global de armazenamento.

Sessões têm duração absoluta máxima de 30 minutos, limitada pela expiração da
identidade. Tentativas duram até quinze minutos e são consumidas uma única vez,
inclusive entre réplicas. O Redis usa registros internos versionados, TTL,
chaves derivadas por SHA-256, admissão atômica de capacidade e revogação de sessão.
A reconstrução de identidade é restrita ao módulo de armazenamento confiável;
`Identity` não implementa `Deserialize` público.

Reiniciar a API preserva os registros do Redis. Memória continua sendo por
processo e perde as sessões no reinício. Não há fallback automático se Redis
falhar. O adaptador usa cookies `HttpOnly`, `SameSite=Lax`, `Secure` em HTTPS e
proteção CSRF para alterações. Consulte o guia da API para configuração e
limitações de durabilidade do Redis, namespaces, ACL e recuperação de backups.

Mapeamento HTTP: falhas de autenticação → 401; `Forbidden` e CSRF → 403;
indisponibilidade do provedor/armazenamento ou capacidade esgotada → 503. Erros não carregam
códigos, tokens ou segredos.

As chaves ficam em memória durante a vida de `OidcAuth`. A API recria e substitui
atomicamente a instância a cada cinco minutos para acompanhar a rotação do Keycloak.
Chaves desconhecidas são rejeitadas até a atualização. Consumidores independentes
do crate precisam definir sua própria rotina de atualização. Ainda não há
renovação de sessão ou logout federado. Identidades e permissões da API são
persistidas no PostgreSQL separado do Keycloak, com consultas atuais por requisição.

## Limites desta etapa

Somente usuários, conforme o escopo inicial. Autenticação entre APIs fica para
uma etapa posterior. Um client secret registrado no Keycloak identifica o cliente
OIDC na troca do código; ele não constitui uma credencial para chamar a API.
Access tokens do Keycloak ficam no servidor e não autenticam chamadas bearer da API.
O ID token é validado exclusivamente no callback, e não aceito como bearer em
rotas de negócio. O cliente configurado não habilita grants entre serviços.

## Verificação

```bash
cargo test -p obz-auth
cargo clippy -p obz-auth --all-targets -- -D warnings
AUTH_TEST_REDIS_URL=redis://127.0.0.1:6380/0 cargo test -p obz-auth -- --include-ignored
```

Os testes cobrem PKCE, state, prazo do login, configuração TLS, tokens RS256
assinados, issuer/audience/nonce incorretos, expiração, assinatura adulterada,
chave desconhecida, substituição de access token e isolamento de proprietários.
A chave privada em `tests/fixtures/test-only-rsa.pem` é pública e exclusiva dos
testes. A regressão E2E usa Keycloak real, cadastro e recuperação com Mailpit.

Referências: [Keycloak](https://www.keycloak.org/docs/26.8.0/server_admin/),
[biblioteca openidconnect](https://docs.rs/openidconnect/4.0.1/openidconnect/).


O contrato `IdentityRepository` pertence a este crate. Sua implementação PostgreSQL,
`PostgresIdentityRepository`, e a CLI `auth_admin` pertencem a `obz-infrastructure`.
Este crate não depende de SQLx. Implementadores externos reconstituem `LocalUser`
com `from_persisted` somente após validar a identidade, o estado da conta e suas
permissões atuais; `Identity::validate_current` verifica a validade temporal sem
permitir a construção de identidades fora do fluxo OIDC verificado.

Cadastro e recuperação: [configuração e política](../../docs/setup-keycloak.md).
A API exige e-mail verificado e introspecção online em cada requisição autenticada;
indisponibilidade do Keycloak retorna 503. Tokens ficam em registros privados versão 2.
