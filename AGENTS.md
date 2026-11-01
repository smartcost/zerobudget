# Diretrizes do ZeroBudget

Estas diretrizes se aplicam a todo o repositório. Instruções explícitas do usuário
têm precedência. Arquivos `AGENTS.md` em subdiretórios podem detalhar regras locais.

## Produto e escopo

O ZeroBudget é uma aplicação Rust de gestão financeira pessoal com orçamento base
zero: a renda deve ser distribuída entre finalidades explícitas. Desenvolva em
etapas pequenas, com regras de negócio verificáveis e isolamento entre usuários.
Não amplie a etapa em andamento para funcionalidades futuras sem necessidade.

Atualmente há autenticação de usuários pelo Keycloak, sessões no servidor, permissões
locais, ciclos planejados/ativos/fechados, envelopes, despesas com estorno, contas
manuais, recebimentos, transferências próprias, cartões e faturas manuais, realocações, compensações, dívida inicial e continuidade entre ciclos.
Autenticação entre serviços, integração bancária, parcelamento e juros não estão implementados. Não trate
propostas de documentação como funcionalidades entregues.

## Arquitetura e dependências

| Crate | Responsabilidade |
| --- | --- |
| `obz-domain` | Entidades, valores e invariantes financeiras; zero dependências externas |
| `obz-cycle` | Casos de uso de ciclos, envelopes e despesas; autorização e contratos de persistência |
| `obz-banking` | Casos de uso e contratos de contas, cartões e faturas; independente de ciclos |
| `obz-auth` | OIDC, autorização, contratos de identidade e sessões; adaptadores de sessão existentes |
| `obz-infrastructure` | Conexão PostgreSQL, migrações e implementações dos repositórios |
| `obz-api` | Transporte HTTP, configuração e composição das dependências |

- O domínio não conhece HTTP, SQLx, Redis, Keycloak ou variáveis de ambiente.
- Casos de uso dependem de contratos de persistência, nunca de implementações
  PostgreSQL. Não coloque SQL, conversão de linhas ou privilégios nesses crates.
- `infrastructure` implementa os contratos de `auth`, `cycle` e `banking`; esses crates não
  dependem de `infrastructure`. A API injeta as implementações.
- O crate `shared` foi removido. Não recrie um agrupamento genérico para esconder
  responsabilidades de infraestrutura ou introduzir dependências circulares.
- Preserve a separação ao adicionar funcionalidades; não refatore subsistemas
  independentes apenas para uniformizar o código.

## Regras financeiras

- Represente valores persistidos e de API em centavos inteiros. Não use ponto
  flutuante para cálculos financeiros; apresentação monetária é uma conversão.
- Valide valores e períodos no domínio, além da validação de formato no HTTP.
- Use aritmética verificada quando somar ou subtrair valores. Use os métodos
  `Money::checked_add` e `Money::checked_sub`; seus valores internos são privados.
- Ciclos planejados começam sem alocação. Depois, o disponível é a renda menos
  a soma dos envelopes. A soma não pode exceder a renda, inclusive sob concorrência.
  A ativação exige saldo zero e congela o planejamento; somente ciclos ativos
  podem ser fechados, sem reabertura. As transições são idempotentes e usam o
  mesmo bloqueio por ciclo das mutações de envelopes.
  Despesas exigem ciclo ativo, data dentro do período e saldo no envelope.
  Lançamentos e estornos são imutáveis; estorno repetido não devolve saldo novamente.
  Gastos não alteram a alocação. Use o mesmo lock por ciclo para gastos, estornos
  e fechamento.
  Novas despesas exigem exatamente uma origem: conta própria para débito imediato
  ou fatura própria aberta para compra no crédito. Ambas consomem o envelope.
  Contas não podem ficar negativas nem exceder i64::MAX. Transferências são uma
  única operação entre contas próprias; não geram receita/despesa no orçamento.
  Recebimentos não alteram a renda planejada automaticamente. O histórico anterior
  à migração bancária pode ter account_id nulo e não produz lançamentos fictícios.
  Limite de crédito é compartilhado entre todas as faturas do cartão. Pagamentos
  de fatura fechada debitam conta e reduzem dívida sem consumir envelope novamente;
  request_id torna o pagamento idempotente, com conflito para dados diferentes.
  Estornos de compra só alteram faturas abertas; fechamento da fatura é definitivo.
  Preserve a ordem dos locks: ciclo primeiro, proprietário bancário depois.
  Realocações ativas só movem saldo não gasto; alocação efetiva é base + entradas − saídas.
  Compensações são integrais, append-only e idempotentes; não apagam movimentos.
  Devolução de compra fechada vira crédito em fatura aberta, preservando ciclo histórico.
  Crédito de cartão não é caixa. Dívida inicial não é despesa nem receita fictícia.
  Próximo ciclo exige origem fechada e sucessor único; transporte não cria recebimento.
  Despesas, recebimentos e transferências HTTP exigem request_id preservado em retries.
  O fim do período é exclusivo. Mudar esses contratos exige regra explícita e testes.

## Autenticação, autorização e isolamento

- Preserve Authorization Code com PKCE, validação de `state`, `nonce`, assinatura,
  issuer, audience e expiração. Identidades devem vir do fluxo OIDC verificado.
- Não aceite tokens do Keycloak como bearer das rotas de negócio nem exponha tokens,
  verificadores PKCE, cookies ou segredos em respostas, logs e commits.
- Cadastro e recuperação pertencem ao Keycloak; exija e-mail verificado no callback.
  Tokens do provedor ficam somente no armazenamento privado do servidor. Faça
  introspecção online em requisições autenticadas e falhe sem fallback. A imagem
  Keycloak inclui política obrigatória de revogação de outras sessões ao trocar senha.
- Negue permissões financeiras por padrão. Login não concede direitos financeiros.
  Consulte estado da conta e permissões atuais nas requisições protegidas.
- O proprietário vem da identidade autenticada, nunca do corpo da requisição.
  Aplique autorização no caso de uso e isolamento também na persistência.
- Preserve filtros por proprietário e RLS forçada. Contexto de usuário no banco
  deve ficar restrito à transação, sem vazar para conexões reutilizadas do pool.
- Operações HTTP que alteram dados exigem sessão e CSRF, com a validação de origem
  existente. Não desabilite CSP, TLS ou proteções do navegador para facilitar testes.
- Falhas de persistência devem falhar de forma fechada; não confunda indisponibilidade
  com sessão ausente e não introduza fallback automático para memória.

## Persistência e desenvolvimento local

- Migrações SQLx ficam em `migrations/`, com um único histórico. Nunca edite uma
  migração já aplicada; crie uma nova para evoluir o schema.
- Migrações e concessões administrativas são operações explícitas de operador.
  A API não migra o banco e usa somente a credencial runtime com privilégio mínimo.
- A CLI de identidades pertence a `obz-infrastructure`:
  `cargo run -p obz-infrastructure --example auth_admin -- <operação>`.
  A preparação completa do banco usa `cargo run -p obz-api --example migrate`.
- Não conceda permissões a usuários nem amplie privilégios persistentes fora do
  escopo autorizado. Não remova volumes ou dados para resolver falhas de testes.
- O banco da aplicação é separado do banco interno do Keycloak. No Compose local,
  Keycloak usa a porta 8080, PostgreSQL da aplicação 5440 e Redis 6380. Preserve serviços
  de outros projetos e processos locais que não foram iniciados para a tarefa.
- A API lê o ambiente, sem carregar `.env` automaticamente. `.env.example` contém
  configuração de desenvolvimento; não versione `.env` ou credenciais reais.
  Credenciais administrativas não devem ser exportadas ao processo da API.

## Qualidade e validação

Execute na raiz, conforme o alcance da mudança:

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

- Testes HTTP usam um provedor OIDC temporário e abrem portas em loopback.
- A regressão E2E usa `./scripts/e2e test`: Keycloak real, navegador e API com serviços
  descartáveis exclusivos. O guia é `docs/api/e2e.md`. Não use o banco local para
  preparar usuários E2E; não faça bypass de login ou CSRF. Playwright MCP usa
  `./scripts/e2e up` e o launcher `./scripts/e2e mcp`; a prova é `mcp-check`.
- Não publique `.runs`, estados de autenticação ou relatórios HTML privados. No CI,
  publique somente o JUnit sanitizado.
- Testes ignorados de integração exigem Redis/PostgreSQL locais e as variáveis
  `AUTH_TEST_DATABASE_URL`, `AUTH_TEST_RUNTIME_DATABASE_URL` e
  `AUTH_TEST_REDIS_URL`. Execute-os com
  `cargo test --workspace -- --include-ignored --test-threads=2` após configurar o ambiente.
- Limite a concorrência das integrações a dois testes para evitar checkpoints
  simultâneos de criação/remoção de bancos. Os cenários mantêm concorrência interna.
- Use bancos descartáveis e namespaces Redis exclusivos. Nunca faça `FLUSHDB`,
  migre o banco de desenvolvimento ou conceda direitos a usuários reais em testes.
- Cubra invariantes e falhas relevantes: valores inválidos, negação por padrão,
  revogação, CSRF, isolamento por proprietário, transações e indisponibilidade.
- Use repositórios falsos para casos de uso e o PostgreSQL real para SQL e RLS.
  Não substitua testes de isolamento por mocks que apenas repetem a implementação.
- Para mudanças somente de documentação, revise conteúdo, comandos e diff;
  executar toda a suíte não é necessário. Informe limitações de validação.

## Colaboração e documentação

- Comunique-se em português e siga o estilo de nomes do código existente.
- Preserve alterações do usuário. Evite mudanças amplas, novas dependências e
  abstrações sem necessidade concreta para a tarefa.
- O contrato HTTP e os artefatos Postman ficam em `docs/api/`. Ao alterar rotas,
  mantenha `openapi.json`, coleção e ambiente consistentes, sem dados de sessão.
- Atualize documentação quando alterar contratos HTTP, comandos, configuração ou
  responsabilidades dos crates. Exemplos devem indicar claramente os parâmetros
  a substituir; não apresente `...` como um comando pronto para executar.
- Use o código e os guias de funcionalidades como referência do estado atual.
  `docs/ARQUITETURA_BACKEND_RUST.md` inclui propostas históricas; confirme sua
  aplicabilidade antes de implementá-las.
- Ao concluir, informe o que mudou, como foi validado e o que permanece pendente.
  Faça commits quando solicitados; não envie ao remoto sem autorização.

Referências: [API](crates/api/README.md), [autenticação](crates/auth/README.md),
[identidades e permissões](docs/identidades-e-permissoes.md) e
[ciclos planejados](docs/ciclos-planejados.md), [envelopes](docs/envelopes.md) e
[despesas e estornos](docs/despesas.md) e [contas](docs/contas.md) e [cartões](docs/cartoes.md).
