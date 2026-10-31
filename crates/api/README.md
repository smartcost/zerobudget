# ZeroBudget API

Integração HTTP do `obz-auth` com Axum. O login usa Authorization Code com
PKCE no Dex; a aplicação mantém uma sessão própria no servidor. ID tokens e
access tokens do Dex não são aceitos como bearer em rotas de negócio.

## Desenvolvimento local

Na raiz do workspace:

```bash
docker compose up -d

# Execute uma vez antes de iniciar a API; credenciais apenas neste processo.
AUTH_ALLOW_LOCAL_HTTP=true \
DATABASE_ADMIN_URL=postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget \
DATABASE_RUNTIME_ROLE=zerobudget_runtime \
cargo run -p obz-api --example migrate

set -a
. ./.env.example
set +a
cargo run -p obz-api
```

Abra **http://localhost:3000/api/auth/login** no navegador e entre com
`admin` / `password`. Após autorizar no Dex, a API recebe o callback e
redireciona para `/auth/session`, uma página HTML com a identidade e o botão Sair.
O endpoint `/api/auth/me` retorna identidade, UUID local (`user_id`), permissões
atuais e token CSRF em JSON. O primeiro login não concede permissões financeiras.
Não é mais necessário copiar URLs para o terminal.

Use `localhost` durante todo o fluxo. Alternar para `127.0.0.1` muda o host do
cookie e invalida a vinculação da tentativa de login.

| Método | Rota | Comportamento |
| --- | --- | --- |
| GET | `/auth/session` | Página HTML para consultar sessão e sair |
| GET | `/health` | 204, sem autenticação; verifica apenas que a API responde |
| GET | `/api/auth/login` | Cria tentativa de cinco minutos e redireciona para Dex |
| GET | `/api/auth/callback` | Consome a tentativa e cria uma nova sessão após validar o ID token |
| GET | `/api/auth/me` | Identidade verificada, expiração Unix e `csrf_token`; 401 sem sessão |
| POST | `/api/auth/logout` | Exige sessão e `X-CSRF-Token`; revoga a sessão e remove o cookie |

Para testar logout, abra **http://localhost:3000/auth/session** e clique em
**Sair**. A página envia o token CSRF, confirma HTTP 204 e consulta novamente a
sessão para verificar HTTP 401. Não é necessário executar código no console.

O visualizador de JSON do Firefox pode aplicar sua própria CSP e bloquear
`fetch` executado no console de `/api/auth/me`. Use a página HTML acima; não
desative a CSP ou outras proteções do navegador.

Sem o header CSRF, logout retorna 403 e mantém a sessão ativa.
Logout encerra a sessão do ZeroBudget; ele não encerra a sessão do Dex ou do
provedor upstream. Um novo login pode aproveitar o SSO existente no Dex.

## Configuração

| Variável | Padrão | Uso |
| --- | --- | --- |
| `API_BIND` | `127.0.0.1:3000` | Endereço do listener |
| `API_PUBLIC_URL` | `http://localhost:3000` | Origem pública, sem caminho; determina callback e cookies seguros |
| `DEX_ISSUER` | `http://localhost:5556` | Issuer OIDC confiável |
| `DEX_CLIENT_ID` | `rust-app` | Cliente registrado no Dex |
| `DEX_CLIENT_SECRET` | obrigatório | Segredo do cliente; não é registrado em logs |
| `AUTH_ALLOW_LOCAL_HTTP` | `false` | Exceção explícita de desenvolvimento para HTTP em loopback |
| `AUTH_SESSION_BACKEND` | obrigatório | `redis` ou `memory`; memória exige modo local explícito |
| `AUTH_REDIS_URL` | obrigatório para Redis | Local: `redis://127.0.0.1:6380/0`; produção: `rediss://` autenticado |
| `AUTH_REDIS_NAMESPACE` | obrigatório para Redis | Identificador exclusivo por aplicação, issuer, cliente e ambiente |
| `AUTH_REDIS_TIMEOUT_MS` | `3000` | Timeout de conexão e operações; intervalo permitido: 1 a 30000 ms |
| `AUTH_MAX_PENDING` | `1024` | Capacidade de tentativas compartilhada pelo namespace |
| `AUTH_MAX_SESSIONS` | `4096` | Capacidade de sessões compartilhada pelo namespace |
| `DATABASE_URL` | obrigatório | Banco da aplicação com papel de runtime restrito |
| `DATABASE_TIMEOUT_MS` | `3000` | Timeout por operação PostgreSQL, até 30000 ms |
| `DATABASE_MAX_CONNECTIONS` | `10` | Limite de conexões PostgreSQL por instância, até 100 |

A aplicação lê variáveis do ambiente; não carrega `.env` automaticamente.
O exemplo acima exporta apenas a configuração local de `.env.example`; em
produção, forneça os valores pelo ambiente de implantação.
O callback é sempre `${API_PUBLIC_URL}/api/auth/callback` e precisa estar
registrado no Dex. Em produção, configure origem pública e issuer HTTPS,
forneça o segredo externamente e mantenha `AUTH_ALLOW_LOCAL_HTTP=false`.
Terminação TLS pode ocorrer no proxy; a origem pública configurada determina
`Secure`, sem confiar em headers encaminhados enviados pelo cliente.

## Segurança e armazenamento

- Cookies são host-only, `HttpOnly`, `SameSite=Lax`, com `Path=/`. Em HTTPS,
  recebem `Secure` e nomes com prefixo `__Host-`.
- `state`, `nonce` e o verificador PKCE permanecem somente no backend do servidor
  (Redis ou memória), sem serem expostos ao navegador.
  A tentativa é selecionada pelo cookie do navegador, removida atomicamente
  antes da troca do código e invalidada mesmo em callbacks recusados ou malformados.
- As chaves Redis usam SHA-256 dos IDs opacos; os payloads são internos e
  versionados. Uma identidade não pode ser reconstruída a partir de claims
  fornecidos por uma requisição.
- A sessão usa um ID opaco aleatório e é rotacionada após cada login válido.
  A sessão anterior do mesmo navegador é revogada quando houver um novo login.
- A duração absoluta máxima é de 30 minutos e nunca ultrapassa a expiração do
  ID token. Não há renovação automática nem cookie persistente além desse limite.
- `Authenticated` protege handlers com uma identidade validada. Para métodos
  que alteram dados, exige CSRF; quando `Origin` é enviado, exige a origem pública
  configurada. Logout aplica as mesmas verificações. CORS não está habilitado.
- A identidade local e as permissões ficam no PostgreSQL da aplicação, separado
  do Dex. Cada requisição protegida consulta o estado atual, aplicando bloqueios
  e revogações mesmo com uma sessão Redis válida.
- Permissões financeiras continuam negadas por padrão. Novos handlers devem
  usar `Authenticated::require` ou `require_owner` e conferir propriedade a
  partir do banco, além do extractor.
  O login não concede permissões financeiras automaticamente.
- Respostas usam `Cache-Control: no-store`, `Referrer-Policy: no-referrer` e
  `X-Content-Type-Options: nosniff`. Tokens, cookies e URLs de callback não são logados.
- Os limites padrão são 1.024 tentativas e 4.096 sessões, configuráveis pelo ambiente.
  Os limites são por namespace no Redis, com admissão atômica entre instâncias.
  TTL remove registros expirados; índices de capacidade são limpos nas escritas
  e a cada minuto. Capacidade esgotada retorna 503. O proxy de produção deve
  também limitar a taxa de login.
- Descoberta e JWKS são atualizados a cada cinco minutos. Uma falha mantém as
  últimas chaves disponíveis; chaves desconhecidas continuam rejeitadas.

## Armazenamento compartilhado

`AUTH_SESSION_BACKEND=redis` guarda sessões e tentativas fora do processo da
API. Reiniciar somente a API preserva registros ainda válidos. Sessões antigas
do backend em memória não são migradas: ao adotar Redis, faça novo login. Réplicas devem
usar o mesmo namespace, issuer, client ID, origem pública e limites de capacidade.
O backend recusa limites incompatíveis entre instâncias no mesmo namespace.
Para mudar limites, faça uma migração coordenada para um novo namespace;
identificadores antigos deixam de ser aceitos.

Login usa consumo atômico da tentativa, e a rotação da sessão revoga o ID
anterior na mesma operação de escrita. Logout verifica CSRF e remove a versão
exata da sessão consultada. Uma falha de rede não provoca repetição automática
da operação pela aplicação: se a resposta do consumo se perder, a tentativa
pode ter sido consumida e o usuário precisará iniciar um novo login.

Conexão e operações têm timeout. Redis indisponível retorna 503 para operações
que dependem da sessão; sessão ausente, expirada ou registro incompatível retorna
401. Não há fallback automático para memória. `/health` continua sendo apenas
liveness, sem testar prontidão do Redis.

O serviço do Compose é acessível em `127.0.0.1:6380`, sem alterar um Redis
existente em 6379. `AUTH_REDIS_PORT` muda a porta publicada; ajuste também
`AUTH_REDIS_URL`. Acesso sem TLS é permitido somente no modo local explícito,
por URL em loopback. O Compose é uma configuração de desenvolvimento; uma API
em container deve usar uma conexão autenticada com TLS ou um adaptador de
infraestrutura apropriado, em vez de desativar a validação TLS.

O Compose usa volume próprio, AOF com `appendfsync everysec`, limite de memória
de 128 MB e `noeviction`. Isso não garante perda zero no crash do Redis nem
alta disponibilidade. Em produção, configure ACL restrita ao namespace,
credenciais externas, rede privada, TLS com certificado confiável e a política
de durabilidade exigida. Mantenha os relógios das instâncias e do Redis sincronizados: a aplicação
verifica os prazos absolutos além do TTL. Os payloads contêm identidade, CSRF, nonce e
verificador PKCE; o armazenamento e seus backups são confiáveis e sensíveis.

Perda do armazenamento invalida as sessões ausentes. Restaurar um backup antigo
pode restaurar sessões já revogadas: após recuperação com possível perda de
escritas, altere o namespace antes de reabrir acesso e exija novo login.
Não trate Redis de autenticação como cache descartável com recomposição local.

Para testes isolados ou desenvolvimento sem Redis, selecione explicitamente
`AUTH_SESSION_BACKEND=memory` com `AUTH_ALLOW_LOCAL_HTTP=true`. Nesse backend,
reiniciar a API encerra todas as sessões e cada instância tem estado próprio.
Não há logout federado, renovação silenciosa ou autenticação de serviços.

## Testes

```bash
cargo test -p obz-api -p obz-auth
cargo clippy -p obz-api -p obz-auth --all-targets -- -D warnings

# Inclui os testes de integração com Redis real:
docker compose up -d redis
AUTH_TEST_DATABASE_URL=postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget \
AUTH_TEST_RUNTIME_DATABASE_URL=postgres://zerobudget_runtime:runtime-dev-password@127.0.0.1:5440/zerobudget \
AUTH_TEST_REDIS_URL=redis://127.0.0.1:6380/0 \
cargo test -p obz-api -p obz-auth -- --include-ignored
```

Os testes HTTP iniciam um provedor OIDC temporário em loopback, com descoberta,
JWKS, tokens RSA assinados e verificação de PKCE no endpoint de troca. Verificam
login/callback/me/logout, cookies seguros, vinculação ao navegador, callback de
uso único, callbacks inválidos, CSRF, origem, expiração e limite de capacidade.
A suíte padrão não exige Docker, mas abre portas locais temporárias. Os testes
Redis são marcados como ignorados por padrão e precisam de `AUTH_TEST_REDIS_URL`
e `--include-ignored`. Cada teste usa namespace exclusivo e limpa somente suas
próprias chaves, sem `FLUSHDB`. Eles cobrem concorrência, reinicialização do
backend, capacidade global, registros inválidos, TTL, revogação e uma falha de
transporte injetada por um proxy local, sem interromper o Redis compartilhado.

Referências: [Axum](https://docs.rs/axum/0.8.9/axum/),
[OWASP: sessões](https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html),
[OWASP: CSRF](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html).

## Identidades e administração

Consulte o [guia de identidades e permissões](../../docs/identidades-e-permissoes.md)
para migrações, papéis do banco, CLI de concessão/revogação, auditoria e critérios
de validação. O PostgreSQL dedicado da aplicação é publicado em loopback na porta
5440 por padrão. Schema e privilégios devem ser preparados por um operador; a API
não migra o banco nem usa a credencial administrativa. Depois de preparado, a
indisponibilidade do banco retorna 503 nas rotas protegidas, sem fallback para
memória. Logout continua disponível com sessão/CSRF válidos, sem depender do banco.

## Envelopes

Consulte [envelopes e alocação](../../docs/envelopes.md) para as rotas de criação,
listagem, alteração e remoção, a migração e o roteiro manual. Os totais dos ciclos
agora refletem a distribuição persistida. A preparação completa usa a migração
do exemplo `migrate` do crate `obz-api`, incluindo os privilégios de envelopes.

## Documentação HTTP e Postman

O [diretório da API](../../docs/api/README.md) contém o contrato OpenAPI, coleção
Postman, ambiente local e roteiro para testar a API sem código no console.

Para a regressão completa sem cópia de cookies, use `./scripts/e2e test`.
Consulte o [guia Playwright E2E/MCP](../../docs/api/e2e.md).

## Ativação e fechamento

`POST /api/cycles/{id}/activate` exige renda totalmente alocada e confirma o
planejamento. `POST /api/cycles/{id}/close` fecha um ciclo ativo definitivamente.
Ambas exigem `budget:write` e CSRF e retornam 200 com o ciclo atualizado; repetir
a mesma transição é idempotente. Envelopes ficam congelados a partir da ativação.
Consulte [ciclo de vida](../../docs/ciclo-de-vida.md) para migração e testes.

## Despesas e estornos

Consulte [despesas manuais](../../docs/despesas.md) para registro, listagem e
estorno, saldos de envelopes e a migração `0005_expenses.sql`. A regressão isolada
usa `./scripts/e2e test expenses`. Gastos não alteram a distribuição da renda.
