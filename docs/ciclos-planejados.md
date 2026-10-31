# Ciclos planejados

A API cria, lista e consulta ciclos de orçamento em BRL. Ativação e fechamento
são descritos em [ciclo de vida](ciclo-de-vida.md); movimentações ainda não existem. A distribuição por envelopes é descrita em
[envelopes e alocação](envelopes.md). A renda é um inteiro de centavos
não negativo; inicialmente o valor alocado é zero e todo o saldo está disponível.
O domínio usa `Money` com aritmética verificada.

## Preparar o banco

As migrações agora ficam em `migrations/`, com um único histórico SQLx. A migração
0001 foi movida sem alterar seu conteúdo ou checksum. A 0002 cria os ciclos e suas
políticas de segurança. Faça backup antes de atualizar uma instalação com dados.

Execute como operador, com DATABASE_ADMIN_URL e DATABASE_RUNTIME_ROLE configurados:

```sh
cargo run -p obz-api --example migrate
```

Em desenvolvimento local, AUTH_ALLOW_LOCAL_HTTP=true permite a conexão sem TLS
somente ao endereço de loopback. A API continua recebendo apenas DATABASE_URL do
papel runtime e nunca executa migrações automaticamente. O novo comando concede
USAGE no schema budgeting, SELECT em cycles e INSERT somente em id, owner_id,
period_start, period_end e income_cents. A etapa de ciclo de vida acrescenta
UPDATE somente de status, sujeito a RLS e trigger; DELETE continua proibido.
O antigo auth_admin continua administrando as permissões de usuários; seu comando
migrate aplica o histórico compartilhado, mas só concede privilégios do schema auth.
Use o comando acima para preparar todos os privilégios da aplicação.

Nenhum usuário recebe permissões financeiras automaticamente. A concessão explícita
de budget:read e budget:write continua sendo feita pelo operador via auth_admin.

## Contrato HTTP

Todas as rotas exigem a sessão de usuário. POST exige também o cabeçalho
X-CSRF-Token obtido de /api/auth/me e obedece à validação de origem já existente.

| Método e rota | Permissão | Resultado |
| --- | --- | --- |
| POST /api/cycles | budget:write | 201 com ciclo planejado |
| GET /api/cycles?limit=20&offset=0 | budget:read | Lista dos próprios ciclos |
| GET /api/cycles/{uuid} | budget:read | Ciclo do usuário ou 404 |
| POST /api/cycles/{uuid}/activate | budget:write | 200 com ciclo ativo, se totalmente alocado |
| POST /api/cycles/{uuid}/close | budget:write | 200 com ciclo fechado, se ativo |

Corpo de criação:

```json
{"period_start":"2026-10-01","period_end":"2026-11-01","income_cents":500000}
```

Datas usam estritamente YYYY-MM-DD, entre os anos 0001 e 9999. O fim é exclusivo
e deve ser posterior ao início. Campos adicionais, inclusive owner_id, são
rejeitados. O proprietário vem exclusivamente da identidade autenticada.
Repetir exatamente o período para o mesmo proprietário retorna 409. Períodos
sobrepostos diferentes são permitidos nesta fase de planejamento.

A resposta inclui id, period_start, period_end, income_cents, allocated_cents,
available_cents, currency=BRL e status=planned. Na criação, alocado é zero e
disponível é igual à renda. Nas consultas, os totais refletem os envelopes. A lista é um array,
ordenado por início e UUID decrescentes; limit aceita 1 a 100 e offset até 100000.
A paginação por offset pode mudar quando outros ciclos são criados entre consultas.

Erros: 401 sem sessão; 403 sem permissão/CSRF; 404 para recurso inexistente ou de
outro proprietário; 409 período duplicado; 422 valores inválidos; 413 corpo acima
de 4096 bytes; 503 persistência indisponível. IDs e query strings malformados
seguem a rejeição 400 do Axum.

## Isolamento

Cada operação usa uma transação com contexto de proprietário local à transação,
filtro explícito por proprietário e RLS forçada no PostgreSQL. As políticas também
consultam conta habilitada e permissões atuais. O contexto não fica na conexão do
pool. Isso protege contra omissão acidental do filtro SQL; o runtime é um componente
confiável capaz de definir o contexto e suas credenciais não devem ser distribuídas
a usuários ou clientes. Papéis superusuários/BYPASSRLS não são adequados ao runtime.

Os testes de integração criam bancos descartáveis; não concedem permissões a usuários
nem alteram ciclos do banco local zerobudget. Cobrem criação com permissão apenas
de escrita, negação por padrão, CSRF, validação, duplicidade, revogação, RLS sem
filtro, tentativa de proprietário falso e reutilização da conexão.

## Organização do código

- `obz-domain`: entidades e invariantes financeiras, sem dependências externas.
- `obz-cycle`: `CycleService`, `EnvelopeService`, autorização, validação e contrato
  assíncrono `CycleRepository`. Não importa SQLx, conexão ou migrações.
- `obz-infrastructure`: `PostgresCycleRepository`, conversão das linhas SQL,
  transações com contexto RLS, verificação do schema e privilégios administrativos.
- `obz-api`: traduz HTTP e injeta o adaptador PostgreSQL em `CycleService`.

O serviço recebe o usuário local autenticado e passa somente seu UUID como
proprietário ao contrato de persistência. O adaptador mantém as verificações de
permissões atuais no banco por meio das políticas RLS. O adaptador PostgreSQL de identidade e sua ferramenta administrativa também ficam
em `infrastructure`. O crate `auth` mantém seus contratos e não depende de SQLx.
