# Plano: E2E de API com Playwright e Playwright MCP

Status: implementação entregue. Consulte [o guia executável](e2e.md) para os
comandos e limites atuais. O texto abaixo preserva o plano que orientou a entrega.
A prova MCP é executada pelo SDK stdio; a conexão global do cliente é opcional e
requer a configuração inicial descrita no guia.

## Resultado esperado

O desenvolvedor executa um comando para preparar e testar a API. O fluxo inicia
um ambiente isolado, aplica migrações, autentica usuários pelo Keycloak real, configura
permissões no banco de teste, executa os cenários e apresenta os resultados.
Não há cópia de cookies, CSRF, códigos de callback ou IDs.

O agente usa Playwright MCP para explorar e diagnosticar o fluxo real. A suíte
versionada usa Playwright Test para execução repetível no terminal e no CI.
A presença de um agente ou de uma conversa não é pré-requisito da regressão.

## Papéis das ferramentas

| Ferramenta | Papel |
| --- | --- |
| Playwright MCP | Navegação assistida pelo agente, inspeção do login Keycloak e reprodução de falhas |
| Playwright Test | Fixtures de autenticação, requisições HTTP autenticadas, asserções e relatórios |
| Testes Rust existentes | Domínio, contratos, PostgreSQL/RLS, Redis e falhas controladas com OIDC temporário |
| OpenAPI | Referência única do contrato HTTP implementado |
| Postman | Cliente exploratório opcional; deixa de ser o caminho obrigatório do E2E |

O login é realizado no navegador automatizado. As requisições de API usam
`browserContext.request`, que compartilha o cookie jar com esse navegador.
A fixture consulta `/api/auth/me` e o cliente de testes aplica o CSRF automaticamente
nas mutações. Não haverá endpoint de login de testes, bearer alternativo, sessão
fabricada, desativação de CSRF ou concessão de permissões no login de produção.

## 1. Prova do fluxo com MCP

- Conectar o servidor oficial Playwright MCP ao agente e verificar as ferramentas
  disponíveis. Nesta conversa, ferramentas Playwright MCP ainda não estão expostas.
- Fixar a versão utilizada e documentar a configuração local, sem segredos.
- Usar contexto de navegador isolado para os usuários de teste.
- Observar os passos reais: `/api/auth/login`, formulário do Keycloak, consentimento,
  callback e chegada a `/auth/session`. Registrar seletores semânticos estáveis.
- Verificar a sessão e uma requisição autenticada com o contexto compartilhado.
- Converter o fluxo aprovado em fixtures versionadas da suíte.

Critério: o agente conclui o login real e consulta a API sem intervenção humana
nem transferência manual de cookies. Uma execução assistida é evidência do fluxo;
a regressão fica coberta pelos arquivos Playwright Test.

## 2. Ambiente E2E isolado

- Criar um Compose exclusivo, sem `container_name` fixo ou volumes de desenvolvimento.
- Cada execução recebe nome de projeto próprio, banco, Redis e dados descartáveis.
- Manter a API e o navegador inicialmente no host. Isso preserva o mesmo issuer
  acessível ao backend e ao navegador e respeita as exceções HTTP apenas em loopback.
- Proposta de portas: API 3100, Keycloak 8180, PostgreSQL da aplicação 5540 e Redis 6480.
  Validar disponibilidade; nunca parar serviços alheios para liberar portas.
- Gerar configuração Keycloak E2E com issuer `http://localhost:8180` e callback
  `http://localhost:3100/api/auth/callback`, consistentes com a origem da API.
- Incluir usuários estáticos exclusivos do Keycloak E2E. O conector atual mockPassword
  com um único admin não basta para testar isolamento entre proprietários.
- Aplicar as mesmas migrações e privilégios runtime da aplicação pelo processo
  administrativo existente. A API continua sem credencial administrativa.
- Aguardar PostgreSQL, Redis, descoberta/JWKS do Keycloak e inicialização da API.
  `/health` sozinho não estabelece prontidão de todas as dependências.
- Iniciar o binário da API no ambiente de testes e registrar apenas seus processos
  e recursos, para encerrar exatamente os que foram criados pela execução.

Critério: duas execuções consecutivas funcionam sem reaproveitar dados, sessões
ou permissões do ambiente de desenvolvimento.

## 3. Fixtures de identidade e permissões

- Criar uma fixture de usuário que abre contexto limpo, efetua login pelo Keycloak,
  recebe a sessão e consulta `/api/auth/me`.
- Preparar usuários de teste: dois proprietários com leitura/escrita, um sem
  permissões e perfis adicionais de somente leitura e somente escrita.
- Conceder os direitos explicitamente pela CLI administrativa existente, somente
  no banco E2E. Reconsultar a sessão para confirmar as permissões provisionadas.
- Começar com um worker e sessões independentes por cenário; login é automático,
  uma vez por cenário, e as chamadas HTTP seguintes reutilizam seu contexto.
- Casos de logout, bloqueio e revogação usam contexto e conta próprios, para não
  invalidar a autenticação de outros testes.
- Não reutilizar estado de autenticação entre execuções. Preferir estado em memória;
  arquivos temporários, quando necessários, ficam ignorados pelo Git.
- O cliente HTTP injeta CSRF, identifica o ator e retorna erros com contexto útil.
  Testes negativos podem deliberadamente omitir CSRF ou usar outra origem.
- Uma falha de mutação não dispara repetição automática ou novo login que possa
  esconder a falha ou duplicar escritas. Novo contexto é preparado antes do cenário.

Critério: os testes escrevem chamadas de domínio como criar ciclo/envelope sem
montar manualmente cookies ou repetir lógica de autenticação.

## 4. Cobertura inicial

| Grupo | Cenários |
| --- | --- |
| Autenticação | Login real, callback, sessão válida, logout e 401 após revogação |
| Permissões | Negação por padrão, somente leitura, somente escrita, concessão e revogação atuais |
| Segurança HTTP | Sem sessão, CSRF ausente/incorreto, origem divergente e campos de proprietário proibidos |
| Ciclos | Criação, listagem, consulta, validação e período duplicado |
| Envelopes | CRUD, nomes duplicados, valores inválidos, PATCH vazio/null e saldos calculados |
| Isolamento | Dois usuários reais; IDs de outro proprietário e envelope vinculado a outro ciclo retornam 404 |
| Concorrência | Requisições simultâneas disputam o saldo; soma final nunca supera a renda |
| Contrato | Status, tipos de conteúdo e schemas das respostas, incluindo erros em texto |

A suíte E2E verifica o sistema integrado com Keycloak, API, Redis e PostgreSQL reais.
Os testes Rust permanecem responsáveis por limites detalhados, estados impossíveis
no navegador e injeção controlada de falhas. A implementação não duplicará toda a
suíte unitária em Playwright.

Ciclos de cada cenário usam períodos próprios controlados pela fixture. Não criar
uma rota DELETE de ciclos apenas para limpeza: descartar o banco do ambiente E2E
resolve os dados persistentes sem alterar o contrato do produto.

Critério: nenhuma etapa depende de um ciclo criado manualmente, um usuário real
do desenvolvedor ou uma sequência anterior de operações no Postman.

## 5. Interface para o desenvolvedor

Comandos propostos, ainda não implementados:

```sh
./scripts/e2e setup
./scripts/e2e test
./scripts/e2e ui
./scripts/e2e clean
```

- `setup`: confere Docker, Node e dependências, instala versões fixadas e Chromium.
  Detecta dependências de sistema ausentes sem alterar a máquina silenciosamente.
- `test`: cria ambiente limpo, prepara o banco, autentica, executa e limpa recursos;
  retorna código de saída diferente de zero se algum teste falhar.
- `ui`: mantém o ambiente E2E próprio para execução e diagnóstico pela interface
  Playwright; as fixtures também fazem login automaticamente nesse modo.
- `clean`: encerra somente processos, containers e volumes identificados como
  pertencentes à execução E2E. Trata interrupção de forma previsível.

A execução local via MCP utiliza esse mesmo ambiente. Não há dependência de
Postman Cloud ou de uma sessão manual no navegador pessoal do desenvolvedor.

## 6. Relatórios e CI

- Relatórios local HTML e CI JUnit, com nome do cenário, ator e expectativa que falhou.
- Diagnósticos básicos sem cookies, CSRF, credenciais, tokens ou respostas completas
  de `/api/auth/me`. Estados de autenticação não são artefatos de CI.
- Traces completos, quando necessários, ficam opt-in, locais e ignorados no Git,
  pois podem conter dados de sessão. Não publicar automaticamente o tráfego de login.
- Rodar a suíte em job separado dos testes Rust, usando versões fixadas de Node,
  Playwright, browser e imagens. Não habilitar retry automático inicialmente.
- Usar o mesmo comando local e no CI. MCP não é requisito do job de regressão.
- Introduzir paralelismo apenas depois de provisionar usuários e dados independentes
  por worker; preservar o caso de concorrência controlada dentro do próprio teste.

Critério: uma falha é reproduzível localmente com o mesmo comando, sem depender
apenas de capturas de tela ou do histórico de uma conversa.

## Estrutura proposta

```text
tests/e2e/
  package.json
  package-lock.json
  playwright.config.ts
  fixtures/
    auth.ts
    api-client.ts
    test-data.ts
  specs/
    auth.spec.ts
    authorization.spec.ts
    cycles.spec.ts
    envelopes.spec.ts
    ownership.spec.ts
    concurrency.spec.ts
config/e2e/
  compose.yaml
  keycloak/realm.json
scripts/e2e
docs/api/e2e.md
```

TypeScript e Node ficam restritos às ferramentas de teste. Nenhuma dependência
Playwright entra nos crates de domínio, aplicação ou infraestrutura Rust.

## Ordem e conclusão

1. Prova MCP e ambiente isolado com um login real.
2. Fixture de sessão/CSRF e um cenário completo de ciclos/envelopes.
3. Segundo proprietário, perfis de permissão e casos negativos.
4. Concorrência, validação do contrato, relatórios e CI.
5. Atualizar AGENTS.md e documentação; Postman passa a opção exploratória.

Entrega concluída quando um checkout preparado executa o roteiro com um comando,
sem copiar valores, com autenticação real, dados descartáveis, limpeza verificável
e saída útil para o desenvolvedor e para o CI.

Referências primárias:
- [Playwright MCP](https://github.com/microsoft/playwright-mcp)
- [APIRequestContext e cookies compartilhados](https://playwright.dev/docs/api-testing)
- [Autenticação e isolamento de sessões](https://playwright.dev/docs/auth)
- [Execução e depuração](https://playwright.dev/docs/running-tests)
