# Operação do piloto

O backend dispõe de artefatos de implantação. A publicação depende de domínio,
provedores e segredos do ambiente; este repositório não publica serviços sozinho.
Não use credenciais do Docker Compose de desenvolvimento em produção.

## Implantação

1. Configure PostgreSQL 18 com TLS e dois papéis: proprietário de migrações e
   `zerobudget_runtime` sem SUPERUSER/BYPASSRLS/CREATEDB/CREATEROLE. Não dê ownership
   de tabelas ao runtime. Configure Redis autenticado com TLS e namespace exclusivo.
2. Configure Keycloak por HTTPS, cliente confidencial e callback exato
   `https://SEU_DOMINIO/api/auth/callback`. O navegador e o backend precisam
   alcançar o mesmo issuer público. Defina a política de contas/MFA no provedor.
3. Copie `deploy/production.env.example` para `deploy/.env.production`, preencha
   URLs/segredos e aplique `chmod 600`. Certificados privados devem ser montados
   em modo somente leitura; use `sslrootcert` na URL do PostgreSQL se necessário.
4. Execute `docker build -t zerobudget:pilot .`. A imagem executa como UID 10001 e
   inclui `zb-migrate` e `zb-auth-admin`. O servidor nunca executa migrações.
5. Com configuração administrativa somente na sessão do operador, execute
   `cargo run --locked -p obz-api --example migrate`, definindo `DATABASE_ADMIN_URL`
   e `DATABASE_RUNTIME_ROLE=zerobudget_runtime`. Em produção deixe
   `AUTH_ALLOW_LOCAL_HTTP=false`. O mesmo comando pode ser executado substituindo
   o entrypoint da imagem por `zb-migrate`. Nunca coloque DATABASE_ADMIN_URL no
   serviço HTTP. O operador deve revisar as migrações 0008 e 0009 e os privilégios
   de INSERT nas novas tabelas, SELECT nas views e colunas de metadados dos ciclos.
6. Defina `API_DOMAIN` e execute `docker compose -f deploy/compose.yaml up -d --build`.
   Caddy termina TLS; apenas ele publica portas. Provisione DNS e acesso ACME nas
   portas 80/443. Bancos e Keycloak são serviços externos ao template.
7. Verifique `/health` (204: processo vivo) e `/ready` (204: PostgreSQL, sessões e Keycloak
   disponíveis; 503 se uma dependência falhar). Faça login real e uma operação
   financeira controlada na conta do piloto antes de liberar uso.

As requisições geram JSON no stderr com request_id, método, rota parametrizada,
status e duração. A resposta contém X-Request-Id. Não são registrados query string,
corpo, identidade, cookies ou tokens. Agregue os logs e monitore 5xx, readiness e
latência. Redis indisponível não produz fallback para memória. Todas as requisições autenticadas dependem de introspecção online no Keycloak;
readiness consulta também seu endpoint JWKS. Falha do provedor resulta em 503.

## Acesso ao piloto

O participante faz login em `/auth/session`, obtendo `user_id` em `/api/auth/me`.
Confirme a identidade junto ao participante antes de conceder acesso. Login continua
sem permissões financeiras por padrão. Com DATABASE_ADMIN_URL apenas no terminal
do operador:

```sh
cargo run --locked -p obz-infrastructure --example auth_admin -- grant UUID budget:read
cargo run --locked -p obz-infrastructure --example auth_admin -- grant UUID budget:write
```

A CLI registra auditoria. Para revogar, use `revoke UUID budget:write` e/ou
`revoke UUID budget:read`; `disable UUID` bloqueia a conta local. A próxima
requisição consulta os privilégios atuais. `enable UUID` reabilita explicitamente.
Nenhuma interface administrativa ou concessão automática foi adicionada.

## Backup e recuperação

Use cliente PostgreSQL 18, PGHOST/PGPORT/PGDATABASE/PGUSER e PGPASSFILE protegido
(ou PGPASSWORD na sessão do operador), além de PGSSLMODE=verify-full em produção.
Não coloque senha na linha de comando. O usuário do backup precisa ler todas as
linhas apesar da RLS (proprietário com BYPASSRLS ou operador autorizado). O runtime
não serve para backup completo. A API deve continuar sem esses privilégios.

```sh
scripts/db-backup /diretorio-protegido/zerobudget-2026-10-31.dump
# Provisione um banco NOVO e vazio, sem writers; ajuste PGDATABASE.
scripts/db-restore /diretorio-protegido/zerobudget-2026-10-31.dump NOME_DO_BANCO_NOVO
```

Backup é snapshot consistente, arquivo 0600, publicação atômica sem sobrescrita.
Restauração exige confirmação exata do banco e recusa schemas/tabelas existentes;
executa em transação única. Não restaura ownership/grants do ambiente de origem.
Revoga EXECUTE público nas funções restauradas; depois execute `zb-migrate` com o
papel runtime para validar checksums e reaplicar privilégios mínimos. Verifique
contagens/saldos, `/ready`, login e isolamento antes de apontar o serviço ao banco.
Não execute restaurações concorrentes nem abra writers no destino durante esse processo.

Mantenha cópia cifrada fora do host e uma política explícita de retenção para o
piloto, por exemplo diário por 14 dias e semanal por 8 semanas. Teste recuperação
regularmente; pg_dump não inclui papéis globais, configuração do Keycloak, Redis ou
segredos. Provisione esses itens separadamente. Perder Redis encerra sessões e
login pendente; o usuário pode entrar novamente, sem perda de lançamentos.
RPO depende da frequência do backup; PITR/WAL deve ser configurado no provedor
quando necessário. RTO deve ser medido no volume real do piloto.

A regressão `tests/operations/backup_restore.py` usa exclusivamente o banco E2E
em loopback e cria/remove seu próprio destino. Com o ambiente de `scripts/e2e up`
ativo, use `./scripts/e2e verify` para automatizar ambiente, suíte e recuperação.
Para execução isolada, configure AUTH_TEST_DATABASE_URL, AUTH_TEST_RUNTIME_DATABASE_URL
e ZB_MIGRATE_BIN (binário migrate compilado com as mesmas migrações do banco E2E). Ela compara o conteúdo de todas
as tabelas, inclusive histórico e migrações, testa recusas e verifica ACL de funções.
