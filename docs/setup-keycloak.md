# Keycloak no ZeroBudget

O backend usa Keycloak 26.8.1 diretamente por OIDC. O Dex foi retirado do Compose
e dos testes. Como o ambiente contém apenas dados descartáveis, não existe
migração nem vinculação das identidades antigas. Nunca associe usuários por e-mail.

## Ambiente local

```sh
docker compose up -d --build
AUTH_ALLOW_LOCAL_HTTP=true \
DATABASE_ADMIN_URL='postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget' \
DATABASE_RUNTIME_ROLE=zerobudget_runtime \
cargo run --locked -p obz-api --example migrate
set -a
. ./.env.example
set +a
cargo run --locked -p obz-api
```

| Serviço | Endereço |
| --- | --- |
| Sessão técnica da API | http://localhost:3000/auth/session |
| Login | http://localhost:3000/api/auth/login |
| Cadastro | http://localhost:3000/api/auth/signup |
| Recuperação | http://localhost:3000/api/auth/recover |
| Keycloak Admin Console | http://localhost:8080/admin |
| E-mails locais no Mailpit | http://localhost:8025 |

Conta de demonstração: `admin@zerobudget.local` / `Local-Password-123!`.
Operador do Keycloak: `operator` / `local-keycloak-operator-password`.
Essas credenciais e a configuração SMTP sem TLS servem exclusivamente para loopback.
O operador do Keycloak não recebe permissões financeiras no ZeroBudget.

O cadastro abre a página padrão do provedor, exige nome e e-mail, envia a
confirmação ao Mailpit, pede a definição da senha após a verificação e só então
permite o callback. Abra o link recebido no mesmo
navegador que iniciou o cadastro. Uma tentativa OIDC dura até 15 minutos; o link
de e-mail expira em 10 minutos. Se expirar, inicie o fluxo novamente. Recuperação
retorna mensagem genérica, inclusive para contas inexistentes. O link é de uso único.

O primeiro login provisiona a identidade local e **não concede budget:read/write**.
O operador concede acesso pelo procedimento existente de auth_admin. Uma política
de onboarding financeiro automático não faz parte desta troca de provedor.

O Compose mantém um PostgreSQL próprio para o Keycloak, separado do PostgreSQL
da aplicação. Alterar realm.json não atualiza um realm que já existe: o import é
somente na primeira inicialização. Ajustes posteriores usam o Admin Console ou
nova inicialização explícita do banco descartável do Keycloak. Não use down -v em
ambientes com dados que precisam ser preservados.

## Contrato de autenticação

- Authorization Code com PKCE S256; cliente confidencial `zerobudget`, redirect
  exato `/api/auth/callback`, sem grants direto/implicit/client_credentials.
- OIDC_ISSUER substitui DEX_ISSUER; OIDC_CLIENT_ID e OIDC_CLIENT_SECRET substituem
  as variáveis antigas. Não há fallback para a configuração do Dex.
- `OidcAuth` substitui `DexAuth`. `AuthConfig::keycloak()` exige email_verified=true
  e introspection_endpoint no discovery. A API sempre habilita essa política.
- O realm inclui audience explícita zerobudget no access token para introspecção.
  Claims e roles do provedor não concedem direitos financeiros na aplicação.
- O ID de sessão é opaco e fica no cookie HttpOnly. O access token permanece
  somente no armazenamento interno do servidor para introspecção a cada requisição
  autenticada. Nunca é aceito como bearer da API ou retornado em /api/auth/me.
- Issuer, subject, client_id, active e exp da introspecção devem corresponder à
  identidade e ao cliente. Resposta inválida/inativa resulta em 401; indisponibilidade
  resulta em 503. Não existe fallback de autenticação se o provedor falhar.
- Sessões Redis usam versão 2; registros da versão anterior são recusados e exigem
  novo login. Sessão tem limite absoluto de 30 minutos e não é renovada automaticamente.
  Use TLS, ACL e proteção dos volumes/backups do Redis, pois ele contém credenciais.
- `/ready` consulta banco, sessões e JWKS do Keycloak; falha em qualquer um retorna 503.
- Logout da API revoga somente aquela sessão local. O SSO do Keycloak pode continuar
  ativo. Recuperação/troca de senha encerra as demais sessões do provedor e a
  introspecção faz suas sessões locais deixarem de autenticar na próxima requisição.

## Política de recuperação

O Keycloak oferece uma opção de encerrar outras sessões no formulário. Como o
requisito do ZeroBudget exige revogação, a imagem inclui ForceLogoutPassword,
uma extensão mínima do Required Action UPDATE_PASSWORD que força essa opção.
Não processamos senhas, e-mails ou links de recuperação no backend Rust.

O provedor estende a implementação da versão fixada do Keycloak e é compilado
contra seus próprios JARs. O teste com dois navegadores desmarca a opção no
formulário e exige revogação de ambos. Toda atualização da imagem deve executar
essa regressão: a extensão usa SPI interno e precisa acompanhar a versão.
As imagens-base do Keycloak e do compilador Java estão fixadas por digest.

## Verificação

```sh
./scripts/e2e test auth.spec signup.spec
./scripts/e2e verify
./scripts/e2e mcp-check
```

E2E usa portas 3100 (API), 8180 (Keycloak), 8125 (Mailpit), 5540 (PostgreSQL) e
6480 (Redis). Os serviços, contas e e-mails são exclusivos do run e removidos ao
final. O teste de cadastro não precisa de permissão financeira; demais regressões
concedem permissões somente aos usuários descartáveis. Não se enviam e-mails reais.

## Produção

Construa e use a mesma imagem Keycloak com ForceLogoutPassword; não use start-dev.
Use `start` com PostgreSQL externo, hostname HTTPS fixo, certificado/proxy confiável,
segredos externos e SMTP autenticado com TLS. Configure o cliente, audience,
verificação de e-mail, proteção de força bruta e política de senha conforme o realm
local, substituindo URLs e segredos. Não importe usuários/senhas locais em produção.
O backend precisa alcançar discovery, JWKS, token e introspecção no issuer público.
Configure limites de requisições de autenticação no proxy e no provedor de e-mail.

Inclua o banco do Keycloak no plano de backup: o dump do banco financeiro não
contém senhas/credenciais do provedor. Perder/recriar o realm muda a identidade e
não deve ocorrer em produção. Nesta entrega o ambiente de desenvolvimento é novo,
conforme a decisão de descartar as identidades anteriores.

Referências: [administração](https://www.keycloak.org/docs/26.8.0/server_admin/),
[containers](https://www.keycloak.org/server/containers),
[Required Action UpdatePassword](https://github.com/keycloak/keycloak/blob/26.8.1/services/src/main/java/org/keycloak/authentication/requiredactions/UpdatePassword.java).
