# API ZeroBudget

Este diretório é a referência do contrato HTTP implementado e dos artefatos de
teste. Para regressão com login automático, use [Playwright E2E](e2e.md):

```sh
./scripts/e2e test
```

O Postman abaixo permanece disponível para exploração manual. Não é necessário copiar JavaScript para o console do navegador.
Os testes e a captura de variáveis já estão incorporados à coleção versionada.

## Arquivos

| Arquivo | Finalidade |
| --- | --- |
| [openapi.json](openapi.json) | Contrato OpenAPI 3.0.3 de saúde, autenticação, ciclos, envelopes, despesas e contas |
| [Coleção Postman](postman/zerobudget.postman_collection.json) | Operações manuais, validação de envelopes e logout separado |
| [Ambiente local](postman/local.postman_environment.json) | Base URL e parâmetros de desenvolvimento, sem sessão ou credenciais exportadas |

O OpenAPI descreve os endpoints existentes; importar a coleção pronta preserva
os testes, variáveis e limpeza que não são gerados apenas pelo contrato OpenAPI.
As páginas `/auth/session` e seus assets são auxiliares do navegador.

## Preparar a API

Na raiz do projeto, inicie os serviços:

```sh
docker compose up -d
```

Se o banco ainda não estiver preparado, um operador deve executar a migração
completa e os privilégios previstos para o runtime. Esta operação altera o banco;
não faz parte do teste Postman. O comando local usa as credenciais de desenvolvimento:

```sh
AUTH_ALLOW_LOCAL_HTTP=true \
DATABASE_ADMIN_URL='postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget' \
DATABASE_RUNTIME_ROLE=zerobudget_runtime \
cargo run -p obz-api --example migrate
```

Inicie a versão atual da API em outro processo, sem credenciais administrativas:

```sh
set -a
. ./.env.example
set +a
cargo run -p obz-api
```

A API lê o ambiente; não carrega `.env` automaticamente. Não interrompa processos
alheios para liberar portas. Se alterar a origem, ajuste também o callback
registrado no Dex e `API_PUBLIC_URL`.

## Importar no Postman

1. Use o aplicativo desktop do Postman para acessar o backend local.
2. Importe os arquivos da coleção e do ambiente pelo botão **Import**.
3. Selecione o ambiente **ZeroBudget — Local**.
4. Confira `base_url=http://localhost:3000`, sem barra final.

A coleção tem três pastas:

- **01 - Operações manuais**: envie requisições individualmente para consultar ou
  alterar dados. Não execute essa pasta inteira como roteiro automático.
- **02 - Validar envelopes (Runner)**: roteiro ordenado e repetível em um ciclo
  existente, com verificação dos saldos e limpeza dos envelopes criados.
- **03 - Encerrar sessão (manual)**: logout explícito, fora do roteiro de teste.

Não execute a coleção inteira: isso mistura alterações manuais com o roteiro e
encerra a sessão ao final. Para validação, execute apenas a pasta 02.

## Login Dex e cookie de sessão

O backend usa Authorization Code + PKCE com Dex e uma sessão opaca própria no
servidor. **Não configure bearer nem o assistente OAuth 2.0 do Postman**: tokens
emitidos pelo Dex não autenticam as rotas de negócio da API.

1. No navegador, abra `http://localhost:3000/auth/session` e entre pelo Dex.
   No ambiente de desenvolvimento atual, o login local é `admin` / `password`.
2. Sincronize o cookie do navegador com o cookie jar do Postman usando
   [Interceptor ou proxy](https://learning.postman.com/docs/use/capturing-request-data/syncing-cookies).
   Restrinja a captura ao domínio local usado no teste. A disponibilidade de
   Interceptor depende do navegador; para Firefox, use um método de captura
   compatível, ou o cadastro manual abaixo.
3. Alternativamente, consulte o cookie nas ferramentas de armazenamento do
   navegador e cadastre seu valor no gerenciador **Cookies** do Postman, para
   `localhost`, com nome `zb_session`, `Path=/` e `HttpOnly`. No HTTP local,
   `Secure` deve estar desmarcado. Use somente o valor atual da própria sessão.
4. Em HTTPS, o cookie chama-se `__Host-zb_session`, deve ter `Secure`, `Path=/`
   e não deve receber atributo Domain. Não desative TLS para usar essa sessão.
5. Envie **01 - Operações manuais → Consultar sessão**. HTTP 200 confirma que o
   cookie foi recebido. O teste salva `csrf_token` e `user_id` no ambiente.

Use sempre o mesmo host: `localhost` e `127.0.0.1` não compartilham cookies.
Não adicione um header Cookie manual junto ao cookie jar: cookies duplicados são
recusados. O cookie é HttpOnly; não é preciso nem adequado lê-lo com JavaScript.
Não copie URLs de callback ou códigos OIDC para o Postman. O login deve começar e
terminar no mesmo navegador para preservar a vinculação da tentativa.

A sessão expira em no máximo 30 minutos, limitada ao ID token. Ao receber 401,
faça novo login, atualize/sincronize o cookie e consulte a sessão novamente.
Não exporte nem compartilhe cookie jars, ambientes preenchidos, respostas de
`/api/auth/me` ou relatórios que contenham segredos. O ambiente versionado contém
CSRF vazio; ele é marcado como sensível, mas isso não substitui o cuidado ao exportar.

## Permissões

O primeiro login não concede direitos financeiros. As operações exigem:

| Tipo | Permissão |
| --- | --- |
| Listar/consultar ciclos, envelopes, despesas e contas | `budget:read` |
| Criar/ativar/fechar ciclos, alterar envelopes, registrar/estornar despesas | `budget:write` |

Use o UUID local retornado por **Consultar sessão**, não o subject do Dex. Se
necessário, o operador pode conceder os direitos para o usuário local de teste:

```sh
AUTH_ALLOW_LOCAL_HTTP=true \
DATABASE_ADMIN_URL='postgres://zerobudget_admin:app-admin-dev-password@127.0.0.1:5440/zerobudget' \
cargo run -p obz-infrastructure --example auth_admin -- grant SEU_UUID budget:read
```

Substitua `SEU_UUID` pelo UUID local. Repita com `budget:write` para permitir
mutações. Esses comandos concedem privilégios persistentes e não são executados
pela coleção. As alterações valem na próxima requisição, sem outro login.

## Preparar um ciclo

Você pode reutilizar um ciclo próprio: execute **Listar ciclos** e preencha
`cycle_id` no ambiente com o ID escolhido.

Para criar um novo ciclo, configure no ambiente:

| Variável | Exemplo | Uso |
| --- | --- | --- |
| `period_start` | `2027-01-01` | Início YYYY-MM-DD |
| `period_end` | `2027-02-01` | Fim exclusivo, posterior ao início |
| `income_cents` | `500000` | Renda de R$ 5.000,00 |

Envie **Criar ciclo planejado** uma vez. HTTP 201 salva `cycle_id` automaticamente.
Um período idêntico já existente retorna 409. Nesse caso, use **Listar ciclos** e
selecione o ID existente, ou escolha outro período deliberadamente. As datas
ficam vazias no artefato para evitar gravações de ciclos por engano.

**Não existe DELETE de ciclos.** Um ciclo criado manualmente permanece no banco.
O roteiro automático usa um ciclo existente e não altera sua renda ou período.

Para operações manuais de envelopes, `envelope_name` e `envelope_amount_cents`
controlam os corpos das requisições. A criação salva `envelope_id`, utilizado
pela alteração e remoção manuais. Essas operações persistem suas alterações.

## Executar o roteiro de envelopes

Pré-requisitos: API atualizada, cookie válido, ambiente selecionado, permissões
read/write e `cycle_id` próprio em estado `planned` com ao menos 2 centavos disponíveis. Use um ciclo
de teste sem outras edições durante a execução.

1. Selecione **02 - Validar envelopes (Runner)** e clique em **Run**.
2. Mantenha a ordem original, uma iteração, sem CSV ou concorrência entre runs.
3. Se a versão oferecer a opção, persista as alterações de variáveis do ambiente
   para que os IDs de limpeza sejam recuperáveis após uma interrupção. Desative
   a gravação de respostas sensíveis em relatórios compartilhados.
4. Confira os testes individuais e o teste final de saldo restaurado.

O roteiro consulta a sessão, captura CSRF, distribui o saldo disponível entre dois
envelopes com nomes únicos e valida:

- totais do ciclo e listagem;
- recusa de excesso de alocação e nome duplicado (409);
- valor negativo, nome vazio, campos proibidos e PATCH vazio/null (422);
- ausência de CSRF (403);
- redução, renomeação, remoção e reaplicação de valores;
- remoção dos envelopes de teste e restauração do saldo original.

As respostas de erro são verificadas pelo status e podem ser JSON ou texto,
conforme a origem da rejeição no Axum. Respostas de sucesso com corpo são JSON.
O ambiente registra os IDs de toda criação confirmada, mesmo se um teste negativo
receber 201 inesperadamente. Falhas HTTP desviam o Runner para a limpeza; falha no
login ou pré-requisitos encerra antes de criar dados. A limpeza pode pular uma
requisição vazia e repetir para os IDs pendentes, usando os recursos documentados
[skipRequest e setNextRequest](https://learning.postman.com/latest-v-12/docs/tests-and-scripts/write-scripts/postman-sandbox-reference/pm-execution).

Esse roteiro é sequencial: concorrência, isolamento entre dois proprietários,
revogação e valores acima do limite seguro do JavaScript continuam cobertos pela
suíte Rust/PostgreSQL. O Runner recusa valores monetários acima do limite inteiro
seguro do JavaScript para não fazer verificações com arredondamento.

### Recuperar uma execução interrompida

Uma falha de transporte ou cancelamento do Runner pode impedir a limpeza. Não
limpe as variáveis `test_*` nem troque `cycle_id` enquanto houver IDs pendentes.
Consulte a sessão novamente para renovar o CSRF se necessário. Execute
**PM - Limpar envelopes de teste** manualmente, uma vez por ID, ou execute apenas
as requisições de limpeza e verificação final no Runner. A requisição usa somente
IDs registrados pela execução, não faz exclusão por prefixo ou em lote do ciclo.

Se uma criação foi persistida mas a resposta se perdeu, seu ID pode não ter sido
registrado. Nesse caso, liste os envelopes e confira os nomes
`Postman-<test_run_id>-...` antes de remover manualmente pelo ID. Não remova
outros envelopes. Não reinicie o roteiro enquanto os dados pendentes não forem
reconciliados. `skipRequest` requer uma versão do Postman que suporte esse recurso;
a execução em Newman/CLI não foi validada nesta entrega.

## Encerrar a sessão

Envie o logout da pasta 03 somente quando desejar sair. HTTP 204 revoga também a
sessão sincronizada do navegador; cookies de outras sessões não são revogados.
Isso não encerra o SSO do Dex. O teste limpa CSRF e ID local do ambiente.

## Validação da entrega

O contrato foi validado como OpenAPI 3.0.3, e a coleção contra o schema oficial
Postman v2.1. As 34 requisições foram conferidas contra as rotas documentadas e
as variáveis do ambiente; os valores de sessão no arquivo exportado estão vazios.

A pasta 02 foi executada no runtime oficial `postman-runtime` contra um servidor
HTTP simulado: 33 verificações no fluxo de sucesso, com saldo restaurado. Também
foram verificadas limpeza após erro HTTP e remoção de uma criação 201 inesperada
em teste negativo. Isso valida os scripts e o encadeamento; o login Dex e a
importação pela interface desktop dependem da sessão local e não foram executados
nessa validação. Os testes de negócio da API continuam na suíte Rust/PostgreSQL.

## Manter o contrato atualizado

Ao alterar uma rota, corpo, resposta, permissão ou configuração, atualize
`openapi.json`, a coleção e este guia no mesmo trabalho. Não adicione endpoints
propostos como se já existissem. Não publique dados reais de execução.
Os testes internos da coleção são versionados; o usuário executa pelo Postman,
sem criar arquivos JavaScript ou colar código no console.

Consulte também [identidades](../identidades-e-permissoes.md),
[ciclos](../ciclos-planejados.md) e [envelopes](../envelopes.md).

## Evolução da experiência E2E

Veja o [plano com Playwright MCP e Playwright Test](plano-e2e-playwright.md) para
autenticação automática, ambiente descartável e regressão executável por comando.
A implementação e os comandos estão no [guia E2E](e2e.md).

## Ativar e fechar um orçamento

Na pasta de operações manuais, **Ativar ciclo** e **Fechar ciclo** usam `cycle_id`,
a sessão e o CSRF já capturados. Não há corpo. Ative somente depois de distribuir
toda a renda: envelopes ficam congelados na ativação e o fechamento é definitivo.
As consultas continuam funcionando. HTTP 409 indica saldo não distribuído ou
estado incompatível; repetir a mesma transição retorna 200.

O Runner de envelopes precisa de um ciclo `planned`. Use o E2E para testar o
ciclo de vida automaticamente, com dados descartáveis:

```sh
./scripts/e2e test lifecycle
```

Regras e privilégios: [ciclo de vida](../ciclo-de-vida.md).

## Despesas manuais

Após ativar um ciclo, use **Registrar despesa**, **Listar despesas** e **Estornar
despesa** na pasta manual. Defina `cycle_id`, `envelope_id`, `expense_description`,
`expense_date`, `expense_amount_cents` e `account_id` próprio; criação bem-sucedida captura `expense_id`.
A listagem de envelopes inclui os gastos e o saldo restante. O estorno preserva
o original, requer ciclo ativo e devolve saldo uma única vez.

Para regressão automática, com login e dados descartáveis:

```sh
./scripts/e2e test expenses
```

Regras, limites e persistência: [despesas e estornos](../despesas.md).

## Contas e movimentações

A pasta manual inclui criar/listar/consultar contas, registrar recebimento,
transferir entre contas próprias e consultar extrato. Defina `account_name`,
`account_kind` (`bank`/`cash`), `account_opened_on` e `opening_balance_cents`.
Criar uma conta captura `account_id`. Para transferir, selecione também
`to_account_id`, `banking_date`, `banking_description` e `banking_amount_cents`.

Novas despesas exigem `account_id`, com saldo suficiente. O estorno devolve saldo
bancário e orçamentário uma única vez. Contas são manuais: não há integração com
bancos. Recebimentos não alteram automaticamente a renda planejada.

```sh
./scripts/e2e test banking
```

Contrato, migração e limites: [contas](../contas.md).
