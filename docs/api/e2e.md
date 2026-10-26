# E2E de API: um comando, login Dex automático

O caminho principal para validar o backend é a suíte Playwright. Ela usa a API
Rust, Dex, Redis e PostgreSQL reais em ambiente descartável e autentica pelo mesmo
fluxo Authorization Code + PKCE do navegador. Cookies e CSRF são compartilhados
pelo contexto Playwright, sem copiar valores ou criar um login alternativo.

## Executar

Pré-requisitos: Docker com Compose v2 ou superior em execução, Cargo/Rust e Node
22 a 24. O CI usa Node 22.23.1 e Rust 1.98.1. Não é preciso Postman, conta em um
serviço de testes, permissões financeiras no banco local ou sessão do navegador.

Na raiz do repositório:

```sh
./scripts/e2e test
```

Na primeira execução o comando instala as dependências do lockfile e o Chromium.
Depois, verifica os tipos, compila a API e as ferramentas administrativas, sobe o
ambiente, aplica migrações, executa os cenários e remove os recursos criados.
Erros de teste retornam exit code diferente de zero. Não há retry automático.

Para preparar somente as ferramentas:

```sh
./scripts/e2e setup
```

Não são instalados pacotes do sistema operacional automaticamente. Se o Chromium
informar bibliotecas ausentes, instale os pré-requisitos adequados à sua distribuição.
No Ubuntu do CI, o workflow tem uma etapa explícita `install --with-deps chromium`.
Em distribuições não suportadas oficialmente, o Playwright pode usar o build de
fallback Ubuntu; a compatibilidade depende das bibliotecas locais.

Para escolher um cenário:

```sh
./scripts/e2e test --grep 'CRUD de envelopes'
```

Para diagnosticar pela interface Playwright:

```sh
./scripts/e2e ui
```

A interface utiliza as mesmas fixtures de login e dados. O ambiente permanece
durante a execução do comando e é limpo ao sair. O modo UI exige sessão gráfica;
`test` usa Chromium headless. Não altere `--workers`: a primeira versão da suíte
usa um worker, contas e contador de períodos compartilhados por execução.
A concorrência financeira é testada explicitamente por requisições simultâneas.

## O que é automatizado

1. Cada execução recebe um projeto Compose `zb-e2e-<id>` e volume próprios.
2. O Dex recebe cinco usuários estáticos de E2E: alice, bob, denied, reader e writer.
   Suas credenciais de teste são `nome@e2e.invalid` / `e2e-password`, válidas apenas
   neste ambiente descartável. A configuração de desenvolvimento não é alterada.
3. As migrações e privilégios runtime são aplicados no banco `zerobudget_e2e`.
4. Cada cenário cria um contexto limpo, preenche o login real e confirma o consentimento.
5. `/api/auth/me` fornece o UUID local e CSRF. A CLI administrativa concede/revoga
   direitos apenas nesse banco de testes, conforme o perfil do cenário.
6. As requisições usam `context.request`, com o mesmo cookie jar. O cliente de
   testes injeta CSRF e não repete mutações após falhas de transporte.
7. Períodos dos ciclos são gerados por contador da execução, inclusive ao repetir
   casos no modo UI. Não é necessário escolher IDs ou limpar ciclos manualmente.
8. O término remove os containers, a rede, o volume e os executáveis copiados
   para aquele ambiente. Relatórios privados ficam no diretório da execução.

Login não concede permissões no código de produção. Usuários sem direitos,
bloqueio e revogação são exercitados pelo E2E; contas de cenários de bloqueio são
reabilitadas no teardown para permitir repetir os testes no UI.

## Portas e isolamento

| Serviço | Porta padrão |
| --- | --- |
| API E2E | 3100 |
| Dex E2E | 5656 |
| PostgreSQL E2E | 5540 |
| Redis E2E | 6480 |

As portas são publicadas apenas em loopback. A API e o navegador rodam no host,
com issuer e callback gerados para a mesma origem. Isso preserva a exigência atual
de TLS fora de loopback. Credenciais administrativas não são passadas à API.
O Dex E2E usa SQLite efêmero próprio; o PostgreSQL da aplicação e o Redis são reais.

Se uma porta estiver ocupada, o comando falha e não encerra o processo existente.
Para usar outro conjunto de portas:

```sh
ZB_E2E_API_PORT=3200 ZB_E2E_DEX_PORT=5756 \
ZB_E2E_POSTGRES_PORT=5640 ZB_E2E_REDIS_PORT=6580 ./scripts/e2e test
```

Cada execução usa arquivos em `tests/e2e/.runs/zb-e2e-<id>/`. O manifesto contém
marcador de propriedade e caminhos verificados; a limpeza não usa o Compose de
desenvolvimento. O PID da API é encerrado somente se `/proc/<pid>/cmdline` indicar
o executável exclusivo daquela execução. O orquestrador suporta Linux, inclusive
CI Ubuntu; execução nativa no Windows requer adaptação ou ambiente Linux/WSL.

Para recuperar recursos após interrupção abrupta:

```sh
./scripts/e2e clean
```

Esse comando remove todos os ambientes E2E registrados e ainda não encerrados
neste checkout. Não o execute durante outro teste que deseje manter ativo.
Não use `docker compose down -v` no ambiente de desenvolvimento como limpeza E2E.

## Playwright MCP

O servidor oficial está fixado no lockfile e é iniciado por um launcher do
repositório. A prova automática usa o SDK MCP via stdio e uma ferramenta do próprio
servidor para fazer login Dex e confirmar HTTP 200 com cookies compartilhados:

```sh
./scripts/e2e mcp-check
```

A prova usa um ambiente próprio, remove-o ao terminar e não registra o CSRF nem
cookies no resultado retornado. Ela confirma comunicação MCP e automação real;
não instala automaticamente uma conexão global no aplicativo do agente.

Para usar as ferramentas MCP em um agente interativo, mantenha um ambiente pronto:

```sh
./scripts/e2e up
```

O comando mostra o endereço e o caminho de `state.json`. No seu cliente MCP,
configure o servidor usando o arquivo gerado automaticamente
`tests/e2e/.cache/mcp-client.json`, que já contém o caminho absoluto deste checkout.
O [exemplo versionado](../../config/e2e/mcp-client.example.json) documenta o formato,
mas não precisa ser editado para cada execução. Isso é configuração inicial do
cliente, não uma etapa por teste.
Recarregue a conexão MCP conforme o cliente utilizado.

`./scripts/e2e mcp` executa o servidor stdio, em contexto isolado, usando o mesmo
Chromium da suíte. Ele não usa o perfil pessoal do navegador. Se houver mais de um
ambiente E2E ativo, configure `ZB_E2E_STATE` no ambiente do servidor com o caminho do
manifesto desejado. Encerre o cliente MCP antes de `./scripts/e2e clean`.

A versão fixada expõe `browser_run_code_unsafe`, capaz de executar código no
processo do servidor. Conecte somente um cliente confiável. A allowlist de origens
limita a navegação usual a API/Dex locais, mas não é uma fronteira de segurança para
código executado nessa ferramenta. Não forneça credenciais de produção ao processo.
O formato específico de configuração global pode variar entre clientes; o arquivo
exemplo não é interpretado automaticamente por todos os aplicativos.

## Cobertura e contrato

Os 16 cenários iniciais cobrem:

- login/callback/PKCE, cookies HttpOnly, sessão e logout;
- ausência de sessão e bearer arbitrário recusado;
- negação por padrão, perfis de leitura/escrita, revogação e bloqueio;
- ciclos e CRUD de envelopes com valores, nomes e períodos inválidos;
- CSRF e origem, erros JSON e texto, limites de corpo;
- dois usuários reais e tentativas de trocar o ciclo de um envelope;
- criações e alterações concorrentes disputando o saldo.

As respostas de sucesso verificadas pelo cliente usam os schemas do
[OpenAPI](openapi.json). A suíte não substitui os testes Rust de domínio, SQL/RLS,
Redis e injeção de falhas controladas. O OpenAPI não gera a lógica dos cenários;
os testes de negócio expressam as invariantes do produto.

## Relatórios e CI

Cada execução Playwright deixa um relatório HTML local privado e `junit.xml` no
seu diretório `.runs`. O JUnit contém nomes, estados e durações, sem headers, corpo
de `/api/auth/me` ou mensagens brutas de erro. Relatórios HTML, snapshots de falha e
logs locais podem conter detalhes de autenticação: não os publique nem versione.
Traces, vídeo e screenshots estão desligados. Não é salvo `storageState` de sessão.

O workflow [.github/workflows/e2e.yml](../../.github/workflows/e2e.yml) separa testes
Rust e E2E, executa a prova MCP e faz cleanup mesmo em falha. Somente o JUnit
sanitizado é publicado. O workflow foi preparado no repositório; a execução no
GitHub depende do próximo push/PR, não é realizada pelo comando local.

Para inspecionar um relatório local, utilize o CLI instalado com o caminho do
run desejado:

```sh
node tests/e2e/node_modules/@playwright/test/cli.js show-report CAMINHO_DO_RUN/report
```

Não compartilhe `state.json`, `api.log`, `services-private.log`, saídas MCP privadas
ou o diretório `.runs` inteiro. Os diretórios de dependências, browser e execução
estão no `.gitignore`.

## Organização

`tests/e2e/fixtures` concentra autenticação, permissões, cliente HTTP e dados.
`tests/e2e/specs` expressa os cenários. `runner.mjs` gerencia o ciclo de vida do
ambiente; `config/e2e` contém somente configuração de testes. Playwright e Node
não são dependências dos crates Rust.

O Postman permanece como cliente exploratório opcional. Para regressão cotidiana,
prefira `./scripts/e2e test`.
